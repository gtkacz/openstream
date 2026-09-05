//! The participant window's egui chrome. Panels read the room snapshot and window-local state and
//! emit room commands; they never touch the room.

pub mod members;
pub mod own_lives;
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
    status::draw(ui, snapshot, ticket, state);
    own_lives::draw(ui, snapshot, state, &mut commands);
    members::draw(ui, snapshot, state, &mut commands);
    let tile_rects = tiles::draw(ui, snapshot, state, &mut commands);
    UiOutput {
        commands,
        tile_rects,
    }
}
