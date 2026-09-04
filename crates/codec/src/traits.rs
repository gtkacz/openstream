use crate::{CodecError, RawFrame};
use brp_proto::{Codec, CodecParams, EncodedFrame, PixelFormat};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub codec: Codec,
}
#[derive(Debug, Clone, Copy)]
pub struct InputImage<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub data: &'a [u8],
    pub capture_ts_us: u64,
}
pub trait VideoEncoder: Send {
    fn name(&self) -> &'static str;
    fn params(&self) -> CodecParams;
    fn encode(
        &mut self,
        frame: &RawFrame,
        force_keyframe: bool,
    ) -> Result<Vec<EncodedFrame>, CodecError>;
}
pub trait VideoDecoder: Send {
    fn decode(&mut self, frame: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError>;
}
pub trait FrameConverter: Send {
    fn convert(&mut self, src: &InputImage<'_>) -> Result<RawFrame, CodecError>;
}
