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
    /// `None` while waiting for a keyframe: at start-up and after a gap times out.
    next: Option<u64>,
    pending: BTreeMap<u64, IncomingFrame>,
    gap_since: Option<Instant>,
}

impl Reorder {
    pub fn new(max_wait: Duration) -> Self {
        Self {
            max_wait,
            next: None,
            pending: BTreeMap::new(),
            gap_since: None,
        }
    }

    pub fn push(&mut self, frame: IncomingFrame, now: Instant) -> Drained {
        let mut out = Drained::default();
        match self.next {
            None => {
                if frame.header.keyframe {
                    self.restart_from(frame, &mut out);
                }
            }
            Some(next) => {
                if frame.header.seq < next || self.pending.contains_key(&frame.header.seq) {
                    return out;
                }
                self.pending.insert(frame.header.seq, frame);
                self.drain(now, &mut out);
            }
        }
        out
    }

    pub fn poll(&mut self, now: Instant) -> Drained {
        let mut out = Drained::default();
        if let Some(since) = self.gap_since
            && now.duration_since(since) >= self.max_wait
        {
            self.pending.clear();
            self.gap_since = None;
            self.next = None;
            out.request_keyframe = true;
        }
        out
    }

    /// A keyframe makes everything before it irrelevant; decoding resumes from it.
    fn restart_from(&mut self, keyframe: IncomingFrame, out: &mut Drained) {
        let seq = keyframe.header.seq;
        self.pending = self.pending.split_off(&(seq + 1));
        self.gap_since = None;
        self.next = Some(seq + 1);
        out.ready.push(keyframe);
        self.drain_contiguous(out);
    }

    fn drain(&mut self, now: Instant, out: &mut Drained) {
        self.drain_contiguous(out);
        let Some(next) = self.next else { return };
        if self.pending.is_empty() {
            self.gap_since = None;
            return;
        }
        let later_keyframe = self
            .pending
            .iter()
            .find(|(seq, f)| **seq > next && f.header.keyframe)
            .map(|(seq, _)| *seq);
        match later_keyframe {
            Some(seq) => {
                let keyframe = self.pending.remove(&seq).expect("found in the map above");
                self.restart_from(keyframe, out);
            }
            None => {
                if self.gap_since.is_none() {
                    self.gap_since = Some(now);
                }
            }
        }
    }

    fn drain_contiguous(&mut self, out: &mut Drained) {
        while let Some(next) = self.next {
            let Some(frame) = self.pending.remove(&next) else {
                break;
            };
            out.ready.push(frame);
            self.next = Some(next + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use brp_proto::FrameKind;

    use super::*;

    const WAIT: Duration = Duration::from_millis(200);

    fn f(seq: u64, keyframe: bool) -> IncomingFrame {
        IncomingFrame {
            header: FrameHeader {
                live_id: 1,
                preset_id: 1,
                kind: FrameKind::Video,
                seq,
                capture_ts_us: 0,
                keyframe,
                len: 0,
            },
            data: Vec::new(),
        }
    }

    fn seqs(d: &Drained) -> Vec<u64> {
        d.ready.iter().map(|x| x.header.seq).collect()
    }

    #[test]
    fn drops_non_keyframes_until_the_first_keyframe() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        assert!(r.push(f(5, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(6, true), t)), vec![6]);
        assert_eq!(seqs(&r.push(f(7, false), t)), vec![7]);
    }

    #[test]
    fn reorders_frames_that_complete_out_of_order() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(2, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(1, false), t)), vec![1, 2]);
    }

    #[test]
    fn a_later_keyframe_skips_the_gap_immediately() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(2, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(3, true), t)), vec![3]);
        assert!(
            r.push(f(1, false), t).ready.is_empty(),
            "late frame from before the jump is stale"
        );
        assert_eq!(seqs(&r.push(f(4, false), t)), vec![4]);
    }

    #[test]
    fn gap_past_the_wait_cap_requests_a_keyframe_and_resets() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        r.push(f(2, false), t);
        let early = r.poll(t + Duration::from_millis(100));
        assert!(early.ready.is_empty() && !early.request_keyframe);
        let late = r.poll(t + WAIT);
        assert!(late.ready.is_empty() && late.request_keyframe);
        assert!(r.push(f(1, false), t + WAIT).ready.is_empty());
        assert!(r.push(f(3, false), t + WAIT).ready.is_empty());
        assert_eq!(seqs(&r.push(f(4, true), t + WAIT)), vec![4]);
    }

    #[test]
    fn duplicates_and_stale_frames_are_dropped() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(0, true), t).ready.is_empty());
        r.push(f(1, false), t);
        assert!(r.push(f(1, false), t).ready.is_empty());
    }
}
