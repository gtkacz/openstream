use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use brp_codec::fake::{FakeDecoder, FakeEncoder};
use brp_codec::{EncoderConfig, RawFrame, VideoEncoder};
use brp_net::ReceivedFrame;
use brp_pipeline::Viewer;
use brp_proto::constants::REORDER_MAX_WAIT;
use brp_proto::{Codec, EncodedFrame, FrameHeader, FrameKind, ViewerMessage};
use tokio::sync::mpsc;

fn encoded_frames(n: u64) -> Vec<EncodedFrame> {
    let mut encoder = FakeEncoder::new(
        EncoderConfig {
            width: 8,
            height: 4,
            fps: 30,
            bitrate_kbps: 1_000,
            codec: Codec::H264,
        },
        1_000,
    );
    (0..n)
        .map(|i| {
            encoder
                .encode(&RawFrame::black(8, 4, i * 100), false)
                .unwrap()
                .remove(0)
        })
        .collect()
}

fn received(frame: &EncodedFrame) -> ReceivedFrame {
    ReceivedFrame {
        header: FrameHeader {
            live_id: 1,
            preset_id: 1,
            kind: FrameKind::Video,
            seq: frame.seq,
            capture_ts_us: frame.capture_ts_us,
            keyframe: frame.keyframe,
            len: frame.data.len() as u32,
        },
        payload: frame.data.clone(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decodes_in_sequence_order_and_publishes_latest_frame() {
    let frames = encoded_frames(3);
    let (tx, rx) = mpsc::channel(8);
    let (control_tx, _control_rx) = mpsc::channel(8);
    let notified = Arc::new(AtomicUsize::new(0));
    let notify_count = notified.clone();
    let viewer = Viewer::start(
        tokio::runtime::Handle::current(),
        rx,
        control_tx,
        Box::new(FakeDecoder),
        Arc::new(move || {
            notify_count.fetch_add(1, Ordering::SeqCst);
        }),
    );

    tx.send(received(&frames[0])).await.unwrap();
    tx.send(received(&frames[2])).await.unwrap();
    tx.send(received(&frames[1])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(notified.load(Ordering::SeqCst), 3);
    let latest = viewer
        .slot()
        .try_take()
        .expect("a decoded frame is waiting");
    assert_eq!(latest.capture_ts_us, 200, "the newest frame wins");
    assert_eq!(viewer.stats().frames_decoded.load(Ordering::SeqCst), 3);
    viewer.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gap_that_outlives_the_wait_cap_requests_a_keyframe() {
    let frames = encoded_frames(5);
    let (tx, rx) = mpsc::channel(8);
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let viewer = Viewer::start(
        tokio::runtime::Handle::current(),
        rx,
        control_tx,
        Box::new(FakeDecoder),
        Arc::new(|| {}),
    );

    tx.send(received(&frames[0])).await.unwrap();
    tx.send(received(&frames[2])).await.unwrap();
    let message = tokio::time::timeout(REORDER_MAX_WAIT * 3, control_rx.recv())
        .await
        .expect("request within the cap")
        .unwrap();
    assert_eq!(message, ViewerMessage::RequestKeyframe);
    assert_eq!(viewer.stats().keyframe_requests.load(Ordering::SeqCst), 1);

    let mut keyframe = received(&frames[4]);
    keyframe.header.keyframe = true;
    tx.send(keyframe).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        viewer.stats().frames_decoded.load(Ordering::SeqCst),
        2,
        "frame 0 and the recovery keyframe"
    );
    viewer.stop();
}
