//! Everything the window holds once a room is open: the room handle, its last snapshot, the
//! watch handles that feed tiles, and the share in flight. The window delegates room commands here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_capture::{SourceId, SourceListing};
use brp_proto::{RoomTicket, SourceKind};
use brp_room::{Room, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

use crate::commands::RoomCommand;
use crate::render::GpuContext;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::settings::AudioApplications;
use crate::ui::state::UiState;
use crate::window::AppEvent;

/// The window's view of an open room: the handle, the snapshot the panels draw from, the ticket
/// the status bar copies, the watch handles that feed tiles, and the share in flight.
pub struct RoomView {
    pub room: Arc<Room>,
    pub snapshot: RoomSnapshot,
    pub ticket: String,
    /// The ticket `ticket` was serialized from, so a refresh only re-serializes when the endpoint
    /// address actually changed (relay changes do not bump the room version).
    last_ticket: RoomTicket,
    handles: HashMap<TileKey, WatchHandle>,
    /// Holds an `Arc<Room>` clone; the shutdown path aborts and awaits it before the leave.
    pub pending_share: Option<JoinHandle<()>>,
}

impl RoomView {
    pub fn new(room: Arc<Room>) -> Self {
        let snapshot = room.snapshot();
        let last_ticket = room.ticket();
        let ticket = last_ticket.to_string();
        Self {
            room,
            snapshot,
            ticket,
            last_ticket,
            handles: HashMap::new(),
            pending_share: None,
        }
    }

    /// The keys of every watch in the current snapshot; pop-outs whose key is absent are closed.
    pub fn watched_keys(&self) -> HashSet<TileKey> {
        self.snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect()
    }

    /// Housekeeping run once per relevant state change (a room-version bump, or the statistics
    /// timer), never per redraw: re-snapshots when the version moved, drops handles and tiles of
    /// ended watches, refreshes the rate meters and age labels, and re-serializes the ticket if the
    /// endpoint address moved. Called from the event handlers in `window.rs`, not from the render
    /// path, so drawing a second window never repeats this work and every window in the same batch
    /// draws the same snapshot.
    pub fn refresh(&mut self, state: &mut UiState, tiles: Option<&mut TileRenderer>) {
        // `Room::snapshot` locks membership and rebuilds from scratch every call, so this is
        // gated on the version even though `refresh` itself already runs only on a relevant event
        // (a `Tick` with nothing changed must not pay for a rebuild it does not need).
        if self.room.version() != self.snapshot.version {
            self.snapshot = self.room.snapshot();
            let live = self.watched_keys();
            self.handles.retain(|key, _| live.contains(key));
            if let Some(tiles) = tiles {
                tiles.retain(|key| live.contains(key));
            }
        }
        state.refresh_rates(&self.snapshot, Instant::now());
        // Unlike the snapshot, the ticket has no version to gate on: the relay address can change
        // without bumping the room version, so this compares the cheap `RoomTicket` on every
        // refresh rather than only on a version change.
        refresh_ticket_cache(&mut self.last_ticket, &mut self.ticket, self.room.ticket());
    }

    /// Uploads the newest decoded frame of every watched live.
    pub fn upload_frames(&self, gpu: &GpuContext, tiles: &mut TileRenderer) {
        for (key, handle) in &self.handles {
            if let Some(frame) = handle.slot.try_take() {
                tiles.upload(&gpu.device, &gpu.queue, *key, &frame);
                // `write_texture` copies from `frame` synchronously before returning (wgpu does
                // not retain the source slice for later GPU work), so the buffer is free to reuse
                // the moment the call above returns.
                handle.recycle(frame);
            }
        }
    }

    /// Applies the commands one egui pass produced. Errors land in the status line. `stored` seeds
    /// a picker these commands open: under `all` only the settings still hold the chosen names.
    pub fn apply(
        &mut self,
        commands: Vec<RoomCommand>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
        stored: &AudioApplications,
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
                RoomCommand::SetAudioApplications(selection) => {
                    self.room.set_audio_applications(selection);
                    Ok(())
                }
                // Mirrors `Share { source: None }`: enumeration runs here on the command-drain
                // path, so a platform listing failure reaches the user as a status line.
                RoomCommand::ChooseApplications => match self.room.audio_sources() {
                    Ok(sources) => {
                        state.open_applications(sources, stored);
                        Ok(())
                    }
                    Err(error) => Err(error),
                },
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

/// Re-serializes `cached` only when `current` differs from `last`, so an unchanged endpoint
/// address costs a cheap comparison instead of a re-encode on every refresh.
fn refresh_ticket_cache(last: &mut RoomTicket, cached: &mut String, current: RoomTicket) {
    if current != *last {
        *cached = current.to_string();
        *last = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_audio::{AudioSelection, FakeOutput, SyntheticTone};
    use brp_capture::SyntheticSource;
    use brp_net::RelaySetting;
    use brp_proto::SourceKind;
    use brp_proto::constants::SOURCE_PRESET_ID;
    use brp_room::codecs::fake::FakeCodecs;
    use brp_room::{RoomConfig, RoomTimings};
    use iroh::{EndpointAddr, SecretKey};
    use std::net::SocketAddr;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn config(nickname: &str) -> RoomConfig {
        RoomConfig {
            secret: SecretKey::generate(),
            relay: RelaySetting::Disabled,
            nickname: nickname.into(),
            target_fps: 30,
            capture: Arc::new(SyntheticSource {
                width: 64,
                height: 32,
                fps: 30,
            }),
            audio_capture: Arc::new(SyntheticTone {
                frequency_hz: 440.0,
                amplitude: 0.5,
            }),
            audio_output: Arc::new(FakeOutput::new().0),
            audio_applications: AudioSelection::All,
            encoders: Arc::new(FakeCodecs),
            decoders: Arc::new(FakeCodecs),
            on_change: Arc::new(|| {}),
            on_frame: Arc::new(|_publisher, _live_id| {}),
            timings: RoomTimings {
                heartbeat: Duration::from_millis(200),
                expiry: Duration::from_secs(1),
                housekeeping: Duration::from_millis(100),
                encoder_grace: Duration::from_millis(300),
                join_timeout: Duration::from_secs(5),
            },
        }
    }

    async fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn sample_addr(port: u16) -> EndpointAddr {
        let id = SecretKey::from_bytes(&[port as u8; 32]).public();
        EndpointAddr::new(id).with_ip_addr(SocketAddr::from(([192, 168, 1, 10], port)))
    }

    #[test]
    fn an_unchanged_ticket_is_not_reserialized_but_a_relay_change_rebuilds_it() {
        // No room version anywhere in this test: the cache must invalidate purely from the
        // endpoint address moving, since the room's version counter never reflects a relay change.
        let topic = [1u8; 32];
        let first = RoomTicket::new(topic, vec![sample_addr(1)]);
        let mut last = first.clone();
        let mut cached = first.to_string();
        cached.push_str("-stale-marker");

        refresh_ticket_cache(&mut last, &mut cached, first.clone());
        assert!(
            cached.ends_with("-stale-marker"),
            "an unchanged ticket must not be re-serialized"
        );

        let relayed = RoomTicket::new(topic, vec![sample_addr(2)]);
        refresh_ticket_cache(&mut last, &mut cached, relayed.clone());
        assert_eq!(cached, relayed.to_string());
        assert_eq!(last, relayed);
    }

    #[tokio::test]
    async fn an_unchanged_version_skips_the_snapshot_rebuild_but_the_ticket_check_still_runs() {
        let room = Room::create(config("alice")).await.unwrap();
        let mut view = RoomView::new(Arc::new(room));
        let version = view.room.version();
        assert_eq!(view.snapshot.version, version, "no change happened yet");

        // Corrupted so that a rebuild (which would produce the room's real nickname) and a
        // skipped rebuild (which leaves this untouched) are distinguishable.
        view.snapshot.nickname = "stale-marker".into();
        // Forced out of sync with the room's real ticket, standing in for a relay change the
        // version does not reflect; the ticket check must still run on this same call.
        let real_ticket = view.room.ticket();
        view.last_ticket = RoomTicket::new(real_ticket.topic, Vec::new());
        view.ticket = "stale-ticket-marker".into();

        let mut state = UiState::new();
        view.refresh(&mut state, None);

        assert_eq!(
            view.snapshot.version, version,
            "version still has not moved"
        );
        assert_eq!(
            view.snapshot.nickname, "stale-marker",
            "an unchanged version must skip the snapshot rebuild"
        );
        assert_eq!(
            view.ticket,
            real_ticket.to_string(),
            "the ticket cache is not gated on the version"
        );
        assert_eq!(view.last_ticket, real_ticket);

        match Arc::try_unwrap(view.room) {
            Ok(room) => room.leave().await,
            Err(_) => panic!("room still referenced"),
        };
    }

    #[tokio::test]
    async fn the_snapshot_stays_coherent_across_a_render_batch_until_refresh_is_called() {
        let room = Room::create(config("alice")).await.unwrap();
        let mut view = RoomView::new(Arc::new(room));
        let before = view.snapshot.version;

        view.room
            .start_live(SourceKind::Monitor, None, "desk".into())
            .await
            .unwrap();
        wait_until("version bump", Duration::from_secs(5), || {
            view.room.version() != before
        })
        .await;

        // Two reads standing in for two windows drawn in the same batch: both must see the
        // snapshot RoomView started with, not the room's newer state, until `refresh` runs.
        assert_eq!(view.snapshot.version, before);
        assert_eq!(view.snapshot.version, before);
        assert!(view.snapshot.own_lives.is_empty());

        let mut state = UiState::new();
        view.refresh(&mut state, None);
        assert_eq!(view.snapshot.version, view.room.version());
        assert_eq!(view.snapshot.own_lives.len(), 1);

        match Arc::try_unwrap(view.room) {
            Ok(room) => room.leave().await,
            Err(_) => panic!("room still referenced"),
        };
    }

    #[tokio::test]
    async fn a_watch_removed_while_a_frame_is_pending_is_dropped_before_it_is_uploaded() {
        let a = Room::create(config("alice")).await.unwrap();
        let live = a
            .start_live(SourceKind::Monitor, None, "desk".into())
            .await
            .unwrap();
        // `carol` joins the room `a` created so she can watch alice's live.
        let carol = Room::join(config("carol"), a.ticket()).await.unwrap();
        wait_until("catalog", Duration::from_secs(5), || {
            !carol.snapshot().members.is_empty() && !carol.snapshot().members[0].lives.is_empty()
        })
        .await;

        let key = (a.id(), live);
        let handle = carol.watch(a.id(), live, SOURCE_PRESET_ID).unwrap();
        wait_until("decoded frames", Duration::from_secs(5), || {
            handle.stats.frames_decoded.load(Ordering::Relaxed) >= 1
        })
        .await;
        assert!(
            handle.slot.try_take().is_some(),
            "a frame must be waiting before the watch is removed"
        );
        // Put a fresh frame back so one is pending when the watch is removed underneath the view.
        wait_until("a second frame", Duration::from_secs(5), || {
            handle.stats.frames_decoded.load(Ordering::Relaxed) >= 2
        })
        .await;

        let mut view = RoomView::new(Arc::new(carol));
        view.handles.insert(key, handle);

        // Removed through the room directly, as if some other path (not `RoomView::apply`) ended
        // it while the view still holds the handle with a frame in its slot.
        view.room.unwatch(a.id(), live).unwrap();

        let mut state = UiState::new();
        view.refresh(&mut state, None);
        assert!(
            !view.handles.contains_key(&key),
            "lifecycle update must drop the handle before it is used again"
        );

        match Arc::try_unwrap(view.room) {
            Ok(room) => room.leave().await,
            Err(_) => panic!("room still referenced"),
        };
        a.leave().await;
    }
}
