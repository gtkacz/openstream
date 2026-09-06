//! Screen capture behind a platform-neutral trait, plus a synthetic source for tests.
pub mod error;
pub mod fallback;
pub mod frame;
pub mod synthetic;
pub use error::CaptureError;
pub use frame::*;
pub use synthetic::SyntheticSource;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub use linux::PortalCapture;
#[cfg(target_os = "linux")]
pub use linux::PortalCapture as PlatformCapture;
