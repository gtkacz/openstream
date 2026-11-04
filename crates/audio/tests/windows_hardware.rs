//! Gated checks of WASAPI process loopback on a real Windows machine: `$env:BRP_HARDWARE_TESTS=1;
//! cargo test -p brp-audio --test windows_hardware -- --nocapture --test-threads 1`. They play a
//! tone through PowerShell, so a default output device is needed and the speakers will sound.
#![cfg(windows)]

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::TAU;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use brp_audio::{AppKey, AudioCapture, AudioSelection, PlatformAudioCapture, SendVolumeSupport};

const PLAYER: &str = "powershell.exe";
const SAMPLE_RATE: u32 = 48_000;
const SETTLE: Duration = Duration::from_secs(3);

fn gated() -> bool {
    std::env::var_os("BRP_HARDWARE_TESTS").is_some()
}

/// A 16-bit stereo WAV of a sine, which is what `Media.SoundPlayer` accepts.
fn write_tone(path: &Path, seconds: u32) {
    let frames = SAMPLE_RATE * seconds;
    let data_bytes = frames * 4;
    let mut bytes = Vec::with_capacity(data_bytes as usize + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for frame in 0..frames {
        let phase = TAU * 440.0 * frame as f32 / SAMPLE_RATE as f32;
        let sample = (phase.sin() * f32::from(i16::MAX) * 0.25) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

/// Plays the tone until dropped.
struct Player(Child);

impl Player {
    fn start(tone: &Path) -> Self {
        let script = format!(
            "(New-Object Media.SoundPlayer '{}').PlaySync()",
            tone.display()
        );
        Self(
            Command::new(PLAYER)
                .args(["-NoProfile", "-Command", &script])
                .spawn()
                .expect("powershell"),
        )
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    dir: PathBuf,
    tone: PathBuf,
}

fn fixture(test: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("brp-audio-{test}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let tone = dir.join("tone.wav");
    write_tone(&tone, 40);
    Fixture { dir, tone }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The level of what the capture receives over the next while.
fn level(received: &Mutex<Vec<f32>>) -> f32 {
    received.lock().unwrap().clear();
    thread::sleep(SETTLE);
    rms(&received.lock().unwrap())
}

/// The session's new-client poll is a second, so the player's client needs a moment to appear.
fn capture(
    selection: AudioSelection,
) -> (
    PlatformAudioCapture,
    Box<dyn brp_audio::AudioCaptureSession>,
    Arc<Mutex<Vec<f32>>>,
) {
    let backend = PlatformAudioCapture::new(std::process::id());
    let received = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = received.clone();
    let session = backend
        .start(
            selection,
            Box::new(move |chunk| sink.lock().unwrap().extend(chunk.samples)),
        )
        .expect("audio capture start");
    (backend, session, received)
}

#[test]
fn a_playing_application_is_listed_and_openstream_never_is() {
    if !gated() {
        return;
    }
    let fixture = fixture("listing");
    let backend = PlatformAudioCapture::new(std::process::id());
    let key = AppKey::new(PLAYER);
    let listed = || backend.sources().unwrap().iter().any(|s| s.key == key);

    let player = Player::start(&fixture.tone);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !listed() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
    }
    let sources = backend.sources().unwrap();
    eprintln!("audible: {sources:#?}");
    assert!(sources.iter().any(|s| s.key == key), "the player appears");
    let own = std::env::current_exe().unwrap();
    let own = AppKey::new(&own.to_string_lossy());
    assert!(sources.iter().all(|s| s.key != own), "never ourselves");
    drop(player);
}

#[test]
fn everything_is_captured_under_all() {
    if !gated() {
        return;
    }
    let fixture = fixture("all");
    let (_backend, session, received) = capture(AudioSelection::All);
    let _player = Player::start(&fixture.tone);
    thread::sleep(SETTLE);
    let heard = level(&received);
    eprintln!("under all: {heard:.4}");
    assert_eq!(session.error(), None, "the stream died");
    assert!(heard > 0.01, "{heard}");
    session.stop();
}

#[test]
fn a_selected_application_reaches_the_room_at_its_own_volume_and_changes_apply_live() {
    if !gated() {
        return;
    }
    let fixture = fixture("volumes");
    let key = AppKey::new(PLAYER);
    let (backend, session, received) = capture(AudioSelection::Only(BTreeSet::from([key.clone()])));
    assert_eq!(
        backend.send_volume_support(),
        SendVolumeSupport::OnlySelected
    );
    let _player = Player::start(&fixture.tone);
    thread::sleep(SETTLE);

    let full = level(&received);
    eprintln!("at 100%: {full:.4}");
    assert!(full > 0.01, "the selected application is heard: {full}");

    backend.set_send_volumes(BTreeMap::from([(key.clone(), 25)]));
    let quarter = level(&received);
    eprintln!("at 25%: {quarter:.4}");
    assert!((quarter / full - 0.25).abs() < 0.05, "{quarter} of {full}");

    backend.set_send_volumes(BTreeMap::from([(key, 0)]));
    let silent = level(&received);
    eprintln!("at 0%: {silent:.4}");
    assert!(silent < 0.001, "{silent}");

    assert_eq!(session.error(), None, "the stream died");
    session.stop();
}

#[test]
fn an_application_outside_the_selection_is_not_heard() {
    if !gated() {
        return;
    }
    let fixture = fixture("selection");
    let (_backend, session, received) =
        capture(AudioSelection::Only(BTreeSet::from([AppKey::new(
            "not-running.exe",
        )])));
    let _player = Player::start(&fixture.tone);
    thread::sleep(SETTLE);
    let heard = level(&received);
    eprintln!("unselected: {heard:.4}");
    assert!(heard < 0.001, "{heard}");
    session.stop();
}
