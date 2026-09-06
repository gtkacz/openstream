//! Lives this participant publishes. Encoders exist only while someone is subscribed.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use brp_audio::{AudioCapture, AudioCaptureSession};
use brp_capture::{CaptureFrame, CaptureSession};
use brp_net::{AudioSubscription, LiveSource, SubscribeRejected, Subscription};
use brp_pipeline::{AudioPublisher, LatestSlot, Pacer, Publisher};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE};
use brp_proto::{LiveInfo, PixelFormat, Preset, ProtoError, SourceKind};

use crate::codecs::EncoderFactory;
use crate::error::RoomError;
use crate::snapshot::{AudioCaptureState, EncoderView, OwnAudioView, OwnLiveView, PresetView};

pub type ChangeNotify = Arc<dyn Fn() + Send + Sync>;

type CaptureSlot = Arc<LatestSlot<Arc<CaptureFrame>>>;

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

struct RunningEncoder {
    publisher: Publisher,
    slot: CaptureSlot,
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
}

struct RunningAudio {
    session: Box<dyn AudioCaptureSession>,
    publisher: AudioPublisher,
    idle_since: Option<Instant>,
}

struct AudioState {
    enabled: bool,
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
    encoders: Arc<dyn EncoderFactory>,
    audio_capture: Arc<dyn AudioCapture>,
    grace: Duration,
    on_change: ChangeNotify,
}

impl LiveRegistry {
    pub fn new(
        encoders: Arc<dyn EncoderFactory>,
        audio_capture: Arc<dyn AudioCapture>,
        grace: Duration,
        on_change: ChangeNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                lives: BTreeMap::new(),
                next_live_id: 1,
                audio: AudioState {
                    enabled: true,
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
            stop_encoder(&live.fan, state);
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
                    stop_encoder(&live.fan, &mut state);
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
            stop_encoder(&live.fan, &mut removed);
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
        if !enabled {
            stop_audio(&mut inner.audio);
        }
        drop(inner);
        (self.on_change)();
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
        lock(&self.inner)
            .lives
            .values()
            .map(|live| OwnLiveView {
                info: live.info.clone(),
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
        let audio_changed = housekeep_audio(&mut inner.audio, now, self.grace);
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
                    stop_encoder(&live.fan, state);
                    stopped_any = true;
                }
            }
        }
        drop(inner);
        if stopped_any || audio_changed {
            (self.on_change)();
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<u32> = lock(&self.inner).lives.keys().copied().collect();
        for id in ids {
            let _ = self.remove_live(id);
        }
        stop_audio(&mut lock(&self.inner).audio);
    }
}

impl LiveSource for LiveRegistry {
    fn subscribe(&self, live_id: u32, preset_id: u32) -> Result<Subscription, SubscribeRejected> {
        let mut inner = lock(&self.inner);
        let live = inner
            .lives
            .get_mut(&live_id)
            .ok_or(SubscribeRejected::UnknownLive(live_id))?;
        let source = brp_capture::SourceInfo {
            width: live.info.source_width,
            height: live.info.source_height,
            fps: live.info.source_fps,
        };
        let format = live.fan.format();
        let fan = live.fan.clone();
        let state = live
            .presets
            .get_mut(&preset_id)
            .ok_or(SubscribeRejected::UnknownPreset(preset_id))?;
        let mut started = false;
        if state.running.is_none() {
            match self.encoders.open(source, format, &state.preset) {
                Ok(parts) => {
                    let slot = fan.attach();
                    // Presets at the source rate pass every frame; only slower presets are paced.
                    let pacer =
                        (state.preset.fps < source.fps).then(|| Pacer::new(state.preset.fps));
                    let publisher = Publisher::start(
                        live_id,
                        preset_id,
                        slot.clone(),
                        parts.converter,
                        parts.encoder,
                        pacer,
                    );
                    state.running = Some(RunningEncoder {
                        publisher,
                        slot,
                        idle_since: None,
                    });
                    state.last_error = None;
                    started = true;
                }
                Err(error) => {
                    state.last_error = Some(error.to_string());
                    drop(inner);
                    (self.on_change)();
                    return Err(SubscribeRejected::EncoderFailed(error.to_string()));
                }
            }
        }
        let running = state.running.as_mut().expect("set above");
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
        let mut inner = lock(&self.inner);
        if !inner.lives.contains_key(&live_id) {
            return Err(SubscribeRejected::UnknownLive(live_id));
        }
        if !inner.audio.advertises() {
            return Err(SubscribeRejected::NoAudio);
        }
        let mut started = false;
        if inner.audio.running.is_none() {
            let outcome = self
                .encoders
                .open_audio()
                .map_err(|e| e.to_string())
                .and_then(|encoder| {
                    let publisher = AudioPublisher::start(encoder);
                    match self.audio_capture.start(publisher.sink()) {
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
                });
            match outcome {
                Ok(running) => {
                    inner.audio.running = Some(running);
                    started = true;
                }
                Err(message) => {
                    tracing::warn!(%message, "audio capture failed to start");
                    inner.audio.last_error = Some(message);
                    drop(inner);
                    (self.on_change)();
                    return Err(SubscribeRejected::NoAudio);
                }
            }
        }
        let running = inner.audio.running.as_mut().expect("set above");
        running.idle_since = None;
        let subscription = AudioSubscription {
            params: running.publisher.params(),
            packets: running.publisher.subscribe(),
        };
        drop(inner);
        if started {
            (self.on_change)();
        }
        Ok(subscription)
    }
}

fn stop_encoder(fan: &CaptureFan, state: &mut PresetState) {
    if let Some(running) = state.running.take() {
        running.publisher.stop();
        fan.detach(&running.slot);
    }
}

/// Stops idle capture after the grace and turns a dead session into a recorded failure.
fn housekeep_audio(audio: &mut AudioState, now: Instant, grace: Duration) -> bool {
    let Some(running) = audio.running.as_mut() else {
        return false;
    };
    if let Some(error) = running.session.error() {
        stop_audio(audio);
        audio.last_error = Some(error);
        return true;
    }
    if running.publisher.subscriber_count() > 0 {
        running.idle_since = None;
        return false;
    }
    let idle_for = now.duration_since(*running.idle_since.get_or_insert(now));
    if idle_for >= grace {
        stop_audio(audio);
        return true;
    }
    false
}

fn stop_audio(audio: &mut AudioState) {
    if let Some(running) = audio.running.take() {
        running.publisher.stop();
        running.session.stop();
    }
}

fn validate_presets(
    presets: &[Preset],
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(), RoomError> {
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
