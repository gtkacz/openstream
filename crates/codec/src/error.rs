use thiserror::Error;
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("fake codec payload is corrupt: {0}")]
    FakePayload(#[from] postcard::Error),
    #[error("no encoder available for {0:?}")]
    NoEncoder(brp_proto::Codec),
    #[error("no decoder available for {0:?}")]
    NoDecoder(brp_proto::Codec),
}
