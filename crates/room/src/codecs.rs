//! Where encoders and decoders come from. The room only knows these traits, so tests swap in fakes.

use std::sync::OnceLock;

use brp_capture::SourceInfo;
use brp_codec::ffmpeg::SwsConverter;
use brp_codec::{
    AudioDecoder, AudioEncoder, CodecError, EncoderConfig, FrameConverter, VideoDecoder,
    VideoEncoder, open_audio_decoder, open_audio_encoder, open_decoder, open_encoder,
};
use brp_proto::{AudioParams, Codec, CodecParams, PixelFormat, Preset};

pub struct EncoderParts {
    pub converter: Box<dyn FrameConverter>,
    pub encoder: Box<dyn VideoEncoder>,
}

pub trait EncoderFactory: Send + Sync + 'static {
    fn open(
        &self,
        source: SourceInfo,
        source_format: PixelFormat,
        preset: &Preset,
    ) -> Result<EncoderParts, CodecError>;

    /// The codec new lives default to. The real factory probes the GPU once; the spec prefers HEVC,
    /// then H.264, then the software AV1 fallback.
    fn preferred_codec(&self) -> Codec;

    fn open_audio(&self) -> Result<Box<dyn AudioEncoder>, CodecError>;
}

pub trait DecoderFactory: Send + Sync + 'static {
    fn open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError>;

    fn open_audio(&self, params: &AudioParams) -> Result<Box<dyn AudioDecoder>, CodecError>;
}

fn config_for(preset: &Preset) -> EncoderConfig {
    EncoderConfig {
        width: preset.width,
        height: preset.height,
        fps: preset.fps,
        bitrate_kbps: preset.bitrate_kbps,
        codec: preset.codec,
    }
}

/// The production factory: swscale for conversion, the spec's probe order for encoders,
/// hardware-first decoding.
#[derive(Debug, Default)]
pub struct FfmpegCodecs {
    probed_codec: OnceLock<Codec>,
}

impl EncoderFactory for FfmpegCodecs {
    fn open(
        &self,
        source: SourceInfo,
        source_format: PixelFormat,
        preset: &Preset,
    ) -> Result<EncoderParts, CodecError> {
        let converter = SwsConverter::new(
            source.width,
            source.height,
            source_format,
            preset.width,
            preset.height,
        )?;
        let encoder = open_encoder(&config_for(preset))?;
        Ok(EncoderParts {
            converter: Box::new(converter),
            encoder,
        })
    }

    fn preferred_codec(&self) -> Codec {
        *self.probed_codec.get_or_init(|| {
            let probe = EncoderConfig {
                width: 64,
                height: 64,
                fps: 30,
                bitrate_kbps: 1_000,
                codec: Codec::Hevc,
            };
            brp_codec::open_encoder_auto(probe, None)
                .map(|e| e.params().codec)
                .unwrap_or(Codec::Av1)
        })
    }

    fn open_audio(&self) -> Result<Box<dyn AudioEncoder>, CodecError> {
        open_audio_encoder()
    }
}

impl DecoderFactory for FfmpegCodecs {
    fn open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
        open_decoder(params)
    }

    fn open_audio(&self, params: &AudioParams) -> Result<Box<dyn AudioDecoder>, CodecError> {
        open_audio_decoder(params)
    }
}

pub mod fake {
    use brp_codec::fake::{
        FakeAudioDecoder, FakeAudioEncoder, FakeDecoder, FakeEncoder, SolidConverter,
    };

    use super::*;

    /// Keyframe every 30 frames, like a real encoder asked for periodic refresh.
    const FAKE_KEYFRAME_INTERVAL: u32 = 30;

    #[derive(Debug, Clone, Copy, Default)]
    pub struct FakeCodecs;

    impl EncoderFactory for FakeCodecs {
        fn open(
            &self,
            _source: SourceInfo,
            _format: PixelFormat,
            preset: &Preset,
        ) -> Result<EncoderParts, CodecError> {
            Ok(EncoderParts {
                converter: Box::new(SolidConverter::new(preset.width, preset.height)),
                encoder: Box::new(FakeEncoder::new(config_for(preset), FAKE_KEYFRAME_INTERVAL)),
            })
        }

        fn preferred_codec(&self) -> Codec {
            Codec::H264
        }

        fn open_audio(&self) -> Result<Box<dyn AudioEncoder>, CodecError> {
            Ok(Box::new(FakeAudioEncoder::default()))
        }
    }

    impl DecoderFactory for FakeCodecs {
        fn open(&self, _params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
            Ok(Box::new(FakeDecoder))
        }

        fn open_audio(&self, _params: &AudioParams) -> Result<Box<dyn AudioDecoder>, CodecError> {
            Ok(Box::new(FakeAudioDecoder))
        }
    }
}

#[cfg(test)]
mod tests {
    use brp_capture::SourceInfo;
    use brp_codec::RawFrame;
    use brp_proto::{Codec, PixelFormat, Preset};

    use super::*;

    #[test]
    fn fake_factory_builds_a_working_pair_for_the_preset() {
        let preset = Preset {
            id: 2,
            name: "720p".into(),
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 5_000,
            codec: Codec::Av1,
        };
        let parts = EncoderFactory::open(
            &fake::FakeCodecs,
            SourceInfo {
                width: 1920,
                height: 1080,
                fps: 60,
            },
            PixelFormat::Bgra,
            &preset,
        )
        .unwrap();
        let mut encoder = parts.encoder;
        let params = encoder.params();
        assert_eq!(
            (params.width, params.height, params.fps, params.codec),
            (1280, 720, 30, Codec::Av1)
        );
        let packets = encoder
            .encode(&RawFrame::black(1280, 720, 9), false)
            .unwrap();
        let mut decoder = DecoderFactory::open(&fake::FakeCodecs, &params).unwrap();
        assert_eq!(decoder.decode(&packets[0]).unwrap()[0].capture_ts_us, 9);
    }
}
