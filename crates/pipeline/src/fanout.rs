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
