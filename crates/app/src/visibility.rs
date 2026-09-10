//! Whether a window's presentation can be skipped: minimized, occluded, or shrunk to nothing.
//! Pure state so it is tested without a real window; the winit-facing side only feeds it events
//! and reacts to its transitions.

/// A window's visibility, tracked from whichever signals the platform actually sends. A platform
/// that never reports one of these (occlusion support varies, and winit has no minimized event)
/// leaves that flag at its default of "not hidden", which is the conservative reading: we would
/// rather redraw a window we can't see than freeze one we can.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Visibility {
    occluded: bool,
    zero_sized: bool,
    minimized: bool,
}

impl Visibility {
    pub fn new() -> Self {
        Self::default()
    }

    /// True when presentation should be skipped: nothing would be shown, so drawing it wastes
    /// the GPU work.
    pub fn is_suspended(&self) -> bool {
        self.occluded || self.zero_sized || self.minimized
    }

    /// Updates the occlusion flag from a winit `Occluded` event. Returns true when the window
    /// just stopped being suspended, so the caller should request a fresh frame.
    pub fn set_occluded(&mut self, occluded: bool) -> bool {
        let was_suspended = self.is_suspended();
        self.occluded = occluded;
        was_suspended && !self.is_suspended()
    }

    /// Updates the zero-size flag from a `Resized` event's new size. Returns true on the same
    /// restored transition as `set_occluded`.
    pub fn set_size(&mut self, width: u32, height: u32) -> bool {
        let was_suspended = self.is_suspended();
        self.zero_sized = width == 0 || height == 0;
        was_suspended && !self.is_suspended()
    }

    /// Updates the minimized flag from `Window::is_minimized`, whose `None` means the platform
    /// does not report it; that is read as "not minimized" per the conservative default above.
    pub fn set_minimized(&mut self, minimized: Option<bool>) -> bool {
        let was_suspended = self.is_suspended();
        self.minimized = minimized.unwrap_or(false);
        was_suspended && !self.is_suspended()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_window_is_not_suspended() {
        assert!(!Visibility::new().is_suspended());
    }

    #[test]
    fn occlusion_alone_suspends_and_lifting_it_reports_restored() {
        let mut v = Visibility::new();
        assert!(!v.set_occluded(true), "becoming suspended is not a restore");
        assert!(v.is_suspended());
        assert!(v.set_occluded(false), "no longer occluded restores it");
        assert!(!v.is_suspended());
    }

    #[test]
    fn a_zero_sized_window_is_suspended_regardless_of_occlusion() {
        let mut v = Visibility::new();
        v.set_size(0, 480);
        assert!(v.is_suspended());
        assert!(v.set_size(1280, 720));
        assert!(!v.is_suspended());
    }

    #[test]
    fn unknown_minimized_state_is_treated_as_visible() {
        let mut v = Visibility::new();
        assert!(!v.set_minimized(None));
        assert!(!v.is_suspended());
    }

    #[test]
    fn restoring_requires_every_suspending_flag_to_clear() {
        let mut v = Visibility::new();
        v.set_occluded(true);
        v.set_size(0, 0);
        // Still suspended by size even though occlusion cleared.
        assert!(!v.set_occluded(false));
        assert!(v.is_suspended());
        assert!(v.set_size(800, 600));
        assert!(!v.is_suspended());
    }

    #[test]
    fn minimized_alone_suspends_a_visible_unoccluded_window() {
        let mut v = Visibility::new();
        assert!(!v.set_minimized(Some(true)));
        assert!(v.is_suspended());
        assert!(v.set_minimized(Some(false)));
        assert!(!v.is_suspended());
    }
}
