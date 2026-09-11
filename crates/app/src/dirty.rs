//! Which watched streams have a frame the window has not yet drawn. Shared between the room's
//! frame-notify callback (called from decode threads) and the window (drained on redraw), so a
//! burst of frames on one stream coalesces into a single pending wakeup instead of one event per
//! frame.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::render::tiles::TileKey;

/// The set of streams with an undrawn frame. Bounded by the number of active streams, not by how
/// many frames arrived, because marking an already-dirty stream is a no-op.
#[derive(Default)]
pub struct DirtyStreams {
    pending: Mutex<HashSet<TileKey>>,
}

impl DirtyStreams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `key` dirty. Returns true when this is the transition from nothing pending to
    /// something pending: the caller should wake the event loop exactly then, so a burst of marks
    /// before the next drain sends only one wakeup.
    pub fn mark(&self, key: TileKey) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let was_empty = pending.is_empty();
        pending.insert(key);
        was_empty
    }

    /// Takes every pending key, leaving the set empty. A `mark` racing with this drain either
    /// lands before (and is taken here) or after (and reports `was_empty` again, so its wakeup is
    /// not lost).
    pub fn drain(&self) -> HashSet<TileKey> {
        std::mem::take(&mut self.pending.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn key(n: u8) -> TileKey {
        (SecretKey::from_bytes(&[n; 32]).public(), u32::from(n))
    }

    #[test]
    fn the_first_mark_wakes_and_later_marks_before_a_drain_do_not() {
        let dirty = DirtyStreams::new();
        assert!(dirty.mark(key(1)));
        assert!(!dirty.mark(key(1)), "already dirty, no second wakeup");
        assert!(
            !dirty.mark(key(2)),
            "still nothing drained since the first mark"
        );
    }

    #[test]
    fn draining_empties_the_set_and_the_next_mark_wakes_again() {
        let dirty = DirtyStreams::new();
        dirty.mark(key(1));
        dirty.mark(key(2));
        let drained = dirty.drain();
        assert_eq!(drained, HashSet::from([key(1), key(2)]));
        assert!(dirty.drain().is_empty(), "a second drain finds nothing new");
        assert!(
            dirty.mark(key(3)),
            "empty again, so marking wakes once more"
        );
    }

    #[test]
    fn pending_state_is_bounded_by_distinct_streams_not_frame_count() {
        let dirty = DirtyStreams::new();
        for _ in 0..1000 {
            dirty.mark(key(1));
        }
        assert_eq!(dirty.drain(), HashSet::from([key(1)]));
    }

    #[test]
    fn concurrent_marks_from_different_streams_all_survive_to_the_next_drain() {
        use std::thread;
        let dirty = std::sync::Arc::new(DirtyStreams::new());
        let handles: Vec<_> = (0..8u8)
            .map(|n| {
                let dirty = dirty.clone();
                thread::spawn(move || dirty.mark(key(n)))
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let drained = dirty.drain();
        assert_eq!(drained.len(), 8);
    }
}
