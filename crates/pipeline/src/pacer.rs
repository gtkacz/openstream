//! Holds an encoder to a preset frame rate below the capture rate.

/// Admits or skips captured frames so an encoder runs at `fps` while the capture runs faster.
/// A quarter-interval tolerance absorbs compositor jitter without letting the next capture through.
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
        let tolerance = self.interval_us / 4;
        if let Some(due) = self.next_due_us
            && capture_ts_us + tolerance < due
        {
            return false;
        }
        self.next_due_us = Some(capture_ts_us + self.interval_us);
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
    fn early_jitter_within_a_quarter_interval_is_admitted() {
        let mut pacer = Pacer::new(30);
        assert!(pacer.admit(0));
        assert!(!pacer.admit(16_667));
        assert!(
            pacer.admit(33_333 - 5_000),
            "5 ms early is inside the tolerance"
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
