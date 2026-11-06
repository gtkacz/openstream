//! A room: membership over signed gossip presence, published lives with lazy encoders, and watches.

pub mod codecs;
pub mod error;
mod gossip;
pub mod membership;
pub mod registry;
mod room;
pub mod snapshot;
mod source;
mod voice;
mod watcher;

pub use error::RoomError;
pub use registry::ChangeNotify;
pub use room::{Room, RoomConfig, RoomTimings};
pub use snapshot::{
    AudioCaptureState, EncoderView, MemberView, OwnAudioView, OwnLiveView, OwnVoiceView,
    PresetView, RoomSnapshot, VoiceLevels, VoiceSample, WatchState, WatchView,
};
pub use watcher::{FrameNotify, WatchHandle};
