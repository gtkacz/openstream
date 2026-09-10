//! Holds an encoder to a preset frame rate below the capture rate, and backs off that rate under
//! sustained conversion/encoding overload.

use brp_proto::constants::PACER_JITTER_TOLERANCE;

/// Consecutive over-budget samples required before the admitted rate steps down. Requiring a run
/// rather than reacting to a single slow frame keeps a brief spike from changing the cadence.
const OVERLOAD_STREAK_THRESHOLD: u32 = 5;
/// Consecutive under-budget samples required before the admitted rate steps back up. Longer than
/// the overload streak on purpose: recovery is gradual, so a rate that just dropped does not
/// immediately climb back and re-trigger the same overload.
const RECOVERY_STREAK_THRESHOLD: u32 = 30;
/// The admitted rate never backs off below this floor (or the ceiling, if that is lower), so an
/// overloaded encoder keeps producing occasional frames instead of stalling entirely.
const MIN_FPS_FLOOR: u32 = 5;

/// Admits or skips captured frames so an encoder runs at `fps` while the capture runs faster.
/// Due times are phase-preserving: an admitted frame advances the next due time by one interval
/// from the slot it filled, not from its own timestamp, so the cadence stays at exactly `fps`
/// instead of drifting with each admission. A frame within `PACER_JITTER_TOLERANCE` of its due
/// time is admitted; a frame arriving more than one interval late is a stall and re-anchors the
/// next due time to its own timestamp, so a burst after it is not admitted whole.
///
/// [`Pacer::record_duration`] feeds back how long conversion and encoding took for the most
/// recently admitted frame. Sustained overload steps the effective admission rate down from the
/// configured ceiling in increments, with hysteresis against oscillation; sustained headroom
/// steps it back up, one increment at a time, up to the ceiling.
#[derive(Debug, Clone)]
pub struct Pacer {
    ceiling_fps: u32,
    ceiling_interval_us: u64,
    current_fps: u32,
    interval_us: u64,
    next_due_us: Option<u64>,
    overload_streak: u32,
    healthy_streak: u32,
}

impl Pacer {
    pub fn new(fps: u32) -> Self {
        let fps = fps.max(1);
        let interval_us = 1_000_000 / u64::from(fps);
        Self {
            ceiling_fps: fps,
            ceiling_interval_us: interval_us,
            current_fps: fps,
            interval_us,
            next_due_us: None,
            overload_streak: 0,
            healthy_streak: 0,
        }
    }

    /// The rate frames are currently admitted at: the ceiling, unless sustained overload has
    /// backed it off.
    pub fn current_fps(&self) -> u32 {
        self.current_fps
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

    /// Feeds the conversion+encoding time spent on the most recently admitted frame. Compares
    /// against the ceiling's frame budget, not the currently backed-off one, so recovery is judged
    /// against the same target the rate is climbing back toward.
    pub fn record_duration(&mut self, duration_us: u64) {
        if duration_us > self.ceiling_interval_us {
            self.healthy_streak = 0;
            self.overload_streak += 1;
            if self.overload_streak >= OVERLOAD_STREAK_THRESHOLD {
                self.overload_streak = 0;
                self.step_down();
            }
        } else {
            self.overload_streak = 0;
            self.healthy_streak += 1;
            if self.healthy_streak >= RECOVERY_STREAK_THRESHOLD {
                self.healthy_streak = 0;
                self.step_up();
            }
        }
    }

    fn step_fps(&self) -> u32 {
        (self.ceiling_fps / 4).max(1)
    }

    fn step_down(&mut self) {
        let floor = MIN_FPS_FLOOR.min(self.ceiling_fps).max(1);
        let next = self.current_fps.saturating_sub(self.step_fps()).max(floor);
        self.set_current_fps(next);
    }

    fn step_up(&mut self) {
        let next = self
            .current_fps
            .saturating_add(self.step_fps())
            .min(self.ceiling_fps);
        self.set_current_fps(next);
    }

    fn set_current_fps(&mut self, fps: u32) {
        if fps == self.current_fps {
            return;
        }
        self.current_fps = fps;
        self.interval_us = 1_000_000 / u64::from(fps.max(1));
    }
}

#[cfg(test)]
mod tests {
    use super::{OVERLOAD_STREAK_THRESHOLD, Pacer, RECOVERY_STREAK_THRESHOLD};

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

    #[test]
    fn sustained_overload_steps_the_rate_down_after_the_streak_threshold() {
        let mut pacer = Pacer::new(60);
        assert_eq!(pacer.current_fps(), 60);
        // Ceiling budget at 60 fps is 16_667us; feed one fewer than the streak threshold of
        // over-budget samples and the rate must not have moved yet.
        for _ in 0..OVERLOAD_STREAK_THRESHOLD - 1 {
            pacer.record_duration(20_000);
        }
        assert_eq!(pacer.current_fps(), 60, "a short streak must not step down");
        pacer.record_duration(20_000);
        assert_eq!(
            pacer.current_fps(),
            45,
            "the streak threshold steps down by one increment"
        );
    }

    #[test]
    fn a_single_slow_frame_amid_healthy_ones_does_not_step_down() {
        let mut pacer = Pacer::new(60);
        for _ in 0..OVERLOAD_STREAK_THRESHOLD - 1 {
            pacer.record_duration(20_000);
        }
        pacer.record_duration(1_000); // resets the overload streak
        assert_eq!(pacer.current_fps(), 60);
        for _ in 0..OVERLOAD_STREAK_THRESHOLD - 1 {
            pacer.record_duration(20_000);
        }
        assert_eq!(
            pacer.current_fps(),
            60,
            "the interrupted streak must not carry over"
        );
    }

    #[test]
    fn recovery_requires_a_longer_healthy_streak_and_climbs_back_to_the_ceiling() {
        let mut pacer = Pacer::new(60);
        for _ in 0..OVERLOAD_STREAK_THRESHOLD {
            pacer.record_duration(20_000);
        }
        assert_eq!(pacer.current_fps(), 45);
        for _ in 0..RECOVERY_STREAK_THRESHOLD - 1 {
            pacer.record_duration(1_000);
        }
        assert_eq!(
            pacer.current_fps(),
            45,
            "recovery needs the full healthy streak, not just one short of it"
        );
        pacer.record_duration(1_000);
        assert_eq!(
            pacer.current_fps(),
            60,
            "one full increment back toward the ceiling"
        );
    }

    #[test]
    fn the_rate_does_not_oscillate_around_a_borderline_workload() {
        let mut pacer = Pacer::new(60);
        // Alternating over/under-budget samples never form a streak long enough to move the rate.
        for i in 0..200 {
            let duration = if i % 2 == 0 { 20_000 } else { 1_000 };
            pacer.record_duration(duration);
        }
        assert_eq!(pacer.current_fps(), 60);
    }

    #[test]
    fn the_rate_never_backs_off_below_the_floor_or_above_the_ceiling() {
        let mut pacer = Pacer::new(20); // floor is MIN_FPS_FLOOR (5), below the ceiling
        for _ in 0..OVERLOAD_STREAK_THRESHOLD * 10 {
            pacer.record_duration(1_000_000);
        }
        assert_eq!(
            pacer.current_fps(),
            5,
            "steps down to the floor and no further"
        );
        for _ in 0..RECOVERY_STREAK_THRESHOLD * 10 {
            pacer.record_duration(0);
        }
        assert_eq!(pacer.current_fps(), 20, "never climbs past the ceiling");
    }
}
