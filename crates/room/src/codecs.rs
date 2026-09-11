//! Where encoders and decoders come from. The room only knows these traits, so tests swap in fakes.

use std::sync::OnceLock;

use brp_capture::SourceInfo;
use brp_codec::ffmpeg::SwsConverter;
use brp_codec::{
    AudioDecoder, AudioEncoder, CodecError, EncoderConfig, FrameConverter, VideoDecoder,
    VideoEncoder, open_audio_decoder, open_audio_encoder, open_decoder, open_encoder,
};
use brp_proto::{AudioParams, Codec, CodecParams, PixelFormat, Preset};

pub trait EncoderFactory: Send + Sync + 'static {
    /// Builds the converter a group of subscribed presets shares: every preset whose output
    /// `width`/`height` matches converts through this one instance instead of each opening its own.
    fn open_converter(
        &self,
        source: SourceInfo,
        source_format: PixelFormat,
        width: u32,
        height: u32,
    ) -> Result<Box<dyn FrameConverter>, CodecError>;

    /// Opens the encoder for one preset. Always per-preset: fps, bitrate, and codec can differ even
    /// between presets that share a converter.
    fn open_encoder(&self, preset: &Preset) -> Result<Box<dyn VideoEncoder>, CodecError>;

    /// The codec new lives default to. The real factory probes the GPU once; the spec prefers HEVC,
    /// then H.264, then the software AV1 fallback.
    fn preferred_codec(&self) -> Codec;

    fn open_audio(&self) -> Result<Box<dyn AudioEncoder>, CodecError>;
}

pub trait DecoderFactory: Send + Sync + 'static {
    fn open(&self, params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError>;

    fn open_audio(&self, params: &AudioParams) -> Result<Box<dyn AudioDecoder>, CodecError>;
}

/// A size every hardware encoder accepts, used only to learn which codec the GPU offers. NVENC
/// rejects anything below roughly 145x49 for H.264 and more for AV1, so a tiny probe would skip
/// every hardware encoder and new lives would default to software AV1.
const PROBE_WIDTH: u32 = 640;
const PROBE_HEIGHT: u32 = 360;

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
    fn open_converter(
        &self,
        source: SourceInfo,
        source_format: PixelFormat,
        width: u32,
        height: u32,
    ) -> Result<Box<dyn FrameConverter>, CodecError> {
        Ok(Box::new(SwsConverter::new(
            source.width,
            source.height,
            source_format,
            width,
            height,
        )?))
    }

    fn open_encoder(&self, preset: &Preset) -> Result<Box<dyn VideoEncoder>, CodecError> {
        open_encoder(&config_for(preset))
    }

    fn preferred_codec(&self) -> Codec {
        *self.probed_codec.get_or_init(|| {
            let probe = EncoderConfig {
                width: PROBE_WIDTH,
                height: PROBE_HEIGHT,
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
        fn open_converter(
            &self,
            _source: SourceInfo,
            _format: PixelFormat,
            width: u32,
            height: u32,
        ) -> Result<Box<dyn FrameConverter>, CodecError> {
            Ok(Box::new(SolidConverter::new(width, height)))
        }

        fn open_encoder(&self, preset: &Preset) -> Result<Box<dyn VideoEncoder>, CodecError> {
            Ok(Box::new(FakeEncoder::new(config_for(preset), FAKE_KEYFRAME_INTERVAL)))
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
    use brp_codec::RawFrame;
    use brp_proto::{Codec, Preset};

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
        let mut encoder = EncoderFactory::open_encoder(&fake::FakeCodecs, &preset).unwrap();
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
