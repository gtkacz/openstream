//! Screen capture behind a platform-neutral trait, plus a synthetic source for tests.
pub mod error;
pub mod fallback;
pub mod frame;
#[cfg(any(target_os = "macos", test))]
pub(crate) mod macos_model;
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

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacCapture as PlatformCapture;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::WindowsCapture as PlatformCapture;
