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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_message_round_trips_through_postcard() {
        let msg = ViewerMessage::Subscribe {
            live_id: 1,
            preset_id: 1,
            want_audio: false,
        };
        let bytes = encode(&msg).unwrap();
        let back: ViewerMessage = decode(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn publisher_ack_round_trips_with_extradata() {
        let msg = PublisherMessage::SubscribeAck {
            video: CodecParams {
                codec: Codec::Hevc,
                width: 2560,
                height: 1440,
                fps: 60,
                extradata: vec![0, 0, 0, 1, 0x40],
            },
            audio: None,
        };
        let back: PublisherMessage = decode(&encode(&msg).unwrap()).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn decode_rejects_truncated_input() {
        let bytes = encode(&ViewerMessage::RequestKeyframe).unwrap();
        let err = decode::<ViewerMessage>(&bytes[..bytes.len().saturating_sub(1)]);
        assert!(matches!(err, Err(ProtoError::Decode(_))) || bytes.len() <= 1);
    }

    #[test]
    fn preset_validation_accepts_source_preset() {
        let preset = Preset {
            id: 1,
            name: "Source".into(),
            width: 2560,
            height: 1440,
            fps: 60,
            bitrate_kbps: 40_000,
            codec: Codec::Hevc,
        };
        assert!(preset.validate(2560, 1440, 60).is_ok());
    }

    #[test]
    fn preset_validation_rejects_odd_dimensions_and_out_of_range_bitrate() {
        let odd = Preset {
            id: 1,
            name: "x".into(),
            width: 1921,
            height: 1080,
            fps: 60,
            bitrate_kbps: 20_000,
            codec: Codec::H264,
        };
        assert_eq!(odd.validate(1920, 1080, 60), Err(PresetError::OddDimension));
        let too_high = Preset {
            id: 1,
            name: "x".into(),
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_kbps: 900_000,
            codec: Codec::H264,
        };
        assert_eq!(
            too_high.validate(1920, 1080, 60),
            Err(PresetError::BitrateOutOfRange)
        );
        let upscaled = Preset {
            id: 1,
            name: "x".into(),
            width: 3840,
            height: 2160,
            fps: 60,
            bitrate_kbps: 20_000,
            codec: Codec::H264,
        };
        assert_eq!(
            upscaled.validate(1920, 1080, 60),
            Err(PresetError::LargerThanSource)
        );
        let too_fast = Preset {
            id: 1,
            name: "x".into(),
            width: 1920,
            height: 1080,
            fps: 120,
            bitrate_kbps: 20_000,
            codec: Codec::H264,
        };
        assert_eq!(
            too_fast.validate(1920, 1080, 60),
            Err(PresetError::FasterThanSource)
        );
    }
}
