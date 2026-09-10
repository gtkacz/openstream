//! What the panels ask the room to do. Panels only emit these; the window applies them after the
//! egui pass, so widget code never holds the room.

use brp_audio::AudioSelection;
use brp_capture::SourceId;
use brp_proto::{Preset, SourceKind};
use iroh::PublicKey;

use crate::render::tiles::TileKey;

/// A command a panel wants applied to the room, queued and drained after the egui pass.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomCommand {
    /// Starts a watch, or switches its preset when the key is already watched.
    Watch { key: TileKey, preset_id: u32 },
    /// Stops watching a live.
    Unwatch(TileKey),
    /// Starts a new live of this kind. Without a source the window asks the room for the
    /// platform's listing and either starts at once or opens the picker.
    Share {
        kind: SourceKind,
        source: Option<SourceId>,
    },
    /// Stops publishing the live with this id.
    StopLive(u32),
    /// Replaces the preset list offered for a live.
    SetPresets { live_id: u32, presets: Vec<Preset> },
    /// Turns this participant's audio on or off for every live at once.
    SetAudio(bool),
    /// Sets how loud one publisher plays, 0 to 1.
    SetVolume { publisher: PublicKey, gain: f32 },
    /// Silences all playback without touching the per-publisher gains.
    SetMasterMute(bool),
    /// Replaces which applications this participant's audio carries.
    SetAudioApplications(AudioSelection),
    /// Lists what is playing audio and opens the application picker, or refreshes an open one.
    ChooseApplications,
}

/// A command a panel wants applied to the windows, not the room: which live gets its own window
/// and whether that window is fullscreen. Queued and drained after the egui pass like
/// [`RoomCommand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    /// Leaves the current room and returns the main window to its start screen.
    LeaveRoom,
    /// Moves the live out of the grid into a new window.
    PopOut(TileKey),
    /// Moves the live into a new window that starts borderless fullscreen.
    PopOutFullscreen(TileKey),
    /// Flips the live's pop-out between borderless fullscreen and windowed.
    ToggleFullscreen(TileKey),
    /// Closes the live's pop-out and puts it back in the grid.
    ReturnToGrid(TileKey),
}
