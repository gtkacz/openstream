//! Holds an encoder to a preset frame rate below the capture rate.

use brp_proto::constants::PACER_JITTER_TOLERANCE;

/// Admits or skips captured frames so an encoder runs at `fps` while the capture runs faster.
/// Due times are phase-preserving: an admitted frame advances the next due time by one interval
/// from the slot it filled, not from its own timestamp, so the cadence stays at exactly `fps`
/// instead of drifting with each admission. A frame within `PACER_JITTER_TOLERANCE` of its due
/// time is admitted; a frame arriving more than one interval late is a stall and re-anchors the
/// next due time to its own timestamp, so a burst after it is not admitted whole.
#[derive(Debug, Clone)]
pub struct Pacer {
    interval_us: u64,
    next_due_us: Option<u64>,
}

impl Pacer {
    pub fn new(fps: u32) -> Self {
        Self {
            interval_us: 1_000_000 / u64::from(fps.max(1)),
            next_due_us: None,
        }
    }

    /// True when the frame should be encoded. `capture_ts_us` is the capture clock in microseconds.
    pub fn admit(&mut self, capture_ts_us: u64) -> bool {
        let tolerance = PACER_JITTER_TOLERANCE.as_micros() as u64;
        let Some(due) = self.next_due_us else {
            self.next_due_us = Some(capture_ts_us + self.interval_us);
            return true;
        };
        if capture_ts_us + tolerance < due {
            return false;
        }
        // Advancing from the due time rather than the frame keeps the cadence at exactly `fps`;
        // only a stall longer than one interval re-anchors, so a burst after it is not admitted whole.
        let stalled = capture_ts_us > due + self.interval_us;
        self.next_due_us = Some(if stalled {
            capture_ts_us + self.interval_us
        } else {
            due + self.interval_us
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Pacer;

    #[test]
    fn halves_a_sixty_hertz_capture_to_thirty() {
        let mut pacer = Pacer::new(30);
        let admitted: Vec<bool> = (0..6).map(|i| pacer.admit(i * 16_667)).collect();
        assert_eq!(admitted, [true, false, true, false, true, false]);
    }

    #[test]
    fn paces_sixty_hertz_to_forty_five_by_admitting_three_of_four() {
        let mut pacer = Pacer::new(45);
        let admitted: Vec<bool> = (0..8).map(|i| pacer.admit(i * 16_667)).collect();
        assert_eq!(admitted, [true, false, true, true, true, false, true, true]);
    }

    #[test]
    fn a_frame_within_the_jitter_tolerance_is_admitted() {
        let mut pacer = Pacer::new(30);
        assert!(pacer.admit(0));
        assert!(pacer.admit(33_333 - 500));
        assert!(
            !pacer.admit(33_333 - 500 + 16_667),
            "the following capture is still early"
        );
    }

    #[test]
    fn a_stall_admits_the_next_frame_and_paces_from_it() {
        let mut pacer = Pacer::new(30);
        assert!(pacer.admit(0));
        assert!(pacer.admit(500_000));
        assert!(!pacer.admit(516_667));
        assert!(pacer.admit(533_333));
    }

    #[test]
    fn first_frame_is_always_admitted() {
        assert!(Pacer::new(1).admit(123));
    }
}
