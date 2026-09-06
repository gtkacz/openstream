use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("PipeWire: {0}")]
    PipeWire(String),
    #[error("windows: {0}")]
    Windows(String),
    #[error("unsupported on this system: {0}")]
    Unsupported(String),
    #[error("audio device: {0}")]
    Device(String),
    #[error("format refused: {0}")]
    Format(String),
}
