//! Read-only views the window renders. Cloned out of the room on every version bump.

use brp_net::PathKind;
use brp_proto::{LiveInfo, Preset};
use iroh::PublicKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderView {
    pub name: &'static str,
    pub subscribers: usize,
    pub frames_encoded: u64,
    pub bytes_encoded: u64,
    pub dropped_at_input: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetView {
    pub preset: Preset,
    pub encoder: Option<EncoderView>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnLiveView {
    pub info: LiveInfo,
    pub presets: Vec<PresetView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCaptureState {
    Off,
    /// Enabled, nobody listening: capture has not been started or was stopped after the grace.
    Idle,
    Capturing,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnAudioView {
    pub enabled: bool,
    pub state: AudioCaptureState,
    pub subscribers: usize,
    pub packets_encoded: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberView {
    pub id: PublicKey,
    pub nickname: String,
    pub lives: Vec<LiveInfo>,
    pub seen_ago_ms: u64,
    pub path: PathKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchState {
    Connecting,
    Live,
    Reconnecting,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchView {
    pub publisher: PublicKey,
    pub live_id: u32,
    pub preset_id: u32,
    pub state: WatchState,
    pub frames_decoded: u64,
    pub keyframe_requests: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSnapshot {
    pub me: PublicKey,
    pub nickname: String,
    pub version: u64,
    pub members: Vec<MemberView>,
    pub own_lives: Vec<OwnLiveView>,
    pub watches: Vec<WatchView>,
}
