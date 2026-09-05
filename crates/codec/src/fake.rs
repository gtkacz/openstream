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

#[cfg(test)]
mod tests {
    use brp_proto::{Codec, EncodedFrame, PixelFormat};

    use super::*;

    fn cfg() -> EncoderConfig {
        EncoderConfig {
            width: 8,
            height: 4,
            fps: 30,
            bitrate_kbps: 2_000,
            codec: Codec::H264,
        }
    }

    #[test]
    fn fake_codec_round_trips_frames_and_numbers_them() {
        let mut enc = FakeEncoder::new(cfg(), 3);
        let mut dec = FakeDecoder;
        for i in 0..5u64 {
            let frame = RawFrame::black(8, 4, i * 1000);
            let packets = enc.encode(&frame, false).unwrap();
            assert_eq!(packets.len(), 1);
            assert_eq!((packets[0].seq, packets[0].capture_ts_us), (i, i * 1000));
            assert_eq!(dec.decode(&packets[0]).unwrap(), vec![frame]);
        }
    }

    #[test]
    fn keyframes_follow_the_interval_and_can_be_forced() {
        let mut enc = FakeEncoder::new(cfg(), 3);
        let flags: Vec<bool> = (0..7)
            .map(|_| enc.encode(&RawFrame::black(8, 4, 0), false).unwrap()[0].keyframe)
            .collect();
        assert_eq!(flags, vec![true, false, false, true, false, false, true]);
        assert!(enc.encode(&RawFrame::black(8, 4, 0), true).unwrap()[0].keyframe);
    }

    #[test]
    fn decoder_rejects_garbage() {
        let bad = EncodedFrame {
            seq: 0,
            capture_ts_us: 0,
            keyframe: true,
            data: vec![0xff, 0x00, 0x13],
        };
        assert!(matches!(
            FakeDecoder.decode(&bad),
            Err(CodecError::FakePayload(_))
        ));
    }

    #[test]
    fn solid_converter_produces_target_size_and_keeps_timestamp() {
        let mut conv = SolidConverter::new(4, 2);
        let pixels = vec![0u8; 16 * 8 * 4];
        let img = InputImage {
            width: 16,
            height: 8,
            stride: 64,
            format: PixelFormat::Bgra,
            data: &pixels,
            capture_ts_us: 5,
        };
        let out = conv.convert(&img).unwrap();
        assert_eq!((out.width, out.height, out.capture_ts_us), (4, 2, 5));
        assert!(out.validate().is_ok());
    }
}
