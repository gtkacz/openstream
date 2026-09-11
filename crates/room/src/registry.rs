//! Lives this participant publishes. Encoders exist only while someone is subscribed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use brp_audio::{AudioCapture, AudioCaptureSession, AudioError, AudioSelection, AudioSource};
use brp_capture::{CaptureFrame, CaptureSession, SourceInfo};
use brp_codec::{CodecError, FrameConverter, InputImage, RawFrame};
use brp_net::{AudioSubscription, LiveSource, SubscribeRejected, Subscription};
use brp_pipeline::{AudioPublisher, LatestSlot, Pacer, Publisher, SlotWait};
use brp_proto::constants::{IDLE_KEYFRAME_RETRY, MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE};
use brp_proto::{LiveInfo, PixelFormat, Preset, ProtoError, SourceKind};

use crate::codecs::EncoderFactory;
use crate::error::RoomError;
use crate::snapshot::{AudioCaptureState, EncoderView, OwnAudioView, OwnLiveView, PresetView};

pub type ChangeNotify = Arc<dyn Fn() + Send + Sync>;

type CaptureSlot = Arc<LatestSlot<Arc<CaptureFrame>>>;
type RawSlot = Arc<LatestSlot<Arc<RawFrame>>>;

/// Delivers each captured frame to every running encoder of one live without copying pixels.
#[derive(Default)]
pub struct CaptureFan {
    slots: Mutex<Vec<CaptureSlot>>,
    last_format: Mutex<Option<PixelFormat>>,
}

impl CaptureFan {
    pub fn push(&self, frame: CaptureFrame) {
        *lock(&self.last_format) = Some(frame.format);
        let frame = Arc::new(frame);
        for slot in lock(&self.slots).iter() {
            slot.put(frame.clone());
        }
    }

    pub fn attach(&self) -> CaptureSlot {
        let slot = LatestSlot::new();
        lock(&self.slots).push(slot.clone());
        slot
    }

    pub fn detach(&self, slot: &CaptureSlot) {
        lock(&self.slots).retain(|s| !Arc::ptr_eq(s, slot));
    }

    /// The compositor's pixel order, known after the first frame. The converter rebuilds itself if
    /// this guess is wrong, so a default before the first frame costs nothing.
    pub fn format(&self) -> PixelFormat {
        lock(&self.last_format).unwrap_or(PixelFormat::Bgrx)
    }
}

/// Converts one captured frame per tick and shares the immutable result with every subscribed
/// preset whose output dimensions match this group's — one `SwsConverter`/`FrameConverter` per
/// distinct (width, height) per live, instead of one per preset. `FrameConverter::convert` lends a
/// mutable scratch buffer the converter reuses on its next call, so a shared borrow can't survive
/// past that call: this loop clones the converted frame into a fresh `Arc<RawFrame>` immediately,
/// once per source frame, and hands that immutable clone to every member before converting again.
/// Each member then paces and encodes independently on its own thread (see `publisher.rs`), so one
/// slow preset backs up only its own slot, never this group's or another preset's.
struct ConversionGroup {
    stop: AtomicBool,
    capture_slot: CaptureSlot,
    members: Mutex<Vec<RawSlot>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ConversionGroup {
    fn start(fan: &Arc<CaptureFan>, converter: Box<dyn FrameConverter>) -> Arc<Self> {
        let capture_slot = fan.attach();
        let group = Arc::new(Self {
            stop: AtomicBool::new(false),
            capture_slot,
            members: Mutex::new(Vec::new()),
            thread: Mutex::new(None),
        });
        let worker = group.clone();
        let handle = thread::Builder::new()
            .name("brp-convert-group".into())
            .spawn(move || conversion_loop(worker, converter))
            .expect("spawning a thread only fails when the system is out of resources");
        *lock(&group.thread) = Some(handle);
        group
    }

    fn attach_member(&self) -> RawSlot {
        let slot = LatestSlot::new();
        lock(&self.members).push(slot.clone());
        slot
    }

    fn detach_member(&self, slot: &RawSlot) {
        lock(&self.members).retain(|s| !Arc::ptr_eq(s, slot));
    }

    fn member_count(&self) -> usize {
        lock(&self.members).len()
    }

    /// Stops the conversion thread and detaches from the fan. Only called once no preset is left
    /// using this group's converted output.
    fn stop(&self, fan: &CaptureFan) {
        self.stop.store(true, Ordering::Relaxed);
        self.capture_slot.close();
        fan.detach(&self.capture_slot);
        if let Some(handle) = lock(&self.thread).take() {
            let _ = handle.join();
        }
    }
}

fn conversion_loop(group: Arc<ConversionGroup>, mut converter: Box<dyn FrameConverter>) {
    while !group.stop.load(Ordering::Relaxed) {
        let frame = match group.capture_slot.take_timeout(IDLE_KEYFRAME_RETRY) {
            SlotWait::Value(frame) => frame,
            SlotWait::Timeout => continue,
            SlotWait::Closed => break,
        };
        let image = InputImage {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: frame.format,
            data: &frame.data,
            capture_ts_us: frame.capture_ts_us,
        };
        match converter.convert(&image) {
            Ok(raw) => {
                // Cloned once, right here, before the next `convert()` call reuses and overwrites
                // the converter's scratch buffer: every member gets its own immutable `Arc`.
                let shared = Arc::new(raw.clone());
                for member in lock(&group.members).iter() {
                    member.put(shared.clone());
                }
            }
            Err(error) => tracing::error!(%error, "shared frame conversion failed"),
        }
    }
}

struct RunningEncoder {
    publisher: Publisher,
    group_key: (u32, u32),
    raw_slot: RawSlot,
    idle_since: Option<Instant>,
}

struct PresetState {
    preset: Preset,
    running: Option<RunningEncoder>,
    last_error: Option<String>,
}

struct OwnLive {
    info: LiveInfo,
    session: Option<Box<dyn CaptureSession>>,
    fan: Arc<CaptureFan>,
    presets: BTreeMap<u32, PresetState>,
    /// One converter per distinct subscribed (width, height); created on first subscribe, stopped
    /// once its last member preset stops.
    groups: BTreeMap<(u32, u32), Arc<ConversionGroup>>,
}

struct RunningAudio {
    session: Box<dyn AudioCaptureSession>,
    publisher: AudioPublisher,
    idle_since: Option<Instant>,
}

struct AudioState {
    enabled: bool,
    selection: AudioSelection,
    running: Option<RunningAudio>,
    last_error: Option<String>,
}

impl AudioState {
    /// What presence says: on, and not known to be broken.
    fn advertises(&self) -> bool {
        self.enabled && self.last_error.is_none()
    }
}

struct Inner {
    lives: BTreeMap<u32, OwnLive>,
    next_live_id: u32,
    audio: AudioState,
}

pub struct LiveRegistry {
    inner: Mutex<Inner>,
    /// Held across a capture start, which can block for `AUDIO_CAPTURE_START_TIMEOUT`, so the
    /// registry lock stays free for `live_infos` and the snapshot. A second subscriber waits here
    /// and then finds the capture already installed.
    audio_start: Mutex<()>,
    encoders: Arc<dyn EncoderFactory>,
    audio_capture: Arc<dyn AudioCapture>,
    grace: Duration,
    on_change: ChangeNotify,
}

impl LiveRegistry {
    pub fn new(
        encoders: Arc<dyn EncoderFactory>,
        audio_capture: Arc<dyn AudioCapture>,
        selection: AudioSelection,
        grace: Duration,
        on_change: ChangeNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            audio_start: Mutex::new(()),
            inner: Mutex::new(Inner {
                lives: BTreeMap::new(),
                next_live_id: 1,
                audio: AudioState {
                    enabled: true,
                    selection,
                    running: None,
                    last_error: None,
                },
            }),
            encoders,
            audio_capture,
            grace,
            on_change,
        })
    }

    pub fn add_live(
        &self,
        title: String,
        kind: SourceKind,
        session: Box<dyn CaptureSession>,
        fan: Arc<CaptureFan>,
        presets: Vec<Preset>,
    ) -> Result<u32, RoomError> {
        let source = session.info();
        validate_presets(&presets, source.width, source.height, source.fps)?;
        let mut inner = lock(&self.inner);
        if inner.lives.len() >= MAX_LIVES_PER_PARTICIPANT {
            return Err(RoomError::TooManyLives);
        }
        let id = inner.next_live_id;
        inner.next_live_id += 1;
        let has_audio = inner.audio.advertises();
        let info = LiveInfo {
            id,
            title,
            kind,
            source_width: source.width,
            source_height: source.height,
            source_fps: source.fps,
            has_audio,
            presets: presets.clone(),
        };
        let presets = presets
            .into_iter()
            .map(|p| {
                (
                    p.id,
                    PresetState {
                        preset: p,
                        running: None,
                        last_error: None,
                    },
                )
            })
            .collect();
        inner.lives.insert(
            id,
            OwnLive {
                info,
                session: Some(session),
                fan,
                presets,
                groups: BTreeMap::new(),
            },
        );
        drop(inner);
        (self.on_change)();
        Ok(id)
    }

    pub fn remove_live(&self, live_id: u32) -> Result<(), RoomError> {
        let mut inner = lock(&self.inner);
        let mut live = inner
            .lives
            .remove(&live_id)
            .ok_or(RoomError::UnknownLive(live_id))?;
        drop(inner);
        for state in live.presets.values_mut() {
            stop_encoder(&live.fan, &mut live.groups, state);
        }
        if let Some(session) = live.session.take() {
            session.stop();
        }
        (self.on_change)();
        Ok(())
    }

    /// Replaces the preset list. Running encoders whose preset is unchanged keep running; removed or
    /// edited presets stop, which ends their subscriptions with live-ended.
    pub fn set_presets(&self, live_id: u32, presets: Vec<Preset>) -> Result<(), RoomError> {
        let mut inner = lock(&self.inner);
        let live = inner
            .lives
            .get_mut(&live_id)
            .ok_or(RoomError::UnknownLive(live_id))?;
        validate_presets(
            &presets,
            live.info.source_width,
            live.info.source_height,
            live.info.source_fps,
        )?;
        let mut old = std::mem::take(&mut live.presets);
        for preset in presets.iter() {
            let state = match old.remove(&preset.id) {
                Some(mut state) if state.preset == *preset => {
                    state.last_error = None;
                    state
                }
                Some(mut state) => {
                    stop_encoder(&live.fan, &mut live.groups, &mut state);
                    PresetState {
                        preset: preset.clone(),
                        running: None,
                        last_error: None,
                    }
                }
                None => PresetState {
                    preset: preset.clone(),
                    running: None,
                    last_error: None,
                },
            };
            live.presets.insert(preset.id, state);
        }
        for mut removed in old.into_values() {
            stop_encoder(&live.fan, &mut live.groups, &mut removed);
        }
        live.info.presets = presets;
        drop(inner);
        (self.on_change)();
        Ok(())
    }

    pub fn live_count(&self) -> usize {
        lock(&self.inner).lives.len()
    }

    pub fn live_infos(&self) -> Vec<LiveInfo> {
        let inner = lock(&self.inner);
        let has_audio = inner.audio.advertises();
        inner
            .lives
            .values()
            .map(|l| LiveInfo {
                has_audio,
                ..l.info.clone()
            })
            .collect()
    }

    pub fn set_audio(&self, enabled: bool) {
        let mut inner = lock(&self.inner);
        inner.audio.enabled = enabled;
        // A retoggle is the retry path: forget the last failure and start fresh on the next listener.
        inner.audio.last_error = None;
        let stopped = (!enabled).then(|| take_audio(&mut inner.audio)).flatten();
        drop(inner);
        if let Some(running) = stopped {
            stop_audio(running);
        }
        (self.on_change)();
    }

    /// Replaces which applications the capture carries. With nothing running that is all it does,
    /// and the next subscriber starts with the new value. Editing a selection also clears a
    /// recorded failure, so it is a retry path like the share-audio retoggle.
    pub fn set_audio_applications(&self, selection: AudioSelection) {
        // The same lock a first start holds: a swap and a start must not overlap, and the swap's
        // start blocks just as long.
        let _starting = lock(&self.audio_start);
        let mut inner = lock(&self.inner);
        inner.audio.selection = selection.clone();
        inner.audio.last_error = None;
        let Some(running) = take_audio(&mut inner.audio) else {
            drop(inner);
            (self.on_change)();
            return;
        };
        drop(inner);

        // The publisher outlives the swap, so the fan-out, the audio sequence space, and every
        // viewer's carrier survive it; the jitter buffer emits silence for the gap and re-primes.
        let RunningAudio {
            session,
            publisher,
            idle_since,
        } = running;
        // Stopped before the new one starts: two sessions feeding one publisher would sum the
        // desktop into itself at double amplitude for the overlap.
        session.stop();
        match self.audio_capture.start(selection, publisher.sink()) {
            Ok(session) => {
                let mut inner = lock(&self.inner);
                if !inner.audio.advertises() {
                    // Share audio was turned off while a slow daemon was answering.
                    drop(inner);
                    stop_audio(RunningAudio {
                        session,
                        publisher,
                        idle_since,
                    });
                    (self.on_change)();
                    return;
                }
                inner.audio.running = Some(RunningAudio {
                    session,
                    publisher,
                    idle_since,
                });
            }
            Err(error) => {
                // No fallback to the old session: reviving it would keep sharing applications the
                // user just deselected. This is phase 4's capture-failure path.
                publisher.stop();
                let message = error.to_string();
                tracing::warn!(%message, "audio capture failed to restart for a new selection");
                lock(&self.inner).audio.last_error = Some(message);
            }
        }
        (self.on_change)();
    }

    /// What the platform reports as playing audio now. Takes no registry lock: the backend's own
    /// deadline is what bounds it.
    pub fn sources(&self) -> Result<Vec<AudioSource>, AudioError> {
        self.audio_capture.sources()
    }

    pub fn audio_enabled(&self) -> bool {
        lock(&self.inner).audio.enabled
    }

    pub fn audio_view(&self) -> OwnAudioView {
        let inner = lock(&self.inner);
        let audio = &inner.audio;
        let state = match (&audio.last_error, &audio.running, audio.enabled) {
            (_, _, false) => AudioCaptureState::Off,
            (Some(error), _, true) => AudioCaptureState::Failed(error.clone()),
            (None, Some(_), true) => AudioCaptureState::Capturing,
            (None, None, true) => AudioCaptureState::Idle,
        };
        OwnAudioView {
            enabled: audio.enabled,
            state,
            selection: audio.selection.clone(),
            subscribers: audio
                .running
                .as_ref()
                .map(|r| r.publisher.subscriber_count())
                .unwrap_or(0),
            packets_encoded: audio
                .running
                .as_ref()
                .map(|r| r.publisher.stats().packets_encoded.load(Ordering::Relaxed))
                .unwrap_or(0),
        }
    }

    pub fn views(&self) -> Vec<OwnLiveView> {
        let inner = lock(&self.inner);
        let has_audio = inner.audio.advertises();
        inner
            .lives
            .values()
            .map(|live| OwnLiveView {
                info: LiveInfo {
                    has_audio,
                    ..live.info.clone()
                },
                presets: live
                    .presets
                    .values()
                    .map(|state| PresetView {
                        preset: state.preset.clone(),
                        encoder: state.running.as_ref().map(|r| EncoderView {
                            name: r.publisher.encoder_name(),
                            subscribers: r.publisher.subscriber_count(),
                            frames_encoded: r
                                .publisher
                                .stats()
                                .frames_encoded
                                .load(Ordering::Relaxed),
                            bytes_encoded: r
                                .publisher
                                .stats()
                                .bytes_encoded
                                .load(Ordering::Relaxed),
                            dropped_at_input: r.publisher.frames_dropped_at_input(),
                        }),
                        last_error: state.last_error.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Stops encoders that have had no subscriber for the whole grace period.
    pub fn housekeeping(&self, now: Instant) {
        let mut inner = lock(&self.inner);
        let stopped_audio = housekeep_audio(&mut inner.audio, now, self.grace);
        let audio_changed = stopped_audio.is_some();
        let mut stopped_any = false;
        for live in inner.lives.values_mut() {
            for state in live.presets.values_mut() {
                let idle_for = match state.running.as_mut() {
                    Some(running) if running.publisher.subscriber_count() == 0 => {
                        now.duration_since(*running.idle_since.get_or_insert(now))
                    }
                    Some(running) => {
                        running.idle_since = None;
                        continue;
                    }
                    None => continue,
                };
                if idle_for >= self.grace {
                    stop_encoder(&live.fan, &mut live.groups, state);
                    stopped_any = true;
                }
            }
        }
        drop(inner);
        if let Some(running) = stopped_audio {
            stop_audio(running);
        }
        if stopped_any || audio_changed {
            (self.on_change)();
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<u32> = lock(&self.inner).lives.keys().copied().collect();
        for id in ids {
            let _ = self.remove_live(id);
        }
        let stopped = take_audio(&mut lock(&self.inner).audio);
        if let Some(running) = stopped {
            stop_audio(running);
        }
    }
}

impl LiveSource for LiveRegistry {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        let mut inner = lock(&self.inner);
        let live = inner
            .lives
            .get_mut(&live_id)
            .ok_or(SubscribeRejected::UnknownLive(live_id))?;
        let source = SourceInfo {
            width: live.info.source_width,
            height: live.info.source_height,
            fps: live.info.source_fps,
        };
        let format = live.fan.format();
        let preset = live
            .presets
            .get(&preset_id)
            .ok_or(SubscribeRejected::UnknownPreset(preset_id))?
            .preset
            .clone();
        let mut started = false;
        if live.presets[&preset_id].running.is_none() {
            match start_preset_encoder(
                &self.encoders,
                live,
                live_id,
                preset_id,
                source,
                format,
                &preset,
            ) {
                Ok(running) => {
                    let state = live.presets.get_mut(&preset_id).expect("checked above");
                    state.running = Some(running);
                    state.last_error = None;
                    started = true;
                }
                Err(error) => {
                    let state = live.presets.get_mut(&preset_id).expect("checked above");
                    state.last_error = Some(error.to_string());
                    drop(inner);
                    (self.on_change)();
                    return Err(SubscribeRejected::EncoderFailed(error.to_string()));
                }
            }
        }
        let running = live
            .presets
            .get_mut(&preset_id)
            .expect("checked above")
            .running
            .as_mut()
            .expect("set above");
        running.idle_since = None;
        let subscription = running.publisher.subscribe(live_id, preset_id);
        drop(inner);
        if started {
            (self.on_change)();
        }
        subscription
    }

    fn request_keyframe(&self, live_id: u32, preset_id: u32) {
        if let Some(running) = lock(&self.inner)
            .lives
            .get(&live_id)
            .and_then(|l| l.presets.get(&preset_id))
            .and_then(|s| s.running.as_ref())
        {
            running.publisher.request_keyframe(live_id, preset_id);
        }
    }

    fn subscribe_audio(&self, live_id: u32) -> Result<AudioSubscription, SubscribeRejected> {
        if let Some(subscription) = self.attach_audio(live_id)? {
            return Ok(subscription);
        }
        // Opening the encoder and starting the platform capture both block; neither runs under the
        // registry lock, which `Room::snapshot` and every presence broadcast need to stay quick.
        let _starting = lock(&self.audio_start);
        if let Some(subscription) = self.attach_audio(live_id)? {
            return Ok(subscription);
        }
        let running = match self.start_audio() {
            Ok(running) => running,
            Err(message) => {
                lock(&self.inner).audio.last_error = Some(message.clone());
                tracing::warn!(%message, "audio capture failed to start");
                (self.on_change)();
                return Err(SubscribeRejected::NoAudio);
            }
        };
        let subscription = {
            let mut inner = lock(&self.inner);
            if !inner.audio.advertises() {
                // Share audio was turned off while the capture was starting.
                drop(inner);
                stop_audio(running);
                return Err(SubscribeRejected::NoAudio);
            }
            let running = inner.audio.running.insert(running);
            running.idle_since = None;
            AudioSubscription {
                params: running.publisher.params(),
                packets: running.publisher.subscribe(),
            }
        };
        (self.on_change)();
        Ok(subscription)
    }
}

impl LiveRegistry {
    /// Subscribes to a capture that is already running. `None` means one has to be started, which
    /// happens off the lock.
    fn attach_audio(&self, live_id: u32) -> Result<Option<AudioSubscription>, SubscribeRejected> {
        let mut inner = lock(&self.inner);
        if !inner.lives.contains_key(&live_id) {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if !inner.audio.advertises() {
            return Err(SubscribeRejected::NoAudio);
        }
        let Some(running) = inner.audio.running.as_mut() else {
            return Ok(None);
        };
        running.idle_since = None;
        Ok(Some(AudioSubscription {
            params: running.publisher.params(),
            packets: running.publisher.subscribe(),
        }))
    }

    /// Opens the encoder and starts the platform capture with the registry unlocked. A failure
    /// stops the publisher it already built, which joins its encode thread.
    fn start_audio(&self) -> Result<RunningAudio, String> {
        let selection = lock(&self.inner).audio.selection.clone();
        let encoder = self.encoders.open_audio().map_err(|e| e.to_string())?;
        let publisher = AudioPublisher::start(encoder);
        match self.audio_capture.start(selection, publisher.sink()) {
            Ok(session) => Ok(RunningAudio {
                session,
                publisher,
                idle_since: None,
            }),
            Err(error) => {
                publisher.stop();
                Err(error.to_string())
            }
        }
    }
}

/// Opens (or reuses) the conversion group for `preset`'s output dimensions and starts its encoder
/// and publisher. On encoder failure, a group newly created for this call is torn down immediately:
/// a preset that fails to subscribe must not leave conversion work running for nobody.
fn start_preset_encoder(
    encoders: &Arc<dyn EncoderFactory>,
    live: &mut OwnLive,
    live_id: u32,
    preset_id: u32,
    source: SourceInfo,
    format: PixelFormat,
    preset: &Preset,
) -> Result<RunningEncoder, CodecError> {
    let group_key = (preset.width, preset.height);
    let group = match live.groups.get(&group_key) {
        Some(group) => group.clone(),
        None => {
            let converter = encoders.open_converter(source, format, group_key.0, group_key.1)?;
            let group = ConversionGroup::start(&live.fan, converter);
            live.groups.insert(group_key, group.clone());
            group
        }
    };
    match encoders.open_encoder(preset) {
        Ok(encoder) => {
            let raw_slot = group.attach_member();
            // Always paced, even at the source rate: below the target rate the pacer skips
            // frames to hold the preset's fps; at or above it every frame still clears the
            // due-time check, so admission is unaffected, but sustained overload can still
            // back the rate off (see `Pacer::record_duration`).
            let pacer = Some(Pacer::new(preset.fps));
            let publisher = Publisher::start(live_id, preset_id, raw_slot.clone(), encoder, pacer);
            Ok(RunningEncoder {
                publisher,
                group_key,
                raw_slot,
                idle_since: None,
            })
        }
        Err(error) => {
            if group.member_count() == 0 {
                group.stop(&live.fan);
                live.groups.remove(&group_key);
            }
            Err(error)
        }
    }
}

fn stop_encoder(
    fan: &CaptureFan,
    groups: &mut BTreeMap<(u32, u32), Arc<ConversionGroup>>,
    state: &mut PresetState,
) {
    if let Some(running) = state.running.take() {
        running.publisher.stop();
        if let Some(group) = groups.get(&running.group_key) {
            group.detach_member(&running.raw_slot);
            if group.member_count() == 0 {
                group.stop(fan);
                groups.remove(&running.group_key);
            }
        }
    }
}

/// Stops idle capture after the grace and turns a dead session into a recorded failure. Returns
/// the capture that was taken, if any, so the caller can stop it (which joins its threads) after
/// dropping the registry lock.
fn housekeep_audio(audio: &mut AudioState, now: Instant, grace: Duration) -> Option<RunningAudio> {
    let running = audio.running.as_mut()?;
    if let Some(error) = running.session.error() {
        audio.last_error = Some(error);
        return take_audio(audio);
    }
    if running.publisher.subscriber_count() > 0 {
        running.idle_since = None;
        return None;
    }
    let idle_for = now.duration_since(*running.idle_since.get_or_insert(now));
    if idle_for >= grace {
        return take_audio(audio);
    }
    None
}

/// Removes the running capture from the state under the lock; the caller stops it once unlocked.
fn take_audio(audio: &mut AudioState) -> Option<RunningAudio> {
    audio.running.take()
}

/// Joins the publisher's and the session's threads. Never call this while the registry is locked.
fn stop_audio(running: RunningAudio) {
    running.publisher.stop();
    running.session.stop();
}

fn validate_presets(
    presets: &[Preset],
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(), RoomError> {
    if presets.is_empty() {
        return Err(RoomError::Proto(ProtoError::Invalid("no presets".into())));
    }
    if presets.len() > MAX_PRESETS_PER_LIVE {
        return Err(RoomError::Proto(ProtoError::Invalid(
            "too many presets".into(),
        )));
    }
    for preset in presets {
        preset.validate(width, height, fps).map_err(|e| {
            RoomError::Proto(ProtoError::Invalid(format!("preset {}: {e}", preset.id)))
        })?;
    }
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
