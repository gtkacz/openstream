//! Network packets -> jitter buffer -> decode -> mixer track, one thread per audio subscription.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_codec::{AudioDecoder, AudioFrame};
use brp_net::ReceivedFrame;
use brp_proto::EncodedFrame;
use brp_proto::constants::AUDIO_PACKET_DURATION;
use tokio::runtime::Handle;
use tokio::sync::mpsc::Receiver;

use crate::jitter::{JitterBuffer, Slot};
use crate::mixer::Track;

#[derive(Default)]
pub struct AudioViewerStats {
    pub packets_received: AtomicU64,
    pub late: AtomicU64,
    pub trimmed: AtomicU64,
}

pub struct AudioViewer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioViewer {
    pub fn start(
        runtime: Handle,
        packets: Receiver<ReceivedFrame>,
        decoder: Box<dyn AudioDecoder>,
        track: Track,
        stats: Arc<AudioViewerStats>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker = DecodeLoop {
            runtime,
            packets,
            decoder,
            track,
            stats,
            stop: stop.clone(),
        };
        let thread = thread::Builder::new()
            .name("brp-audio-decode".into())
            .spawn(move || worker.run())
            .expect("spawning a thread only fails when the system is out of resources");
        Self {
            stop,
            thread: Some(thread),
        }
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
    packets: Receiver<ReceivedFrame>,
    decoder: Box<dyn AudioDecoder>,
    track: Track,
    stats: Arc<AudioViewerStats>,
    stop: Arc<AtomicBool>,
}

impl DecodeLoop {
    fn run(mut self) {
        let _runtime = self.runtime.enter();
        let mut jitter = JitterBuffer::new(Instant::now());
        // One slot is played per packet duration, on a fixed cadence so the track fills at the
        // rate the device drains it.
        let mut next_tick = Instant::now() + AUDIO_PACKET_DURATION;
        while !self.stop.load(Ordering::Relaxed) {
            let wait = next_tick.saturating_duration_since(Instant::now());
            match self
                .runtime
                .block_on(tokio::time::timeout(wait, self.packets.recv()))
            {
                Ok(Some(frame)) => {
                    self.stats.packets_received.fetch_add(1, Ordering::Relaxed);
                    jitter.push(
                        EncodedFrame {
                            seq: frame.header.seq,
                            capture_ts_us: frame.header.capture_ts_us,
                            keyframe: true,
                            data: frame.payload,
                        },
                        Instant::now(),
                    );
                    self.stats.late.store(jitter.late(), Ordering::Relaxed);
                    self.stats
                        .trimmed
                        .store(jitter.trimmed(), Ordering::Relaxed);
                    continue;
                }
                Ok(None) => break,
                Err(_elapsed) => {}
            }
            next_tick += AUDIO_PACKET_DURATION;
            match jitter.pop(Instant::now()) {
                Slot::Packet(packet) => match self.decoder.decode(&packet) {
                    Ok(frames) => {
                        for frame in frames {
                            self.track.push(&frame.samples);
                        }
                    }
                    Err(error) => {
                        tracing::debug!(seq = packet.seq, %error, "audio decode failed");
                        self.track
                            .push(&AudioFrame::silence(packet.capture_ts_us).samples);
                    }
                },
                Slot::Silence => self.track.push(&AudioFrame::silence(0).samples),
                Slot::Waiting => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use brp_codec::AudioEncoder;
    use brp_codec::fake::{FakeAudioDecoder, FakeAudioEncoder};
    use brp_proto::constants::AUDIO_PRESET_ID;
    use brp_proto::{FrameHeader, FrameKind};
    use tokio::sync::mpsc;

    use super::*;
    use crate::mixer::Mixer;

    fn received(packet: &EncodedFrame) -> ReceivedFrame {
        ReceivedFrame {
            header: FrameHeader {
                live_id: 1,
                preset_id: AUDIO_PRESET_ID,
                kind: FrameKind::Audio,
                seq: packet.seq,
                capture_ts_us: packet.capture_ts_us,
                keyframe: true,
                len: packet.data.len() as u32,
            },
            payload: packet.data.clone(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn decoded_audio_reaches_the_track_in_order() {
        let mixer = Mixer::new();
        let track = mixer.add_track([7; 32]);
        let (tx, rx) = mpsc::channel(16);
        let stats = Arc::new(AudioViewerStats::default());
        let viewer = AudioViewer::start(
            Handle::current(),
            rx,
            Box::new(FakeAudioDecoder),
            track.clone(),
            stats.clone(),
        );
        let mut encoder = FakeAudioEncoder::default();
        for i in 0..6u64 {
            let mut frame = AudioFrame::silence(i * 20_000);
            frame.samples.fill(i as f32 / 10.0);
            let packet = encoder.encode(&frame).unwrap().remove(0);
            tx.send(received(&packet)).await.unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while track.queued() < 3 * AudioFrame::FRAME_LEN && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            track.queued() >= 3 * AudioFrame::FRAME_LEN,
            "queued {}",
            track.queued()
        );
        let mut out = vec![0.0; AudioFrame::FRAME_LEN];
        mixer.render(&mut out);
        assert!(out.iter().all(|s| *s == 0.0), "packet 0 is silence");
        mixer.render(&mut out);
        assert!((out[0] - 0.1).abs() < 1e-6, "packet 1 follows");
        assert_eq!(stats.packets_received.load(Ordering::Relaxed), 6);
        drop(tx);
        viewer.stop();
    }
}
