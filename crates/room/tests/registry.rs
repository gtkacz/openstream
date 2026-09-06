use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use brp_audio::{AudioCapture, AudioCaptureSession, AudioError, AudioSink, SyntheticTone};
use brp_capture::{CaptureBackend, CaptureSession, SourceInfo, SourceRequest, SyntheticSource};
use brp_net::{LiveSource, SubscribeRejected};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, SOURCE_PRESET_ID};
use brp_proto::{Codec, SourceKind, template_presets};
use brp_room::AudioCaptureState;
use brp_room::codecs::fake::FakeCodecs;
use brp_room::registry::{CaptureFan, LiveRegistry};

const GRACE: Duration = Duration::from_millis(300);

fn registry(grace: Duration) -> Arc<LiveRegistry> {
    LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        grace,
        Arc::new(|| {}),
    )
}

struct DummySession;

impl CaptureSession for DummySession {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            width: 64,
            height: 32,
            fps: 30,
        }
    }
    fn stop(self: Box<Self>) {}
}

async fn synthetic_live(registry: &LiveRegistry, title: &str) -> u32 {
    let fan = Arc::new(CaptureFan::default());
    let sink = fan.clone();
    let session = SyntheticSource {
        width: 64,
        height: 32,
        fps: 60,
    }
    .start(
        SourceRequest {
            kind: SourceKind::Monitor,
            source: None,
            target_fps: 60,
        },
        Box::new(move |f| sink.push(f)),
    )
    .await
    .unwrap();
    let presets = template_presets(64, 32, 60, Codec::H264);
    registry
        .add_live(title.into(), SourceKind::Monitor, session, fan, presets)
        .unwrap()
}

#[tokio::test]
async fn encoders_start_on_first_subscription_and_stop_after_the_grace() {
    let changes = Arc::new(AtomicUsize::new(0));
    let counter = changes.clone();
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        GRACE,
        Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let live = synthetic_live(&registry, "desk").await;
    assert_eq!(
        registry.live_infos()[0].presets.len(),
        1,
        "64x32 has no smaller template"
    );
    assert!(registry.views()[0].presets[0].encoder.is_none());

    let mut sub = registry.subscribe(live, SOURCE_PRESET_ID).unwrap();
    let encoder = registry.views()[0].presets[0]
        .encoder
        .clone()
        .expect("encoder started lazily");
    assert_eq!((encoder.name, encoder.subscribers), ("fake", 1));
    let first = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(first.keyframe);

    drop(sub);
    let t = Instant::now();
    registry.housekeeping(t);
    assert!(
        registry.views()[0].presets[0].encoder.is_some(),
        "still inside the grace"
    );
    registry.housekeeping(t + GRACE);
    assert!(
        registry.views()[0].presets[0].encoder.is_none(),
        "stopped after the grace"
    );
    assert!(
        changes.load(Ordering::SeqCst) >= 3,
        "add, start, stop each notified"
    );

    assert_eq!(
        registry.subscribe(99, 1).unwrap_err(),
        SubscribeRejected::UnknownLive(99)
    );
    assert_eq!(
        registry.subscribe(live, 99).unwrap_err(),
        SubscribeRejected::UnknownPreset(99)
    );
    registry.remove_live(live).unwrap();
    assert!(registry.live_infos().is_empty());
}

#[tokio::test]
async fn removing_a_preset_stops_its_encoder_and_ends_its_subscription() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        GRACE,
        Arc::new(|| {}),
    );
    let live = synthetic_live(&registry, "desk").await;
    let mut presets = template_presets(64, 32, 60, Codec::H264);
    presets.push(brp_proto::Preset {
        id: 2,
        name: "tiny".into(),
        width: 32,
        height: 16,
        fps: 30,
        bitrate_kbps: 1_000,
        codec: Codec::H264,
    });
    registry.set_presets(live, presets.clone()).unwrap();
    let mut sub = registry.subscribe(live, 2).unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(frame.keyframe);

    registry.set_presets(live, presets[..1].to_vec()).unwrap();
    assert_eq!(registry.live_infos()[0].presets.len(), 1);
    let ended = tokio::time::timeout(Duration::from_secs(2), async {
        while sub.frames.recv().await.is_some() {}
    })
    .await;
    assert!(
        ended.is_ok(),
        "frame channel closes when the preset's encoder stops"
    );
    registry.stop_all();
}

#[test]
fn live_limit_and_preset_validation_are_enforced() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }),
        GRACE,
        Arc::new(|| {}),
    );
    for i in 0..MAX_LIVES_PER_PARTICIPANT {
        registry
            .add_live(
                format!("l{i}"),
                SourceKind::Window,
                Box::new(DummySession),
                Arc::new(CaptureFan::default()),
                template_presets(64, 32, 30, Codec::H264),
            )
            .unwrap();
    }
    let over = registry.add_live(
        "too many".into(),
        SourceKind::Window,
        Box::new(DummySession),
        Arc::new(CaptureFan::default()),
        vec![],
    );
    assert!(matches!(over, Err(brp_room::RoomError::TooManyLives)));
    let bad = vec![brp_proto::Preset {
        id: 1,
        name: "huge".into(),
        width: 4096,
        height: 2160,
        fps: 30,
        bitrate_kbps: 5_000,
        codec: Codec::H264,
    }];
    assert!(matches!(
        registry.set_presets(1, bad),
        Err(brp_room::RoomError::Proto(_))
    ));
}

#[tokio::test]
async fn audio_is_on_by_default_and_advertised_on_every_live_until_toggled_off() {
    let registry = registry(GRACE);
    synthetic_live(&registry, "desk").await;
    synthetic_live(&registry, "game").await;
    assert!(registry.live_infos().iter().all(|l| l.has_audio));
    assert_eq!(registry.audio_view().state, AudioCaptureState::Idle);
    registry.set_audio(false);
    assert!(registry.live_infos().iter().all(|l| !l.has_audio));
    assert_eq!(registry.audio_view().state, AudioCaptureState::Off);
    assert!(matches!(
        registry.subscribe_audio(1),
        Err(SubscribeRejected::NoAudio)
    ));
}

#[tokio::test]
async fn the_first_audio_subscriber_starts_capture_and_the_grace_stops_it() {
    let registry = registry(GRACE);
    let live = synthetic_live(&registry, "desk").await;
    assert!(matches!(
        registry.subscribe_audio(99),
        Err(SubscribeRejected::UnknownLive(99))
    ));
    let mut audio = registry.subscribe_audio(live).unwrap();
    assert_eq!(audio.params, brp_proto::AudioParams::STANDARD);
    let packet = tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(packet.keyframe);
    let view = registry.audio_view();
    assert_eq!(
        (view.state, view.subscribers),
        (AudioCaptureState::Capturing, 1)
    );

    drop(audio);
    let start = Instant::now();
    loop {
        registry.housekeeping(Instant::now());
        if registry.audio_view().state == AudioCaptureState::Idle {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "capture never stopped"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(start.elapsed() >= GRACE);
}

struct FailingCapture;

impl AudioCapture for FailingCapture {
    fn start(&self, _sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        Err(AudioError::Unsupported("no loopback here".into()))
    }
}

#[tokio::test]
async fn a_failing_capture_clears_has_audio_and_rejects_until_retoggled() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(FailingCapture),
        GRACE,
        Arc::new(|| {}),
    );
    let live = synthetic_live(&registry, "desk").await;
    assert!(matches!(
        registry.subscribe_audio(live),
        Err(SubscribeRejected::NoAudio)
    ));
    assert!(matches!(
        registry.audio_view().state,
        AudioCaptureState::Failed(ref m) if m.contains("no loopback")
    ));
    assert!(!registry.live_infos()[0].has_audio);
    registry.set_audio(false);
    registry.set_audio(true);
    assert_eq!(registry.audio_view().state, AudioCaptureState::Idle);
    assert!(registry.live_infos()[0].has_audio);
}

struct DyingCapture;

struct DeadSession;

impl AudioCaptureSession for DeadSession {
    fn error(&self) -> Option<String> {
        Some("device unplugged".into())
    }
    fn stop(self: Box<Self>) {}
}

impl AudioCapture for DyingCapture {
    fn start(&self, _sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        Ok(Box::new(DeadSession))
    }
}

#[tokio::test]
async fn a_capture_that_dies_is_treated_as_failed_by_housekeeping() {
    let registry = LiveRegistry::new(
        Arc::new(FakeCodecs),
        Arc::new(DyingCapture),
        GRACE,
        Arc::new(|| {}),
    );
    let live = synthetic_live(&registry, "desk").await;
    let _audio = registry.subscribe_audio(live).unwrap();
    registry.housekeeping(Instant::now());
    assert!(matches!(
        registry.audio_view().state,
        AudioCaptureState::Failed(ref m) if m.contains("unplugged")
    ));
    assert!(!registry.live_infos()[0].has_audio);
}

#[tokio::test]
async fn turning_audio_off_while_capturing_stops_it_and_views_agree() {
    let registry = registry(GRACE);
    synthetic_live(&registry, "desk").await;
    let live = synthetic_live(&registry, "game").await;
    let mut audio = registry.subscribe_audio(live).unwrap();
    tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(registry.audio_view().state, AudioCaptureState::Capturing);

    registry.set_audio(false);
    assert_eq!(registry.audio_view().state, AudioCaptureState::Off);
    assert!(registry.views().iter().all(|v| !v.info.has_audio));
    assert!(registry.live_infos().iter().all(|l| !l.has_audio));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
            .await
            .unwrap()
            .is_none(),
        "packets receiver should end when capture stops"
    );
}

#[tokio::test]
async fn stop_all_stops_running_audio() {
    let registry = registry(GRACE);
    let live = synthetic_live(&registry, "desk").await;
    let mut audio = registry.subscribe_audio(live).unwrap();
    tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
        .await
        .unwrap()
        .unwrap();

    registry.stop_all();
    // audio.enabled is untouched by stop_all; with no lives, no error, and nothing running,
    // that reads as Idle rather than Off.
    assert_eq!(registry.audio_view().state, AudioCaptureState::Idle);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), audio.packets.recv())
            .await
            .unwrap()
            .is_none(),
        "packets receiver should end when stop_all stops capture"
    );
}
