//! Everything the window holds once a room is open: the room handle, its last snapshot, the
//! watch handles that feed tiles, and the share in flight. The window delegates room commands here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_audio::{AudioOutput, CpalInput, CpalOutput, VoiceCapture};
use brp_capture::{SourceId, SourceListing};
use brp_proto::{RoomTicket, SourceKind};
use brp_room::{Room, RoomError, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

use crate::commands::RoomCommand;
use crate::render::GpuContext;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::settings::AudioSettings;
use crate::ui::state::UiState;
use crate::window::AppEvent;

/// The window's view of an open room: the handle, the snapshot the panels draw from, the ticket
/// the room rail copies, the watch handles that feed tiles, and the share in flight.
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
    /// The picker's background re-listing, while one runs.
    pub pending_listing: Option<JoinHandle<()>>,
}

impl RoomView {
    /// Cancels work that owns room clones before closing streams and the endpoint.
    pub async fn leave(self) {
        let Self {
            room,
            pending_share,
            pending_listing,
            handles,
            ..
        } = self;
        drop(handles);
        let tasks: Vec<_> = pending_share.into_iter().chain(pending_listing).collect();
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        match Arc::try_unwrap(room) {
            Ok(room) => room.leave().await,
            Err(_) => tracing::warn!("room still referenced; skipping the orderly leave"),
        }
    }

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
            pending_listing: None,
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
        // Every time, not only when the version moved: round-trip times and heartbeat ages change
        // without bumping it, and this runs only on a relevant event, at most a few times a second.
        self.snapshot = self.room.snapshot();
        let live = self.watched_keys();
        self.handles.retain(|key, _| live.contains(key));
        if let Some(tiles) = tiles {
            tiles.retain(|key| live.contains(key) || state.keeps_picture(key));
        }
        state.refresh_rates(&self.snapshot, Instant::now());
        // Unlike the snapshot, the ticket has no version to gate on: the relay address can change
        // without bumping the room version, so this compares the cheap `RoomTicket` on every
        // refresh rather than only on a version change.
        refresh_ticket_cache(&mut self.last_ticket, &mut self.ticket, self.room.ticket());
    }

    /// The ticket to keep for coming back later, naming the other members. `None` alone in the room,
    /// where it could only name this participant, which leads nowhere: joining through yourself
    /// opens an empty room. The ticket that got this participant in is the better guess then.
    pub async fn way_back(&self) -> Option<String> {
        let ticket = self.room.rejoin_ticket().await;
        let me = self.room.id();
        ticket
            .bootstrap
            .iter()
            .any(|addr| addr.id != me)
            .then(|| ticket.to_string())
    }

    /// Reads what each watched stream has carried since the last draw.
    pub fn sample_traffic(&self, state: &mut UiState) {
        use std::sync::atomic::Ordering;
        let now = Instant::now();
        state
            .traffic
            .retain(|key, _| self.handles.contains_key(key));
        for (key, handle) in &self.handles {
            state.traffic.entry(*key).or_default().sample(
                handle.stats.bytes_received.load(Ordering::Relaxed),
                handle.stats.keyframes_received.load(Ordering::Relaxed),
                now,
            );
        }
        let levels = self.room.voice_levels();
        state
            .voice
            .retain(|key, _| *key == self.snapshot.me || levels.members.contains_key(key));
        state
            .voice
            .entry(self.snapshot.me)
            .or_default()
            .sample(levels.own, now);
        for (peer, sample) in levels.members {
            state.voice.entry(peer).or_default().sample(sample, now);
        }
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

    /// Applies the commands one egui pass produced. Errors land in the status line. `audio.applications` seeds
    /// a picker these commands open: under `all` only the settings still hold the chosen names.
    pub fn apply(
        &mut self,
        commands: Vec<RoomCommand>,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &mut UiState,
        audio: &AudioSettings,
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
                            state.status =
                                "A share is already in progress. Finish it, then try again.".into();
                        }
                        Ok(())
                    }
                    Err(error) => {
                        state.status = share_failure(&error);
                        Ok(())
                    }
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
                RoomCommand::SetAudioVolumes(volumes) => {
                    self.room.set_audio_volumes(volumes);
                    Ok(())
                }
                // Mirrors `Share { source: None }`: enumeration runs here on the command-drain
                // path, so a platform listing failure reaches the user as a status line.
                RoomCommand::ChooseApplications => match self.room.audio_sources() {
                    Ok(sources) => {
                        state.open_applications(
                            sources,
                            &audio.applications,
                            self.room.send_volume_support(),
                        );
                        Ok(())
                    }
                    Err(error) => {
                        state.status = format!("Could not list applications. Try again. ({error})");
                        Ok(())
                    }
                },
                RoomCommand::JoinVoice => {
                    self.join_voice(
                        Arc::new(CpalInput::new(audio.input_device.clone())),
                        Arc::new(CpalOutput::new(audio.voice_output())),
                        state,
                    );
                    Ok(())
                }
                RoomCommand::LeaveVoice => {
                    self.room.leave_voice();
                    Ok(())
                }
                RoomCommand::SetVoiceMuted(muted) => {
                    self.room.set_voice_muted(muted);
                    Ok(())
                }
                RoomCommand::SetVoiceDeafened(deafened) => {
                    self.room.set_voice_deafened(deafened);
                    Ok(())
                }
                RoomCommand::SetHearing { peer, hearing } => {
                    self.room.set_hearing(peer, hearing);
                    Ok(())
                }
                RoomCommand::SetSending { peer, sending } => {
                    self.room.set_sending(peer, sending);
                    Ok(())
                }
                RoomCommand::SetVoiceVolume { peer, gain } => {
                    self.room.set_voice_volume(peer, gain);
                    Ok(())
                }
            };
            if let Err(error) = result {
                state.status = error.to_string();
            }
        }
    }

    /// Puts the room in voice with these devices. A failure lands in the status line.
    pub fn join_voice(
        &self,
        capture: Arc<dyn VoiceCapture>,
        output: Arc<dyn AudioOutput>,
        state: &mut UiState,
    ) {
        if let Err(error) = self.room.join_voice(capture, output) {
            state.status = error.to_string();
        }
    }

    /// Re-lists what is playing while the application picker is open, so it follows applications
    /// as they start and stop. Off the window thread, since a sick daemon can take a whole
    /// `AUDIO_SOURCE_LIST_TIMEOUT` to answer, and never two at once. A failed listing keeps the
    /// rows already shown: one bad poll is no reason to blank a list being edited, and the user
    /// gets an error from Refresh if they ask.
    pub fn keep_applications_current(
        &mut self,
        runtime: &Handle,
        proxy: &EventLoopProxy<AppEvent>,
        state: &UiState,
    ) {
        if state.applications.is_none()
            || self
                .pending_listing
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let room = self.room.clone();
        let proxy = proxy.clone();
        self.pending_listing = Some(runtime.spawn_blocking(move || match room.audio_sources() {
            Ok(sources) => {
                let _ = proxy.send_event(AppEvent::ApplicationsListed(sources));
            }
            Err(error) => tracing::debug!(%error, "could not re-list applications"),
        }));
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
                .map_err(|error| share_failure(&error));
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
}

/// What the dock says when a share cannot start. A refusal and the room's limit already say what
/// to do, and a generic next step would only contradict them.
fn share_failure(error: &RoomError) -> String {
    match error {
        RoomError::TooManyLives => error.to_string(),
        RoomError::Capture(capture) if capture.is_refusal() => error.to_string(),
        _ => format!(
            "Could not start sharing. Try again or choose another monitor or window. ({error})"
        ),
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
    use brp_capture::{CaptureError, SyntheticSource};
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
    fn a_failed_share_gets_a_next_step_unless_the_error_already_says_what_to_do() {
        let refused = RoomError::Capture(CaptureError::PortalDenied);
        assert_eq!(share_failure(&refused), refused.to_string());
        assert_eq!(
            share_failure(&RoomError::TooManyLives),
            RoomError::TooManyLives.to_string()
        );
        let failed = share_failure(&RoomError::Capture(CaptureError::Portal("no reply".into())));
        assert!(
            failed.starts_with("Could not start sharing. Try again"),
            "{failed}"
        );
        assert!(failed.contains("no reply"), "{failed}");
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

    struct BrokenOutput;

    impl AudioOutput for BrokenOutput {
        fn start(
            &self,
            _render: brp_audio::RenderFn,
        ) -> Result<Box<dyn brp_audio::AudioOutputSession>, brp_audio::AudioError> {
            Err(brp_audio::AudioError::Device("no sound card".into()))
        }
    }

    fn synthetic_voice() -> Arc<dyn VoiceCapture> {
        Arc::new(SyntheticTone {
            frequency_hz: 440.0,
            amplitude: 0.5,
        })
    }

    #[tokio::test]
    async fn joining_voice_from_the_window_puts_the_room_in_voice() {
        let room = Arc::new(Room::create(config("alice")).await.unwrap());
        let view = RoomView::new(room);
        let mut state = UiState::new();
        view.join_voice(synthetic_voice(), Arc::new(FakeOutput::new().0), &mut state);
        assert!(state.status.is_empty(), "{}", state.status);
        assert!(view.room.snapshot().voice.joined);
        view.leave().await;
    }

    #[tokio::test]
    async fn a_voice_output_failure_lands_in_the_status_line() {
        let room = Arc::new(Room::create(config("alice")).await.unwrap());
        let view = RoomView::new(room);
        let mut state = UiState::new();
        view.join_voice(synthetic_voice(), Arc::new(BrokenOutput), &mut state);
        assert!(
            state.status.starts_with("Could not join voice"),
            "{}",
            state.status
        );
        view.leave().await;
    }

    #[tokio::test]
    async fn leaving_waits_for_background_room_references_and_allows_rejoining() {
        let alice = Room::create(config("alice")).await.unwrap();
        let bob_config = config("bob");
        let bob_secret = bob_config.secret.clone();
        let bob = Arc::new(Room::join(bob_config, alice.ticket()).await.unwrap());
        let weak = Arc::downgrade(&bob);
        let mut view = RoomView::new(bob);
        let share_room = view.room.clone();
        view.pending_share = Some(tokio::spawn(async move {
            let _room = share_room;
            std::future::pending::<()>().await;
        }));
        // Blocking enumeration cannot be cancelled once it starts; leave must await it too.
        let listing_room = view.room.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        view.pending_listing = Some(tokio::task::spawn_blocking(move || {
            let _room = listing_room;
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
        }));
        started_rx.await.unwrap();
        let mut leaving = tokio::spawn(view.leave());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut leaving)
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), leaving)
            .await
            .unwrap()
            .unwrap();
        assert!(
            weak.upgrade().is_none(),
            "all room references must be released"
        );

        let mut config = config("bob");
        config.secret = bob_secret;
        let rejoined = Room::join(config, alice.ticket()).await.unwrap();
        wait_until("bob rejoins", Duration::from_secs(10), || {
            alice
                .snapshot()
                .members
                .iter()
                .any(|member| member.id == rejoined.id())
                && rejoined
                    .snapshot()
                    .members
                    .iter()
                    .any(|member| member.id == alice.id())
        })
        .await;
        rejoined.leave().await;
        alice.leave().await;
    }

    #[tokio::test]
    async fn the_way_back_names_the_others_and_is_nothing_when_alone() {
        let room = Room::create(config("alice")).await.unwrap();
        let ticket = room.ticket();
        let alice = RoomView::new(Arc::new(room));
        assert_eq!(alice.way_back().await, None);

        let bob = Room::join(config("bob"), ticket).await.unwrap();
        wait_until("alice to see bob", Duration::from_secs(10), || {
            !alice.room.snapshot().members.is_empty()
        })
        .await;
        let way_back: RoomTicket = alice
            .way_back()
            .await
            .expect("bob is there to come back through")
            .parse()
            .unwrap();
        assert!(way_back.bootstrap.iter().any(|addr| addr.id == bob.id()));
        assert!(
            way_back
                .bootstrap
                .iter()
                .all(|addr| addr.id != alice.room.id())
        );
        bob.leave().await;
    }

    #[tokio::test]
    async fn every_refresh_rebuilds_the_snapshot_and_checks_the_ticket() {
        let room = Room::create(config("alice")).await.unwrap();
        let mut view = RoomView::new(Arc::new(room));

        // Round-trip times and heartbeat ages move without the version moving, so a rebuild has to
        // happen regardless of it. A rebuild puts the room's real nickname back.
        view.snapshot.nickname = "stale-marker".into();
        // Forced out of sync with the room's real ticket, standing in for a relay change the
        // version does not reflect.
        let real_ticket = view.room.ticket();
        view.last_ticket = RoomTicket::new(real_ticket.topic, Vec::new());
        view.ticket = "stale-ticket-marker".into();

        let mut state = UiState::new();
        view.refresh(&mut state, None);

        assert_eq!(view.snapshot.nickname, "alice");
        assert_eq!(view.ticket, real_ticket.to_string());
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
