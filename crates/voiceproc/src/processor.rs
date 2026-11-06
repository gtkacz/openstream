use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};

use brp_proto::constants::ECHO_REFERENCE_PENDING_MAX;
use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};
use nnnoiseless::DenoiseState;
use sonora::config::{
    AdaptiveDigital, EchoCanceller, FixedDigital, GainController2, HighPassFilter,
    MaxProcessingRate, Pipeline,
};
use sonora::{AudioProcessing, Config, StreamConfig};
use tracing::warn;

use crate::config::{ProcessingConfig, Suppression};
use crate::reference::{EchoReference, HOP};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
// Voice and streams are separate render channels so only the streams echo is modelled.
const REFERENCE_CHANNELS: u16 = 2;
// RNNoise works on the i16 scale, not on [-1, 1].
const I16_SCALE: f32 = 32768.0;

type Hop = [f32; HOP];

pub struct VoiceProcessor {
    reference: Option<EchoReference>,
    pending_reference: VecDeque<[Hop; REFERENCE_CHANNELS as usize]>,
    capture: Option<AudioProcessing>,
    denoise: Option<Box<DenoiseState<'static>>>,
    deep_filter: Option<Box<DfTract>>,
    gain: Option<AudioProcessing>,
}

fn apm(config: Config, render_channels: u16) -> AudioProcessing {
    let stream = StreamConfig::new(SAMPLE_RATE, 1);
    AudioProcessing::builder()
        .config(config)
        .capture_config(stream)
        .render_config(StreamConfig::new(SAMPLE_RATE, render_channels))
        .build()
}

fn pipeline() -> Pipeline {
    Pipeline {
        maximum_internal_processing_rate: MaxProcessingRate::Rate48kHz,
        ..Default::default()
    }
}

// Loading takes ~175 ms, which is why the processor is built on the encode thread.
fn load_deep_filter() -> Option<Box<DfTract>> {
    match DfTract::new(DfParams::default(), &RuntimeParams::default_with_ch(1)) {
        Ok(model) => Some(Box::new(model)),
        Err(e) => {
            warn!(stage = "strong noise suppression", reason = %e, "voice processing stage disabled for this session");
            None
        }
    }
}

fn run_deep_filter(model: &mut DfTract, hop: &mut Hop) -> Result<(), String> {
    let input = *hop;
    let input = ArrayView2::from_shape((1, HOP), &input).map_err(|e| e.to_string())?;
    let output = ArrayViewMut2::from_shape((1, HOP), &mut hop[..]).map_err(|e| e.to_string())?;
    model.process(input, output).map_err(|e| e.to_string())?;
    Ok(())
}

/// Feeds the oldest pending hop, one per capture hop. Rendering a whole frame ahead of its
/// capture hops (render, render, capture, capture) leaves AEC3 unable to converge on two
/// channels. Pairing is by count, so a backlog would become a standing lag: it is trimmed.
fn pending_cap() -> usize {
    ECHO_REFERENCE_PENDING_MAX.as_millis() as usize * SAMPLE_RATE as usize / 1000 / HOP
}

fn render_reference(
    apm: &mut AudioProcessing,
    reference: &EchoReference,
    pending: &mut VecDeque<[Hop; REFERENCE_CHANNELS as usize]>,
) -> Result<(), String> {
    pending.extend(reference.drain_hops());
    let excess = pending.len().saturating_sub(pending_cap());
    pending.drain(..excess);
    let Some([voice, streams]) = pending.pop_front() else {
        return Ok(());
    };
    let (mut sink_voice, mut sink_streams) = ([0.0f32; HOP], [0.0f32; HOP]);
    // The order of the channels does not affect the result.
    apm.process_render_f32(
        &[&streams, &voice],
        &mut [&mut sink_streams[..], &mut sink_voice[..]],
    )
    .map_err(|e| e.to_string())
}

fn run_apm(apm: &mut AudioProcessing, hop: &mut Hop) -> Result<(), String> {
    let input = *hop;
    apm.process_capture_f32(&[&input], &mut [&mut hop[..]])
        .map_err(|e| e.to_string())
}

/// A stage that fails is dropped for the session so one bad stage cannot silence the microphone.
fn run_stage<S>(
    stage: &mut Option<S>,
    name: &'static str,
    hop: &mut Hop,
    f: impl FnOnce(&mut S, &mut Hop) -> Result<(), String>,
) {
    let Some(state) = stage.as_mut() else {
        return;
    };
    let mut copy = *hop;
    let outcome = catch_unwind(AssertUnwindSafe(|| f(state, &mut copy)));
    let reason = match outcome {
        Ok(Ok(())) => {
            *hop = copy;
            return;
        }
        Ok(Err(reason)) => reason,
        Err(_) => "panicked".to_owned(),
    };
    warn!(stage = name, %reason, "voice processing stage disabled for this session");
    *stage = None;
}

impl VoiceProcessor {
    pub fn new(config: ProcessingConfig, reference: &EchoReference) -> Self {
        if config.is_off() {
            return Self {
                reference: None,
                pending_reference: VecDeque::new(),
                capture: None,
                denoise: None,
                deep_filter: None,
                gain: None,
            };
        }
        let capture = apm(
            Config {
                pipeline: pipeline(),
                high_pass_filter: Some(HighPassFilter::default()),
                echo_canceller: config.echo_cancellation.then(EchoCanceller::default),
                ..Default::default()
            },
            if config.echo_cancellation {
                REFERENCE_CHANNELS
            } else {
                1
            },
        );
        let reference = config.echo_cancellation.then(|| {
            reference.activate();
            reference.clone()
        });
        let gain = config.automatic_gain.then(|| {
            apm(
                Config {
                    pipeline: pipeline(),
                    gain_controller2: Some(GainController2 {
                        input_volume_controller: false,
                        adaptive_digital: Some(AdaptiveDigital::default()),
                        fixed_digital: FixedDigital::default(),
                    }),
                    ..Default::default()
                },
                1,
            )
        });
        Self {
            reference,
            pending_reference: VecDeque::new(),
            capture: Some(capture),
            denoise: (config.noise_suppression == Suppression::Light).then(DenoiseState::new),
            deep_filter: (config.noise_suppression == Suppression::Strong)
                .then(load_deep_filter)
                .flatten(),
            gain,
        }
    }

    /// Takes one interleaved stereo frame and leaves the same mono result on both channels,
    /// or leaves the frame untouched when no stage is active.
    pub fn process(&mut self, stereo: &mut [f32]) {
        if self.capture.is_none()
            && self.denoise.is_none()
            && self.deep_filter.is_none()
            && self.gain.is_none()
        {
            return;
        }
        for frames in stereo.as_chunks_mut::<{ HOP * CHANNELS }>().0 {
            let mut hop = [0.0f32; HOP];
            for (m, f) in hop.iter_mut().zip(frames.as_chunks::<CHANNELS>().0.iter()) {
                *m = (f[0] + f[1]) / 2.0;
            }
            run_stage(&mut self.capture, "high-pass", &mut hop, |a, h| {
                if let Some(reference) = &self.reference {
                    render_reference(a, reference, &mut self.pending_reference)?;
                }
                run_apm(a, h)
            });
            if self.capture.is_none()
                && let Some(reference) = self.reference.take()
            {
                reference.deactivate();
            }
            run_stage(&mut self.denoise, "noise suppression", &mut hop, |d, h| {
                let scaled = h.map(|s| s * I16_SCALE);
                let mut out = [0.0f32; HOP];
                d.process_frame(&mut out, &scaled);
                *h = out.map(|s| s / I16_SCALE);
                Ok(())
            });
            run_stage(
                &mut self.deep_filter,
                "strong noise suppression",
                &mut hop,
                |m, h| run_deep_filter(m, h),
            );
            run_stage(&mut self.gain, "automatic gain", &mut hop, |a, h| {
                run_apm(a, h)
            });
            for (m, f) in hop
                .iter()
                .zip(frames.as_chunks_mut::<CHANNELS>().0.iter_mut())
            {
                f.fill(*m);
            }
        }
    }
}

impl Drop for VoiceProcessor {
    fn drop(&mut self) {
        if let Some(reference) = &self.reference {
            reference.deactivate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::ReferenceChannel;
    use crate::test_signals::{
        at_peak, at_rms, clicks, looped, noise, rms_db, run, sparse_echo, speech, stretched,
    };

    const SECOND: usize = 48_000;

    fn processor(noise_suppression: Suppression, automatic_gain: bool) -> VoiceProcessor {
        VoiceProcessor::new(
            ProcessingConfig {
                echo_cancellation: false,
                noise_suppression,
                automatic_gain,
            },
            &EchoReference::new(),
        )
    }

    fn tail(x: &[f32], seconds: usize) -> &[f32] {
        &x[x.len() - seconds * SECOND..]
    }

    #[test]
    fn with_everything_off_the_frame_is_untouched() {
        let left = noise(960, 1);
        let right = noise(960, 2);
        let mut frame: Vec<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        let before = frame.clone();
        VoiceProcessor::new(ProcessingConfig::OFF, &EchoReference::new()).process(&mut frame);
        assert_eq!(frame, before);
    }

    #[test]
    fn light_removes_steady_noise() {
        let input = at_rms(&noise(3 * SECOND, 7), -40.0);
        let out = run(&mut processor(Suppression::Light, false), &input);
        assert!(rms_db(tail(&out, 1)) <= rms_db(tail(&input, 1)) - 10.0);
    }

    #[test]
    fn strong_removes_keyboard_clicks() {
        let input = at_peak(&clicks(7, 3 * SECOND), -14.0);
        let out = run(&mut processor(Suppression::Strong, false), &input);
        assert!(rms_db(tail(&out, 2)) <= rms_db(tail(&input, 2)) - 10.0);
    }

    // Impulses are removed as noise, so the delay is read off the correlation with speech.
    #[test]
    fn strong_delays_by_its_lookahead_only() {
        let input = looped(&speech(), 3.0);
        let out = run(&mut processor(Suppression::Strong, false), &input);
        let correlation = |lag: usize| -> f32 {
            input[..input.len() - lag]
                .iter()
                .zip(&out[lag..])
                .map(|(a, b)| a * b)
                .sum()
        };
        let best = (0..4800)
            .max_by(|a, b| correlation(*a).total_cmp(&correlation(*b)))
            .unwrap();
        assert!(best.abs_diff(1440) <= 480, "best lag {best}");
    }

    fn gain_change(peak_dbfs: f32) -> f32 {
        let input = looped(&at_peak(&speech(), peak_dbfs), 40.0);
        let out = run(&mut processor(Suppression::Off, true), &input);
        rms_db(tail(&out, 10)) - rms_db(tail(&input, 10))
    }

    #[test]
    fn automatic_gain_raises_quiet_speech() {
        assert!(gain_change(-30.0) >= 10.0);
    }

    #[test]
    fn automatic_gain_leaves_loud_speech_within_3_db() {
        assert!(gain_change(-1.0).abs() <= 3.0);
    }

    #[test]
    fn a_one_sided_microphone_comes_out_on_both_channels() {
        let mut p = processor(Suppression::Light, true);
        for chunk in looped(&speech(), 3.0).as_chunks::<960>().0.iter() {
            let mut frame: Vec<f32> = chunk.iter().flat_map(|&s| [s, 0.0]).collect();
            p.process(&mut frame);
            assert!(frame.iter().all(|s| s.is_finite()));
            assert!(frame.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
        }
    }

    #[test]
    fn a_stage_that_errors_is_dropped_and_its_input_passes_through() {
        let mut stage = Some(());
        let mut hop = [1.0f32; HOP];
        run_stage(&mut stage, "test", &mut hop, |_, h| {
            h.fill(9.0);
            Err("no".into())
        });
        assert!(stage.is_none());
        assert_eq!(hop, [1.0f32; HOP]);
    }

    #[test]
    fn a_stage_that_panics_is_dropped_and_its_input_passes_through() {
        let mut stage = Some(());
        let mut hop = [1.0f32; HOP];
        run_stage(&mut stage, "test", &mut hop, |_, _| panic!("boom"));
        assert!(stage.is_none());
        assert_eq!(hop, [1.0f32; HOP]);
    }

    const FRAME: usize = 960;

    fn echo_processor(reference: &EchoReference) -> VoiceProcessor {
        VoiceProcessor::new(
            ProcessingConfig {
                echo_cancellation: true,
                noise_suppression: Suppression::Off,
                automatic_gain: false,
            },
            reference,
        )
    }

    fn stereo(mono: &[f32]) -> Vec<f32> {
        mono.iter().flat_map(|&s| [s, s]).collect()
    }

    /// Plays `voice` and `streams` while the microphone hears `mic`, a frame at a time, and
    /// returns the echo removed, in dB, over the last 5 s where `heard` is not silent.
    fn echo_removed(voice: &[f32], streams: Option<&[f32]>, mic: &[f32], heard: &[f32]) -> f32 {
        let reference = EchoReference::new();
        let mut p = echo_processor(&reference);
        let (mut mic_kept, mut out_kept) = (Vec::new(), Vec::new());
        for (k, m) in mic.as_chunks::<FRAME>().0.iter().enumerate() {
            let range = k * FRAME..(k + 1) * FRAME;
            reference.push(ReferenceChannel::Voice, &stereo(&voice[range.clone()]));
            if let Some(streams) = streams {
                reference.push(ReferenceChannel::Streams, &stereo(&streams[range.clone()]));
            }
            let mut frame = stereo(m);
            p.process(&mut frame);
            if range.start >= mic.len() - 5 * SECOND && rms_db(&heard[range]) > -60.0 {
                mic_kept.extend_from_slice(m);
                out_kept.extend(frame.as_chunks::<2>().0.iter().map(|f| f[0]));
            }
        }
        rms_db(&mic_kept) - rms_db(&out_kept)
    }

    #[test]
    fn echo_cancellation_removes_a_speaker_echo() {
        let far = looped(&speech(), 15.0);
        let mic = sparse_echo(&far, 40, 1);
        let removed = echo_removed(&far, None, &mic, &far);
        assert!(removed >= 30.0);
    }

    #[test]
    fn only_the_stream_echo_is_cancelled_while_voice_plays_on_headphones() {
        // Distinct material per channel: a shifted copy of one short loop is so correlated
        // with it that the canceller cannot tell which channel echoes.
        let streams: Vec<f32> = looped(&stretched(&speech(), 0.8), 20.0)
            .iter()
            .map(|s| 0.8 * s)
            .collect();
        let mic = sparse_echo(&streams, 60, 2);
        for voice_starts_s in [0.0, 0.5, 3.0] {
            let lead = (voice_starts_s * SECOND as f32) as usize;
            let mut voice = vec![0.0; lead];
            voice.extend(looped(&speech(), 20.0).iter().take(20 * SECOND - lead));
            let removed = echo_removed(&voice, Some(&streams), &mic, &streams);
            assert!(
                removed >= 30.0,
                "voice at {voice_starts_s} s: {removed:.1} dB"
            );
        }
    }

    #[test]
    fn a_stale_reference_backlog_is_trimmed_to_the_pending_cap() {
        let reference = EchoReference::new();
        let mut p = echo_processor(&reference);
        reference.push(ReferenceChannel::Voice, &vec![0.1; 20 * HOP * 2]);
        reference.push(ReferenceChannel::Streams, &vec![0.1; 20 * HOP * 2]);
        p.process(&mut vec![0.0; HOP * 2]);
        assert!(p.pending_reference.len() <= pending_cap());
    }

    #[test]
    fn capture_runs_with_no_reference_pending() {
        let reference = EchoReference::new();
        let mut p = echo_processor(&reference);
        let out = run(&mut p, &looped(&speech(), 1.0));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(out.iter().any(|s| *s != 0.0));
    }

    #[test]
    fn dropping_the_capture_stage_stops_the_reference_collecting() {
        let reference = EchoReference::new();
        let mut p = VoiceProcessor::new(
            ProcessingConfig {
                echo_cancellation: true,
                noise_suppression: Suppression::Off,
                automatic_gain: true,
            },
            &reference,
        );
        p.capture = None;
        p.process(&mut vec![0.0; 960]);
        reference.push(ReferenceChannel::Voice, &vec![0.5; 960 * 2]);
        assert!(reference.drain_hops().is_empty());
    }

    #[test]
    fn dropping_the_processor_stops_the_reference_collecting() {
        let reference = EchoReference::new();
        drop(echo_processor(&reference));
        reference.push(ReferenceChannel::Voice, &vec![0.5; 960 * 2]);
        assert!(reference.drain_hops().is_empty());
    }

    #[test]
    fn without_echo_cancellation_the_reference_stays_inactive() {
        let reference = EchoReference::new();
        let _p = VoiceProcessor::new(
            ProcessingConfig {
                echo_cancellation: false,
                noise_suppression: Suppression::Light,
                automatic_gain: true,
            },
            &reference,
        );
        reference.push(ReferenceChannel::Voice, &vec![0.5; 960 * 2]);
        assert!(reference.drain_hops().is_empty());
    }
}
