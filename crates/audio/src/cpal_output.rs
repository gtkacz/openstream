//! Playback through the default output device. cpal's stream type is not `Send` on every host, so
//! a dedicated thread owns it and parks until the session is dropped.

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::chunk::{AudioOutput, AudioOutputSession, RenderFn};
use crate::error::AudioError;

#[derive(Debug, Default, Clone, Copy)]
pub struct CpalOutput;

struct Session {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioOutput for CpalOutput {
    fn start(&self, mut render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("brp-audio-out".into())
            .spawn(move || {
                let stream = match open_stream(&mut render) {
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

fn open_stream(render: &mut RenderFn) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| AudioError::Device("no default output device".into()))?;
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
