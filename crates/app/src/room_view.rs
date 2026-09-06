//! Everything the window holds once a room is open: the room handle, its last snapshot, the
//! watch handles that feed tiles, and the share in flight. The window delegates room commands here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_capture::{SourceId, SourceListing};
use brp_proto::SourceKind;
use brp_room::{Room, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

use crate::commands::RoomCommand;
use crate::render::GpuContext;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::ui::state::UiState;
use crate::window::AppEvent;

/// The window's view of an open room: the handle, the snapshot the panels draw from, the ticket
/// the status bar copies, the watch handles that feed tiles, and the share in flight.
pub struct RoomView {
    pub room: Arc<Room>,
    pub snapshot: RoomSnapshot,
    pub ticket: String,
    handles: HashMap<TileKey, WatchHandle>,
    /// Holds an `Arc<Room>` clone; the shutdown path aborts and awaits it before the leave.
    pub pending_share: Option<JoinHandle<()>>,
}

impl RoomView {
    pub fn new(room: Arc<Room>) -> Self {
        let snapshot = room.snapshot();
        let ticket = room.ticket().to_string();
        Self {
            room,
            snapshot,
            ticket,
            handles: HashMap::new(),
            pending_share: None,
        }
    }

    /// Re-snapshots when the room's version moved, drops handles and tiles of ended watches, and
    /// refreshes the rate meters.
    pub fn refresh(&mut self, state: &mut UiState, tiles: Option<&mut TileRenderer>) {
        if self.room.version() != self.snapshot.version {
            self.snapshot = self.room.snapshot();
        }
        // The relay address can arrive after the first snapshot without bumping the version.
        self.ticket = self.room.ticket().to_string();
        let live: HashSet<TileKey> = self
            .snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        self.handles.retain(|key, _| live.contains(key));
        if let Some(tiles) = tiles {
            tiles.retain(|key| live.contains(key));
        }
        state.refresh_rates(&self.snapshot, Instant::now());
    }

    /// Uploads the newest decoded frame of every watched live.
    pub fn upload_frames(&self, gpu: &GpuContext, tiles: &mut TileRenderer) {
        for (key, handle) in &self.handles {
            if let Some(frame) = handle.slot.try_take() {
                tiles.upload(&gpu.device, &gpu.queue, *key, &frame);
            }
        }
    }

    /// Applies the commands one egui pass produced. Errors land in the status line.
    pub fn apply(
        &mut self,
        commands: Vec<RoomCommand>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
    ) {
        if commands.is_empty() {
            return;
        }
        // `Room::watch` spawns its task with `tokio::spawn`, which needs a runtime on this thread.
        let _guard = runtime.enter();
        state.status.clear();
        for command in commands {
            let result = match command {
                RoomCommand::Watch { key, preset_id } => {
                    self.room.watch(key.0, key.1, preset_id).map(|handle| {
                        self.handles.insert(key, handle);
                    })
                }
                RoomCommand::Unwatch(key) => self.room.unwatch(key.0, key.1).map(|()| {
                    self.handles.remove(&key);
                }),
                RoomCommand::StopLive(live_id) => self.room.stop_live(live_id),
                RoomCommand::SetPresets { live_id, presets } => {
                    self.room.set_presets(live_id, presets)
                }
                RoomCommand::Share {
                    kind,
                    source: Some(source),
                } => {
                    self.share(kind, Some(source), runtime, proxy, state);
                    Ok(())
                }
                RoomCommand::Share { kind, source: None } => match self.room.sources(kind) {
                    Ok(SourceListing::PlatformPicker) => {
                        self.share(kind, None, runtime, proxy, state);
                        Ok(())
                    }
                    Ok(SourceListing::Choices(choices)) => {
                        if !state.open_picker(kind, choices) {
                            state.status = "a share is already in progress".into();
                        }
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
                RoomCommand::SetAudio(enabled) => {
                    self.room.set_audio(enabled);
                    Ok(())
                }
                RoomCommand::SetVolume { publisher, gain } => {
                    self.room.set_volume(publisher, gain);
                    Ok(())
                }
                RoomCommand::SetMasterMute(muted) => {
                    self.room.set_master_mute(muted);
                    Ok(())
                }
            };
            if let Err(error) = result {
                state.status = error.to_string();
            }
        }
    }

    fn share(
        &mut self,
        kind: SourceKind,
        source: Option<SourceId>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
    ) {
        if self.pending_share.is_some() {
            return;
        }
        let title = state.next_title(kind);
        state.share_pending = true;
        state.status.clear();
        let room = self.room.clone();
        let proxy = proxy.clone();
        self.pending_share = Some(runtime.spawn(async move {
            let outcome = room
                .start_live(kind, source, title)
                .await
                .map(|_live_id| ())
                .map_err(|error| error.to_string());
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
}
