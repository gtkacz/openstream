//! Lives this participant publishes. Encoders exist only while someone is subscribed.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use brp_capture::{CaptureFrame, CaptureSession};
use brp_net::{LiveSource, SubscribeRejected, Subscription};
use brp_pipeline::{LatestSlot, Publisher};
use brp_proto::constants::{MAX_LIVES_PER_PARTICIPANT, MAX_PRESETS_PER_LIVE};
use brp_proto::{LiveInfo, PixelFormat, Preset, ProtoError, SourceKind};

use crate::codecs::EncoderFactory;
use crate::error::RoomError;
use crate::snapshot::{EncoderView, OwnLiveView, PresetView};

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

struct Inner {
    lives: BTreeMap<u32, OwnLive>,
    next_live_id: u32,
}

pub struct LiveRegistry {
    inner: Mutex<Inner>,
    encoders: Arc<dyn EncoderFactory>,
    grace: Duration,
    on_change: ChangeNotify,
}

impl LiveRegistry {
    pub fn new(
        encoders: Arc<dyn EncoderFactory>,
        grace: Duration,
        on_change: ChangeNotify,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                lives: BTreeMap::new(),
                next_live_id: 1,
            }),
            encoders,
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
        let info = LiveInfo {
            id,
            title,
            kind,
            source_width: source.width,
            source_height: source.height,
            source_fps: source.fps,
            has_audio: false,
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
        lock(&self.inner)
            .lives
            .values()
            .map(|l| l.info.clone())
            .collect()
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
        if stopped_any {
            (self.on_change)();
        }
    }

    pub fn stop_all(&self) {
        let ids: Vec<u32> = lock(&self.inner).lives.keys().copied().collect();
        for id in ids {
            let _ = self.remove_live(id);
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
                    let publisher = Publisher::start(
                        live_id,
                        preset_id,
                        slot.clone(),
                        parts.converter,
                        parts.encoder,
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
}

fn stop_encoder(fan: &CaptureFan, state: &mut PresetState) {
    if let Some(running) = state.running.take() {
        running.publisher.stop();
        fan.detach(&running.slot);
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
