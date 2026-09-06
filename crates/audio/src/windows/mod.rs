//! Windows capture: WASAPI process loopback in exclude mode, which records every process except
//! brp's own. Requires Windows 10 version 2004, the README's minimum.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use brp_proto::constants::{AUDIO_CHANNELS, AUDIO_SAMPLE_RATE};
use brp_proto::monotonic_us;
use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::chunk::{AudioCapture, AudioCaptureSession, AudioChunk, AudioSink};
use crate::error::AudioError;

/// How long one wait for the capture event may take before the loop checks the stop flag.
const EVENT_TIMEOUT_MS: u32 = 1000;

pub struct ProcessLoopbackCapture {
    process_id: u32,
}

impl ProcessLoopbackCapture {
    pub fn new(process_id: u32) -> Self {
        Self { process_id }
    }
}

struct Session {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture for ProcessLoopbackCapture {
    fn start(&self, sink: AudioSink) -> Result<Box<dyn AudioCaptureSession>, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let (flag, error_slot, process_id) = (stop.clone(), error.clone(), self.process_id);
        let thread = thread::Builder::new()
            .name("brp-audio-wasapi".into())
            .spawn(move || {
                if let Err(e) = run(process_id, sink, flag, ready_tx.clone()) {
                    let message = e.to_string();
                    let _ = ready_tx.send(Err(e));
                    *error_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                }
            })
            .map_err(|e| AudioError::Windows(format!("failed to spawn the WASAPI thread: {e}")))?;
        ready_rx
            .recv()
            .map_err(|_| AudioError::Windows("WASAPI thread exited before reporting".into()))??;
        Ok(Box::new(Session {
            stop,
            error,
            thread: Some(thread),
        }))
    }
}

fn run(
    process_id: u32,
    mut sink: AudioSink,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), AudioError>>,
) -> Result<(), AudioError> {
    initialize_mta()
        .ok()
        .map_err(|e| AudioError::Windows(format!("CoInitializeEx: {e}")))?;
    // Exclude mode: everything but this process tree.
    let mut client = AudioClient::new_application_loopback_client(process_id, false)
        .map_err(|e| AudioError::Unsupported(format!("process loopback activation: {e}")))?;
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        AUDIO_SAMPLE_RATE as usize,
        AUDIO_CHANNELS as usize,
        None,
    );
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    client
        .initialize_client(&format, &Direction::Capture, &mode)
        .map_err(|e| AudioError::Format(format!("IAudioClient::Initialize: {e}")))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| AudioError::Windows(format!("SetEventHandle: {e}")))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| AudioError::Windows(format!("GetService(IAudioCaptureClient): {e}")))?;
    client
        .start_stream()
        .map_err(|e| AudioError::Windows(format!("Start: {e}")))?;
    let _ = ready.send(Ok(()));

    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        capture
            .read_from_device_to_deque(&mut bytes)
            .map_err(|e| AudioError::Windows(format!("GetBuffer: {e}")))?;
        if bytes.len() >= 4 {
            let whole = bytes.len() - bytes.len() % 4;
            let samples: Vec<f32> = bytes
                .drain(..whole)
                .collect::<Vec<u8>>()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            sink(AudioChunk {
                samples,
                capture_ts_us: monotonic_us(),
            });
        }
        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            // A timeout is normal while nothing plays; the stop flag is what ends the loop.
            continue;
        }
    }
    client
        .stop_stream()
        .map_err(|e| AudioError::Windows(format!("Stop: {e}")))?;
    Ok(())
}

impl AudioCaptureSession for Session {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    fn stop(mut self: Box<Self>) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
