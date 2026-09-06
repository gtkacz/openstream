//! Opus through FFmpeg's libopus wrapper. Input and output are interleaved 48 kHz stereo float.

use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;

use brp_proto::constants::{
    AUDIO_CHANNELS, AUDIO_FRAME_SAMPLES, AUDIO_SAMPLE_RATE, OPUS_BITRATE_KBPS,
};
use brp_proto::{AudioParams, EncodedFrame};
use ffmpeg_sys_next as ff;

use crate::audio::{AudioDecoder, AudioEncoder, AudioFrame};
use crate::error::CodecError;
use crate::ffmpeg::ffi::{
    CodecContext, Frame, Packet, again, check, cstring, init_logging, set_opt,
};

const ENCODER_NAME: &str = "libopus";

pub struct OpusEncoder {
    ctx: CodecContext,
    frame: Frame,
    packet: Packet,
    next_seq: u64,
    next_pts: i64,
    /// libopus emits exactly one packet per frame, in order, so a queue of timestamps suffices.
    pending_ts: VecDeque<u64>,
}

impl OpusEncoder {
    pub fn open() -> Result<Self, CodecError> {
        init_logging();
        let cname = cstring(ENCODER_NAME)?;
        let codec = unsafe { ff::avcodec_find_encoder_by_name(cname.as_ptr()) };
        if codec.is_null() {
            return Err(CodecError::EncoderMissing(ENCODER_NAME));
        }
        let mut ctx = CodecContext::alloc(codec)?;
        unsafe {
            let c = &mut *ctx.0;
            c.sample_rate = AUDIO_SAMPLE_RATE as c_int;
            c.sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLT;
            ff::av_channel_layout_default(&mut c.ch_layout, c_int::from(AUDIO_CHANNELS));
            c.bit_rate = i64::from(OPUS_BITRATE_KBPS) * 1000;
            c.time_base = ff::AVRational {
                num: 1,
                den: AUDIO_SAMPLE_RATE as c_int,
            };
        }
        // Game audio, not speech: keeps libopus in its full-band music mode.
        set_opt(&ctx, "application", "audio")?;
        ctx.open(codec)?;
        let frame_size = unsafe { (*ctx.0).frame_size } as usize;
        if frame_size != AUDIO_FRAME_SAMPLES {
            return Err(CodecError::InvalidFrame(format!(
                "libopus wants {frame_size} samples per frame, brp sends {AUDIO_FRAME_SAMPLES}"
            )));
        }
        let frame = Frame::new()?;
        unsafe {
            let f = &mut *frame.0;
            f.nb_samples = AUDIO_FRAME_SAMPLES as c_int;
            f.format = ff::AVSampleFormat::AV_SAMPLE_FMT_FLT as c_int;
            f.sample_rate = AUDIO_SAMPLE_RATE as c_int;
            ff::av_channel_layout_default(&mut f.ch_layout, c_int::from(AUDIO_CHANNELS));
            check("av_frame_get_buffer", ff::av_frame_get_buffer(frame.0, 0))?;
        }
        Ok(Self {
            ctx,
            frame,
            packet: Packet::new()?,
            next_seq: 0,
            next_pts: 0,
            pending_ts: VecDeque::new(),
        })
    }

    fn drain(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), CodecError> {
        loop {
            let result = unsafe { ff::avcodec_receive_packet(self.ctx.0, self.packet.0) };
            if result == again() || result == ff::AVERROR_EOF {
                return Ok(());
            }
            check("avcodec_receive_packet", result)?;
            out.push(EncodedFrame {
                seq: self.next_seq,
                capture_ts_us: self.pending_ts.pop_front().unwrap_or(0),
                keyframe: true,
                data: self.packet.data().to_vec(),
            });
            self.next_seq += 1;
            self.packet.unref();
        }
    }
}

impl AudioEncoder for OpusEncoder {
    fn name(&self) -> &'static str {
        ENCODER_NAME
    }
    fn params(&self) -> AudioParams {
        AudioParams::STANDARD
    }
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<EncodedFrame>, CodecError> {
        frame.validate()?;
        check("av_frame_make_writable", unsafe {
            ff::av_frame_make_writable(self.frame.0)
        })?;
        unsafe {
            let f = &mut *self.frame.0;
            ptr::copy_nonoverlapping(
                frame.samples.as_ptr(),
                f.data[0] as *mut f32,
                frame.samples.len(),
            );
            f.pts = self.next_pts;
        }
        self.pending_ts.push_back(frame.capture_ts_us);
        self.next_pts += AUDIO_FRAME_SAMPLES as i64;
        check("avcodec_send_frame", unsafe {
            ff::avcodec_send_frame(self.ctx.0, self.frame.0)
        })?;
        let mut output = Vec::with_capacity(1);
        self.drain(&mut output)?;
        Ok(output)
    }
}

pub struct OpusDecoder {
    ctx: CodecContext,
    packet: Packet,
    frame: Frame,
    name: &'static str,
}

impl OpusDecoder {
    pub fn open(params: &AudioParams) -> Result<Self, CodecError> {
        init_logging();
        if *params != AudioParams::STANDARD {
            return Err(CodecError::InvalidFrame(format!(
                "audio params {params:?} are not 48 kHz stereo"
            )));
        }
        let (codec, name) = find_decoder()?;
        let mut ctx = CodecContext::alloc(codec)?;
        unsafe {
            let c = &mut *ctx.0;
            c.sample_rate = AUDIO_SAMPLE_RATE as c_int;
            ff::av_channel_layout_default(&mut c.ch_layout, c_int::from(AUDIO_CHANNELS));
            c.request_sample_fmt = ff::AVSampleFormat::AV_SAMPLE_FMT_FLT;
        }
        ctx.open(codec)?;
        Ok(Self {
            ctx,
            packet: Packet::new()?,
            frame: Frame::new()?,
            name,
        })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// libopus first; FFmpeg's own decoder is always compiled in and serves as the fallback.
fn find_decoder() -> Result<(*const ff::AVCodec, &'static str), CodecError> {
    let cname = cstring("libopus")?;
    let libopus = unsafe { ff::avcodec_find_decoder_by_name(cname.as_ptr()) };
    if !libopus.is_null() {
        return Ok((libopus, "libopus"));
    }
    let native = unsafe { ff::avcodec_find_decoder(ff::AVCodecID::AV_CODEC_ID_OPUS) };
    if native.is_null() {
        return Err(CodecError::DecoderMissing("opus"));
    }
    Ok((native, "opus"))
}

impl AudioDecoder for OpusDecoder {
    fn decode(&mut self, encoded: &EncodedFrame) -> Result<Vec<AudioFrame>, CodecError> {
        let size = c_int::try_from(encoded.data.len())
            .map_err(|_| CodecError::InvalidFrame("packet larger than c_int".into()))?;
        unsafe {
            check("av_new_packet", ff::av_new_packet(self.packet.0, size))?;
            ptr::copy_nonoverlapping(
                encoded.data.as_ptr(),
                (*self.packet.0).data,
                encoded.data.len(),
            );
            (*self.packet.0).flags |= ff::AV_PKT_FLAG_KEY;
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
            let samples = interleaved_samples(unsafe { &*self.frame.0 })?;
            self.frame.unref();
            output.push(AudioFrame {
                samples,
                capture_ts_us: encoded.capture_ts_us,
            });
        }
    }
}

/// The libopus decoder honours the interleaved request; the native decoder always emits planar.
fn interleaved_samples(frame: &ff::AVFrame) -> Result<Vec<f32>, CodecError> {
    let samples = frame.nb_samples as usize;
    let channels = frame.ch_layout.nb_channels as usize;
    if channels != AUDIO_CHANNELS as usize {
        return Err(CodecError::InvalidFrame(format!(
            "decoder produced {channels} channels"
        )));
    }
    let mut out = vec![0.0f32; samples * channels];
    unsafe {
        if frame.format == ff::AVSampleFormat::AV_SAMPLE_FMT_FLT as c_int {
            ptr::copy_nonoverlapping(frame.data[0] as *const f32, out.as_mut_ptr(), out.len());
        } else if frame.format == ff::AVSampleFormat::AV_SAMPLE_FMT_FLTP as c_int {
            for channel in 0..channels {
                let plane = frame.data[channel] as *const f32;
                for n in 0..samples {
                    out[n * channels + channel] = *plane.add(n);
                }
            }
        } else {
            return Err(CodecError::InvalidFrame(format!(
                "decoder produced unsupported sample format {}",
                frame.format
            )));
        }
    }
    Ok(out)
}
