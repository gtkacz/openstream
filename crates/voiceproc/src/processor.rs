use std::panic::{AssertUnwindSafe, catch_unwind};

use nnnoiseless::DenoiseState;
use sonora::config::{
    AdaptiveDigital, FixedDigital, GainController2, HighPassFilter, MaxProcessingRate, Pipeline,
};
use sonora::{AudioProcessing, Config, StreamConfig};
use tracing::warn;

use crate::config::{ProcessingConfig, Suppression};
use crate::reference::{EchoReference, HOP};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
// RNNoise works on the i16 scale, not on [-1, 1].
const I16_SCALE: f32 = 32768.0;

type Hop = [f32; HOP];

pub struct VoiceProcessor {
    capture: Option<AudioProcessing>,
    denoise: Option<Box<DenoiseState<'static>>>,
    gain: Option<AudioProcessing>,
}

fn apm(config: Config) -> AudioProcessing {
    let stream = StreamConfig::new(SAMPLE_RATE, 1);
    AudioProcessing::builder()
        .config(config)
        .capture_config(stream)
        .render_config(stream)
        .build()
}

fn pipeline() -> Pipeline {
    Pipeline {
        maximum_internal_processing_rate: MaxProcessingRate::Rate48kHz,
        ..Default::default()
    }
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
    pub fn new(config: ProcessingConfig, _reference: &EchoReference) -> Self {
        if config.is_off() {
            return Self {
                capture: None,
                denoise: None,
                gain: None,
            };
        }
        let capture = apm(Config {
            pipeline: pipeline(),
            high_pass_filter: Some(HighPassFilter::default()),
            ..Default::default()
        });
        let gain = config.automatic_gain.then(|| {
            apm(Config {
                pipeline: pipeline(),
                gain_controller2: Some(GainController2 {
                    input_volume_controller: false,
                    adaptive_digital: Some(AdaptiveDigital::default()),
                    fixed_digital: FixedDigital::default(),
                }),
                ..Default::default()
            })
        });
        Self {
            capture: Some(capture),
            denoise: (config.noise_suppression == Suppression::Light).then(DenoiseState::new),
            gain,
        }
    }

    /// Takes one interleaved stereo frame and leaves the same mono result on both channels,
    /// or leaves the frame untouched when no stage is active.
    pub fn process(&mut self, stereo: &mut [f32]) {
        if self.capture.is_none() && self.denoise.is_none() && self.gain.is_none() {
            return;
        }
        for frames in stereo.as_chunks_mut::<{ HOP * CHANNELS }>().0 {
            let mut hop = [0.0f32; HOP];
            for (m, f) in hop.iter_mut().zip(frames.as_chunks::<CHANNELS>().0.iter()) {
                *m = (f[0] + f[1]) / 2.0;
            }
            run_stage(&mut self.capture, "high-pass", &mut hop, |a, h| {
                run_apm(a, h)
            });
            run_stage(&mut self.denoise, "noise suppression", &mut hop, |d, h| {
                let scaled = h.map(|s| s * I16_SCALE);
                let mut out = [0.0f32; HOP];
                d.process_frame(&mut out, &scaled);
                *h = out.map(|s| s / I16_SCALE);
                Ok(())
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signals::{at_peak, at_rms, looped, noise, rms_db, run, speech};

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
}
