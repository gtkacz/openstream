//! Hardware-first encoder selection and software decoder fallback.

use crate::error::CodecError;
use crate::ffmpeg::decoder::{FfmpegDecoder, HwDecode};
use crate::ffmpeg::encoder::FfmpegEncoder;
use crate::traits::{EncoderConfig, VideoDecoder, VideoEncoder};
use brp_proto::{Codec, CodecParams};

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
    ("hevc_vaapi", Codec::Hevc),
    ("h264_vaapi", Codec::H264),
    ("av1_vaapi", Codec::Av1),
    ("libsvtav1", Codec::Av1),
];

pub fn open_encoder(cfg: &EncoderConfig) -> Result<Box<dyn VideoEncoder>, CodecError> {
    for &(name, codec) in PROBE_ORDER.iter().filter(|(_, codec)| *codec == cfg.codec) {
        match open_named(name, cfg) {
            Ok(encoder) => return Ok(encoder),
            Err(error) => tracing::debug!(encoder = name, error = %error, "encoder unavailable"),
        }
    }
    Err(CodecError::NoEncoder(cfg.codec))
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
    match FfmpegDecoder::open(params, HwDecode::Auto) {
        Ok(decoder) => Ok(Box::new(decoder)),
        Err(error) => {
            tracing::warn!(error = %error, "hardware decoder failed, falling back to software");
            Ok(Box::new(
                FfmpegDecoder::open(params, HwDecode::Software)
                    .map_err(|_| CodecError::NoDecoder(params.codec))?,
            ))
        }
    }
}
