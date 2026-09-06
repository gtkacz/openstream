//! Orders audio packets by sequence and delays playout by an adaptive depth. QUIC streams are
//! reliable, so packets are late, never lost: the buffer never stalls the mixer waiting for one.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use brp_proto::EncodedFrame;
use brp_proto::constants::{
    AUDIO_PACKET_DURATION, JITTER_INITIAL_DEPTH, JITTER_MAX_DEPTH, JITTER_SHRINK_AFTER, JITTER_STEP,
};

pub enum Slot {
    Packet(EncodedFrame),
    /// The slot's packet is missing or the stream is idle; play one packet of silence.
    Silence,
    /// Playout has not started: not enough audio is queued yet.
    Waiting,
}

pub struct JitterBuffer {
    packets: BTreeMap<u64, EncodedFrame>,
    next_seq: u64,
    started: bool,
    target: Duration,
    /// The last late arrival, or the last shrink: the quiet period is measured from here.
    quiet_since: Instant,
    late: u64,
    trimmed: u64,
}

impl JitterBuffer {
    pub fn new(now: Instant) -> Self {
        Self {
            packets: BTreeMap::new(),
            next_seq: 0,
            started: false,
            target: JITTER_INITIAL_DEPTH,
            quiet_since: now,
            late: 0,
            trimmed: 0,
        }
    }

    pub fn push(&mut self, packet: EncodedFrame, now: Instant) {
        if self.started && packet.seq < self.next_seq {
            self.late += 1;
            self.target = (self.target + JITTER_STEP).min(JITTER_MAX_DEPTH);
            self.quiet_since = now;
            return;
        }
        self.packets.insert(packet.seq, packet);
        // Two machines' clocks drift; audio piling up past twice the target is the symptom.
        while self.queued() > self.target * 2 {
            let Some(oldest) = self.packets.keys().next().copied() else {
                break;
            };
            self.packets.remove(&oldest);
            self.trimmed += 1;
            if let Some(first) = self.packets.keys().next() {
                self.next_seq = self.next_seq.max(*first);
            }
        }
    }

    pub fn pop(&mut self, now: Instant) -> Slot {
        // Shrinking happens even while re-priming, so a long idle stretch still relaxes the depth.
        if now.duration_since(self.quiet_since) >= JITTER_SHRINK_AFTER {
            self.target = (self.target.saturating_sub(JITTER_STEP)).max(JITTER_INITIAL_DEPTH);
            self.quiet_since = now;
        }
        if !self.started {
            if self.queued() < self.target {
                return Slot::Waiting;
            }
            self.started = true;
            self.next_seq = *self
                .packets
                .keys()
                .next()
                .expect("queued audio is non-empty");
        }
        if let Some(packet) = self.packets.remove(&self.next_seq) {
            self.next_seq += 1;
            return Slot::Packet(packet);
        }
        if self.packets.is_empty() {
            // Idle publisher or stalled stream: re-prime to full depth when audio returns.
            self.started = false;
            return Slot::Silence;
        }
        // A gap with audio queued behind it: wait until the queue is a full target deep, then
        // presume the missing packet late and move on.
        if self.queued() >= self.target {
            self.next_seq += 1;
        }
        Slot::Silence
    }

    pub fn queued(&self) -> Duration {
        AUDIO_PACKET_DURATION * self.packets.len() as u32
    }

    pub fn target(&self) -> Duration {
        self.target
    }

    pub fn late(&self) -> u64 {
        self.late
    }

    pub fn trimmed(&self) -> u64 {
        self.trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(seq: u64) -> EncodedFrame {
        EncodedFrame {
            seq,
            capture_ts_us: seq * 20_000,
            keyframe: true,
            data: vec![seq as u8],
        }
    }

    fn seq_of(slot: Slot) -> Option<u64> {
        match slot {
            Slot::Packet(p) => Some(p.seq),
            _ => None,
        }
    }

    #[test]
    fn playout_starts_once_the_target_depth_is_queued() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        assert!(matches!(jb.pop(t0), Slot::Waiting));
        jb.push(packet(0), t0);
        jb.push(packet(1), t0);
        assert!(
            matches!(jb.pop(t0), Slot::Waiting),
            "40 ms is under the 60 ms target"
        );
        jb.push(packet(2), t0);
        assert_eq!(seq_of(jb.pop(t0)), Some(0));
        assert_eq!(seq_of(jb.pop(t0)), Some(1));
        assert_eq!(seq_of(jb.pop(t0)), Some(2));
    }

    #[test]
    fn a_packet_arriving_after_its_slot_is_late_and_grows_the_target() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        for seq in [0, 2, 3] {
            jb.push(packet(seq), t0);
        }
        assert_eq!(seq_of(jb.pop(t0)), Some(0));
        // Gap at 1 with 40 ms queued behind it: under target, so wait.
        assert!(matches!(jb.pop(t0), Slot::Silence));
        jb.push(packet(4), t0);
        // Now 60 ms is queued behind the gap: skip it.
        assert!(matches!(jb.pop(t0), Slot::Silence));
        assert_eq!(seq_of(jb.pop(t0)), Some(2));
        jb.push(packet(1), t0);
        assert_eq!(jb.late(), 1);
        assert_eq!(jb.target(), JITTER_INITIAL_DEPTH + JITTER_STEP);
        assert_eq!(seq_of(jb.pop(t0)), Some(3));
    }

    #[test]
    fn the_target_is_capped_and_shrinks_after_a_quiet_period() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        for seq in 0..3 {
            jb.push(packet(seq), t0);
        }
        for _ in 0..3 {
            jb.pop(t0);
        }
        // Twenty arrivals of an already-played slot: every one is late.
        for _ in 0..20 {
            jb.push(packet(0), t0);
        }
        assert_eq!(jb.target(), JITTER_MAX_DEPTH);
        jb.pop(t0 + JITTER_SHRINK_AFTER);
        assert_eq!(jb.target(), JITTER_MAX_DEPTH - JITTER_STEP);
        jb.pop(t0 + JITTER_SHRINK_AFTER + Duration::from_millis(1));
        assert_eq!(
            jb.target(),
            JITTER_MAX_DEPTH - JITTER_STEP,
            "one step per quiet period"
        );
        jb.pop(t0 + 2 * JITTER_SHRINK_AFTER);
        assert_eq!(jb.target(), JITTER_MAX_DEPTH - 2 * JITTER_STEP);
    }

    #[test]
    fn queued_audio_over_twice_the_target_trims_the_oldest() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        for seq in 0..7 {
            jb.push(packet(seq), t0);
        }
        // Seven packets are 140 ms, over twice the 60 ms target: the oldest went.
        assert_eq!(jb.trimmed(), 1);
        assert_eq!(seq_of(jb.pop(t0)), Some(1));
    }

    #[test]
    fn an_empty_buffer_yields_silence_and_re_primes_when_packets_return() {
        let t0 = Instant::now();
        let mut jb = JitterBuffer::new(t0);
        for seq in 0..3 {
            jb.push(packet(seq), t0);
        }
        for _ in 0..3 {
            jb.pop(t0);
        }
        assert!(matches!(jb.pop(t0), Slot::Silence));
        assert!(
            matches!(jb.pop(t0), Slot::Waiting),
            "re-priming after running dry"
        );
        // The publisher went quiet and resumes with the next sequence number: nothing is late.
        jb.push(packet(3), t0);
        assert!(matches!(jb.pop(t0), Slot::Waiting));
        jb.push(packet(4), t0);
        jb.push(packet(5), t0);
        assert_eq!(seq_of(jb.pop(t0)), Some(3));
        assert_eq!(jb.late(), 0);
    }
}
