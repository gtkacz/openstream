use thiserror::Error;
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("FFmpeg has no encoder named {0}")]
    EncoderMissing(&'static str),
    #[error("FFmpeg has no decoder named {0}")]
    DecoderMissing(&'static str),
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
    #[error("no encoder available for {0:?}")]
    NoEncoder(brp_proto::Codec),
    #[error("no decoder available for {0:?}")]
    NoDecoder(brp_proto::Codec),
}
