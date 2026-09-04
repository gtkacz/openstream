//! Screen capture behind a platform-neutral trait, plus a synthetic source for tests.
pub mod error;
pub mod frame;
pub mod synthetic;
pub use error::CaptureError;
pub use frame::*;
pub use synthetic::SyntheticSource;
