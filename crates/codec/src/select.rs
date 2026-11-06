//! Hardware-first encoder selection and software decoder fallback.

use crate::audio::{AudioDecoder, AudioEncoder};
use crate::error::CodecError;
use crate::ffmpeg::decoder::{FfmpegDecoder, HwDecode};
use crate::ffmpeg::encoder::FfmpegEncoder;
use crate::ffmpeg::opus::{OpusDecoder, OpusEncoder};
use crate::traits::{EncoderConfig, VideoDecoder, VideoEncoder};
use brp_proto::{AudioParams, ChromaLayout, Codec, CodecParams};

pub const PROBE_ORDER: &[(&str, Codec)] = &[
    ("hevc_nvenc", Codec::Hevc),
    ("h264_nvenc", Codec::H264),
    ("av1_nvenc", Codec::Av1),
    ("hevc_amf", Codec::Hevc),
    ("h264_amf", Codec::H264),
    ("av1_amf", Codec::Av1),
    ("hevc_qsv", Codec::Hevc),
    ("h264_qsv", Codec::H264),
    ("av1_qsv", Codec::Av1),
    ("hevc_mf", Codec::Hevc),
    ("h264_mf", Codec::H264),
    ("hevc_videotoolbox", Codec::Hevc),
    ("h264_videotoolbox", Codec::H264),
    ("hevc_vaapi", Codec::Hevc),
    ("h264_vaapi", Codec::H264),
    ("av1_vaapi", Codec::Av1),
    ("libsvtav1", Codec::Av1),
];

/// True for the software encoder at the tail of [`PROBE_ORDER`], so the UI can explain to the
/// user in plain language why an encoder is running on the CPU instead of the GPU. Detailed
/// per-attempt failures for the hardware encoders that were tried first stay in the logs.
pub fn is_software_encoder(name: &str) -> bool {
    name == "libsvtav1"
}

/// Whether the encoder called `name` is worth opening at `chroma`. Of the encoders probed only
/// NVENC takes planar 4:4:4 and a lossless tune; the rest would fail, or quietly send 4:2:0.
fn offers(name: &str, chroma: ChromaLayout) -> bool {
    chroma == ChromaLayout::Half || matches!(name, "hevc_nvenc" | "h264_nvenc")
}

pub fn open_encoder(cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError> {
    let chroma = cfg.quality.chroma();
    for &(name, _) in PROBE_ORDER
        .iter()
        .filter(|(name, codec)| *codec == cfg.codec && offers(name, chroma))
    {
        match open_named(name, cfg) {
            Ok(encoder) => {
                tracing::info!(encoder = name, codec = ?cfg.codec, "encoder opened");
                return Ok(encoder);
            }
            Err(error) => tracing::debug!(encoder = name, error = %error, "encoder unavailable"),
        }
    }
    Err(match chroma {
        ChromaLayout::Half => CodecError::NoEncoder(cfg.codec),
        ChromaLayout::Full => CodecError::NoFullChromaEncoder(cfg.codec),
    })
}

pub fn open_encoder_auto(
    cfg: EncoderConfig,
    codec: Option<Codec>,
) -> Result<Box<dyn VideoEncoder>, CodecError> {
    let order = codec.map_or_else(
        || vec![Codec::Hevc, Codec::H264, Codec::Av1],
        |codec| vec![codec],
    );
    let mut last = CodecError::NoEncoder(cfg.codec);
    for codec in order {
        match open_encoder(&EncoderConfig { codec, ..cfg }) {
            Ok(encoder) => return Ok(encoder),
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn open_named(
    name: &'static str,
    cfg: &EncoderConfig,
) -> Result<Box<dyn VideoEncoder>, CodecError> {
    if name.ends_with("_vaapi") {
        Ok(Box::new(crate::ffmpeg::VaapiEncoder::open(name, cfg)?))
    } else {
        Ok(Box::new(FfmpegEncoder::open(name, cfg)?))
    }
}

pub fn open_decoder(params: &CodecParams) -> Result<Box<dyn VideoDecoder>, CodecError> {
    let decoder = match FfmpegDecoder::open(params, HwDecode::Auto) {
        Ok(decoder) => decoder,
        Err(error) => {
            tracing::warn!(error = %error, "hardware decoder failed, falling back to software");
            FfmpegDecoder::open(params, HwDecode::Software)
                .map_err(|_| CodecError::NoDecoder(params.codec))?
        }
    };
    tracing::info!(
        decoder = decoder.name(),
        hardware = decoder.hardware_device().unwrap_or("none"),
        codec = ?params.codec,
        "decoder opened"
    );
    Ok(Box::new(decoder))
}

pub fn open_audio_encoder() -> Result<Box<dyn AudioEncoder>, CodecError> {
    Ok(Box::new(OpusEncoder::open()?))
}

pub fn open_voice_encoder() -> Result<Box<dyn AudioEncoder>, CodecError> {
    Ok(Box::new(OpusEncoder::open_voice()?))
}

pub fn open_audio_decoder(params: &AudioParams) -> Result<Box<dyn AudioDecoder>, CodecError> {
    Ok(Box::new(OpusDecoder::open(params)?))
}

#[cfg(test)]
mod tests {
    use super::{ChromaLayout, PROBE_ORDER, offers};

    fn position(name: &str) -> usize {
        PROBE_ORDER
            .iter()
            .position(|(candidate, _)| *candidate == name)
            .unwrap_or_else(|| panic!("{name} is not in the probe order"))
    }

    #[test]
    fn full_chroma_is_only_asked_of_encoders_that_offer_it() {
        let full: Vec<&str> = PROBE_ORDER
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| offers(name, ChromaLayout::Full))
            .collect();
        assert_eq!(full, ["hevc_nvenc", "h264_nvenc"]);
        assert!(
            PROBE_ORDER
                .iter()
                .all(|(name, _)| offers(name, ChromaLayout::Half))
        );
    }

    #[test]
    fn videotoolbox_is_probed_after_media_foundation_and_before_the_software_fallback() {
        assert!(position("h264_mf") < position("hevc_videotoolbox"));
        assert!(position("hevc_videotoolbox") < position("h264_videotoolbox"));
        assert!(position("h264_videotoolbox") < position("libsvtav1"));
        assert!(!super::is_software_encoder("hevc_videotoolbox"));
        assert!(!super::is_software_encoder("h264_videotoolbox"));
    }

    #[test]
    fn media_foundation_is_probed_after_qsv_and_before_the_software_fallback() {
        assert!(position("av1_qsv") < position("hevc_mf"));
        assert!(position("hevc_mf") < position("h264_mf"));
        assert!(position("h264_mf") < position("libsvtav1"));
    }
}
