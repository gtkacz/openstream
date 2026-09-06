//! Playback through the default output device or a saved one. cpal's stream type is not `Send` on
//! every host, so a dedicated thread owns it and parks until the session is dropped.

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::chunk::{AudioOutput, AudioOutputSession, RenderFn};
use crate::error::AudioError;

/// Playback through one output device: the system default, or the device whose cpal id was
/// saved in settings.
#[derive(Debug, Default, Clone)]
pub struct CpalOutput {
    device: Option<String>,
}

impl CpalOutput {
    /// `None` plays through the default device. `Some(id)` is a [`cpal::DeviceId`] in its
    /// `Display` form; a device that is absent or an id that does not parse fails `start`, so
    /// audio never silently moves to a device the user did not choose.
    pub fn new(device: Option<String>) -> Self {
        Self { device }
    }
}

/// An output device as the settings dialog lists it: the stable id to save and the name to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// Every output device of the default host. Devices whose id or description cannot be read are
/// skipped rather than failing the list.
pub fn output_devices() -> Result<Vec<OutputDevice>, AudioError> {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .map_err(|e| AudioError::Device(format!("could not list output devices: {e}")))?;
    Ok(devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let name = device.description().ok()?.name().to_string();
            Some(OutputDevice { id, name })
        })
        .collect())
}

pub fn parse_device_id(id: &str) -> Result<cpal::DeviceId, AudioError> {
    id.parse()
        .map_err(|e| AudioError::Device(format!("output device {id:?}: {e}")))
}

struct Session {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput for CpalOutput {
    fn start(&self, mut render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError> {
        let device = self.device.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("brp-audio-out".into())
            .spawn(move || {
                let stream = match open_stream(device.as_deref(), &mut render) {
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
            .map_err(|e| AudioError::Device(format!("failed to spawn the output thread: {e}")))?;
        ready_rx
            .recv()
            .map_err(|_| AudioError::Device("output thread exited before reporting".into()))??;
        Ok(Box::new(Session {
            stop: Some(stop_tx),
            thread: Some(thread),
        }))
    }
}

fn open_stream(device: Option<&str>, render: &mut RenderFn) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = match device {
        None => host
            .default_output_device()
            .ok_or_else(|| AudioError::Device("no default output device".into()))?,
        Some(id) => {
            let parsed = parse_device_id(id)?;
            host.device_by_id(&parsed)
                .ok_or_else(|| AudioError::Device(format!("output device {id:?} not found")))?
        }
    };
    let config = cpal::StreamConfig {
        channels: u16::from(AUDIO_CHANNELS),
        sample_rate: AUDIO_SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };
    let mut render = std::mem::replace(render, Box::new(|_| {}));
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| render(data),
            |error| tracing::warn!(%error, "audio output stream error"),
            None,
        )
        .map_err(|e| AudioError::Format(format!("48 kHz stereo float output refused: {e}")))?;
    stream
        .play()
        .map_err(|e| AudioError::Device(format!("could not start playback: {e}")))?;
    Ok(stream)
}

impl AudioOutputSession for Session {}

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

    #[test]
    fn a_malformed_device_id_is_refused_by_name_without_touching_a_device() {
        let error = parse_device_id("nonsense").unwrap_err();
        assert!(matches!(error, AudioError::Device(_)), "{error}");
        assert!(error.to_string().contains("nonsense"), "{error}");
    }

    #[test]
    fn a_missing_device_fails_the_start_with_its_id_in_the_message() {
        let output = CpalOutput::new(Some("nonsense".into()));
        let error = output
            .start(Box::new(|_| {}))
            .err()
            .expect("start must fail");
        assert!(error.to_string().contains("nonsense"), "{error}");
    }

    #[test]
    fn listing_devices_does_not_fail_without_hardware() {
        // CI has no audio devices; an empty list is fine, an error is not.
        let devices = output_devices().expect("listing must succeed");
        for device in devices {
            assert!(!device.id.is_empty());
        }
    }
}
