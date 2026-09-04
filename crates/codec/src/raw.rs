use crate::CodecError;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub y_stride: usize,
    pub uv_stride: usize,
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
    pub capture_ts_us: u64,
}
impl RawFrame {
    pub fn black(w: u32, h: u32, ts: u64) -> Self {
        let s = w as usize;
        Self {
            width: w,
            height: h,
            y_stride: s,
            uv_stride: s,
            y: vec![16; s * h as usize],
            uv: vec![128; s * (h as usize).div_ceil(2)],
            capture_ts_us: ts,
        }
    }
    pub fn chroma_rows(&self) -> usize {
        (self.height as usize).div_ceil(2)
    }
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(CodecError::InvalidFrame(
                "dimensions must be even and non-zero".into(),
            ));
        }
        if self.y_stride < self.width as usize || self.uv_stride < self.width as usize {
            return Err(CodecError::InvalidFrame("stride shorter than width".into()));
        }
        if self.y.len() < self.y_stride * self.height as usize
            || self.uv.len() < self.uv_stride * self.chroma_rows()
        {
            return Err(CodecError::InvalidFrame(
                "buffer shorter than stride".into(),
            ));
        }
        Ok(())
    }
}
