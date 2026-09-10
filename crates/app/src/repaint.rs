//! Per-window repaint deadlines. Each window's egui pass may ask for its next frame at some
//! instant (a cursor blink, an animation); that deadline must wake only that window, not every
//! window sharing the event loop.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

/// The earliest requested repaint of every window that has one pending, keyed by window id.
#[derive(Debug)]
pub struct RepaintSchedule<Id> {
    deadlines: HashMap<Id, Instant>,
}

impl<Id: Copy + Eq + Hash> RepaintSchedule<Id> {
    pub fn new() -> Self {
        Self {
            deadlines: HashMap::new(),
        }
    }

    /// Records `id`'s next requested repaint, keeping the earlier deadline if one was already
    /// pending for it.
    pub fn note(&mut self, id: Id, deadline: Instant) {
        self.deadlines
            .entry(id)
            .and_modify(|existing| *existing = (*existing).min(deadline))
            .or_insert(deadline);
    }

    /// Forgets `id`'s pending deadline, if any: the window closed or no longer needs waking.
    pub fn remove(&mut self, id: Id) {
        self.deadlines.remove(&id);
    }

    /// The earliest deadline across every window, for the event loop to wait until.
    pub fn next_wake(&self) -> Option<Instant> {
        self.deadlines.values().min().copied()
    }

    /// Removes and returns every window whose deadline is at or before `now`.
    pub fn due(&mut self, now: Instant) -> Vec<Id> {
        let due: Vec<Id> = self
            .deadlines
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            self.deadlines.remove(id);
        }
        due
    }
}

impl<Id: Copy + Eq + Hash> Default for RepaintSchedule<Id> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_windows_past_their_own_deadline_come_due() {
        let now = Instant::now();
        let mut schedule = RepaintSchedule::new();
        schedule.note(1u32, now + Duration::from_millis(10));
        schedule.note(2u32, now + Duration::from_millis(1000));
        let mut due = schedule.due(now + Duration::from_millis(20));
        due.sort_unstable();
        assert_eq!(due, [1]);
        assert_eq!(
            schedule.next_wake(),
            Some(now + Duration::from_millis(1000))
        );
    }

    #[test]
    fn noting_a_later_deadline_does_not_clobber_an_earlier_pending_one() {
        let now = Instant::now();
        let mut schedule = RepaintSchedule::new();
        schedule.note(1u32, now + Duration::from_millis(500));
        schedule.note(1u32, now + Duration::from_millis(50));
        // A later note for the same window must not push the deadline back out.
        schedule.note(1u32, now + Duration::from_millis(900));
        assert_eq!(schedule.next_wake(), Some(now + Duration::from_millis(50)));
    }

    #[test]
    fn due_windows_are_consumed_and_removed_windows_never_come_due() {
        let now = Instant::now();
        let mut schedule = RepaintSchedule::new();
        schedule.note(1u32, now);
        schedule.note(2u32, now);
        schedule.remove(2u32);
        assert_eq!(schedule.due(now), [1]);
        assert!(schedule.due(now).is_empty(), "already consumed");
        assert_eq!(schedule.next_wake(), None);
    }
}
