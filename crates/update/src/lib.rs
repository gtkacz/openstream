//! Self-update against GitHub Releases: learn the latest version, download and verify the
//! platform archive, and swap it into the directory the running binary was started from.
//!
//! Nothing here knows about windows or settings. The app decides when to check, when to
//! download, and how to relaunch.

use std::time::Duration;

pub mod apply;
pub mod archive;
pub mod error;
pub mod fetch;
pub mod install;
pub mod release;

pub use apply::{Staged, apply};
pub use error::UpdateError;
pub use fetch::{check, download};
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

/// The half of a release asset's name that says which build this process can run, or `None`
/// where the release workflow publishes no build for it: such a build is never offered an update
/// rather than offered one it cannot download, or one for another architecture.
#[cfg(all(windows, target_arch = "x86_64"))]
pub const PLATFORM: Option<&str> = Some("windows-x86_64");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const PLATFORM: Option<&str> = Some("macos-aarch64");
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const PLATFORM: Option<&str> = Some("linux-x86_64");
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
pub const PLATFORM: Option<&str> = None;

/// The platform name the staging scripts give a published build for `os` and `arch` (Rust's
/// `std::env::consts` spellings), or `None` where no release is published.
pub fn platform_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        _ => None,
    }
}
/// The archive format the publish job uses for this platform, and so the one `extract` reads.
#[cfg(windows)]
pub const ARCHIVE_EXTENSION: &str = ".zip";
#[cfg(not(windows))]
pub const ARCHIVE_EXTENSION: &str = ".tar.gz";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_published_platform_is_named_the_way_the_staging_scripts_do() {
        assert_eq!(platform_name("linux", "x86_64"), Some("linux-x86_64"));
        assert_eq!(platform_name("windows", "x86_64"), Some("windows-x86_64"));
        assert_eq!(platform_name("macos", "aarch64"), Some("macos-aarch64"));
    }

    #[test]
    fn a_platform_without_a_published_release_has_no_name() {
        // The release workflow builds Apple Silicon only; an Intel build must not ask for an
        // archive that is never uploaded, nor an ARM Linux build for the x86_64 one.
        assert_eq!(platform_name("macos", "x86_64"), None);
        assert_eq!(platform_name("linux", "aarch64"), None);
        assert_eq!(platform_name("windows", "aarch64"), None);
        assert_eq!(platform_name("freebsd", "x86_64"), None);
    }

    #[test]
    fn this_build_asks_for_its_own_platform() {
        assert_eq!(
            PLATFORM,
            platform_name(std::env::consts::OS, std::env::consts::ARCH)
        );
    }

    #[test]
    fn windows_releases_are_zips_and_every_other_platform_is_a_tarball() {
        let expected = if cfg!(windows) { ".zip" } else { ".tar.gz" };
        assert_eq!(ARCHIVE_EXTENSION, expected);
    }
}
