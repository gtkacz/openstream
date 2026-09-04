use crate::{constants::MAX_FRAME_BYTES, error::ProtoError};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameKind {
    Video,
    Audio,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameHeader {
    pub live_id: u32,
    pub preset_id: u32,
    pub kind: FrameKind,
    pub seq: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
    pub len: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub seq: u64,
    pub capture_ts_us: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}
impl FrameHeader {
    pub fn encode_prefix(&self) -> Result<Vec<u8>, ProtoError> {
        postcard::to_allocvec(self).map_err(ProtoError::Encode)
    }
    pub fn decode_prefixed(bytes: &[u8]) -> Result<(Self, &[u8]), ProtoError> {
        let (h, p): (Self, &[u8]) = postcard::take_from_bytes(bytes).map_err(ProtoError::Decode)?;
        if h.len as usize > MAX_FRAME_BYTES {
            return Err(ProtoError::FrameTooLarge(h.len));
        }
        if p.len() != h.len as usize {
            return Err(ProtoError::LengthMismatch {
                declared: h.len,
                actual: p.len(),
            });
        }
        Ok((h, p))
    }
}
