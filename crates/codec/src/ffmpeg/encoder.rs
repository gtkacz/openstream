use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use brp_proto::{CodecParams, EncodedFrame};
use ffmpeg_sys_next as ff;

use crate::error::CodecError;
use crate::ffmpeg::ffi::{
    CodecContext, Frame, Packet, again, check, cstring, init_logging, set_opt, set_opt_int,
};
use crate::raw::RawFrame;
use crate::traits::{EncoderConfig, VideoEncoder};

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputLayout {
    Nv12,
    I420,
}

/// How many `libsvtav1` instances are currently open in this process. Guards the count so a
/// failed or dropped encoder always releases its slot, including on an early `?` return from
/// `open`.
static ACTIVE_SOFTWARE_ENCODERS: AtomicUsize = AtomicUsize::new(0);

struct SoftwareEncoderSlot;

impl SoftwareEncoderSlot {
    fn acquire() -> Self {
        ACTIVE_SOFTWARE_ENCODERS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for SoftwareEncoderSlot {
    fn drop(&mut self) {
        ACTIVE_SOFTWARE_ENCODERS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Logical processors to hand SVT-AV1's `lp` parameter: the machine's parallelism split across
/// every currently active software encoder, so several subscribed presets each get a bounded
/// share instead of each independently claiming every core.
fn software_encoder_lp(active_count: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    (cpus / active_count.max(1)).max(1)
}

pub struct FfmpegEncoder {
    ctx: CodecContext,
    frame: Frame,
    packet: Packet,
    name: &'static str,
    cfg: EncoderConfig,
    layout: InputLayout,
    next_seq: u64,
    next_pts: i64,
    in_flight: VecDeque<(i64, u64)>,
    // Held only for `libsvtav1`; `None` for a hardware encoder. Releases this encoder's share of
    // `ACTIVE_SOFTWARE_ENCODERS` when dropped.
    _software_slot: Option<SoftwareEncoderSlot>,
}

impl FfmpegEncoder {
    pub fn open(name: &'static str, cfg: &EncoderConfig) -> Result<Self, CodecError> {
        init_logging();
        let cname = cstring(name)?;
        let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
        if codec.is_null() {
            return Err(CodecError::EncoderMissing(name));
        }
        let layout = if name == "libsvtav1" {
            InputLayout::I420
        } else {
            InputLayout::Nv12
        };
        // Acquired before the config is built so the probed active count already includes this
        // instance; dropping `software_slot` on any later `?` failure releases it again.
        let software_slot = (name == "libsvtav1").then(SoftwareEncoderSlot::acquire);
        let software_lp = software_encoder_lp(ACTIVE_SOFTWARE_ENCODERS.load(Ordering::Relaxed));
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
            c.pix_fmt = if layout == InputLayout::Nv12 {
                ff::AVPixelFormat::AV_PIX_FMT_NV12
            } else {
                ff::AVPixelFormat::AV_PIX_FMT_YUV420P
            };
            c.bit_rate = i64::from(cfg.bitrate_kbps) * 1000;
            c.rc_buffer_size = (c.bit_rate / i64::from(cfg.fps.max(1))) as c_int;
            c.gop_size = c_int::MAX;
            c.max_b_frames = 0;
            c.flags |= ff::AV_CODEC_FLAG_LOW_DELAY as c_int;
            if layout == InputLayout::Nv12 {
                c.rc_max_rate = c.bit_rate;
            }
        }
        apply_low_latency_options(name, &ctx, software_lp)?;
        ctx.open(codec)?;
        let frame = Frame::new()?;
        unsafe {
            let f = &mut *frame.0;
            f.width = cfg.width as c_int;
            f.height = cfg.height as c_int;
            f.format = (*ctx.0).pix_fmt as c_int;
            check("av_frame_get_buffer", ff::av_frame_get_buffer(frame.0, 0))?;
        }
        Ok(Self {
            ctx,
            frame,
            packet: Packet::new()?,
            name,
            cfg: *cfg,
            layout,
            next_seq: 0,
            next_pts: 0,
            in_flight: VecDeque::new(),
            _software_slot: software_slot,
        })
    }

    fn fill_frame(&mut self, src: &RawFrame) -> Result<(), CodecError> {
        check("av_frame_make_writable", unsafe {
            ff::av_frame_make_writable(self.frame.0)
        })?;
        let f = unsafe { &mut *self.frame.0 };
        let width = src.width as usize;
        unsafe {
            for row in 0..src.height as usize {
                ptr::copy_nonoverlapping(
                    src.y.as_ptr().add(row * src.y_stride),
                    f.data[0].add(row * f.linesize[0] as usize),
                    width,
                );
            }
            match self.layout {
                InputLayout::Nv12 => {
                    for row in 0..src.chroma_rows() {
                        ptr::copy_nonoverlapping(
                            src.uv.as_ptr().add(row * src.uv_stride),
                            f.data[1].add(row * f.linesize[1] as usize),
                            width,
                        );
                    }
                }
                InputLayout::I420 => {
                    for row in 0..src.chroma_rows() {
                        let uv = &src.uv[row * src.uv_stride..row * src.uv_stride + width];
                        let u = f.data[1].add(row * f.linesize[1] as usize);
                        let v = f.data[2].add(row * f.linesize[2] as usize);
                        for (i, pair) in uv.as_chunks::<2>().0.iter().enumerate() {
                            *u.add(i) = pair[0];
                            *v.add(i) = pair[1];
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn drain(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), CodecError> {
        loop {
            let result = unsafe { ff::avcodec_receive_packet(self.ctx.0, self.packet.0) };
            if result == again() || result == ff::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", result)?;
            let pts = unsafe { (*self.packet.0).pts };
            let capture_ts_us = self.take_capture_ts(pts);
            out.push(EncodedFrame {
                seq: self.next_seq,
                capture_ts_us,
                keyframe: self.packet.is_keyframe(),
                data: self.packet.data().to_vec(),
            });
            self.next_seq += 1;
            self.packet.unref();
        }
    }
    fn take_capture_ts(&mut self, pts: i64) -> u64 {
        while let Some(&(front_pts, timestamp)) = self.in_flight.front() {
            self.in_flight.pop_front();
            if front_pts >= pts {
                return timestamp;
            }
        }
        0
    }
}

fn apply_low_latency_options(
    name: &str,
    ctx: &CodecContext,
    software_lp: usize,
) -> Result<(), CodecError> {
    match name {
        "h264_nvenc" | "hevc_nvenc" | "av1_nvenc" => {
            set_opt(ctx, "preset", "p4")?;
            set_opt(ctx, "tune", "ull")?;
            set_opt(ctx, "rc", "cbr")?;
            set_opt_int(ctx, "zerolatency", 1)?;
            set_opt_int(ctx, "delay", 0)?;
            set_opt_int(ctx, "forced-idr", 1)?;
            set_opt_int(ctx, "rc-lookahead", 0)?;
        }
        "h264_amf" | "hevc_amf" | "av1_amf" => {
            set_opt(ctx, "usage", "ultralowlatency")?;
            set_opt(ctx, "rc", "cbr")?;
        }
        "h264_qsv" | "hevc_qsv" | "av1_qsv" => {
            set_opt_int(ctx, "async_depth", 1)?;
            set_opt_int(ctx, "low_power", 1)?;
        }
        "h264_mf" | "hevc_mf" => {
            // Hardware only: a software Media Foundation transform would defeat the probe order,
            // where software AV1 is the deliberate last resort.
            set_opt_int(ctx, "hw_encoding", 1)?;
            set_opt(ctx, "rate_control", "cbr")?;
            set_opt(ctx, "scenario", "display_remoting")?;
        }
        "libsvtav1" => {
            set_opt(ctx, "preset", "10")?;
            // `lp` bounds SVT-AV1's own thread pool so several concurrently subscribed presets
            // each get a share of the machine instead of every instance claiming every core.
            set_opt(
                ctx,
                "svtav1-params",
                &format!("rc=2:pred-struct=1:rtc=1:lp={software_lp}"),
            )?;
        }
        _ => {}
    }
    Ok(())
}

impl VideoEncoder for FfmpegEncoder {
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
        self.fill_frame(frame)?;
        unsafe {
            (*self.frame.0).pts = self.next_pts;
            (*self.frame.0).pict_type = if force_keyframe {
                ff::AVPictureType::AV_PICTURE_TYPE_I
            } else {
                ff::AVPictureType::AV_PICTURE_TYPE_NONE
            };
            (*self.frame.0).flags = if force_keyframe {
                ff::AV_FRAME_FLAG_KEY
            } else {
                0
            };
        }
        self.in_flight
            .push_back((self.next_pts, frame.capture_ts_us));
        self.next_pts += 1;
        check("avcodec_send_frame", unsafe {
            ff::avcodec_send_frame(self.ctx.0, self.frame.0)
        })?;
        let mut output = Vec::with_capacity(1);
        self.drain(&mut output)?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::software_encoder_lp;

    #[test]
    fn a_single_active_encoder_gets_every_available_core() {
        assert_eq!(
            software_encoder_lp(1),
            std::thread::available_parallelism().unwrap().get()
        );
    }

    #[test]
    fn several_active_encoders_split_the_cores_and_never_reach_zero() {
        let cpus = std::thread::available_parallelism().unwrap().get();
        assert_eq!(software_encoder_lp(2), (cpus / 2).max(1));
        assert_eq!(
            software_encoder_lp(cpus * 10),
            1,
            "always at least one logical processor"
        );
    }
}
