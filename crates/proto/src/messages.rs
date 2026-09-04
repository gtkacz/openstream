use crate::{
    constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS},
    error::ProtoError,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Monitor,
    Window,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub codec: Codec,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PresetError {
    #[error("width and height must be even for 4:2:0 encoding")]
    OddDimension,
    #[error("preset exceeds the source resolution")]
    LargerThanSource,
    #[error("preset frame rate exceeds the source frame rate")]
    FasterThanSource,
    #[error("bitrate must be between {MIN_BITRATE_KBPS} and {MAX_BITRATE_KBPS} kbps")]
    BitrateOutOfRange,
}
impl Preset {
    pub fn validate(&self, w: u32, h: u32, f: u32) -> Result<(), PresetError> {
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(PresetError::OddDimension);
        }
        if self.width > w || self.height > h {
            return Err(PresetError::LargerThanSource);
        }
        if self.fps == 0 || self.fps > f {
            return Err(PresetError::FasterThanSource);
        }
        if !(MIN_BITRATE_KBPS..=MAX_BITRATE_KBPS).contains(&self.bitrate_kbps) {
            return Err(PresetError::BitrateOutOfRange);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecParams {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub extradata: Vec<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioParams {
    pub sample_rate: u32,
    pub channels: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewerMessage {
    Subscribe {
        live_id: u32,
        preset_id: u32,
        want_audio: bool,
    },
    SwitchPreset {
        preset_id: u32,
    },
    RequestKeyframe,
    Unsubscribe,
    Stats {
        frames_received: u32,
        frames_dropped: u32,
        decode_fps: u16,
        rtt_ms: u16,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublisherMessage {
    SubscribeAck {
        video: CodecParams,
        audio: Option<AudioParams>,
    },
    SubscribeError {
        reason: String,
    },
    PresetSwitched {
        preset_id: u32,
        video: CodecParams,
    },
    LiveEnded,
}
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtoError> {
    postcard::to_allocvec(msg).map_err(ProtoError::Encode)
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtoError> {
    postcard::from_bytes(bytes).map_err(ProtoError::Decode)
}
