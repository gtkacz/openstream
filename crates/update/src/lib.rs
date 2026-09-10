//! Self-update against GitHub Releases: learn the latest version, download and verify the
//! platform archive, and swap it into the directory the running binary was started from.
//!
//! Nothing here knows about windows or settings. The app decides when to check, when to
//! download, and how to relaunch.

use std::time::Duration;

pub mod apply;
pub mod archive;
pub mod error;
pub mod install;
pub mod release;

pub use apply::{Staged, apply};
pub use error::UpdateError;
pub use install::{Install, cleanup_stale};
pub use release::{Release, Version};

/// The releases of the renamed repository. GitHub redirects the old name here, and would redirect
/// this name too after another rename, which is why the check follows redirects.
pub const RELEASES_URL: &str = "https://github.com/gtkacz/openstream/releases";
/// Written by the publish job beside the archives.
pub const CHECKSUMS_FILE: &str = "SHA256SUMS";
/// Both staging scripts write it and no `cargo build` does: its presence beside the exe means the
/// directory is an extracted release that an update may replace.
pub const RELEASE_MARKER: &str = "FFMPEG-LICENSE.txt";
/// Prefix of the directory a download is staged in, inside the install directory so the final
/// renames stay on one file system.
pub const STAGING_PREFIX: &str = ".brp-update-";
/// Suffix the previous files are renamed to; a running exe and loaded DLLs can be renamed on
/// Windows but not deleted, so deletion waits for the next launch.
pub const OLD_SUFFIX: &str = ".old";
/// Connect and read timeout for every request. One redirect chain or one chunk that takes longer
/// means the network is unusable for this purpose.
pub const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// GitHub requires a user agent; the version is the only detail sent.
pub const USER_AGENT: &str = concat!("brp/", env!("CARGO_PKG_VERSION"));

#[cfg(windows)]
pub const PLATFORM: &str = "windows-x86_64";
#[cfg(not(windows))]
pub const PLATFORM: &str = "linux-x86_64";
#[cfg(windows)]
pub const ARCHIVE_EXTENSION: &str = ".zip";
#[cfg(not(windows))]
pub const ARCHIVE_EXTENSION: &str = ".tar.gz";
