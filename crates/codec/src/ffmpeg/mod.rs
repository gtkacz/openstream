//! FFmpeg-backed codec implementations over the raw `ffmpeg-sys-next` bindings.

pub mod convert;
pub mod decoder;
pub mod encoder;
pub(crate) mod ffi;
pub mod opus;
pub mod vaapi;

pub use convert::SwsConverter;
pub use decoder::{FfmpegDecoder, HwDecode};
pub use encoder::FfmpegEncoder;
pub use opus::{OpusDecoder, OpusEncoder};
pub use vaapi::VaapiEncoder;
