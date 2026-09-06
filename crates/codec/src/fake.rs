use crate::audio::{AudioDecoder, AudioEncoder, AudioFrame};
use crate::traits::*;
use crate::{CodecError, RawFrame};
use brp_proto::{AudioParams, CodecParams, EncodedFrame};
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

/// Carries the samples through as 16-bit PCM so integration tests can assert on what reaches the
/// output. A frame of raw `f32` would be 7680 bytes, over the wire's `MAX_AUDIO_PACKET_BYTES`.
#[derive(Default)]
pub struct FakeAudioEncoder {
    next: u64,
}
impl AudioEncoder for FakeAudioEncoder {
    fn name(&self) -> &'static str {
        "fake-audio"
    }
    fn params(&self) -> AudioParams {
        AudioParams::STANDARD
    }
    fn encode(&mut self, f: &AudioFrame) -> Result<Vec<EncodedFrame>, CodecError> {
        f.validate()?;
        let mut data = Vec::with_capacity(f.samples.len() * 2);
        for sample in &f.samples {
            let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            data.extend_from_slice(&scaled.to_le_bytes());
        }
        let p = EncodedFrame {
            seq: self.next,
            capture_ts_us: f.capture_ts_us,
            keyframe: true,
            data,
        };
        self.next += 1;
        Ok(vec![p])
    }
}
pub struct FakeAudioDecoder;
impl AudioDecoder for FakeAudioDecoder {
    fn decode(&mut self, p: &EncodedFrame) -> Result<Vec<AudioFrame>, CodecError> {
        let (pairs, rest) = p.data.as_chunks::<2>();
        if !rest.is_empty() || pairs.len() != AudioFrame::FRAME_LEN {
            return Err(CodecError::InvalidFrame(format!(
                "fake audio packet holds {} bytes, not one frame",
                p.data.len()
            )));
        }
        Ok(vec![AudioFrame {
            samples: pairs
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / f32::from(i16::MAX))
                .collect(),
            capture_ts_us: p.capture_ts_us,
        }])
    }
}

#[cfg(test)]
mod tests {
    use brp_proto::{Codec, EncodedFrame, PixelFormat};

    use super::*;
    use crate::audio::{AudioDecoder, AudioEncoder, AudioFrame};

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

    #[test]
    fn fake_audio_codec_round_trips_frames_and_numbers_packets() {
        let mut enc = FakeAudioEncoder::default();
        let mut dec = FakeAudioDecoder;
        for i in 0..3u64 {
            let mut frame = AudioFrame::silence(i * 20_000);
            frame.samples[0] = i as f32 / 4.0;
            frame.samples[1] = -(i as f32) / 4.0;
            let packets = enc.encode(&frame).unwrap();
            assert_eq!(packets.len(), 1);
            assert_eq!(
                (
                    packets[0].seq,
                    packets[0].capture_ts_us,
                    packets[0].keyframe
                ),
                (i, i * 20_000, true)
            );
            assert!(
                packets[0].data.len() <= brp_proto::constants::MAX_AUDIO_PACKET_BYTES,
                "a fake packet must fit the wire's audio cap"
            );
            let back = dec.decode(&packets[0]).unwrap();
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].capture_ts_us, i * 20_000);
            assert!((back[0].samples[0] - i as f32 / 4.0).abs() < 1e-3);
            assert!((back[0].samples[1] + i as f32 / 4.0).abs() < 1e-3);
            assert!(back[0].samples[2..].iter().all(|s| *s == 0.0));
        }
        assert_eq!(enc.params(), brp_proto::AudioParams::STANDARD);
    }

    #[test]
    fn the_fake_audio_decoder_refuses_a_packet_that_is_not_one_frame() {
        assert!(matches!(
            FakeAudioDecoder.decode(&EncodedFrame {
                seq: 0,
                capture_ts_us: 0,
                keyframe: true,
                data: vec![0; 8],
            }),
            Err(CodecError::InvalidFrame(_))
        ));
    }

    #[test]
    fn fake_audio_encoder_rejects_a_partial_frame() {
        let short = AudioFrame {
            samples: vec![0.0; 10],
            capture_ts_us: 0,
        };
        assert!(matches!(
            FakeAudioEncoder::default().encode(&short),
            Err(CodecError::InvalidFrame(_))
        ));
    }
}
