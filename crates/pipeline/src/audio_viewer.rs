//! Network packets -> jitter buffer -> decode -> mixer track, one thread per audio subscription.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use brp_codec::{AudioDecoder, AudioFrame};
use brp_net::ReceivedFrame;
use brp_proto::EncodedFrame;
use brp_proto::constants::{
    AUDIO_CHANNELS, AUDIO_PACKET_DURATION, AUDIO_SAMPLE_RATE, MIXER_TRACK_CUSHION,
};
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
    ended: Option<Receiver<()>>,
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
        let (ended_tx, ended_rx) = tokio::sync::mpsc::channel(1);
        let worker = DecodeLoop {
            runtime,
            packets,
            decoder,
            track,
            stats,
            stop: stop.clone(),
            _ended: ended_tx,
        };
        let thread = thread::Builder::new()
            .name("brp-audio-decode".into())
            .spawn(move || worker.run())
            .expect("spawning a thread only fails when the system is out of resources");
        Self {
            stop,
            ended: Some(ended_rx),
            thread: Some(thread),
        }
    }

    /// Closes once the decode thread has exited. Before `stop`, that means the publisher's packet
    /// stream closed, which is how a watch learns its audio is gone. Yields `None` after the first
    /// call, since the channel is handed to whoever waits on it.
    pub fn take_ended(&mut self) -> Option<Receiver<()>> {
        self.ended.take()
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
    /// Never sent on: dropping it with the thread is what signals the end.
    _ended: tokio::sync::mpsc::Sender<()>,
}

impl DecodeLoop {
    fn run(mut self) {
        let _runtime = self.runtime.enter();
        let mut jitter = JitterBuffer::new(Instant::now());
        // One slot is played per packet duration, on a fixed cadence so the track fills at the
        // rate the device drains it.
        let mut next_tick = Instant::now() + AUDIO_PACKET_DURATION;
        let cushion = vec![0.0f32; cushion_samples()];
        // False while the jitter buffer is priming; the first packet of every run pre-rolls the
        // cushion so the device never asks for more than the track holds.
        let mut primed = false;
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
                Slot::Packet(packet) => {
                    if !primed {
                        self.track.push(&cushion);
                        primed = true;
                    }
                    match self.decoder.decode(&packet) {
                        // A decoder can swallow a packet (libopus does on its pre-skip); the slot
                        // is spent either way, so the track still owes the device its 20 ms.
                        Ok(frames) if frames.is_empty() => self
                            .track
                            .push(&AudioFrame::silence(packet.capture_ts_us).samples),
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
                    }
                }
                Slot::Silence => self.track.push(&AudioFrame::silence(0).samples),
                Slot::Waiting => primed = false,
            }
        }
    }
}

/// Interleaved samples in one cushion.
fn cushion_samples() -> usize {
    (AUDIO_SAMPLE_RATE as u128 * MIXER_TRACK_CUSHION.as_millis() / 1000) as usize
        * AUDIO_CHANNELS as usize
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
        let wanted = cushion_samples() + 3 * AudioFrame::FRAME_LEN;
        let deadline = Instant::now() + Duration::from_secs(3);
        while track.queued() < wanted && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(track.queued() >= wanted, "queued {}", track.queued());
        let mut cushion = vec![0.0; cushion_samples()];
        mixer.render(&mut cushion);
        assert!(
            cushion.iter().all(|s| *s == 0.0),
            "the first packet is preceded by the cushion"
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

    /// A device buffer larger than one packet is the normal case: cpal's PipeWire host hands the
    /// callback the server quantum, 1024 frames on a stock daemon. Without a cushion in front of
    /// the track, production and consumption both run at 20 ms and every callback underruns.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_device_sized_callback_stops_underrunning_once_the_cushion_is_in() {
        const DEVICE_FRAMES: u64 = 1024;
        const WARM_UP: Duration = Duration::from_millis(200);
        const RUN: Duration = Duration::from_millis(1200);
        // Both sides run off the wall clock at exactly 48 kHz, as a sound card and a publisher do,
        // so any reserve the track holds comes from the cushion and not from a pacing mismatch.
        let due_packets = |elapsed: Duration| {
            (elapsed.as_micros() / AUDIO_PACKET_DURATION.as_micros()) as u64 + 1
        };
        let due_renders = |elapsed: Duration| {
            (elapsed.as_micros() as u64 * u64::from(AUDIO_SAMPLE_RATE) / 1_000_000) / DEVICE_FRAMES
        };

        let mixer = Mixer::new();
        let track = mixer.add_track([3; 32]);
        let (tx, rx) = mpsc::channel(16);
        let viewer = AudioViewer::start(
            Handle::current(),
            rx,
            Box::new(FakeAudioDecoder),
            track.clone(),
            Arc::new(AudioViewerStats::default()),
        );

        let mut encoder = FakeAudioEncoder::default();
        let mut out = vec![0.0f32; (DEVICE_FRAMES * u64::from(AUDIO_CHANNELS)) as usize];
        let start = Instant::now();
        let (mut warm, mut audible) = (None, false);
        let (mut seq, mut renders) = (0u64, 0u64);
        while start.elapsed() < RUN {
            let elapsed = start.elapsed();
            while seq < due_packets(elapsed) {
                let mut frame = AudioFrame::silence(seq * 20_000);
                frame.samples.fill(0.5);
                let packet = encoder.encode(&frame).unwrap().remove(0);
                tx.send(received(&packet)).await.unwrap();
                seq += 1;
            }
            while renders < due_renders(elapsed) {
                mixer.render(&mut out);
                audible |= out.iter().all(|s| (*s - 0.5).abs() < 1e-6);
                renders += 1;
            }
            if warm.is_none() && elapsed >= WARM_UP {
                warm = Some((track.underruns(), renders));
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let (warm_underruns, warm_renders) = warm.expect("the warm-up window elapsed");
        let (underruns, renders) = (track.underruns() - warm_underruns, renders - warm_renders);
        // Steady state is zero underruns; a loaded machine deschedules the decode thread past the
        // cushion once or twice per run, while without the cushion this harness underran seven
        // callbacks in fifty, so a twentieth separates the two comfortably.
        assert!(
            underruns * 20 <= renders,
            "{underruns} of the {renders} device callbacks after warm-up underran"
        );
        assert!(
            audible,
            "a whole callback of the publisher's audio was mixed"
        );
        drop(tx);
        viewer.stop();
    }
}
