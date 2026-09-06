//! Video encoder and decoder traits with a deterministic fake implementation.
pub mod audio;
pub mod error;
pub mod fake;
pub mod ffmpeg;
pub mod raw;
pub mod select;
pub mod traits;
pub use audio::{AudioDecoder, AudioEncoder, AudioFrame};
pub use error::CodecError;
pub use raw::RawFrame;
pub use select::{
    open_audio_decoder, open_audio_encoder, open_decoder, open_encoder, open_encoder_auto,
};
pub use traits::{EncoderConfig, FrameConverter, InputImage, VideoDecoder, VideoEncoder};
