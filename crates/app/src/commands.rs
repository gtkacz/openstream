//! What the panels ask the room to do. Panels only emit these; the window applies them after the
//! egui pass, so widget code never holds the room.

use brp_proto::{Preset, SourceKind};

use crate::render::tiles::TileKey;

/// A command a panel wants applied to the room, queued and drained after the egui pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomCommand {
    /// Starts a watch, or switches its preset when the key is already watched.
    Watch { key: TileKey, preset_id: u32 },
    /// Stops watching a live.
    Unwatch(TileKey),
    /// Opens the portal picker for a new live of this kind.
    Share(SourceKind),
    /// Stops publishing the live with this id.
    StopLive(u32),
    /// Replaces the preset list offered for a live.
    SetPresets { live_id: u32, presets: Vec<Preset> },
}
