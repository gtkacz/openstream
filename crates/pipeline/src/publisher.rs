//! Capture slot -> convert -> encode -> fan-out, on one dedicated thread per preset.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_capture::{CaptureFrame, CaptureSession};
use brp_codec::{FrameConverter, InputImage, VideoEncoder};
use brp_net::{LiveSource, SubscribeRejected, Subscription};
use brp_proto::CodecParams;
use brp_proto::constants::IDLE_KEYFRAME_RETRY;

use crate::fanout::{FanOut, KeyframeRequest};
use crate::slot::{LatestSlot, SlotWait};

#[derive(Default)]
pub struct PublisherStats {
    pub frames_encoded: AtomicU64,
    pub bytes_encoded: AtomicU64,
}

#[derive(Clone)]
pub struct Publisher {
    inner: Arc<Inner>,
}

struct Inner {
    live_id: u32,
    preset_id: u32,
    params: CodecParams,
    encoder_name: &'static str,
    fanout: Mutex<FanOut>,
    keyframe: KeyframeRequest,
    stop: AtomicBool,
    stats: PublisherStats,
    slot: Arc<LatestSlot<CaptureFrame>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    session: Mutex<Option<Box<dyn CaptureSession>>>,
}

impl Publisher {
    pub fn start(
        live_id: u32,
        preset_id: u32,
        slot: Arc<LatestSlot<CaptureFrame>>,
        session: Box<dyn CaptureSession>,
        converter: Box<dyn FrameConverter>,
        encoder: Box<dyn VideoEncoder>,
    ) -> Self {
        let keyframe = KeyframeRequest::new();
        let inner = Arc::new(Inner {
            live_id,
            preset_id,
            params: encoder.params(),
            encoder_name: encoder.name(),
            fanout: Mutex::new(FanOut::new(keyframe.clone())),
            keyframe,
            stop: AtomicBool::new(false),
            stats: PublisherStats::default(),
            slot,
            thread: Mutex::new(None),
            session: Mutex::new(Some(session)),
        });
        let worker = inner.clone();
        let handle = thread::Builder::new()
            .name(format!("brp-encode-{live_id}-{preset_id}"))
            .spawn(move || encode_loop(worker, converter, encoder))
            .expect("spawning a thread only fails when the system is out of resources");
        *lock(&inner.thread) = Some(handle);
        Self { inner }
    }

    pub fn params(&self) -> &CodecParams {
        &self.inner.params
    }

    pub fn encoder_name(&self) -> &'static str {
        self.inner.encoder_name
    }

    pub fn stats(&self) -> &PublisherStats {
        &self.inner.stats
    }

    pub fn frames_dropped_at_input(&self) -> u64 {
        self.inner.slot.dropped()
    }

    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner.fanout).subscriber_count()
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        self.inner.slot.close();
        if let Some(handle) = lock(&self.inner.thread).take() {
            let _ = handle.join();
        }
        if let Some(session) = lock(&self.inner.session).take() {
            session.stop();
        }
    }
}

impl LiveSource for Publisher {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        if live_id != self.inner.live_id {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if preset_id != self.inner.preset_id {
            return Err(SubscribeRejected::UnknownPreset(preset_id));
        }
        let frames = lock(&self.inner.fanout).add();
        Ok(Subscription {
            params: self.inner.params.clone(),
            frames,
        })
    }

    fn request_keyframe(&self, live_id: u32, preset_id: u32) {
        if live_id == self.inner.live_id && preset_id == self.inner.preset_id {
            self.inner.keyframe.request();
        }
    }
}

fn encode_loop(
    inner: Arc<Inner>,
    mut converter: Box<dyn FrameConverter>,
    mut encoder: Box<dyn VideoEncoder>,
) {
    let mut last: Option<CaptureFrame> = None;
    while !inner.stop.load(Ordering::Relaxed) {
        let frame = match inner.slot.take_timeout(IDLE_KEYFRAME_RETRY) {
            SlotWait::Value(frame) => {
                last = Some(frame);
                last.as_ref()
            }
            SlotWait::Timeout if inner.keyframe.pending() => last.as_ref(),
            SlotWait::Timeout => continue,
            SlotWait::Closed => break,
        };
        let Some(frame) = frame else { continue };
        let force = inner.keyframe.take_if_allowed(Instant::now());
        let image = InputImage {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: frame.format,
            data: &frame.data,
            capture_ts_us: frame.capture_ts_us,
        };
        let raw = match converter.convert(&image) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::error!(%error, "frame conversion failed");
                continue;
            }
        };
        match encoder.encode(&raw, force) {
            Ok(packets) => {
                for packet in packets {
                    inner.stats.frames_encoded.fetch_add(1, Ordering::Relaxed);
                    inner
                        .stats
                        .bytes_encoded
                        .fetch_add(packet.data.len() as u64, Ordering::Relaxed);
                    lock(&inner.fanout).push(Arc::new(packet));
                }
            }
            Err(error) => tracing::error!(%error, "encode failed"),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
