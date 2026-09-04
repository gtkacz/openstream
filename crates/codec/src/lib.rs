//! Video encoder and decoder traits with a deterministic fake implementation.
pub mod error;
pub mod fake;
pub mod raw;
pub mod traits;
pub use error::CodecError;
pub use raw::RawFrame;
pub use traits::{EncoderConfig, FrameConverter, InputImage, VideoDecoder, VideoEncoder};
