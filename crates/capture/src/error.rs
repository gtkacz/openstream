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
    #[error(
        "screen recording is not allowed: turn on OpenStream under System Settings › Privacy & Security › Screen & System Audio Recording ({0})"
    )]
    PermissionDenied(String),
    #[error("macos: {0}")]
    MacOs(String),
}
