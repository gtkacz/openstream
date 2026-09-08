//! The contracts between the platform backends and the pipeline.

use crate::error::AudioError;
use crate::selection::{AudioSelection, AudioSource};

/// Interleaved stereo float at 48 kHz, any length. Backends deliver what the platform hands them;
/// the pipeline cuts frames.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub capture_ts_us: u64,
}

pub type AudioSink = Box<dyn FnMut(AudioChunk) + Send + 'static>;

pub trait AudioCapture: Send + Sync {
    /// The applications the platform reports as producing audio right now, collapsed by identity
    /// and never including brp's own. Only what is audible now: unioning that with a selection
    /// whose applications are closed is the caller's business.
    fn sources(&self) -> Result<Vec<AudioSource>, AudioError>;

    /// Starts capturing what `selection` names. The selection is an argument, not state: a backend
    /// never learns that it can change, because a change swaps the session.
    fn start(
        &self,
        selection: AudioSelection,
        sink: AudioSink,
    ) -> Result<Box<dyn AudioCaptureSession>, AudioError>;
}

pub trait AudioCaptureSession: Send {
    /// Set once the backend's thread has died; the registry polls it and treats it as a failure.
    fn error(&self) -> Option<String>;
    fn stop(self: Box<Self>);
}

/// Fills the whole buffer with interleaved stereo float at 48 kHz. Called from the device thread.
pub type RenderFn = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

pub trait AudioOutput: Send + Sync {
    fn start(&self, render: RenderFn) -> Result<Box<dyn AudioOutputSession>, AudioError>;
}

/// Dropping the session stops playback.
pub trait AudioOutputSession: Send + Sync {}
