//! Which watched lives are shown in a window of their own. Pure bookkeeping between window ids
//! and tile keys, generic over the id so it is tested with integers instead of winit windows.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::render::tiles::TileKey;

/// The pop-out windows: one tile key per window, and each key in at most one window.
#[derive(Debug)]
pub struct PopOuts<Id> {
    by_window: HashMap<Id, TileKey>,
}

impl<Id: Copy + Eq + Hash> PopOuts<Id> {
    pub fn new() -> Self {
        Self {
            by_window: HashMap::new(),
        }
    }

    /// Records that window `id` shows `key`. Returns false, and changes nothing, when the key is
    /// already popped out: a live is never in two windows.
    pub fn insert(&mut self, id: Id, key: TileKey) -> bool {
        if self.is_popped(key) {
            return false;
        }
        self.by_window.insert(id, key);
        true
    }

    pub fn key_of(&self, id: Id) -> Option<TileKey> {
        self.by_window.get(&id).copied()
    }

    pub fn window_of(&self, key: TileKey) -> Option<Id> {
        self.by_window
            .iter()
            .find_map(|(id, k)| (*k == key).then_some(*id))
    }

    pub fn is_popped(&self, key: TileKey) -> bool {
        self.window_of(key).is_some()
    }

    pub fn remove_window(&mut self, id: Id) -> Option<TileKey> {
        self.by_window.remove(&id)
    }

    pub fn remove_key(&mut self, key: TileKey) -> Option<Id> {
        let id = self.window_of(key)?;
        self.by_window.remove(&id);
        Some(id)
    }

    /// Forgets every window whose live is not in `watched` and returns those ids so the caller
    /// closes the windows.
    pub fn retain_watched(&mut self, watched: &HashSet<TileKey>) -> Vec<Id> {
        let closed: Vec<Id> = self
            .by_window
            .iter()
            .filter(|(_, key)| !watched.contains(key))
            .map(|(id, _)| *id)
            .collect();
        for id in &closed {
            self.by_window.remove(id);
        }
        closed
    }

    /// The keys the grid must skip.
    pub fn popped(&self) -> HashSet<TileKey> {
        self.by_window.values().copied().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.by_window.is_empty()
    }
}

impl<Id: Copy + Eq + Hash> Default for PopOuts<Id> {
    fn default() -> Self {
        Self::new()
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
    fn a_popped_key_is_found_from_both_sides() {
        let mut popouts = PopOuts::new();
        assert!(popouts.insert(1u32, key(1)));
        assert_eq!(popouts.key_of(1), Some(key(1)));
        assert_eq!(popouts.window_of(key(1)), Some(1));
        assert!(popouts.is_popped(key(1)));
        assert!(!popouts.is_popped(key(2)));
        assert_eq!(popouts.key_of(2), None);
    }

    #[test]
    fn a_key_is_never_in_two_windows() {
        let mut popouts = PopOuts::new();
        assert!(popouts.insert(1u32, key(1)));
        assert!(!popouts.insert(2, key(1)));
        assert_eq!(popouts.window_of(key(1)), Some(1));
        assert_eq!(popouts.key_of(2), None);
        assert_eq!(popouts.popped(), HashSet::from([key(1)]));
    }

    #[test]
    fn removing_by_window_and_by_key_are_inverses() {
        let mut popouts = PopOuts::new();
        popouts.insert(1u32, key(1));
        popouts.insert(2, key(2));
        assert_eq!(popouts.remove_window(1), Some(key(1)));
        assert_eq!(popouts.remove_window(1), None);
        assert_eq!(popouts.remove_key(key(2)), Some(2));
        assert_eq!(popouts.remove_key(key(2)), None);
        assert!(popouts.is_empty());
    }

    #[test]
    fn retain_watched_closes_exactly_the_windows_whose_live_ended() {
        let mut popouts = PopOuts::new();
        popouts.insert(1u32, key(1));
        popouts.insert(2, key(2));
        popouts.insert(3, key(3));
        let watched = HashSet::from([key(1), key(3), key(9)]);
        let mut closed = popouts.retain_watched(&watched);
        closed.sort_unstable();
        assert_eq!(closed, [2]);
        assert_eq!(popouts.popped(), HashSet::from([key(1), key(3)]));
        assert!(popouts.retain_watched(&watched).is_empty());
    }
}
