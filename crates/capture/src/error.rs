use thiserror::Error;
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("Not sharing: the system share dialog was closed or denied. Share again to reopen it.")]
    PortalDenied,
    #[error("portal: {0}")]
    Portal(String),
    #[error("PipeWire: {0}")]
    PipeWire(String),
    #[error("source lost: {0}")]
    SourceLost(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("Windows: {0}")]
    Windows(String),
    #[error(
        "Screen recording is not allowed. Turn on OpenStream under System Settings › Privacy & Security › Screen & System Audio Recording. ({0})"
    )]
    PermissionDenied(String),
    #[error("macOS: {0}")]
    MacOs(String),
}

impl CaptureError {
    /// Whether the user, or a policy, refused the capture, as opposed to it failing.
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::PortalDenied | Self::PermissionDenied(_))
    }
}
