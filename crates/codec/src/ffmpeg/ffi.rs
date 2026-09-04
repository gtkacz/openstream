//! Small RAII wrappers around FFmpeg's ownership-based C API.

use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr;
use std::sync::Once;

use ffmpeg_sys_next as ff;

use crate::error::CodecError;

pub(crate) fn init_logging() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { ff::av_log_set_level(ff::AV_LOG_WARNING as c_int) });
}

pub(crate) fn check(call: &'static str, code: c_int) -> Result<c_int, CodecError> {
    if code < 0 {
        Err(CodecError::Ffmpeg {
            call,
            code,
            message: error_string(code),
        })
    } else {
        Ok(code)
    }
}

pub(crate) fn error_string(code: c_int) -> String {
    let mut buf = [0 as c_char; ff::AV_ERROR_MAX_STRING_SIZE];
    unsafe {
        ff::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub(crate) const fn again() -> c_int {
    ff::AVERROR(ff::EAGAIN)
}

pub(crate) fn null_error(call: &'static str) -> CodecError {
    CodecError::Ffmpeg {
        call,
        code: ff::AVERROR(ff::ENOMEM),
        message: "returned null".into(),
    }
}

pub(crate) fn cstring(s: &str) -> Result<CString, CodecError> {
    CString::new(s)
        .map_err(|_| CodecError::InvalidFrame(format!("string contains a NUL byte: {s:?}")))
}

pub(crate) struct Frame(pub(crate) *mut ff::AVFrame);
impl Frame {
    pub(crate) fn new() -> Result<Self, CodecError> {
        let p = unsafe { ff::av_frame_alloc() };
        if p.is_null() {
            Err(null_error("av_frame_alloc"))
        } else {
            Ok(Self(p))
        }
    }
    pub(crate) fn unref(&mut self) {
        unsafe { ff::av_frame_unref(self.0) }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) }
    }
}

pub(crate) struct Packet(pub(crate) *mut ff::AVPacket);
impl Packet {
    pub(crate) fn new() -> Result<Self, CodecError> {
        let p = unsafe { ff::av_packet_alloc() };
        if p.is_null() {
            Err(null_error("av_packet_alloc"))
        } else {
            Ok(Self(p))
        }
    }
    pub(crate) fn unref(&mut self) {
        unsafe { ff::av_packet_unref(self.0) }
    }
    pub(crate) fn data(&self) -> &[u8] {
        unsafe {
            let p = &*self.0;
            if p.data.is_null() || p.size <= 0 {
                &[]
            } else {
                std::slice::from_raw_parts(p.data, p.size as usize)
            }
        }
    }
    pub(crate) fn is_keyframe(&self) -> bool {
        unsafe { (*self.0).flags & ff::AV_PKT_FLAG_KEY != 0 }
    }
}
impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ff::av_packet_free(&mut self.0) }
    }
}

pub(crate) struct CodecContext(pub(crate) *mut ff::AVCodecContext);
impl CodecContext {
    pub(crate) fn alloc(codec: *const ff::AVCodec) -> Result<Self, CodecError> {
        let p = unsafe { ff::avcodec_alloc_context3(codec) };
        if p.is_null() {
            Err(null_error("avcodec_alloc_context3"))
        } else {
            Ok(Self(p))
        }
    }
    pub(crate) fn open(&mut self, codec: *const ff::AVCodec) -> Result<(), CodecError> {
        check("avcodec_open2", unsafe {
            ff::avcodec_open2(self.0, codec, ptr::null_mut())
        })
        .map(|_| ())
    }
    pub(crate) fn extradata(&self) -> Vec<u8> {
        unsafe {
            let c = &*self.0;
            if c.extradata.is_null() || c.extradata_size <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.extradata, c.extradata_size as usize).to_vec()
            }
        }
    }
}
impl Drop for CodecContext {
    fn drop(&mut self) {
        unsafe { ff::avcodec_free_context(&mut self.0) }
    }
}

pub(crate) struct BufferRef(pub(crate) *mut ff::AVBufferRef);
impl BufferRef {
    pub(crate) fn from_raw(
        call: &'static str,
        p: *mut ff::AVBufferRef,
    ) -> Result<Self, CodecError> {
        if p.is_null() {
            Err(null_error(call))
        } else {
            Ok(Self(p))
        }
    }
    pub(crate) fn new_ref(&self, call: &'static str) -> Result<*mut ff::AVBufferRef, CodecError> {
        let p = unsafe { ff::av_buffer_ref(self.0) };
        if p.is_null() {
            Err(null_error(call))
        } else {
            Ok(p)
        }
    }
}
impl Drop for BufferRef {
    fn drop(&mut self) {
        unsafe { ff::av_buffer_unref(&mut self.0) }
    }
}

pub(crate) fn set_opt(ctx: &CodecContext, name: &str, value: &str) -> Result<(), CodecError> {
    let (k, v) = (cstring(name)?, cstring(value)?);
    check("av_opt_set", unsafe {
        ff::av_opt_set((*ctx.0).priv_data, k.as_ptr(), v.as_ptr(), 0)
    })
    .map(|_| ())
}
pub(crate) fn set_opt_int(ctx: &CodecContext, name: &str, value: i64) -> Result<(), CodecError> {
    let k = cstring(name)?;
    check("av_opt_set_int", unsafe {
        ff::av_opt_set_int((*ctx.0).priv_data, k.as_ptr(), value, 0)
    })
    .map(|_| ())
}

unsafe impl Send for Frame {}
unsafe impl Send for Packet {}
unsafe impl Send for CodecContext {}
unsafe impl Send for BufferRef {}
