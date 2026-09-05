//! FFmpeg-backed codec implementations over the raw `ffmpeg-sys-next` bindings.

pub mod convert;
pub mod decoder;
pub mod encoder;
pub(crate) mod ffi;
pub mod vaapi;

pub use convert::SwsConverter;
pub use decoder::{FfmpegDecoder, HwDecode};
pub use encoder::FfmpegEncoder;
pub use vaapi::VaapiEncoder;

/// Runs `f` with FFmpeg's log output silenced, then restores the normal level. Encoder probes
/// deliberately try backends the machine lacks, and their failure messages read like errors.
pub fn with_quiet_logs<T>(f: impl FnOnce() -> T) -> T {
    ffi::init_logging();
    unsafe { ffmpeg_sys_next::av_log_set_level(ffmpeg_sys_next::AV_LOG_QUIET as std::ffi::c_int) };
    let out = f();
    unsafe {
        ffmpeg_sys_next::av_log_set_level(ffmpeg_sys_next::AV_LOG_WARNING as std::ffi::c_int)
    };
    out
}
