use std::sync::atomic::Ordering;
use std::time::Duration;

use brp_capture::{
    CaptureBackend, CaptureFrame, CaptureSession, SourceInfo, SourceRequest, SyntheticSource,
};
use brp_codec::EncoderConfig;
use brp_codec::fake::{FakeEncoder, SolidConverter};
use brp_net::{LiveSource, SubscribeRejected};
use brp_pipeline::{LatestSlot, Publisher};
use brp_proto::{Codec, PixelFormat, SourceKind};

fn cfg() -> EncoderConfig {
    EncoderConfig {
        width: 32,
        height: 16,
        fps: 60,
        bitrate_kbps: 5_000,
        codec: Codec::H264,
    }
}

#[tokio::test]
async fn subscriber_receives_a_keyframe_first_then_ordered_frames() {
    let slot = LatestSlot::new();
    let sink_slot = slot.clone();
    let session = SyntheticSource {
        width: 64,
        height: 32,
        fps: 60,
    }
    .start(
        SourceRequest {
            kind: SourceKind::Monitor,
            target_fps: 60,
        },
        Box::new(move |frame| sink_slot.put(frame)),
    )
    .await
    .unwrap();
    let publisher = Publisher::start(
        1,
        1,
        slot,
        session,
        Box::new(SolidConverter::new(32, 16)),
        Box::new(FakeEncoder::new(cfg(), 30)),
    );
    assert_eq!(publisher.encoder_name(), "fake");
    assert_eq!(
        (publisher.params().width, publisher.params().height),
        (32, 16)
    );

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut sub = publisher.subscribe(1, 1).unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(first.keyframe);
    let mut previous = first.seq;
    for _ in 0..5 {
        let frame = tokio::time::timeout(Duration::from_secs(2), sub.frames.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(frame.seq > previous);
        previous = frame.seq;
    }
    assert_eq!(publisher.subscriber_count(), 1);
    assert!(publisher.stats().frames_encoded.load(Ordering::Relaxed) >= 6);
    assert_eq!(
        publisher.subscribe(2, 1).unwrap_err(),
        SubscribeRejected::UnknownLive(2)
    );
    assert_eq!(
        publisher.subscribe(1, 9).unwrap_err(),
        SubscribeRejected::UnknownPreset(9)
    );
    drop(sub);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(publisher.subscriber_count(), 0);
    publisher.stop();
}

struct StaticSession;

impl CaptureSession for StaticSession {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            width: 8,
            height: 8,
            fps: 60,
        }
    }
    fn stop(self: Box<Self>) {}
}

#[tokio::test]
async fn static_screen_still_serves_a_late_subscriber_a_keyframe() {
    let slot = LatestSlot::new();
    let publisher = Publisher::start(
        1,
        1,
        slot.clone(),
        Box::new(StaticSession),
        Box::new(SolidConverter::new(8, 8)),
        Box::new(FakeEncoder::new(cfg(), 1_000)),
    );
    slot.put(CaptureFrame {
        width: 8,
        height: 8,
        stride: 32,
        format: PixelFormat::Bgra,
        data: vec![0; 256],
        capture_ts_us: 1,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut sub = publisher.subscribe(1, 1).unwrap();
    let frame = tokio::time::timeout(Duration::from_millis(1_500), sub.frames.recv())
        .await
        .expect("re-encoded within the idle retry")
        .unwrap();
    assert!(frame.keyframe);
    assert_eq!(frame.capture_ts_us, 1);
    publisher.stop();
}
