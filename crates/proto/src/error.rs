use thiserror::Error;
#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("failed to encode message: {0}")]
    Encode(postcard::Error),
    #[error("failed to decode message: {0}")]
    Decode(postcard::Error),
    #[error("frame declares {declared} bytes but {actual} follow the header")]
    LengthMismatch { declared: u32, actual: usize },
    #[error("frame of {0} bytes exceeds the maximum frame size")]
    FrameTooLarge(u32),
    #[error("ticket is malformed: {0}")]
    Ticket(String),
}
