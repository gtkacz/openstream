use crate::traits::*;
use crate::{CodecError, RawFrame};
use brp_proto::{CodecParams, EncodedFrame};
pub struct FakeEncoder {
    cfg: EncoderConfig,
    next: u64,
    interval: u64,
}
impl FakeEncoder {
    pub fn new(cfg: EncoderConfig, i: u32) -> Self {
        Self {
            cfg,
            next: 0,
            interval: u64::from(i.max(1)),
        }
    }
}
impl VideoEncoder for FakeEncoder {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn params(&self) -> CodecParams {
        CodecParams {
            codec: self.cfg.codec,
            width: self.cfg.width,
            height: self.cfg.height,
            fps: self.cfg.fps,
            extradata: b"fake".to_vec(),
        }
    }
    fn encode(&mut self, f: &RawFrame, force: bool) -> Result<Vec<EncodedFrame>, CodecError> {
        let p = EncodedFrame {
            seq: self.next,
            capture_ts_us: f.capture_ts_us,
            keyframe: force || self.next.is_multiple_of(self.interval),
            data: postcard::to_allocvec(f)?,
        };
        self.next += 1;
        Ok(vec![p])
    }
}
pub struct FakeDecoder;
impl VideoDecoder for FakeDecoder {
    fn decode(&mut self, f: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError> {
        Ok(vec![postcard::from_bytes(&f.data)?])
    }
}
pub struct SolidConverter {
    w: u32,
    h: u32,
}
impl SolidConverter {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }
}
impl FrameConverter for SolidConverter {
    fn convert(&mut self, s: &InputImage<'_>) -> Result<RawFrame, CodecError> {
        Ok(RawFrame::black(self.w, self.h, s.capture_ts_us))
    }
}
