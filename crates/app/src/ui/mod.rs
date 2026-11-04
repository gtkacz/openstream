//! The participant window's egui chrome. Panels read the room snapshot and window-local state and
//! emit room commands; they never touch the room.

pub mod applications;
pub mod members;
pub mod motion;
pub mod own_lives;
pub mod picker;
pub mod popout;
pub mod settings;
pub mod start;
pub mod state;
pub mod status;
pub mod theme;
pub mod tiles;
pub mod update;

use std::collections::HashSet;

use brp_room::RoomSnapshot;

use crate::commands::{RoomCommand, WindowCommand};
use crate::render::tiles::TileKey;
use state::UiState;

/// Everything a completed egui pass produced: commands to apply to the room, commands to apply to
/// the windows, and where the video renderer should place each watched live's frame.
#[derive(Debug, Default)]
pub struct UiOutput {
    pub commands: Vec<RoomCommand>,
    pub window_commands: Vec<WindowCommand>,
    /// The dock's Settings button was clicked.
    pub open_settings: bool,
    /// The dock's Update button was clicked.
    pub update_clicked: bool,
    /// Where the video renderer draws each watched live, in egui points.
    pub tile_rects: Vec<(TileKey, egui::Rect)>,
}

/// Panels are declared outermost first; the central panel takes what remains.
///
/// Takes the root [`egui::Ui`] for the pass (as built by [`egui::Context::run_ui`]) rather than
/// the [`egui::Context`] itself: egui 0.36 attaches top-level panels to a [`egui::Ui`], not a
/// [`egui::Context`].
///
/// `popped` lists the lives shown in their own windows, which the grid skips.
///
/// egui may run the closure that calls this more than once per frame; the caller keeps the last
/// output, so commands from an earlier pass are discarded.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    update_state: &update::UpdateState,
) -> UiOutput {
    let mut commands = Vec::new();
    let mut window_commands = Vec::new();
    let mut open_settings = false;
    let mut update_clicked = false;
    let compact = theme::is_compact(ui.ctx());
    motion::dissolve_in(ui.ctx(), "room", theme::GROUND);
    if !compact {
        members::draw(ui, snapshot, ticket, state, &mut commands);
    }
    status::draw(
        ui,
        snapshot,
        state,
        &mut commands,
        &mut open_settings,
        update_state,
        &mut update_clicked,
    );
    if compact {
        compact_sheet(ui, snapshot, ticket, state, &mut commands);
    }
    let tile_rects = tiles::draw(
        ui,
        snapshot,
        state,
        popped,
        &mut commands,
        &mut window_commands,
    );
    picker::draw(ui.ctx(), state, &mut commands);
    UiOutput {
        commands,
        window_commands,
        open_settings,
        update_clicked,
        tile_rects,
    }
}

/// Slides up over the bottom of the video, which keeps playing above it while a stream is chosen.
fn compact_sheet(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let height = (ui.ctx().content_rect().height() * 0.6).max(160.0);
    // Sized by a spring rather than slid, so the video above tracks the sheet's edge on every
    // frame, and a second tap reverses it from wherever it has reached.
    let shown = motion::follow(
        ui.ctx(),
        egui::Id::new("sheet-shown"),
        state.sheet_open,
        motion::SURFACE,
    );
    if shown <= 0.0 {
        return;
    }
    egui::Panel::bottom("sheet")
        .exact_size(height * shown)
        .frame(theme::rail())
        .show(ui, |ui| match state.sheet {
            state::CompactSheet::Room => members::contents(ui, snapshot, ticket, state, commands),
            state::CompactSheet::Share => own_lives::share_sheet(ui, snapshot, state, commands),
        });
}

/// A volume slider with a mute toggle. Returns the new gain when the user changed it. Mute sets
/// the gain to zero and unmute restores full volume; the room remembers nothing else.
pub fn volume_control(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    gain: f32,
) -> Option<f32> {
    let mut value = gain;
    ui.push_id(id_salt, |ui| {
        let mut muted = gain == 0.0;
        let glyph = if muted {
            theme::icons::MUTED
        } else {
            theme::icons::VOLUME
        };
        if theme::icon_toggle(ui, &mut muted, glyph, "Mute").changed() {
            value = toggled_gain(muted);
        }
        ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
    });
    (value != gain).then_some(value)
}

/// Mute is gain zero; unmute is full volume.
pub fn toggled_gain(muted: bool) -> f32 {
    if muted { 0.0 } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::toggled_gain;

    #[test]
    fn mute_is_silence_and_unmute_is_full_volume() {
        assert_eq!(toggled_gain(true), 0.0);
        assert_eq!(toggled_gain(false), 1.0);
    }
}
