use brp_proto::{
    EncodedFrame,
    constants::{FORCED_KEYFRAME_MIN_INTERVAL, SENDER_BACKLOG_FRAMES},
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
}
pub struct FanOut {
    subs: Vec<Sub>,
    keyframe: KeyframeRequest,
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
        }
    }
    pub fn add(&mut self) -> Receiver<Arc<EncodedFrame>> {
        let (tx, rx) = mpsc::channel(SENDER_BACKLOG_FRAMES);
        self.subs.push(Sub { tx, waiting: true });
        self.keyframe.request();
        rx
    }
    pub fn subscriber_count(&self) -> usize {
        self.subs.len()
    }
    pub fn push(&mut self, f: Arc<EncodedFrame>) -> PushOutcome {
        let mut o = PushOutcome::default();
        let mut req = false;
        self.subs.retain_mut(|s| {
            if s.waiting && !f.keyframe {
                o.skipped += 1;
                return true;
            }
            match s.tx.try_send(f.clone()) {
                Ok(()) => {
                    s.waiting = false;
                    o.delivered += 1;
                    true
                }
                Err(TrySendError::Full(_)) => {
                    s.waiting = true;
                    req = true;
                    o.skipped += 1;
                    true
                }
                Err(TrySendError::Closed(_)) => false,
            }
        });
        if req {
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

    #[test]
    fn dropped_receiver_is_removed_on_push() {
        let mut fanout = FanOut::new(KeyframeRequest::new());
        let rx = fanout.add();
        assert_eq!(fanout.subscriber_count(), 1);
        drop(rx);
        fanout.push(frame(1, true));
        assert_eq!(fanout.subscriber_count(), 0);
    }
}
