use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("capture failed: {0}")]
    Capture(#[from] brp_capture::CaptureError),
    #[error("codec failed: {0}")]
    Codec(#[from] brp_codec::CodecError),
    #[error("network failed: {0}")]
    Net(#[from] brp_net::NetError),
    #[error("unknown live {0}")]
    UnknownLive(u32),
    #[error("unknown preset {0}")]
    UnknownPreset(u32),
}
