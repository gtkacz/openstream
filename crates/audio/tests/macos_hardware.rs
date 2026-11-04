//! Gated checks of ScreenCaptureKit audio on a real Mac: `BRP_HARDWARE_TESTS=1 cargo test -p
//! brp-audio --test macos_hardware -- --nocapture --test-threads 1`. They play a system sound with
//! `afplay` and need the Screen & System Audio Recording permission for the app that runs them.
#![cfg(target_os = "macos")]

use std::collections::BTreeSet;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brp_audio::{AppKey, AudioCapture, AudioChunk, AudioSelection, PlatformAudioCapture};

const SOUND: &str = "/System/Library/Sounds/Submarine.aiff";

fn gated() -> bool {
    std::env::var_os("BRP_HARDWARE_TESTS").is_some()
}

/// Plays the sound over and over until dropped.
struct Player(Child);

impl Player {
    fn start() -> Self {
        Self(
            Command::new("/bin/sh")
                .args(["-c", &format!("while true; do afplay {SOUND}; done")])
                .spawn()
                .expect("afplay"),
        )
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = Command::new("pkill")
            .args(["-f", &format!("afplay {SOUND}")])
            .status();
        let _ = self.0.kill();
    }
}

/// Captures `selection` for two seconds and returns the samples and the loudest one.
fn capture(selection: AudioSelection) -> (usize, f32) {
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let sink = {
        let samples = samples.clone();
        Box::new(move |chunk: AudioChunk| samples.lock().unwrap().extend(chunk.samples))
    };
    let backend = PlatformAudioCapture::new(std::process::id());
    let session = backend.start(selection, sink).expect("audio capture start");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(session.error(), None, "the stream died");
    session.stop();
    let samples = samples.lock().unwrap();
    let peak = samples.iter().fold(0f32, |peak, s| peak.max(s.abs()));
    (samples.len(), peak)
}

#[test]
fn the_audible_list_never_includes_openstream() {
    if !gated() {
        return;
    }
    let _player = Player::start();
    std::thread::sleep(Duration::from_millis(500));
    let sources = PlatformAudioCapture::new(std::process::id())
        .sources()
        .expect("source list");
    eprintln!("audible: {sources:#?}");
    let own = AppKey::new(brp_proto::constants::MACOS_BUNDLE_ID);
    assert!(sources.iter().all(|source| source.key != own));
}

#[test]
fn everything_captures_what_the_machine_plays_as_stereo() {
    if !gated() {
        return;
    }
    let _player = Player::start();
    let (len, peak) = capture(AudioSelection::All);
    eprintln!("All: {len} samples, peak {peak}");
    assert!(
        len > 48_000 * 2,
        "under a second of stereo in two seconds: {len}"
    );
    assert_eq!(len % 2, 0, "interleaved stereo");
    assert!(
        peak > 0.001,
        "the sound being played was not captured (peak {peak})"
    );
}

#[test]
fn only_an_application_that_is_not_running_is_silence() {
    if !gated() {
        return;
    }
    let _player = Player::start();
    let nobody = AudioSelection::Only(BTreeSet::from([AppKey::new("com.example.not-running")]));
    let (len, peak) = capture(nobody);
    eprintln!("Only(not running): {len} samples, peak {peak}");
    assert!(
        peak < 0.001,
        "audio from an unselected application leaked (peak {peak})"
    );
}
