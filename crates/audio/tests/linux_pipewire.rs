//! Runs the PipeWire backend against the machine's real daemon. Gated, because it needs a session
//! with PipeWire and `pw-cat`. It plays into a throwaway null sink, so nothing reaches the speakers.
//!
//! ```text
//! BRP_AUDIO_TESTS=1 cargo test -p brp-audio --test linux_pipewire -- --nocapture --test-threads=1
//! ```
#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use brp_audio::{AppKey, AudioCapture, AudioSelection, PlatformAudioCapture};

const NULL_SINK: &str = "brp-test-null-sink";
const AMPLITUDE: f32 = 0.5;
const SAMPLE_RATE: u32 = 48_000;
const SETTLE: Duration = Duration::from_millis(1500);

fn gated() -> bool {
    std::env::var_os("BRP_AUDIO_TESTS").is_some()
}

/// Kills a child even when an assertion fails first.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A 48 kHz stereo float WAV of a loud-enough sine.
fn write_tone(path: &Path, seconds: u32) {
    let frames = SAMPLE_RATE * seconds;
    let data_bytes = frames * 2 * 4;
    let mut bytes = Vec::with_capacity(data_bytes as usize + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 8).to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for frame in 0..frames {
        let sample = AMPLITUDE * (TAU * 440.0 * frame as f32 / SAMPLE_RATE as f32).sin();
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

/// A null sink that lives as long as the `pw-cli` holding it.
fn null_sink() -> Running {
    let mut child = Command::new("pw-cli")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.as_mut().unwrap(),
        "create-node adapter factory.name=support.null-audio-sink media.class=Audio/Sink \
         node.name={NULL_SINK} audio.position=[FL,FR] monitor.channel-volumes=false"
    )
    .unwrap();
    thread::sleep(Duration::from_millis(500));
    Running(child)
}

/// `pw-cat` under another name, so it has an application identity of its own.
fn player(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy("/usr/bin/pw-cat", &path).unwrap();
    path
}

fn play(binary: &Path, tone: &Path) -> Running {
    Running(
        Command::new(binary)
            .args(["-p", "--target", NULL_SINK])
            .arg(tone)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The level of what the capture receives over the next while, after giving links time to settle.
fn level(received: &Mutex<Vec<f32>>) -> f32 {
    thread::sleep(SETTLE);
    received.lock().unwrap().clear();
    thread::sleep(SETTLE);
    rms(&received.lock().unwrap())
}

fn close_to(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= expected.max(0.02) * 0.15
}

struct Fixture {
    dir: PathBuf,
    tone: PathBuf,
    _sink: Running,
}

fn fixture(test: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("brp-audio-{test}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tone = dir.join("tone.wav");
    write_tone(&tone, 30);
    Fixture {
        dir,
        tone,
        _sink: null_sink(),
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn each_application_reaches_the_room_at_its_own_volume_and_changes_apply_live() {
    if !gated() {
        return;
    }
    let fixture = fixture("volumes");
    let (loud, quiet) = (
        player(&fixture.dir, "brp-tone-loud"),
        player(&fixture.dir, "brp-tone-quiet"),
    );
    let capture = PlatformAudioCapture::new(std::process::id());
    let quiet_key = AppKey::new("brp-tone-quiet");
    capture.set_send_volumes(BTreeMap::from([(quiet_key.clone(), 25)]));

    let received = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = received.clone();
    let selection = AudioSelection::Only(BTreeSet::from([
        AppKey::new("brp-tone-loud"),
        quiet_key.clone(),
    ]));
    let session = capture
        .start(
            selection,
            Box::new(move |chunk| sink.lock().unwrap().extend(chunk.samples)),
        )
        .unwrap();

    let full = AMPLITUDE / 2f32.sqrt();

    let playing = play(&loud, &fixture.tone);
    let alone = level(&received);
    eprintln!("loud alone at 100%: {alone:.4} (expected {full:.4})");
    assert!(close_to(alone, full), "{alone}");
    drop(playing);

    let _playing = play(&quiet, &fixture.tone);
    let turned_down = level(&received);
    eprintln!("quiet alone at 25%: {turned_down:.4}");
    assert!(close_to(turned_down, full * 0.25), "{turned_down}");

    capture.set_send_volumes(BTreeMap::from([(quiet_key.clone(), 100)]));
    let turned_up = level(&received);
    eprintln!("same stream, now at 100%: {turned_up:.4}");
    assert!(close_to(turned_up, full), "{turned_up}");

    capture.set_send_volumes(BTreeMap::from([(quiet_key, 0)]));
    let silenced = level(&received);
    eprintln!("same stream, now at 0%: {silenced:.4}");
    assert!(silenced < 0.001, "{silenced}");

    session.stop();
}

/// Everything else playing on the machine is captured too under `All`, so this asks for silence on
/// the desktop while it runs and only prints when it does not get it.
#[test]
fn under_all_an_application_still_gets_its_own_volume() {
    if !gated() {
        return;
    }
    let fixture = fixture("all");
    let binary = player(&fixture.dir, "brp-tone-all");
    let capture = PlatformAudioCapture::new(std::process::id());
    let key = AppKey::new("brp-tone-all");
    let received = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = received.clone();
    let session = capture
        .start(
            AudioSelection::All,
            Box::new(move |chunk| sink.lock().unwrap().extend(chunk.samples)),
        )
        .unwrap();
    let background = level(&received);
    let _playing = play(&binary, &fixture.tone);
    let at_full = level(&received);
    capture.set_send_volumes(BTreeMap::from([(key, 25)]));
    let at_quarter = level(&received);
    eprintln!("background {background:.4}, full {at_full:.4}, quarter {at_quarter:.4}");
    if background > 0.001 {
        eprintln!("something else is playing; not asserting levels");
    } else {
        let full = AMPLITUDE / 2f32.sqrt();
        assert!(close_to(at_full, full), "{at_full}");
        assert!(close_to(at_quarter, full * 0.25), "{at_quarter}");
    }
    session.stop();
}

#[test]
fn an_application_outside_the_selection_is_not_heard_at_any_volume() {
    if !gated() {
        return;
    }
    let fixture = fixture("selection");
    let other = player(&fixture.dir, "brp-tone-other");
    let capture = PlatformAudioCapture::new(std::process::id());
    let received = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = received.clone();
    let session = capture
        .start(
            AudioSelection::Only(BTreeSet::from([AppKey::new("brp-tone-chosen")])),
            Box::new(move |chunk| sink.lock().unwrap().extend(chunk.samples)),
        )
        .unwrap();
    let _playing = play(&other, &fixture.tone);
    let heard = level(&received);
    eprintln!("unselected application: {heard:.4}");
    assert!(heard < 0.001, "{heard}");
    session.stop();
}

#[test]
fn the_listing_follows_an_application_starting_and_stopping() {
    if !gated() {
        return;
    }
    let fixture = fixture("listing");
    let binary = player(&fixture.dir, "brp-tone-listed");
    let capture = PlatformAudioCapture::new(std::process::id());
    let key = AppKey::new("brp-tone-listed");
    let listed = || {
        capture
            .sources()
            .unwrap()
            .iter()
            .any(|source| source.key == key)
    };
    let eventually = |wanted: bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if listed() == wanted {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    };
    assert!(!listed());
    let playing = play(&binary, &fixture.tone);
    assert!(eventually(true), "a started application appears");
    drop(playing);
    assert!(eventually(false), "a stopped application disappears");
}
