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

/// Sum of the `lp` (logical processors) values currently committed to open `libsvtav1` instances
/// in this process. SVT-AV1 cannot change its thread count after the encoder starts, so this is
/// not a count of encoders to divide evenly by (that recomputes a fresh average on every open but
/// never revisits an already-running encoder's share, so the running total can exceed the
/// machine's cores as soon as a second encoder opens). Instead each new encoder reserves a claim
/// against this shared total, so the aggregate the process has committed is tracked directly and
/// never invalidated by an encoder that opened earlier.
static TOTAL_COMMITTED_LP: AtomicUsize = AtomicUsize::new(0);

/// How large a claim a newly-opening software encoder should reserve from `TOTAL_COMMITTED_LP`,
/// given `already_committed` (the sum every other currently-open software encoder holds) and the
/// machine's total logical processors. Takes half of whatever is left (floored, minimum 1 since
/// SVT-AV1 needs at least one thread to run at all).
///
/// Because each claim only ever takes half the remainder, the running total climbs toward `cpus`
/// but stays strictly below it through as many concurrent opens as there are halvings to give
/// (roughly `log2(cpus)` of them) — unlike an even split recomputed from the encoder count, which
/// already overshoots with just two encoders (the first claims all of `cpus`, the second claims
/// `cpus / 2`, for a total of 1.5x `cpus`). Only once the remainder is exhausted does an
/// additional concurrent open add its unavoidable one-thread minimum on top; that is the
/// unavoidable floor for running more encoders than the machine has cores for, since a live
/// encoder's `lp` can't be revisited or reduced after the fact.
fn claim_software_encoder_lp(cpus: usize, already_committed: usize) -> usize {
    let remaining = cpus.saturating_sub(already_committed);
    (remaining / 2).max(1)
}

/// Reserves and, on drop, releases one encoder's claim against `TOTAL_COMMITTED_LP`. The
/// compare-and-swap retry means concurrent opens each see an up-to-date `already_committed` rather
/// than racing on a stale read.
struct SoftwareEncoderSlot {
    lp: usize,
}

impl SoftwareEncoderSlot {
    fn acquire() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        let mut already_committed = TOTAL_COMMITTED_LP.load(Ordering::Relaxed);
        let lp = loop {
            let claim = claim_software_encoder_lp(cpus, already_committed);
            match TOTAL_COMMITTED_LP.compare_exchange_weak(
                already_committed,
                already_committed + claim,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break claim,
                Err(actual) => already_committed = actual,
            }
        };
        Self { lp }
    }
}

impl Drop for SoftwareEncoderSlot {
    fn drop(&mut self) {
        TOTAL_COMMITTED_LP.fetch_sub(self.lp, Ordering::Relaxed);
    }
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
    // Held only for `libsvtav1`; `None` for a hardware encoder. Releases this encoder's claim
    // against `TOTAL_COMMITTED_LP` when dropped.
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
        // Reserved before the config is built so the claim is committed up front; dropping
        // `software_slot` on any later `?` failure releases it again.
        let software_slot = (name == "libsvtav1").then(SoftwareEncoderSlot::acquire);
        let software_lp = software_slot.as_ref().map_or(0, |slot| slot.lp);
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
    use std::sync::atomic::Ordering;

    use super::{SoftwareEncoderSlot, TOTAL_COMMITTED_LP, claim_software_encoder_lp};

    #[test]
    fn a_lone_encoder_claims_only_half_the_machine_reserving_room_for_more() {
        // Unlike an even split recomputed from the encoder count, the first encoder does not
        // claim every core: that headroom is exactly what keeps a second encoder from pushing the
        // committed total over `cpus`.
        assert_eq!(claim_software_encoder_lp(16, 0), 8);
    }

    #[test]
    fn sequential_claims_never_push_the_committed_total_past_the_machine() {
        let cpus = 64;
        let mut committed = 0usize;
        for _ in 0..6 {
            let claim = claim_software_encoder_lp(cpus, committed);
            assert!(claim >= 1, "every open still gets at least one thread");
            committed += claim;
            assert!(
                committed <= cpus,
                "running total {committed} exceeds the machine's {cpus} cores"
            );
        }
    }

    #[test]
    fn a_claim_is_never_smaller_than_one_logical_processor() {
        assert_eq!(claim_software_encoder_lp(4, 4), 1, "remainder exhausted");
        assert_eq!(
            claim_software_encoder_lp(4, 3),
            1,
            "remainder rounds down to zero"
        );
        assert_eq!(
            claim_software_encoder_lp(1, 0),
            1,
            "a single-core machine still gets a usable claim"
        );
    }

    #[test]
    fn releasing_an_earlier_claim_frees_room_for_the_next_one() {
        // Mirrors what `SoftwareEncoderSlot::drop` does to `TOTAL_COMMITTED_LP`: an encoder that
        // stops hands its claim back, so the next one to open sees the freed capacity rather than
        // treating the stopped encoder's old share as still spoken for.
        let cpus = 16;
        let first = claim_software_encoder_lp(cpus, 0); // 8
        let second = claim_software_encoder_lp(cpus, first); // 4
        let committed_after_first_stops = first + second - first; // release `first`
        let third = claim_software_encoder_lp(cpus, committed_after_first_stops);
        assert_eq!(committed_after_first_stops + third, second + third);
        assert!(committed_after_first_stops + third <= cpus);
    }

    #[test]
    fn slots_track_a_real_shared_total_and_release_correctly_on_drop() {
        // Exercises the real `TOTAL_COMMITTED_LP` static through acquire/drop directly (rather
        // than through `FfmpegEncoder::open`, which needs a real libsvtav1 build), across an
        // open-open-close-open sequence: closing an encoder must free exactly its own claim, and
        // the next open must see that freed room rather than the stale, still-running total.
        let before = TOTAL_COMMITTED_LP.load(Ordering::Relaxed);
        let a = SoftwareEncoderSlot::acquire();
        let b = SoftwareEncoderSlot::acquire();
        assert_eq!(
            TOTAL_COMMITTED_LP.load(Ordering::Relaxed),
            before + a.lp + b.lp
        );
        drop(a);
        assert_eq!(TOTAL_COMMITTED_LP.load(Ordering::Relaxed), before + b.lp);
        let c = SoftwareEncoderSlot::acquire();
        assert_eq!(
            TOTAL_COMMITTED_LP.load(Ordering::Relaxed),
            before + b.lp + c.lp
        );
        drop(b);
        drop(c);
        assert_eq!(TOTAL_COMMITTED_LP.load(Ordering::Relaxed), before);
    }
}
