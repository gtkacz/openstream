use thiserror::Error;
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("FFmpeg has no encoder named {0}")]
    EncoderMissing(&'static str),
    #[error("FFmpeg has no decoder named {0}")]
    DecoderMissing(&'static str),
    #[error("the {0} runtime libraries are not installed")]
    HwRuntimeMissing(&'static str),
    #[error("{call} failed with code {code}: {message}")]
    Ffmpeg {
        call: &'static str,
        code: i32,
        message: String,
    },
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("fake codec payload is corrupt: {0}")]
    FakePayload(#[from] postcard::Error),
    #[error("no encoder available for {}", .0.name())]
    NoEncoder(brp_proto::Codec),
    #[error("no encoder on this machine can send {} at 4:4:4", .0.name())]
    NoFullChromaEncoder(brp_proto::Codec),
    #[error("no decoder available for {}", .0.name())]
    NoDecoder(brp_proto::Codec),
}
