use brp_net::KeyframeLimit;
use brp_proto::{
    EncodedFrame,
    constants::{
        AUDIO_SENDER_BACKLOG_PACKETS, FORCED_KEYFRAME_MIN_INTERVAL, SENDER_BACKLOG_FRAMES,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::mpsc::{self, Receiver, Sender, error::TrySendError};
#[derive(Clone, Default)]
pub struct KeyframeRequest {
    inner: Arc<KInner>,
}
#[derive(Default)]
struct KInner {
    requested: AtomicBool,
    last: Mutex<Option<Instant>>,
}
impl KeyframeRequest {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn request(&self) {
        self.inner.requested.store(true, Ordering::Release)
    }
    pub fn pending(&self) -> bool {
        self.inner.requested.load(Ordering::Acquire)
    }
    pub fn take_if_allowed(&self, now: Instant) -> bool {
        if !self.pending() {
            return false;
        }
        let mut l = self.inner.last.lock().unwrap_or_else(|p| p.into_inner());
        if l.is_none_or(|t| now.duration_since(t) >= FORCED_KEYFRAME_MIN_INTERVAL) {
            *l = Some(now);
            self.inner.requested.store(false, Ordering::Release);
            true
        } else {
            false
        }
    }
}
struct Sub {
    tx: Sender<Arc<EncodedFrame>>,
    waiting: bool,
    /// Keeps asking while this viewer waits, since a waiting viewer is only tried on a keyframe and
    /// would otherwise be stranded once a refused request left nothing coming.
    keyframes: KeyframeLimit,
}
pub struct FanOut {
    subs: Vec<Sub>,
    keyframe: KeyframeRequest,
    /// Video subscribers wait for a keyframe after joining or dropping; audio packets stand alone.
    gate_keyframes: bool,
    backlog: usize,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PushOutcome {
    pub delivered: usize,
    pub skipped: usize,
}
impl FanOut {
    pub fn new(keyframe: KeyframeRequest) -> Self {
        Self {
            subs: vec![],
            keyframe,
            gate_keyframes: true,
            backlog: SENDER_BACKLOG_FRAMES,
        }
    }
    /// A fan-out for Opus packets: no keyframe gating and a deeper backlog.
    pub fn new_audio() -> Self {
        Self {
            subs: vec![],
            keyframe: KeyframeRequest::new(),
            gate_keyframes: false,
            backlog: AUDIO_SENDER_BACKLOG_PACKETS,
        }
    }
    pub fn add(&mut self) -> Receiver<Arc<EncodedFrame>> {
        let (tx, rx) = mpsc::channel(self.backlog);
        let mut keyframes = KeyframeLimit::default();
        keyframes.allow(Instant::now());
        self.subs.push(Sub {
            tx,
            waiting: true,
            keyframes,
        });
        if self.gate_keyframes {
            self.keyframe.request();
        }
        rx
    }
    pub fn subscriber_count(&self) -> usize {
        self.subs.len()
    }
    /// Drops subscribers whose receiver is gone. `push` does this as a side effect; callers that
    /// only count, such as idle detection, need it explicitly.
    pub fn prune(&mut self) {
        self.subs.retain(|s| !s.tx.is_closed());
    }
    pub fn push(&mut self, f: Arc<EncodedFrame>) -> PushOutcome {
        self.push_at(f, Instant::now())
    }
    fn push_at(&mut self, f: Arc<EncodedFrame>, now: Instant) -> PushOutcome {
        let mut o = PushOutcome::default();
        let mut req = false;
        let gate = self.gate_keyframes;
        self.subs.retain_mut(|s| {
            if gate && s.waiting && !f.keyframe {
                o.skipped += 1;
                req |= s.keyframes.allow(now);
                return true;
            }
            match s.tx.try_send(f.clone()) {
                Ok(()) => {
                    s.waiting = false;
                    s.keyframes = KeyframeLimit::default();
                    o.delivered += 1;
                    true
                }
                Err(TrySendError::Full(_)) => {
                    s.waiting = true;
                    req |= s.keyframes.allow(now);
                    o.skipped += 1;
                    true
                }
                Err(TrySendError::Closed(_)) => false,
            }
        });
        if req && gate {
            self.keyframe.request()
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn frame(seq: u64, keyframe: bool) -> Arc<EncodedFrame> {
        Arc::new(EncodedFrame {
            seq,
            capture_ts_us: seq * 16_667,
            keyframe,
            data: vec![seq as u8],
        })
    }

    #[test]
    fn keyframe_request_is_rate_limited() {
        let kf = KeyframeRequest::new();
        let t0 = Instant::now();
        assert!(!kf.take_if_allowed(t0));
        kf.request();
        assert!(kf.pending());
        assert!(kf.take_if_allowed(t0));
        assert!(!kf.pending());
        kf.request();
        assert!(!kf.take_if_allowed(t0 + Duration::from_millis(100)));
        assert!(kf.pending());
        assert!(kf.take_if_allowed(t0 + FORCED_KEYFRAME_MIN_INTERVAL));
    }

    #[test]
    fn new_subscriber_waits_for_a_keyframe_and_requests_one() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        assert!(kf.pending());
        assert_eq!(
            fanout.push(frame(1, false)),
            PushOutcome {
                delivered: 0,
                skipped: 1
            }
        );
        assert_eq!(
            fanout.push(frame(2, true)),
            PushOutcome {
                delivered: 1,
                skipped: 0
            }
        );
        assert_eq!(rx.try_recv().unwrap().seq, 2);
        fanout.push(frame(3, false));
        assert_eq!(rx.try_recv().unwrap().seq, 3);
    }

    #[test]
    fn full_channel_skips_until_next_keyframe_and_requests_one() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        assert!(kf.take_if_allowed(Instant::now()));
        fanout.push(frame(1, true));
        fanout.push(frame(2, false));
        assert!(!kf.pending());
        assert_eq!(
            fanout.push(frame(3, false)),
            PushOutcome {
                delivered: 0,
                skipped: 1
            }
        );
        assert!(kf.pending());
        assert_eq!(rx.try_recv().unwrap().seq, 1);
        assert_eq!(rx.try_recv().unwrap().seq, 2);
        assert_eq!(
            fanout.push(frame(4, false)),
            PushOutcome {
                delivered: 0,
                skipped: 1
            }
        );
        assert_eq!(
            fanout.push(frame(5, true)),
            PushOutcome {
                delivered: 1,
                skipped: 0
            }
        );
        assert_eq!(rx.try_recv().unwrap().seq, 5);
    }

    /// Plays a publisher at ten frames a second, a keyframe whenever one is granted, and returns how
    /// many keyframes it made in `seconds`.
    fn keyframes_made(
        fanout: &mut FanOut,
        kf: &KeyframeRequest,
        start: Instant,
        seconds: u64,
    ) -> u32 {
        let mut made = 0;
        for tenth in 0..seconds * 10 {
            let now = start + Duration::from_millis(tenth * 100);
            let keyframe = kf.take_if_allowed(now);
            made += u32::from(keyframe);
            fanout.push_at(frame(tenth, keyframe), now);
        }
        made
    }

    #[test]
    fn a_stuck_viewer_costs_the_others_fewer_and_fewer_keyframes() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let _never_read = fanout.add();
        let made = keyframes_made(&mut fanout, &kf, Instant::now(), 60);
        assert!(
            made <= 20,
            "a stuck viewer asked for {made} keyframes in a minute, not far off one a second"
        );
        assert!(made >= 3, "a stuck viewer stopped asking after {made}");
    }

    #[test]
    fn a_viewer_refused_while_full_still_recovers_once_it_catches_up() {
        let kf = KeyframeRequest::new();
        let mut fanout = FanOut::new(kf.clone());
        let mut rx = fanout.add();
        let start = Instant::now();
        keyframes_made(&mut fanout, &kf, start, 30);
        while rx.try_recv().is_ok() {}
        let later = start + Duration::from_secs(30);
        keyframes_made(&mut fanout, &kf, later, 8);
        assert!(
            rx.try_recv().is_ok_and(|frame| frame.keyframe),
            "with nothing else asking, the viewer's own next request must bring it back"
        );
    }

    #[test]
    fn dropped_receiver_is_removed_on_push() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        assert_eq!(fanout.subscriber_count(), 1);
        drop(rx);
        fanout.push(frame(1, true));
        assert_eq!(fanout.subscriber_count(), 0);
    }

    #[test]
    fn prune_forgets_closed_receivers_without_a_push() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        let _kept = fanout.add();
        drop(rx);
        fanout.prune();
        assert_eq!(fanout.subscriber_count(), 1);
    }

    #[test]
    fn audio_fanout_delivers_without_waiting_for_a_keyframe() {
        let mut fanout = FanOut::new_audio();
        let mut rx = fanout.add();
        assert_eq!(
            fanout.push(frame(1, false)),
            PushOutcome {
                delivered: 1,
                skipped: 0
            }
        );
        assert_eq!(rx.try_recv().unwrap().seq, 1);
    }

    #[test]
    fn audio_fanout_holds_the_audio_backlog_and_keeps_delivering_after_a_full_channel() {
        let mut fanout = FanOut::new_audio();
        let mut rx = fanout.add();
        for seq in 0..brp_proto::constants::AUDIO_SENDER_BACKLOG_PACKETS as u64 {
            assert_eq!(fanout.push(frame(seq, false)).delivered, 1);
        }
        assert_eq!(fanout.push(frame(99, false)).skipped, 1);
        assert_eq!(rx.try_recv().unwrap().seq, 0);
        // A non-keyframe after the drop is delivered at once; audio has no keyframes to wait for.
        assert_eq!(fanout.push(frame(100, false)).delivered, 1);
    }
}
