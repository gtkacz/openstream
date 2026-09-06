//! Capture chunks -> 20 ms frames -> Opus -> fan-out, on one dedicated thread per publisher.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use brp_audio::{AudioChunk, AudioSink};
use brp_codec::{AudioEncoder, AudioFrame};
use brp_proto::constants::AUDIO_PACKET_DURATION;
use brp_proto::{AudioParams, EncodedFrame};
use tokio::sync::mpsc::Receiver;

use crate::fanout::FanOut;

/// Cuts arbitrary chunks into whole frames. The frame timestamp is the chunk timestamp of its
/// first sample, advanced by one packet duration for every frame cut from the same run.
#[derive(Default)]
pub struct FrameAssembler {
    samples: Vec<f32>,
    first_ts: Option<u64>,
}

impl FrameAssembler {
    pub fn push(&mut self, chunk: AudioChunk) -> Vec<AudioFrame> {
        if self.samples.is_empty() {
            self.first_ts = Some(chunk.capture_ts_us);
        }
        self.samples.extend_from_slice(&chunk.samples);
        let mut frames = Vec::new();
        while self.samples.len() >= AudioFrame::FRAME_LEN {
            let rest = self.samples.split_off(AudioFrame::FRAME_LEN);
            let samples = std::mem::replace(&mut self.samples, rest);
            let ts = self.first_ts.unwrap_or(chunk.capture_ts_us);
            self.first_ts = Some(ts + AUDIO_PACKET_DURATION.as_micros() as u64);
            frames.push(AudioFrame {
                samples,
                capture_ts_us: ts,
            });
        }
        if self.samples.is_empty() {
            self.first_ts = None;
        }
        frames
    }
}

/// Chunks a slow encoder thread has not drained yet. Half a second at 10 ms chunks; beyond that
/// dropping is better than growing latency.
const CHUNK_QUEUE: usize = 50;

#[derive(Default)]
pub struct AudioPublisherStats {
    pub packets_encoded: AtomicU64,
    pub bytes_encoded: AtomicU64,
    pub chunks_dropped: AtomicU64,
}

#[derive(Clone)]
pub struct AudioPublisher {
    inner: Arc<Inner>,
}

struct Inner {
    params: AudioParams,
    encoder_name: &'static str,
    fanout: Mutex<FanOut>,
    stop: AtomicBool,
    stats: AudioPublisherStats,
    chunks: SyncSender<AudioChunk>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl AudioPublisher {
    pub fn start(encoder: Box<dyn AudioEncoder>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<AudioChunk>(CHUNK_QUEUE);
        let inner = Arc::new(Inner {
            params: encoder.params(),
            encoder_name: encoder.name(),
            fanout: Mutex::new(FanOut::new_audio()),
            stop: AtomicBool::new(false),
            stats: AudioPublisherStats::default(),
            chunks: tx,
            thread: Mutex::new(None),
        });
        let worker = inner.clone();
        let handle = thread::Builder::new()
            .name("brp-audio-encode".into())
            .spawn(move || encode_loop(worker, rx, encoder))
            .expect("spawning a thread only fails when the system is out of resources");
        *lock(&inner.thread) = Some(handle);
        Self { inner }
    }

    /// The closure a capture backend calls. Never blocks the capture thread: a full queue drops
    /// the chunk and counts it.
    pub fn sink(&self) -> AudioSink {
        let inner = self.inner.clone();
        Box::new(move |chunk| {
            if inner.chunks.try_send(chunk).is_err() {
                inner.stats.chunks_dropped.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    pub fn params(&self) -> AudioParams {
        self.inner.params
    }

    pub fn encoder_name(&self) -> &'static str {
        self.inner.encoder_name
    }

    pub fn subscribe(&self) -> Receiver<Arc<EncodedFrame>> {
        lock(&self.inner.fanout).add()
    }

    pub fn subscriber_count(&self) -> usize {
        let mut fanout = lock(&self.inner.fanout);
        fanout.prune();
        fanout.subscriber_count()
    }

    pub fn stats(&self) -> &AudioPublisherStats {
        &self.inner.stats
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = lock(&self.inner.thread).take() {
            let _ = handle.join();
        }
    }
}

fn encode_loop(
    inner: Arc<Inner>,
    chunks: mpsc::Receiver<AudioChunk>,
    mut encoder: Box<dyn AudioEncoder>,
) {
    let mut assembler = FrameAssembler::default();
    // Polling at the packet duration bounds how late a stop is noticed.
    let poll = Duration::from_millis(AUDIO_PACKET_DURATION.as_millis() as u64 * 5);
    while !inner.stop.load(Ordering::Relaxed) {
        let chunk = match chunks.recv_timeout(poll) {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        for frame in assembler.push(chunk) {
            match encoder.encode(&frame) {
                Ok(packets) => {
                    for packet in packets {
                        inner.stats.packets_encoded.fetch_add(1, Ordering::Relaxed);
                        inner
                            .stats
                            .bytes_encoded
                            .fetch_add(packet.data.len() as u64, Ordering::Relaxed);
                        lock(&inner.fanout).push(Arc::new(packet));
                    }
                }
                Err(error) => tracing::error!(%error, "audio encode failed"),
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(len: usize, ts: u64, value: f32) -> AudioChunk {
        AudioChunk {
            samples: vec![value; len],
            capture_ts_us: ts,
        }
    }

    #[test]
    fn odd_chunks_become_whole_frames_stamped_with_their_first_sample() {
        let mut assembler = FrameAssembler::default();
        assert!(assembler.push(chunk(1000, 100, 1.0)).is_empty());
        let frames = assembler.push(chunk(3000, 200, 2.0));
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].capture_ts_us, 100);
        assert_eq!(frames[0].samples[..1000], vec![1.0; 1000][..]);
        assert_eq!(frames[0].samples[1000..], vec![2.0; 920][..]);
        // The second frame starts one packet after the first.
        assert_eq!(frames[1].capture_ts_us, 100 + 20_000);
        assert!(frames.iter().all(|f| f.validate().is_ok()));
        let tail = assembler.push(chunk(1760, 300, 3.0));
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].capture_ts_us, 100 + 40_000);
    }

    #[test]
    fn a_partial_tail_waits_and_a_fresh_start_takes_the_new_timestamp() {
        let mut assembler = FrameAssembler::default();
        assert_eq!(assembler.push(chunk(1920, 10, 0.0)).len(), 1);
        assert!(assembler.push(chunk(100, 500, 0.0)).is_empty());
        let frames = assembler.push(chunk(1820, 600, 0.0));
        assert_eq!(frames[0].capture_ts_us, 500);
    }

    #[test]
    fn the_publisher_encodes_and_fans_out_what_the_sink_receives() {
        let publisher =
            AudioPublisher::start(Box::new(brp_codec::fake::FakeAudioEncoder::default()));
        let mut rx = publisher.subscribe();
        let mut sink = publisher.sink();
        for i in 0..3u64 {
            sink(chunk(1920, i * 20_000, 0.5));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut got = Vec::new();
        while got.len() < 3 && std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(packet) => got.push(packet),
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
        assert_eq!(got.len(), 3);
        assert_eq!(got[2].seq, 2);
        assert_eq!(got[1].capture_ts_us, 20_000);
        assert_eq!(publisher.stats().packets_encoded.load(Ordering::Relaxed), 3);
        assert_eq!(publisher.subscriber_count(), 1);
        publisher.stop();
    }
}
