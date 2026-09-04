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

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> FrameHeader {
        FrameHeader {
            live_id: 1,
            preset_id: 1,
            kind: FrameKind::Video,
            seq: 42,
            capture_ts_us: 1_000_000,
            keyframe: true,
            len: 3,
        }
    }

    #[test]
    fn prefixed_frame_splits_into_header_and_payload() {
        let mut bytes = header().encode_prefix().unwrap();
        bytes.extend_from_slice(&[9, 8, 7]);
        let (h, payload) = FrameHeader::decode_prefixed(&bytes).unwrap();
        assert_eq!(h, header());
        assert_eq!(payload, &[9, 8, 7]);
    }

    #[test]
    fn prefixed_frame_rejects_length_mismatch() {
        let mut bytes = header().encode_prefix().unwrap();
        bytes.extend_from_slice(&[9, 8]);
        assert!(matches!(
            FrameHeader::decode_prefixed(&bytes),
            Err(ProtoError::LengthMismatch {
                declared: 3,
                actual: 2
            })
        ));
    }

    #[test]
    fn prefixed_frame_rejects_oversized_declared_length() {
        let mut header = header();
        header.len = (crate::constants::MAX_FRAME_BYTES + 1) as u32;
        let bytes = header.encode_prefix().unwrap();
        assert!(matches!(
            FrameHeader::decode_prefixed(&bytes),
            Err(ProtoError::FrameTooLarge(_))
        ));
    }
}
