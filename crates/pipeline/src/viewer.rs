//! Network frames -> reorder -> decode -> latest-frame slot, on one dedicated thread per subscription.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_codec::{RawFrame, VideoDecoder};
use brp_net::ReceivedFrame;
use brp_proto::constants::REORDER_MAX_WAIT;
use brp_proto::{EncodedFrame, ViewerMessage};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::reorder::{Drained, IncomingFrame, Reorder};
use crate::slot::LatestSlot;

pub type FrameNotify = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct ViewerStats {
    pub frames_received: AtomicU64,
    pub frames_decoded: AtomicU64,
    pub keyframe_requests: AtomicU64,
}

pub struct ViewerSink {
    pub slot: Arc<LatestSlot<RawFrame>>,
    pub stats: Arc<ViewerStats>,
    pub notify: FrameNotify,
}

pub struct Viewer {
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Viewer {
    pub fn start(
        runtime: Handle,
        frames: Receiver<ReceivedFrame>,
        control: Sender<ViewerMessage>,
        decoder: Box<dyn VideoDecoder>,
        sink: ViewerSink,
    ) -> Self {
        let ViewerSink {
            slot,
            stats,
            notify,
        } = sink;
        let stop = Arc::new(AtomicBool::new(false));
        let worker = DecodeLoop {
            runtime,
            frames,
            control,
            decoder,
            notify,
            slot: slot.clone(),
            stats: stats.clone(),
            stop: stop.clone(),
        };
        let thread = thread::Builder::new()
            .name("brp-decode".into())
            .spawn(move || worker.run())
            .expect("spawning a thread only fails when the system is out of resources");
        Self {
            slot,
            stats,
            stop,
            thread: Some(thread),
        }
    }

    pub fn slot(&self) -> Arc<LatestSlot<RawFrame>> {
        self.slot.clone()
    }

    pub fn stats(&self) -> Arc<ViewerStats> {
        self.stats.clone()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct DecodeLoop {
    runtime: Handle,
    frames: Receiver<ReceivedFrame>,
    control: Sender<ViewerMessage>,
    decoder: Box<dyn VideoDecoder>,
    notify: FrameNotify,
    slot: Arc<LatestSlot<RawFrame>>,
    stats: Arc<ViewerStats>,
    stop: Arc<AtomicBool>,
}

impl DecodeLoop {
    fn run(mut self) {
        let _runtime = self.runtime.enter();
        let mut reorder = Reorder::new(REORDER_MAX_WAIT);
        // Polling at a quarter of the cap bounds how late a timed-out gap is noticed.
        let poll = REORDER_MAX_WAIT / 4;
        while !self.stop.load(Ordering::Relaxed) {
            let drained = match self
                .runtime
                .block_on(tokio::time::timeout(poll, self.frames.recv()))
            {
                Ok(Some(frame)) => {
                    self.stats.frames_received.fetch_add(1, Ordering::Relaxed);
                    reorder.push(
                        IncomingFrame {
                            header: frame.header,
                            data: frame.payload,
                        },
                        Instant::now(),
                    )
                }
                Ok(None) => break,
                Err(_elapsed) => reorder.poll(Instant::now()),
            };
            self.handle(drained);
        }
    }

    fn handle(&mut self, drained: Drained) {
        if drained.request_keyframe {
            self.request_keyframe();
        }
        for frame in drained.ready {
            let encoded = EncodedFrame {
                seq: frame.header.seq,
                capture_ts_us: frame.header.capture_ts_us,
                keyframe: frame.header.keyframe,
                data: frame.data,
            };
            match self.decoder.decode(&encoded) {
                Ok(raws) => {
                    for raw in raws {
                        self.stats.frames_decoded.fetch_add(1, Ordering::Relaxed);
                        // A frame the slot replaces here was superseded before display ever read
                        // it, so it is safe to hand back to the decoder's pool.
                        if let Some(superseded) = self.slot.put(raw) {
                            self.decoder.recycle(superseded);
                        }
                        (self.notify)();
                    }
                }
                Err(error) => {
                    tracing::warn!(seq = encoded.seq, %error, "decode failed, asking for a keyframe");
                    self.request_keyframe();
                }
            }
        }
    }

    fn request_keyframe(&self) {
        self.stats.keyframe_requests.fetch_add(1, Ordering::Relaxed);
        if self
            .control
            .try_send(ViewerMessage::RequestKeyframe)
            .is_err()
        {
            tracing::debug!("control channel full or closed; keyframe request dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use brp_codec::CodecError;
    use brp_proto::{FrameHeader, FrameKind};

    use super::*;

    /// Decodes one black frame per call and records every frame handed back through `recycle`, so
    /// a test can assert on pooling without a real decoder or thread.
    struct RecordingDecoder {
        recycled: Arc<Mutex<Vec<RawFrame>>>,
    }
    impl VideoDecoder for RecordingDecoder {
        fn decode(&mut self, frame: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError> {
            Ok(vec![RawFrame::black(8, 4, frame.capture_ts_us)])
        }
        fn recycle(&mut self, frame: RawFrame) {
            self.recycled.lock().unwrap().push(frame);
        }
    }

    fn incoming(seq: u64, capture_ts_us: u64, keyframe: bool) -> IncomingFrame {
        IncomingFrame {
            header: FrameHeader {
                live_id: 1,
                preset_id: 1,
                kind: FrameKind::Video,
                seq,
                capture_ts_us,
                keyframe,
                len: 0,
            },
            data: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_frame_overwritten_before_display_is_handed_back_to_the_decoder() {
        let recycled = Arc::new(Mutex::new(Vec::new()));
        let (_frames_tx, frames_rx) = tokio::sync::mpsc::channel(1);
        let (control_tx, _control_rx) = tokio::sync::mpsc::channel(1);
        let mut worker = DecodeLoop {
            runtime: Handle::current(),
            frames: frames_rx,
            control: control_tx,
            decoder: Box::new(RecordingDecoder {
                recycled: recycled.clone(),
            }),
            notify: Arc::new(|| {}),
            slot: LatestSlot::new(),
            stats: Arc::new(ViewerStats::default()),
            stop: Arc::new(AtomicBool::new(false)),
        };

        // Two frames drained in one batch: nothing has consumed the first before the second
        // overwrites it, so it must come back through `recycle` instead of being dropped.
        worker.handle(Drained {
            ready: vec![incoming(0, 10, true), incoming(1, 20, false)],
            request_keyframe: false,
        });

        let recycled = recycled.lock().unwrap();
        assert_eq!(recycled.len(), 1, "only the superseded frame is recycled");
        assert_eq!(recycled[0].capture_ts_us, 10);
        assert_eq!(
            worker.slot.try_take().unwrap().capture_ts_us,
            20,
            "the newest frame is still the one waiting for display"
        );
    }
}
