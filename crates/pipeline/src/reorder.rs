use brp_proto::FrameHeader;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingFrame {
    pub header: FrameHeader,
    pub data: Vec<u8>,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drained {
    pub ready: Vec<IncomingFrame>,
    pub request_keyframe: bool,
}
pub struct Reorder {
    max_wait: Duration,
    next: Option<u64>,
    pending: BTreeMap<u64, IncomingFrame>,
    gap_since: Option<Instant>,
    waiting: bool,
}
impl Reorder {
    pub fn new(max_wait: Duration) -> Self {
        Self {
            max_wait,
            next: None,
            pending: BTreeMap::new(),
            gap_since: None,
            waiting: true,
        }
    }
    pub fn push(&mut self, f: IncomingFrame, now: Instant) -> Drained {
        if self.waiting && !f.header.keyframe {
            return Drained::default();
        }
        if self.waiting && f.header.keyframe {
            self.next = Some(f.header.seq);
            self.waiting = false
        }
        self.pending.insert(f.header.seq, f);
        self.drain(now)
    }
    pub fn poll(&mut self, now: Instant) -> Drained {
        self.drain(now)
    }
    fn drain(&mut self, now: Instant) -> Drained {
        let mut out = Drained::default();
        while let Some(next) = self.next {
            if let Some(f) = self.pending.remove(&next) {
                self.next = Some(next + 1);
                self.gap_since = None;
                out.ready.push(f)
            } else {
                if self.pending.is_empty() {
                    break;
                }
                let since = self.gap_since.get_or_insert(now);
                if now.duration_since(*since) >= self.max_wait {
                    self.pending.clear();
                    self.waiting = true;
                    self.next = None;
                    self.gap_since = None;
                    out.request_keyframe = true
                }
                break;
            }
        }
        out
    }
}
