//! The participant window's egui chrome. Panels read the room snapshot and window-local state and
//! emit room commands; they never touch the room.

pub mod members;
pub mod own_lives;
pub mod picker;
pub mod start;
pub mod state;
pub mod status;
pub mod tiles;

use brp_room::RoomSnapshot;

use crate::commands::RoomCommand;
use crate::render::tiles::TileKey;
use state::UiState;

/// Everything a completed egui pass produced: commands to apply to the room, and where the video
/// renderer should place each watched live's frame.
#[derive(Debug, Default)]
pub struct UiOutput {
    pub commands: Vec<RoomCommand>,
    /// Where the video renderer draws each watched live, in egui points.
    pub tile_rects: Vec<(TileKey, egui::Rect)>,
}

/// Panels are declared outermost first; the central panel takes what remains.
///
/// Takes the root [`egui::Ui`] for the pass (as built by [`egui::Context::run_ui`]) rather than
/// the [`egui::Context`] itself: egui 0.36 attaches top-level panels to a [`egui::Ui`], not a
/// [`egui::Context`].
///
/// egui may run the closure that calls this more than once per frame; the caller keeps the last
/// output, so commands from an earlier pass are discarded.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
) -> UiOutput {
    let mut commands = Vec::new();
    status::draw(ui, snapshot, ticket, state, &mut commands);
    own_lives::draw(ui, snapshot, state, &mut commands);
    members::draw(ui, snapshot, state, &mut commands);
    let tile_rects = tiles::draw(ui, snapshot, state, &mut commands);
    picker::draw(ui.ctx(), state, &mut commands);
    UiOutput {
        commands,
        tile_rects,
    }
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
        ui.add(egui::Slider::new(&mut value, 0.0..=1.0).show_value(false));
        let mut muted = gain == 0.0;
        if ui.toggle_value(&mut muted, "mute").changed() {
            value = toggled_gain(muted);
        }
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
