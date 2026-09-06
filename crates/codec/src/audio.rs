//! Audio frame and codec traits. Frames are interleaved stereo float at 48 kHz.

use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES};
use brp_proto::{AudioParams, EncodedFrame};
use serde::{Deserialize, Serialize};

use crate::CodecError;

/// One block of interleaved stereo samples. Encoder input is exactly one Opus frame; decoder
/// output may be shorter on the first packet, where libopus drops its pre-skip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFrame {
    pub samples: Vec<f32>,
    pub capture_ts_us: u64,
}

impl AudioFrame {
    pub const FRAME_LEN: usize = AUDIO_FRAME_SAMPLES * AUDIO_CHANNELS as usize;

    pub fn silence(capture_ts_us: u64) -> Self {
        Self {
            samples: vec![0.0; Self::FRAME_LEN],
            capture_ts_us,
        }
    }

    /// Encoder input must be one whole frame.
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.samples.len() != Self::FRAME_LEN {
            return Err(CodecError::InvalidFrame(format!(
                "audio frame has {} samples, expected {}",
                self.samples.len(),
                Self::FRAME_LEN
            )));
        }
        Ok(())
    }
}

pub trait AudioEncoder: Send {
    fn name(&self) -> &'static str;
    fn params(&self) -> AudioParams;
    /// One frame in, normally one packet out. Packets are numbered from zero and always keyframes.
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<EncodedFrame>, CodecError>;
}

pub trait AudioDecoder: Send {
    fn decode(&mut self, packet: &EncodedFrame) -> Result<Vec<AudioFrame>, CodecError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_one_full_stereo_frame_of_zeros() {
        let frame = AudioFrame::silence(7);
        assert_eq!(frame.samples.len(), 1920);
        assert!(frame.samples.iter().all(|s| *s == 0.0));
        assert_eq!(frame.capture_ts_us, 7);
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn validate_rejects_a_frame_of_the_wrong_length() {
        let short = AudioFrame {
            samples: vec![0.0; 1918],
            capture_ts_us: 0,
        };
        assert!(matches!(short.validate(), Err(CodecError::InvalidFrame(_))));
    }
}
