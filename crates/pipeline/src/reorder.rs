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
    gap: Option<Gap>,
}

/// The wait for whatever `pending` is queued behind: frame `next` while decoding, a keyframe while
/// `next` is `None`. Keyed by that state so a gap that closes while a later one is already open
/// gets its own timer instead of inheriting the earlier one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gap {
    behind: Option<u64>,
    since: Instant,
}

impl Reorder {
    pub fn new(max_wait: Duration) -> Self {
        Self {
            max_wait,
            next: None,
            pending: BTreeMap::new(),
            gap: None,
        }
    }

    pub fn push(&mut self, frame: IncomingFrame, now: Instant) -> Drained {
        let mut out = Drained::default();
        match self.next {
            None if frame.header.keyframe => self.restart_from(frame, &mut out),
            // Every frame travels on its own QUIC stream, so the small frames after a keyframe
            // routinely finish ahead of it; they are held for `restart_from` rather than dropped.
            None => {
                self.pending.insert(frame.header.seq, frame);
            }
            Some(next) => {
                if frame.header.seq < next || self.pending.contains_key(&frame.header.seq) {
                    return out;
                }
                self.pending.insert(frame.header.seq, frame);
                self.drain(&mut out);
            }
        }
        self.note_gap(now);
        self.expire(now, &mut out);
        out
    }

    pub fn poll(&mut self, now: Instant) -> Drained {
        let mut out = Drained::default();
        self.expire(now, &mut out);
        out
    }

    /// Runs on every push as well as on poll: with frames arriving steadily the decode loop never
    /// idles long enough to poll, and a gap checked only there would never time out.
    fn expire(&mut self, now: Instant, out: &mut Drained) {
        if let Some(gap) = self.gap
            && now.duration_since(gap.since) >= self.max_wait
        {
            self.pending.clear();
            self.gap = None;
            self.next = None;
            out.request_keyframe = true;
        }
    }

    fn note_gap(&mut self, now: Instant) {
        if self.pending.is_empty() {
            self.gap = None;
        } else if self.gap.is_none_or(|gap| gap.behind != self.next) {
            self.gap = Some(Gap {
                behind: self.next,
                since: now,
            });
        }
    }

    /// A keyframe makes everything before it irrelevant; decoding resumes from it.
    fn restart_from(&mut self, keyframe: IncomingFrame, out: &mut Drained) {
        let seq = keyframe.header.seq;
        self.pending = self.pending.split_off(&(seq + 1));
        self.next = Some(seq + 1);
        out.ready.push(keyframe);
        self.drain_contiguous(out);
    }

    fn drain(&mut self, out: &mut Drained) {
        self.drain_contiguous(out);
        let Some(next) = self.next else { return };
        let later_keyframe = self
            .pending
            .iter()
            .find(|(seq, f)| **seq > next && f.header.keyframe)
            .map(|(seq, _)| *seq);
        if let Some(seq) = later_keyframe {
            let keyframe = self.pending.remove(&seq).expect("found in the map above");
            self.restart_from(keyframe, out);
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
    fn frames_that_finish_ahead_of_the_first_keyframe_are_kept_for_it() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        assert!(r.push(f(1, false), t).ready.is_empty());
        assert!(r.push(f(2, false), t).ready.is_empty());
        assert_eq!(seqs(&r.push(f(0, true), t)), vec![0, 1, 2]);
        assert_eq!(seqs(&r.push(f(3, false), t)), vec![3]);
    }

    #[test]
    fn waiting_on_a_keyframe_past_the_cap_asks_for_one() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        assert!(!r.push(f(1, false), t).request_keyframe);
        let late = r.push(f(2, false), t + WAIT);
        assert!(late.ready.is_empty() && late.request_keyframe);
        // The held frames went with the reset: only the keyframe itself comes out.
        assert_eq!(seqs(&r.push(f(0, true), t + WAIT)), vec![0]);
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
    fn a_gap_times_out_while_later_frames_keep_arriving() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        assert!(r.push(f(2, false), t).ready.is_empty());
        let mid = r.push(f(3, false), t + Duration::from_millis(100));
        assert!(mid.ready.is_empty() && !mid.request_keyframe);
        let late = r.push(f(4, false), t + WAIT);
        assert!(late.ready.is_empty() && late.request_keyframe);
        assert_eq!(seqs(&r.push(f(5, true), t + WAIT)), vec![5]);
    }

    #[test]
    fn a_gap_that_opens_after_an_earlier_one_closed_gets_its_own_timer() {
        let mut r = Reorder::new(WAIT);
        let t = Instant::now();
        r.push(f(0, true), t);
        r.push(f(2, false), t);
        let later = t + Duration::from_millis(150);
        r.push(f(4, false), later);
        // Closing the gap at 1 leaves 4 waiting on 3: a new gap, timed from now.
        assert_eq!(seqs(&r.push(f(1, false), later)), vec![1, 2]);
        assert!(!r.poll(t + WAIT).request_keyframe);
        assert!(r.poll(later + WAIT).request_keyframe);
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
