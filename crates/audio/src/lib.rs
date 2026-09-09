//! Audio I/O behind platform-neutral traits: capture of everything the machine plays except brp,
//! and playback through the default output device or a saved one.
pub mod chunk;
pub mod cpal_output;
pub mod error;
pub mod fake_output;
#[cfg(any(windows, test))]
pub(crate) mod mix;
#[cfg(any(windows, test))]
pub(crate) mod process_tree;
pub mod selection;
pub mod synthetic;

pub use chunk::*;
pub use cpal_output::{CpalOutput, OutputDevice, output_devices};
pub use error::AudioError;
pub use fake_output::{FakeOutput, FakeOutputHandle};
pub use selection::{AppKey, AudioSelection, AudioSource};
pub use synthetic::SyntheticTone;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::PipeWireCapture as PlatformAudioCapture;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::ProcessLoopbackCapture as PlatformAudioCapture;
