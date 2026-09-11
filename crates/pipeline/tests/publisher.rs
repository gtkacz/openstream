use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brp_capture::{CaptureBackend, CaptureFrame, SourceRequest, SyntheticSource};
use brp_codec::{EncoderConfig, FrameConverter, InputImage};
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
    // Conversion happens once, upstream of the publisher's slot, exactly as the shared
    // conversion group in the room crate does for a group of compatible presets.
    let converter = Arc::new(Mutex::new(SolidConverter::new(32, 16)));
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
        Box::new(move |frame: CaptureFrame| {
            let image = InputImage {
                width: frame.width,
                height: frame.height,
                stride: frame.stride,
                format: frame.format,
                data: &frame.data,
                capture_ts_us: frame.capture_ts_us,
            };
            let raw = converter.lock().unwrap().convert(&image).unwrap().clone();
            sink_slot.put(Arc::new(raw));
        }),
    )
    .await
    .unwrap();
    let publisher = Publisher::start(1, 1, slot, Box::new(FakeEncoder::new(cfg(), 30)), None);
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
    assert_eq!(
        publisher.subscriber_count(),
        0,
        "counting prunes closed receivers"
    );
    publisher.stop();
    session.stop();
}

#[tokio::test]
async fn static_screen_still_serves_a_late_subscriber_a_keyframe() {
    let slot = LatestSlot::new();
    let publisher = Publisher::start(
        1,
        1,
        slot.clone(),
        Box::new(FakeEncoder::new(cfg(), 1_000)),
        None,
    );
    let image = InputImage {
        width: 8,
        height: 8,
        stride: 32,
        format: PixelFormat::Bgra,
        data: &[0; 256],
        capture_ts_us: 1,
    };
    let raw = SolidConverter::new(8, 8).convert(&image).unwrap().clone();
    slot.put(Arc::new(raw));
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
