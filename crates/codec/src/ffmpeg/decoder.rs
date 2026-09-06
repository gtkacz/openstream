use crate::error::CodecError;
use crate::ffmpeg::ffi::{
    BufferRef, CodecContext, Frame, Packet, again, check, cstring, init_logging,
};
use crate::raw::RawFrame;
use crate::traits::VideoDecoder;
use brp_proto::{Codec, CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;
use std::ffi::{c_int, c_void};
use std::ptr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwDecode {
    Auto,
    Software,
}
pub struct FfmpegDecoder {
    ctx: CodecContext,
    _device: Option<BufferRef>,
    hw_pix_fmt: Option<ff::AVPixelFormat>,
    packet: Packet,
    frame: Frame,
    sw_frame: Frame,
    name: &'static str,
}
/// Hardware decoders tried before software: the platform's own API first, then NVDEC.
#[cfg(windows)]
const HW_DEVICE_ORDER: [(ff::AVHWDeviceType, &str); 2] = [
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, "d3d11va"),
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
];
/// Hardware decoders tried before software: the platform's own API first, then NVDEC.
#[cfg(not(windows))]
const HW_DEVICE_ORDER: [(ff::AVHWDeviceType, &str); 2] = [
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, "vaapi"),
    (ff::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, "cuda"),
];
impl FfmpegDecoder {
    pub fn open(params: &CodecParams, hw: HwDecode) -> Result<Self, CodecError> {
        init_logging();
        let (codec, name) = find_decoder(params.codec)?;
        let mut ctx = CodecContext::alloc(codec)?;
        let mut device = None;
        let mut hw_pix_fmt = None;
        if hw == HwDecode::Auto && params.codec != Codec::Av1 {
            for (device_type, _) in HW_DEVICE_ORDER {
                if let Some((candidate, format)) = try_hw_device(codec, device_type) {
                    unsafe {
                        (*ctx.0).hw_device_ctx = candidate.new_ref("av_buffer_ref")?;
                        (*ctx.0).opaque = format as isize as *mut c_void;
                        (*ctx.0).get_format = Some(pick_hw_format);
                    }
                    device = Some(candidate);
                    hw_pix_fmt = Some(format);
                    break;
                }
            }
        }
        unsafe {
            let c = &mut *ctx.0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
            c.thread_type = ff::FF_THREAD_SLICE as c_int;
            if !params.extradata.is_empty() {
                let size = params.extradata.len();
                let buffer =
                    ff::av_mallocz(size + ff::AV_INPUT_BUFFER_PADDING_SIZE as usize) as *mut u8;
                if buffer.is_null() {
                    return Err(CodecError::Ffmpeg {
                        call: "av_mallocz",
                        code: ff::AVERROR(ff::ENOMEM),
                        message: "returned null".into(),
                    });
                }
                ptr::copy_nonoverlapping(params.extradata.as_ptr(), buffer, size);
                c.extradata = buffer;
                c.extradata_size = size as c_int;
            }
        }
        ctx.open(codec)?;
        Ok(Self {
            ctx,
            _device: device,
            hw_pix_fmt,
            packet: Packet::new()?,
            frame: Frame::new()?,
            sw_frame: Frame::new()?,
            name,
        })
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn is_hardware(&self) -> bool {
        self.hw_pix_fmt.is_some()
    }
}
fn find_decoder(codec: Codec) -> Result<(*const ff::AVCodec, &'static str), CodecError> {
    let (pointer, name) = match codec {
        Codec::H264 => (
            unsafe { ff::avcodec_find_decoder(ff::AVCodecID::AV_CODEC_ID_H264) },
            "h264",
        ),
        Codec::Hevc => (
            unsafe { ff::avcodec_find_decoder(ff::AVCodecID::AV_CODEC_ID_HEVC) },
            "hevc",
        ),
        Codec::Av1 => {
            let value = cstring("libdav1d")?;
            (
                unsafe { ff::avcodec_find_decoder_by_name(value.as_ptr()) },
                "libdav1d",
            )
        }
    };
    if pointer.is_null() {
        Err(CodecError::DecoderMissing(name))
    } else {
        Ok((pointer, name))
    }
}
fn try_hw_device(
    codec: *const ff::AVCodec,
    device_type: ff::AVHWDeviceType,
) -> Option<(BufferRef, ff::AVPixelFormat)> {
    let mut format = None;
    for index in 0.. {
        let config = unsafe { ff::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            break;
        }
        let config = unsafe { &*config };
        if config.device_type == device_type
            && config.methods & ff::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as c_int != 0
        {
            format = Some(config.pix_fmt);
            break;
        }
    }
    let format = format?;
    let mut device_ptr = ptr::null_mut();
    if unsafe {
        ff::av_hwdevice_ctx_create(
            &mut device_ptr,
            device_type,
            ptr::null(),
            ptr::null_mut(),
            0,
        )
    } < 0
    {
        return None;
    }
    Some((
        BufferRef::from_raw("av_hwdevice_ctx_create", device_ptr).ok()?,
        format,
    ))
}
unsafe extern "C" fn pick_hw_format(
    ctx: *mut ff::AVCodecContext,
    formats: *const ff::AVPixelFormat,
) -> ff::AVPixelFormat {
    unsafe {
        let wanted = (*ctx).opaque as isize as c_int;
        let mut current = formats;
        while *current != ff::AVPixelFormat::AV_PIX_FMT_NONE {
            if *current as c_int == wanted {
                return *current;
            }
            current = current.add(1);
        }
        *formats
    }
}
impl VideoDecoder for FfmpegDecoder {
    fn decode(&mut self, encoded: &EncodedFrame) -> Result<Vec<RawFrame>, CodecError> {
        let size = c_int::try_from(encoded.data.len())
            .map_err(|_| CodecError::InvalidFrame("packet larger than c_int".into()))?;
        unsafe {
            check("av_new_packet", ff::av_new_packet(self.packet.0, size))?;
            ptr::copy_nonoverlapping(
                encoded.data.as_ptr(),
                (*self.packet.0).data,
                encoded.data.len(),
            );
            (*self.packet.0).pts = encoded.capture_ts_us as i64;
            (*self.packet.0).dts = encoded.capture_ts_us as i64;
            if encoded.keyframe {
                (*self.packet.0).flags |= ff::AV_PKT_FLAG_KEY;
            }
        }
        let sent = unsafe { ff::avcodec_send_packet(self.ctx.0, self.packet.0) };
        self.packet.unref();
        check("avcodec_send_packet", sent)?;
        let mut output = Vec::with_capacity(1);
        loop {
            let result = unsafe { ff::avcodec_receive_frame(self.ctx.0, self.frame.0) };
            if result == again() || result == ff::AVERROR_EOF {
                return Ok(output);
            }
            check("avcodec_receive_frame", result)?;
            let decoded = unsafe { &*self.frame.0 };
            let source = if let Some(format) = self
                .hw_pix_fmt
                .filter(|format| decoded.format == *format as c_int)
            {
                let _ = format;
                self.sw_frame.unref();
                check("av_hwframe_transfer_data", unsafe {
                    ff::av_hwframe_transfer_data(self.sw_frame.0, self.frame.0, 0)
                })?;
                unsafe { (*self.sw_frame.0).pts = decoded.pts };
                self.sw_frame.0
            } else {
                self.frame.0
            };
            output.push(raw_from_avframe(unsafe { &*source })?);
            self.frame.unref();
        }
    }
}
fn raw_from_avframe(frame: &ff::AVFrame) -> Result<RawFrame, CodecError> {
    let (width, height) = (frame.width as u32, frame.height as u32);
    let mut output = RawFrame::black(width, height, frame.pts.max(0) as u64);
    let rows = output.chroma_rows();
    let width = width as usize;
    unsafe {
        for row in 0..height as usize {
            ptr::copy_nonoverlapping(
                frame.data[0].add(row * frame.linesize[0] as usize),
                output.y.as_mut_ptr().add(row * output.y_stride),
                width,
            );
        }
        if frame.format == ff::AVPixelFormat::AV_PIX_FMT_NV12 as c_int {
            for row in 0..rows {
                ptr::copy_nonoverlapping(
                    frame.data[1].add(row * frame.linesize[1] as usize),
                    output.uv.as_mut_ptr().add(row * output.uv_stride),
                    width,
                );
            }
        } else if frame.format == ff::AVPixelFormat::AV_PIX_FMT_YUV420P as c_int
            || frame.format == ff::AVPixelFormat::AV_PIX_FMT_YUVJ420P as c_int
        {
            for row in 0..rows {
                let u = frame.data[1].add(row * frame.linesize[1] as usize);
                let v = frame.data[2].add(row * frame.linesize[2] as usize);
                let dst = &mut output.uv[row * output.uv_stride..row * output.uv_stride + width];
                for (index, pair) in dst.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    pair[0] = *u.add(index);
                    pair[1] = *v.add(index);
                }
            }
        } else {
            return Err(CodecError::InvalidFrame(format!(
                "decoder produced unsupported pixel format {}",
                frame.format
            )));
        }
    }
    Ok(output)
}
