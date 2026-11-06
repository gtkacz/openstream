//! Microphone capture through the default input device or a saved one. As with playback, a
//! dedicated thread owns cpal's stream, which is not `Send` on every host.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use brp_proto::constants::AUDIO_SAMPLE_RATE;
use brp_proto::monotonic_us;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::chunk::{AudioCaptureSession, AudioChunk, AudioSink, VoiceCapture};
use crate::cpal_output::AudioDevice;
use crate::error::AudioError;

const I16_SCALE: f32 = 32768.0;

/// Capture from one input device: the system default, or the device whose cpal id was saved in
/// settings.
#[derive(Debug, Default, Clone)]
pub struct CpalInput {
    device: Option<String>,
}

impl CpalInput {
    /// `None` records from the default device. `Some(id)` is a [`cpal::DeviceId`] in its
    /// `Display` form; a device that is absent fails `start`, so the microphone never silently
    /// moves to a device the user did not choose.
    pub fn new(device: Option<String>) -> Self {
        Self { device }
    }
}

/// Every input device of the default host. Devices whose id or description cannot be read are
/// skipped rather than failing the list.
pub fn input_devices() -> Result<Vec<AudioDevice>, AudioError> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::Device(format!("could not list input devices: {e}")))?;
    Ok(devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let name = device.description().ok()?.name().to_string();
            Some(AudioDevice { id, name })
        })
        .collect())
}

/// One range a device offers, reduced to what the choice depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputOffer {
    channels: u16,
    min_rate: u32,
    max_rate: u32,
    format: cpal::SampleFormat,
}

impl From<cpal::SupportedStreamConfigRange> for InputOffer {
    fn from(range: cpal::SupportedStreamConfigRange) -> Self {
        Self {
            channels: range.channels(),
            min_rate: range.min_sample_rate(),
            max_rate: range.max_sample_rate(),
            format: range.sample_format(),
        }
    }
}

/// The codec path is fixed at 48 kHz, so a microphone without it is refused rather than resampled.
/// Float beats 16-bit, then fewer channels beat more.
fn pick_input_config(offers: &[InputOffer]) -> Result<InputOffer, AudioError> {
    offers
        .iter()
        .filter(|offer| offer.min_rate <= AUDIO_SAMPLE_RATE && AUDIO_SAMPLE_RATE <= offer.max_rate)
        .filter_map(|offer| match offer.format {
            cpal::SampleFormat::F32 => Some((0, offer)),
            cpal::SampleFormat::I16 => Some((1, offer)),
            _ => None,
        })
        .min_by_key(|(rank, offer)| (*rank, offer.channels))
        .map(|(_, offer)| *offer)
        .ok_or_else(|| AudioError::Format("microphone offers no 48 kHz input".into()))
}

/// Interleaved stereo from any channel count: mono is duplicated, stereo passes through, wider
/// layouts are averaged into both sides.
fn to_stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        1 => samples.iter().flat_map(|&s| [s, s]).collect(),
        2 => samples.to_vec(),
        _ => samples
            .chunks_exact(channels)
            .flat_map(|frame| {
                let mean = frame.iter().sum::<f32>() / channels as f32;
                [mean, mean]
            })
            .collect(),
    }
}

/// cpal keeps the stream running after a glitch, a reroute to the new default device, or a refused
/// real-time priority. WASAPI reports a glitch for any dropped capture packet.
fn ends_the_stream(error: &cpal::Error) -> bool {
    !matches!(
        error.kind(),
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied
    )
}

struct Session {
    error: Arc<Mutex<Option<String>>>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl VoiceCapture for CpalInput {
    fn start(&self, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let device = self.device.clone();
        let error = Arc::new(Mutex::new(None));
        let stream_error = error.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("brp-audio-in".into())
            .spawn(move || {
                let stream = match open_stream(device.as_deref(), sink, stream_error) {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                // Blocks until the session drops its sender; then the stream drops with the thread.
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| AudioError::Device(format!("failed to spawn the input thread: {e}")))?;
        ready_rx
            .recv()
            .map_err(|_| AudioError::Device("input thread exited before reporting".into()))??;
        Ok(Box::new(Session {
            error,
            stop: Some(stop_tx),
            thread: Some(thread),
        }))
    }
}

fn open_stream(
    device: Option<&str>,
    sink: AudioSink,
    stored_error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = match device {
        None => host
            .default_input_device()
            .ok_or_else(|| AudioError::Device("no default input device".into()))?,
        Some(id) => {
            let parsed: cpal::DeviceId = id
                .parse()
                .map_err(|e| AudioError::Device(format!("input device {id:?}: {e}")))?;
            host.device_by_id(&parsed)
                .ok_or_else(|| AudioError::Device(format!("input device {id:?} not found")))?
        }
    };
    let offers: Vec<InputOffer> = device
        .supported_input_configs()
        .map_err(|e| AudioError::Device(format!("could not read input formats: {e}")))?
        .map(InputOffer::from)
        .collect();
    let offer = pick_input_config(&offers)?;
    let config = cpal::StreamConfig {
        channels: offer.channels,
        sample_rate: AUDIO_SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };
    let channels = usize::from(offer.channels);
    let on_error = move |error: cpal::Error| {
        tracing::warn!(%error, "audio input stream error");
        if ends_the_stream(&error) {
            *stored_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
        }
    };
    let mut sink = sink;
    let stream = match offer.format {
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let floats: Vec<f32> = data.iter().map(|&s| f32::from(s) / I16_SCALE).collect();
                sink(AudioChunk {
                    samples: to_stereo(&floats, channels),
                    capture_ts_us: monotonic_us(),
                });
            },
            on_error,
            None,
        ),
        _ => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                sink(AudioChunk {
                    samples: to_stereo(data, channels),
                    capture_ts_us: monotonic_us(),
                });
            },
            on_error,
            None,
        ),
    }
    .map_err(|e| AudioError::Format(format!("48 kHz microphone input refused: {e}")))?;
    stream
        .play()
        .map_err(|e| AudioError::Device(format!("could not start the microphone: {e}")))?;
    Ok(stream)
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn stop(self: Box<Self>) {}
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(channels: u16, rate: u32, format: cpal::SampleFormat) -> InputOffer {
        InputOffer {
            channels,
            min_rate: rate,
            max_rate: rate,
            format,
        }
    }

    #[test]
    fn mono_input_is_duplicated_to_both_channels() {
        assert_eq!(to_stereo(&[0.1, 0.2], 1), vec![0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn stereo_input_passes_through() {
        assert_eq!(to_stereo(&[0.1, 0.2], 2), vec![0.1, 0.2]);
    }

    #[test]
    fn wider_input_is_averaged_into_both_channels() {
        assert_eq!(to_stereo(&[0.2, 0.4, 0.6, 0.8], 4), vec![0.5, 0.5]);
    }

    #[test]
    fn an_input_without_48_khz_is_refused_by_name() {
        let error = pick_input_config(&[offer(2, 44_100, cpal::SampleFormat::F32)]).unwrap_err();
        assert!(matches!(error, AudioError::Format(_)), "{error}");
        assert!(error.to_string().contains("48 kHz"), "{error}");
    }

    #[test]
    fn the_fewest_channels_at_48_khz_float_are_preferred() {
        let picked = pick_input_config(&[
            offer(2, 48_000, cpal::SampleFormat::F32),
            offer(1, 48_000, cpal::SampleFormat::F32),
            offer(1, 48_000, cpal::SampleFormat::I16),
        ])
        .unwrap();
        assert_eq!(picked, offer(1, 48_000, cpal::SampleFormat::F32));
    }

    #[test]
    fn a_glitch_keeps_the_microphone_but_a_lost_device_ends_it() {
        assert!(!ends_the_stream(&cpal::ErrorKind::Xrun.into()));
        assert!(!ends_the_stream(&cpal::ErrorKind::DeviceChanged.into()));
        assert!(!ends_the_stream(&cpal::ErrorKind::RealtimeDenied.into()));
        assert!(ends_the_stream(&cpal::ErrorKind::DeviceNotAvailable.into()));
        assert!(ends_the_stream(&cpal::ErrorKind::StreamInvalidated.into()));
    }

    #[test]
    fn a_missing_microphone_fails_the_start_with_its_id_in_the_message() {
        let error = CpalInput::new(Some("nonsense".into()))
            .start(Box::new(|_| {}))
            .err()
            .expect("start must fail");
        assert!(error.to_string().contains("nonsense"), "{error}");
    }

    #[test]
    fn listing_inputs_does_not_fail_without_hardware() {
        assert!(input_devices().is_ok());
    }
}
