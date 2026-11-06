//! The sending half of voice: one microphone, one encoder, and a forwarder per listener that the
//! room says is in voice and hearing.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use brp_audio::{AudioCaptureSession, VoiceCapture};
use brp_codec::{AudioEncoder, AudioFrame, CodecError};
use brp_net::{AudioSubscription, SubscribeRejected};
use brp_pipeline::{AudioPublisher, FrameHook};
use brp_proto::EncodedFrame;
use brp_proto::constants::AUDIO_SENDER_BACKLOG_PACKETS;
use brp_voiceproc::{EchoReference, ProcessingConfig, VoiceProcessor};
use iroh::PublicKey;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::rules::listens_to;
use crate::gossip::lock;
use crate::membership::Membership;

/// An encoder may swallow a frame or two before its first packet comes out.
const SILENCE_ATTEMPTS: usize = 3;

/// One encoded packet of silence, which stands in for the microphone toward a listener the user
/// deafened.
pub(crate) fn silence_packet(encoder: &mut dyn AudioEncoder) -> Result<Vec<u8>, CodecError> {
    for _ in 0..SILENCE_ATTEMPTS {
        if let Some(packet) = encoder.encode(&AudioFrame::silence(0))?.into_iter().next() {
            return Ok(packet.data);
        }
    }
    Err(CodecError::InvalidFrame(
        "the encoder produced no packet for silence".into(),
    ))
}

pub(crate) struct VoiceSend {
    publisher: AudioPublisher,
    silence: Arc<Vec<u8>>,
    membership: Arc<Mutex<Membership>>,
    not_sending: Arc<Mutex<HashSet<PublicKey>>>,
    runtime: Handle,
    // f32 bits: std has no atomic float.
    level: Arc<AtomicU32>,
    // Gates the hook's level writes so a closed mic can read zero without racing queued frames.
    metering: Arc<AtomicBool>,
    session: Mutex<Option<Box<dyn AudioCaptureSession>>>,
    open_error: Mutex<Option<String>>,
}

impl VoiceSend {
    pub fn start(
        encoder: Box<dyn AudioEncoder>,
        silence: Vec<u8>,
        processing: ProcessingConfig,
        reference: EchoReference,
        membership: Arc<Mutex<Membership>>,
        not_sending: Arc<Mutex<HashSet<PublicKey>>>,
        runtime: Handle,
    ) -> Self {
        let level = Arc::new(AtomicU32::new(0));
        let metering = Arc::new(AtomicBool::new(false));
        let hook_level = level.clone();
        let hook_metering = metering.clone();
        // The processor is built on the encode thread: it is not `Send`, and building it may be
        // slow.
        let make_hook: Box<dyn FnOnce() -> FrameHook + Send> = Box::new(move || {
            let mut processor = VoiceProcessor::new(processing, &reference);
            Box::new(move |frame| {
                processor.process(&mut frame.samples);
                if hook_metering.load(Ordering::Relaxed) {
                    let peak = frame
                        .samples
                        .iter()
                        .fold(0.0_f32, |peak, s| peak.max(s.abs()));
                    hook_level.store(peak.to_bits(), Ordering::Relaxed);
                }
            })
        });
        Self {
            publisher: AudioPublisher::start_with_hook(encoder, make_hook),
            silence: Arc::new(silence),
            membership,
            not_sending,
            runtime,
            level,
            metering,
            session: Mutex::new(None),
            open_error: Mutex::new(None),
        }
    }

    pub fn open_mic(&self, capture: &dyn VoiceCapture) -> Result<(), String> {
        self.close_mic();
        self.metering.store(true, Ordering::Relaxed);
        match capture.start(self.publisher.sink()) {
            Ok(session) => {
                *lock(&self.open_error) = None;
                *lock(&self.session) = Some(session);
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                *lock(&self.open_error) = Some(message.clone());
                Err(message)
            }
        }
    }

    pub fn close_mic(&self) {
        if let Some(session) = lock(&self.session).take() {
            session.stop();
        }
        self.metering.store(false, Ordering::Relaxed);
        self.level.store(0, Ordering::Relaxed);
    }

    pub fn mic_error(&self) -> Option<String> {
        lock(&self.open_error)
            .clone()
            .or_else(|| lock(&self.session).as_ref().and_then(|s| s.error()))
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn bytes_encoded(&self) -> u64 {
        self.publisher.stats().bytes_encoded.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self, peer: PublicKey) -> Result<AudioSubscription, SubscribeRejected> {
        if !self.peer_listens(peer) {
            return Err(SubscribeRejected::NoVoice);
        }
        let (tx, packets) = mpsc::channel(AUDIO_SENDER_BACKLOG_PACKETS);
        let mut source = self.publisher.subscribe();
        let membership = self.membership.clone();
        let not_sending = self.not_sending.clone();
        let silence = self.silence.clone();
        self.runtime.spawn(async move {
            loop {
                // Without this a forwarder outlives its subscriber until the next packet arrives.
                let packet = tokio::select! {
                    packet = source.recv() => match packet {
                        Some(packet) => packet,
                        None => break,
                    },
                    () = tx.closed() => break,
                };
                if !listens(&membership, peer) {
                    break;
                }
                let packet = if lock(&not_sending).contains(&peer) {
                    Arc::new(EncodedFrame {
                        seq: packet.seq,
                        capture_ts_us: packet.capture_ts_us,
                        keyframe: true,
                        data: silence.to_vec(),
                    })
                } else {
                    packet
                };
                if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(packet) {
                    break;
                }
            }
        });
        Ok(AudioSubscription {
            params: self.publisher.params(),
            packets,
        })
    }

    pub fn stop(self) {
        self.close_mic();
        self.publisher.stop();
    }

    fn peer_listens(&self, peer: PublicKey) -> bool {
        listens(&self.membership, peer)
    }
}

fn listens(membership: &Mutex<Membership>, peer: PublicKey) -> bool {
    listens_to(
        lock(membership)
            .get(&peer)
            .and_then(|m| m.presence.voice.as_ref()),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use brp_audio::SyntheticTone;
    use brp_codec::fake::FakeAudioEncoder;
    use brp_proto::constants::PROTOCOL_VERSION;
    use brp_proto::{Presence, VoiceState};
    use brp_voiceproc::Suppression;
    use iroh::SecretKey;
    use tokio::time::timeout;

    use super::*;

    const WAIT: Duration = Duration::from_secs(2);

    fn key(seed: u8) -> PublicKey {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn with_voice(ts: u64, voice: Option<VoiceState>) -> Presence {
        Presence {
            version: PROTOCOL_VERSION,
            ts_unix_ms: ts,
            nickname: "peer".into(),
            lives: vec![],
            audio_mix: None,
            quality_presets: Vec::new(),
            voice,
        }
    }

    fn tone() -> SyntheticTone {
        SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        }
    }

    fn silence() -> Vec<u8> {
        silence_packet(&mut FakeAudioEncoder::default()).unwrap()
    }

    struct Fixture {
        send: VoiceSend,
        membership: Arc<Mutex<Membership>>,
        not_sending: Arc<Mutex<HashSet<PublicKey>>>,
    }

    fn fixture(listeners: &[(u8, Option<VoiceState>)]) -> Fixture {
        fixture_with(listeners, ProcessingConfig::OFF)
    }

    fn fixture_with(
        listeners: &[(u8, Option<VoiceState>)],
        processing: ProcessingConfig,
    ) -> Fixture {
        let membership = Arc::new(Mutex::new(Membership::new(Duration::from_secs(20))));
        for (seed, voice) in listeners {
            lock(&membership).apply(key(*seed), with_voice(1, *voice), Instant::now());
        }
        let not_sending = Arc::new(Mutex::new(HashSet::new()));
        let send = VoiceSend::start(
            Box::new(FakeAudioEncoder::default()),
            silence(),
            processing,
            EchoReference::new(),
            membership.clone(),
            not_sending.clone(),
            Handle::current(),
        );
        Fixture {
            send,
            membership,
            not_sending,
        }
    }

    async fn next(subscription: &mut AudioSubscription) -> Arc<EncodedFrame> {
        timeout(WAIT, subscription.packets.recv())
            .await
            .expect("a packet within the wait")
            .expect("the channel stays open")
    }

    #[tokio::test]
    async fn a_listener_out_of_voice_is_refused() {
        let f = fixture(&[(1, None)]);
        assert_eq!(
            f.send.subscribe(key(1)).unwrap_err(),
            SubscribeRejected::NoVoice
        );
    }

    #[tokio::test]
    async fn a_deafened_listener_is_refused() {
        let deafened = VoiceState {
            muted: true,
            deafened: true,
        };
        let f = fixture(&[(1, Some(deafened))]);
        assert_eq!(
            f.send.subscribe(key(1)).unwrap_err(),
            SubscribeRejected::NoVoice
        );
    }

    #[tokio::test]
    async fn a_listener_receives_the_microphone() {
        let f = fixture(&[(1, Some(VoiceState::default()))]);
        let mut subscription = f.send.subscribe(key(1)).unwrap();
        f.send.open_mic(&tone()).unwrap();
        let silence = silence();
        for _ in 0..3 {
            assert_ne!(next(&mut subscription).await.data, silence);
        }
    }

    #[tokio::test]
    async fn a_listener_you_deafened_receives_silence_at_the_same_rate() {
        let f = fixture(&[
            (1, Some(VoiceState::default())),
            (2, Some(VoiceState::default())),
        ]);
        lock(&f.not_sending).insert(key(2));
        let mut heard = f.send.subscribe(key(1)).unwrap();
        let mut deafened = f.send.subscribe(key(2)).unwrap();
        f.send.open_mic(&tone()).unwrap();
        let silence = silence();
        for _ in 0..3 {
            let a = next(&mut heard).await;
            let b = next(&mut deafened).await;
            assert_eq!(a.seq, b.seq);
            assert_eq!(b.data, silence);
        }
    }

    #[tokio::test]
    async fn a_deafened_listener_gets_silence_at_the_same_rate_with_processing_on() {
        let f = fixture_with(
            &[
                (1, Some(VoiceState::default())),
                (2, Some(VoiceState::default())),
            ],
            ProcessingConfig {
                echo_cancellation: true,
                noise_suppression: Suppression::Light,
                automatic_gain: true,
            },
        );
        lock(&f.not_sending).insert(key(2));
        let mut heard = f.send.subscribe(key(1)).unwrap();
        let mut deafened = f.send.subscribe(key(2)).unwrap();
        f.send.open_mic(&tone()).unwrap();
        let silence = silence();
        for _ in 0..3 {
            let a = next(&mut heard).await;
            let b = next(&mut deafened).await;
            assert_eq!(a.seq, b.seq);
            assert_eq!(b.data, silence);
        }
    }

    #[tokio::test]
    async fn a_listener_who_leaves_voice_stops_receiving() {
        let f = fixture(&[(1, Some(VoiceState::default()))]);
        let mut subscription = f.send.subscribe(key(1)).unwrap();
        f.send.open_mic(&tone()).unwrap();
        next(&mut subscription).await;
        lock(&f.membership).apply(key(1), with_voice(2, None), Instant::now());
        timeout(WAIT, async {
            while subscription.packets.recv().await.is_some() {}
        })
        .await
        .expect("the channel closes once the listener is out of voice");
    }

    #[tokio::test]
    async fn closing_the_mic_stops_packets_and_zeroes_the_level() {
        let f = fixture(&[(1, Some(VoiceState::default()))]);
        let mut subscription = f.send.subscribe(key(1)).unwrap();
        f.send.open_mic(&tone()).unwrap();
        next(&mut subscription).await;
        assert!(f.send.level() > 0.0);
        f.send.close_mic();
        assert_eq!(f.send.level(), 0.0);
        // Chunks already queued for the encoder still come out; let them drain first.
        tokio::time::sleep(Duration::from_millis(200)).await;
        while subscription.packets.try_recv().is_ok() {}
        assert!(
            timeout(Duration::from_millis(200), subscription.packets.recv())
                .await
                .is_err()
        );
    }

    #[test]
    fn silence_is_one_packet_of_encoded_zeros() {
        assert!(!silence().is_empty());
    }
}
