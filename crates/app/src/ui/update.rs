//! The update notice and button, drawn on the start screen and in the status bar. Kept apart from
//! `UiState`, which a room opening resets: a download in flight must survive that.

use brp_update::{RELEASES_URL, Release};

/// Decimal megabytes, the unit the progress line shows.
pub const BYTES_PER_MB: u64 = 1_000_000;
/// One progress event per this many bytes, so a 50 MB download does not wake the event loop
/// once per network chunk.
pub const PROGRESS_STEP_BYTES: u64 = BYTES_PER_MB;

/// Where a download stands: not started, in flight with a byte count, or failed with the message to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Downloading { received: u64, total: Option<u64> },
    Failed(String),
}

/// What the window knows about updates: the release the launch check found, whether this install
/// can be replaced, and how far a download has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateState {
    pub available: Option<Release>,
    /// The install is a release layout, so the button is offered; a dev build only gets the notice.
    pub can_apply: bool,
    pub phase: UpdatePhase,
}

impl UpdateState {
    pub fn new(can_apply: bool) -> Self {
        Self {
            available: None,
            can_apply,
            phase: UpdatePhase::Idle,
        }
    }

    /// The button was clicked: the release to download, or `None` while one is already in flight
    /// or nothing can be applied.
    pub fn start(&mut self) -> Option<Release> {
        if !self.can_apply || matches!(self.phase, UpdatePhase::Downloading { .. }) {
            return None;
        }
        let release = self.available.clone()?;
        self.phase = UpdatePhase::Downloading {
            received: 0,
            total: None,
        };
        Some(release)
    }

    /// Records bytes received; ignored unless a download is in flight.
    pub fn progress(&mut self, received: u64, total: Option<u64>) {
        if matches!(self.phase, UpdatePhase::Downloading { .. }) {
            self.phase = UpdatePhase::Downloading { received, total };
        }
    }

    /// The download or swap failed; the button returns so the user can retry.
    pub fn failed(&mut self, message: String) {
        self.phase = UpdatePhase::Failed(message);
    }
}

/// The notice text: the version, plus the releases page when this install cannot be replaced.
pub fn notice(release: &Release, can_apply: bool) -> String {
    if can_apply {
        format!("v{} is available", release.version)
    } else {
        format!("v{} is available at {RELEASES_URL}", release.version)
    }
}

/// The progress line in whole decimal megabytes, with the total when the server stated one.
pub fn progress_text(received: u64, total: Option<u64>) -> String {
    let received = received / BYTES_PER_MB;
    match total {
        Some(total) => format!("downloading {received} MB / {} MB", total / BYTES_PER_MB),
        None => format!("downloading {received} MB"),
    }
}

/// Draws the notice with the button, the progress line, or nothing when no release is known.
/// Returns true when the button was clicked.
pub fn draw(ui: &mut egui::Ui, state: &UpdateState) -> bool {
    let Some(release) = &state.available else {
        return false;
    };
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.weak(notice(release, state.can_apply));
        if !state.can_apply {
            return;
        }
        match &state.phase {
            UpdatePhase::Downloading { received, total } => {
                ui.weak(progress_text(*received, *total));
            }
            UpdatePhase::Idle | UpdatePhase::Failed(_) => {
                if ui.button("Update and restart").clicked() {
                    clicked = true;
                }
            }
        }
    });
    if let UpdatePhase::Failed(message) = &state.phase {
        ui.colored_label(egui::Color32::LIGHT_RED, message);
    }
    clicked
}

#[cfg(test)]
mod tests {
    use brp_update::Version;

    use super::*;

    fn release() -> Release {
        Release {
            version: Version(0, 6, 0),
            tag: "v0.6.0".into(),
        }
    }

    #[test]
    fn start_hands_out_the_release_once_until_the_download_settles() {
        let mut state = UpdateState::new(true);
        assert_eq!(state.start(), None, "nothing available yet");
        state.available = Some(release());
        assert_eq!(state.start(), Some(release()));
        assert!(matches!(state.phase, UpdatePhase::Downloading { .. }));
        assert_eq!(state.start(), None, "one download at a time");
        state.progress(5, Some(10));
        assert_eq!(
            state.phase,
            UpdatePhase::Downloading {
                received: 5,
                total: Some(10)
            }
        );
        state.failed("network".into());
        assert_eq!(state.phase, UpdatePhase::Failed("network".into()));
        assert_eq!(state.start(), Some(release()), "a failure allows a retry");
    }

    #[test]
    fn without_a_release_layout_nothing_can_be_started() {
        let mut state = UpdateState::new(false);
        state.available = Some(release());
        assert_eq!(state.start(), None);
        assert_eq!(state.phase, UpdatePhase::Idle);
    }

    #[test]
    fn progress_outside_a_download_is_ignored() {
        let mut state = UpdateState::new(true);
        state.progress(1, None);
        assert_eq!(state.phase, UpdatePhase::Idle);
    }

    #[test]
    fn the_notice_names_the_version_and_points_at_the_page_when_nothing_can_be_applied() {
        assert_eq!(notice(&release(), true), "v0.6.0 is available");
        assert_eq!(
            notice(&release(), false),
            format!("v0.6.0 is available at {RELEASES_URL}")
        );
    }

    #[test]
    fn progress_text_truncates_to_whole_megabytes() {
        assert_eq!(
            progress_text(12_999_999, Some(41_000_000)),
            "downloading 12 MB / 41 MB"
        );
        assert_eq!(progress_text(999_999, None), "downloading 0 MB");
    }
}
