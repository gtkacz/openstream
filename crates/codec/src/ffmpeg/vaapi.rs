use crate::error::CodecError;
use crate::ffmpeg::ffi::{
    BufferRef, CodecContext, Frame, Packet, again, check, cstring, init_logging, set_opt,
    set_opt_int,
};
use crate::raw::RawFrame;
use crate::traits::{EncoderConfig, VideoEncoder};
use brp_proto::{CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;
use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;

const SURFACE_POOL_SIZE: c_int = 20;
pub struct VaapiEncoder {
    ctx: CodecContext,
    _device: BufferRef,
    _frames: BufferRef,
    sw_frame: Frame,
    hw_frame: Frame,
    packet: Packet,
    name: &'static str,
    cfg: EncoderConfig,
    next_seq: u64,
    next_pts: i64,
    in_flight: VecDeque<(i64, u64)>,
}
impl VaapiEncoder {
    pub fn open(name: &'static str, cfg: &EncoderConfig) -> Result<Self, CodecError> {
        init_logging();
        let cname = cstring(name)?;
        let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
        if codec.is_null() {
            return Err(CodecError::EncoderMissing(name));
        }
        let mut device_ptr = ptr::null_mut();
        check("av_hwdevice_ctx_create", unsafe {
            ff::av_hwdevice_ctx_create(
                &mut device_ptr,
                ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        })?;
        let device = BufferRef::from_raw("av_hwdevice_ctx_create", device_ptr)?;
        let frames = BufferRef::from_raw("av_hwframe_ctx_alloc", unsafe {
            ff::av_hwframe_ctx_alloc(device.0)
        })?;
        unsafe {
            let fc = &mut *((*frames.0).data as *mut ff::AVHWFramesContext);
            fc.format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            fc.sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
            fc.width = cfg.width as c_int;
            fc.height = cfg.height as c_int;
            fc.initial_pool_size = SURFACE_POOL_SIZE;
        }
        check("av_hwframe_ctx_init", unsafe {
            ff::av_hwframe_ctx_init(frames.0)
        })?;
        let mut ctx = CodecContext::alloc(codec)?;
        unsafe {
            let c = &mut *ctx.0;
            c.width = cfg.width as c_int;
            c.height = cfg.height as c_int;
            c.time_base = ff::AVRational {
                num: 1,
                den: cfg.fps as c_int,
            };
            c.framerate = ff::AVRational {
                num: cfg.fps as c_int,
                den: 1,
            };
            c.pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            c.hw_frames_ctx = frames.new_ref("av_buffer_ref")?;
            c.bit_rate = i64::from(cfg.bitrate_kbps) * 1000;
            c.rc_max_rate = c.bit_rate;
            c.rc_buffer_size = (c.bit_rate / i64::from(cfg.fps.max(1))) as c_int;
            c.gop_size = c_int::MAX;
            c.max_b_frames = 0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
        }
        set_opt(&ctx, "rc_mode", "CBR")?;
        set_opt_int(&ctx, "async_depth", 1)?;
        ctx.open(codec)?;
        let sw_frame = Frame::new()?;
        unsafe {
            let f = &mut *sw_frame.0;
            f.width = cfg.width as c_int;
            f.height = cfg.height as c_int;
            f.format = ff::AVPixelFormat::AV_PIX_FMT_NV12 as c_int;
            check(
                "av_frame_get_buffer",
                ff::av_frame_get_buffer(sw_frame.0, 0),
            )?;
        }
        Ok(Self {
            ctx,
            _device: device,
            _frames: frames,
            sw_frame,
            hw_frame: Frame::new()?,
            packet: Packet::new()?,
            name,
            cfg: *cfg,
            next_seq: 0,
            next_pts: 0,
            in_flight: VecDeque::new(),
        })
    }
    fn upload(&mut self, src: &RawFrame) -> Result<(), CodecError> {
        check("av_frame_make_writable", unsafe {
            ff::av_frame_make_writable(self.sw_frame.0)
        })?;
        let f = unsafe { &mut *self.sw_frame.0 };
        let width = src.width as usize;
        unsafe {
            for row in 0..src.height as usize {
                ptr::copy_nonoverlapping(
                    src.y.as_ptr().add(row * src.y_stride),
                    f.data[0].add(row * f.linesize[0] as usize),
                    width,
                );
            }
            for row in 0..src.chroma_rows() {
                ptr::copy_nonoverlapping(
                    src.uv.as_ptr().add(row * src.uv_stride),
                    f.data[1].add(row * f.linesize[1] as usize),
                    width,
                );
            }
            self.hw_frame.unref();
            check(
                "av_hwframe_get_buffer",
                ff::av_hwframe_get_buffer((*self.ctx.0).hw_frames_ctx, self.hw_frame.0, 0),
            )?;
            check(
                "av_hwframe_transfer_data",
                ff::av_hwframe_transfer_data(self.hw_frame.0, self.sw_frame.0, 0),
            )?;
        }
        Ok(())
    }
    fn drain(&mut self, output: &mut Vec<EncodedFrame>) -> Result<(), CodecError> {
        loop {
            let result = unsafe { ff::avcodec_receive_packet(self.ctx.0, self.packet.0) };
            if result == again() || result == ff::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", result)?;
            let pts = unsafe { (*self.packet.0).pts };
            let timestamp = self
                .in_flight
                .pop_front()
                .map_or(0, |(_, timestamp)| timestamp);
            output.push(EncodedFrame {
                seq: self.next_seq,
                capture_ts_us: timestamp,
                keyframe: self.packet.is_keyframe(),
                data: self.packet.data().to_vec(),
            });
            self.next_seq += 1;
            self.packet.unref();
            let _ = pts;
        }
    }
}
impl VideoEncoder for VaapiEncoder {
    fn name(&self) -> &'static str {
        self.name
    }
    fn params(&self) -> CodecParams {
        CodecParams {
            codec: self.cfg.codec,
            width: self.cfg.width,
            height: self.cfg.height,
            fps: self.cfg.fps,
            extradata: self.ctx.extradata(),
        }
    }
    fn encode(
        &mut self,
        frame: &RawFrame,
        force_keyframe: bool,
    ) -> Result<Vec<EncodedFrame>, CodecError> {
        frame.validate()?;
        if (frame.width, frame.height) != (self.cfg.width, self.cfg.height) {
            return Err(CodecError::InvalidFrame(
                "encoder and frame dimensions differ".into(),
            ));
        }
        self.upload(frame)?;
        unsafe {
            (*self.hw_frame.0).pts = self.next_pts;
            (*self.hw_frame.0).pict_type = if force_keyframe {
                ff::AVPictureType::AV_PICTURE_TYPE_I
            } else {
                ff::AVPictureType::AV_PICTURE_TYPE_NONE
            };
        }
        self.in_flight
            .push_back((self.next_pts, frame.capture_ts_us));
        self.next_pts += 1;
        check("avcodec_send_frame", unsafe {
            ff::avcodec_send_frame(self.ctx.0, self.hw_frame.0)
        })?;
        let mut output = Vec::with_capacity(1);
        self.drain(&mut output)?;
        Ok(output)
    }
}
