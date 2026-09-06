//! Audio I/O behind platform-neutral traits: capture of everything the machine plays except brp,
//! and playback through the default output device.
pub mod chunk;
pub mod error;
pub mod fake_output;
pub mod synthetic;

pub use chunk::*;
pub use error::AudioError;
pub use fake_output::{FakeOutput, FakeOutputHandle};
pub use synthetic::SyntheticTone;
