use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use brp_capture::{CaptureBackend, CaptureSession, SourceInfo, SourceRequest, SyntheticSource};
use brp_net::{LiveSource, SubscribeRejected};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, SOURCE_PRESET_ID};
use brp_proto::{Codec, SourceKind, template_presets};
use brp_room::codecs::fake::FakeCodecs;
use brp_room::registry::{CaptureFan, LiveRegistry};

const GRACE: Duration = Duration::from_millis(300);

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
    let registry = LiveRegistry::new(Arc::new(FakeCodecs), GRACE, Arc::new(|| {}));
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
    let registry = LiveRegistry::new(Arc::new(FakeCodecs), GRACE, Arc::new(|| {}));
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
