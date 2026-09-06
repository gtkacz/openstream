use thiserror::Error;
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("portal denied")]
    PortalDenied,
    #[error("portal: {0}")]
    Portal(String),
    #[error("PipeWire: {0}")]
    PipeWire(String),
    #[error("source lost: {0}")]
    SourceLost(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("windows: {0}")]
    Windows(String),
}
