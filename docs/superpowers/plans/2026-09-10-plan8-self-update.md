# Plan 8: Self-update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** brp notices a newer GitHub Release at launch, and one click downloads it, verifies it, swaps every file of the installed directory in place, and relaunches into the same room.

**Architecture:** A new crate `brp-update` owns everything that touches the network and the file system: the version check against the latest-release redirect, the download with its `SHA256SUMS` verification, archive extraction with path sanitisation, the rename-based in-place swap with rollback, and stale-file cleanup. The app crate adds a `check_updates` setting, an `UpdateState` machine with one widget drawn on the start screen and in the status bar, three `AppEvent` variants, and a `Relaunch` that `participant::run` spawns after the orderly leave. No protocol change, no release workflow change.

**Tech Stack:** Rust 2024, reqwest 0.13 (`rustls-no-provider`, already in the tree through iroh), sha2 0.11 (in the tree), tar 0.4, flate2 1.1, zip 8, egui 0.36, winit 0.30, tokio 1.53, toml 1.1.

**Spec:** `docs/superpowers/specs/2026-09-10-phase8-self-update-design.md`. Read it whole; sections 5, 8, and 11 are the contract this plan implements. Task 11 records the amendments listed under Global Constraints in that spec.

## Global Constraints

- **The repository was renamed.** `github.com/gtkacz/brp_sharing` answers with a 301 to `github.com/gtkacz/openstream`, and only then does `releases/latest` answer 302 to `releases/tag/vX.Y.Z`. Therefore `RELEASES_URL` is `https://github.com/gtkacz/openstream/releases`, and the check **follows redirects and reads the final URL** (`Response::url()`) instead of reading one `Location` header as the spec's section 5.1 says. A HEAD request is used so the tag page's HTML is never downloaded. This is spec amendment 1.
- **Both archive formats compile and are tested on both platforms.** `tar`, `flate2`, and `zip` are unconditional dependencies; only the choice of which archive to download and which extractor `extract` calls is `cfg`-selected. The spec's section 5.1 puts them behind `cfg`; this plan compiles both so the Windows extractor is exercised by the Linux suite, given that no Windows machine has run brp. Spec amendment 2.
- **Verified against the live v0.5.0 release.** The tarball holds `brp-0.5.0-linux-x86_64/` then its files with Unix modes (`brp` and the four `.so` files are `0755`). The zip holds `brp-0.5.0-windows-x86_64/<file>` entries with forward slashes and no directory entry. `SHA256SUMS` lines are `<64 lowercase hex>  <asset name>`. The install directory therefore contains, on Linux: `brp`, `libavcodec.so.62`, `libavutil.so.60`, `libswscale.so.9`, `libswresample.so.6`, `FFMPEG-LICENSE.txt`, `LICENSE`, `README.md`; on Windows: `brp.exe`, `avcodec-62.dll`, `avutil-60.dll`, `swscale-9.dll`, `swresample-6.dll`, `FFMPEG-LICENSE.txt`, `LICENSE`, `windows-enable-crash-dumps.reg`.
- **Constants live in `crates/update/src/lib.rs`, not in `brp-proto`.** Exact values: `RELEASES_URL = "https://github.com/gtkacz/openstream/releases"`, `CHECKSUMS_FILE = "SHA256SUMS"`, `RELEASE_MARKER = "FFMPEG-LICENSE.txt"`, `STAGING_PREFIX = ".brp-update-"`, `OLD_SUFFIX = ".old"`, `UPDATE_CHECK_TIMEOUT = Duration::from_secs(10)`, `USER_AGENT = concat!("brp/", env!("CARGO_PKG_VERSION"))`, and per target `PLATFORM = "linux-x86_64" | "windows-x86_64"` with `ARCHIVE_EXTENSION = ".tar.gz" | ".zip"`. The spec's `ASSET_SUFFIX` is split into `PLATFORM` and `ARCHIVE_EXTENSION` because the archive's top-level directory is `brp-<version>-<PLATFORM>`. `UPDATE_CHECK_TIMEOUT` is also the connect and read timeout of the download so a stalled transfer fails instead of hanging. Spec amendment 3.
- **`UpdateError` has no `NoRedirect` variant.** With redirects followed, a non-2xx final status is `Http` (through `error_for_status`) and a final URL that is not a tag is `Version`. Spec amendment 4.
- **Copy, verbatim.** Notice: `v0.6.0 is available`; without a release layout: `v0.6.0 is available at https://github.com/gtkacz/openstream/releases`. Button: `Update and restart`. Progress: `downloading 12 MB / 41 MB`, or `downloading 12 MB` without a total, decimal megabytes truncated. Settings checkbox: `Check for updates at launch (from the next launch)`.
- **Relaunch arguments.** In a room: `join <ticket>` then `--nickname <n>` if `WindowArgs.nickname` is `Some`, `--fps <n>` if `WindowArgs.fps` is `Some`, `--no-relay` if set. On the start screen: no arguments. The exe is `Install.exe`, captured at startup, never `current_exe()` at relaunch time (on Linux `/proc/self/exe` resolves to the renamed `.old` inode after the swap).
- **`brp-update` gains no egui, winit, or serde dependency.** The app crate gains only `brp-update`.
- **Progress events are throttled** in the app: one `AppEvent::UpdateProgress` per `PROGRESS_STEP_BYTES = 1_000_000` bytes received and one when the total is reached, so a 50 MB download does not wake the event loop thousands of times. `BYTES_PER_MB = 1_000_000` is the same value under the name the text formatter uses. Both live in `crates/app/src/ui/update.rs`. Spec amendment 5.
- **No network in tests.** `check` and `download` are glue over pure functions and are not unit-tested; everything they call is.
- Comments explain why. Doc comments state contracts on new public items. No task ids, branch names, or ticket numbers in code. Match the surrounding style: `//!` module docs, tests named by the behaviour they prove, `assert!(cond, "{value}")` with the value in the message.
- One Conventional Commit per task, imperative subject, with the `Claude-Session:` trailer the harness requires and no other trailer. Before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass. Run `git status --short` before committing and stage only this task's files; other agents work in this repository, so never `git add -A`, never rebase or reset, and if `Cargo.lock` shows unrelated changes leave them unstaged.
- `cargo build -p brp` on Linux must keep working with `RUSTFLAGS` unset: the new crate must not need FFmpeg or any system library.

## File Structure

```
Cargo.toml                                   + brp-update path dep; reqwest, sha2, tar, flate2, zip workspace deps

crates/update/Cargo.toml                     new crate brp-update
crates/update/src/lib.rs                     module decls, re-exports, the constants
crates/update/src/error.rs                   UpdateError
crates/update/src/release.rs                 Version, Release, newer_release, release_dir_name, asset_name, checksum_for
crates/update/src/install.rs                 Install, cleanup_stale
crates/update/src/archive.rs                 extract, extract_tar_gz, extract_zip, member_name
crates/update/src/apply.rs                   Staged, apply (with rollback)
crates/update/src/fetch.rs                   check, download (the only network code)

crates/app/Cargo.toml                        + brp-update
crates/app/src/lib.rs                        + pub mod relaunch
crates/app/src/settings.rs                   Settings.check_updates
crates/app/src/launch.rs                     test fixture gains the field
crates/app/src/ui/settings.rs                the checkbox row
crates/app/src/ui/update.rs                  new: UpdatePhase, UpdateState, notice, progress_text, draw
crates/app/src/ui/mod.rs                     + pub mod update; UiOutput.update_clicked; draw takes &UpdateState
crates/app/src/ui/start.rs                   StartAction::Update; draw takes &UpdateState
crates/app/src/ui/status.rs                  the notice and button after Settings
crates/app/src/relaunch.rs                   new: Relaunch, relaunch_args
crates/app/src/window.rs                     AppEvent variants, App fields, start_update, event handling, Shutdown.relaunch
crates/app/src/participant.rs                Install::current, cleanup_stale, the check task, the relaunch spawn

README.md                                    usage, privacy, platform sections, crate table, roadmap
docs/superpowers/specs/2026-09-10-phase8-self-update-design.md   section 15 amendments
```

---

### Task 1: Crate scaffold, constants, `Version`, `Release`, `UpdateError`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/update/Cargo.toml`
- Create: `crates/update/src/lib.rs`
- Create: `crates/update/src/error.rs`
- Create: `crates/update/src/release.rs`

**Interfaces:**
- Produces: `brp_update::{Version, Release, UpdateError}`, the constants listed in Global Constraints, and `brp_update::release::{release_dir_name, asset_name}`.

- [ ] **Step 1: Add the workspace dependencies**

In the root `Cargo.toml`, add to `[workspace.dependencies]` after `brp-room = { path = "crates/room" }`:

```toml
brp-update = { path = "crates/update" }
```

and at the end of the same table:

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls-no-provider"] }
sha2 = "0.11"
tar = "0.4"
flate2 = "1.1"
zip = { version = "8", default-features = false, features = ["deflate"] }
```

`rustls-no-provider` matches the feature set iroh already selects, so no second TLS stack is compiled; rustls resolves the ring provider iroh enables as the only one present. The `deflate` feature is what the release zip uses (method 8).

- [ ] **Step 2: Create the crate manifest**

`crates/update/Cargo.toml`:

```toml
[package]
name = "brp-update"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
reqwest.workspace = true
sha2.workspace = true
tar.workspace = true
flate2.workspace = true
zip.workspace = true
tokio = { workspace = true, features = ["fs", "io-util", "rt"] }
thiserror.workspace = true
tracing.workspace = true
```

- [ ] **Step 3: Write the failing tests for `Version`**

`crates/update/src/release.rs`, tests only for now (the module body comes in step 5):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_three_part_versions_parse_and_print_back() {
        assert_eq!("0.4.0".parse::<Version>().unwrap(), Version(0, 4, 0));
        assert_eq!("12.3.45".parse::<Version>().unwrap(), Version(12, 3, 45));
        assert_eq!(Version(12, 3, 45).to_string(), "12.3.45");
    }

    #[test]
    fn anything_but_three_numbers_is_rejected() {
        for text in ["v0.4.0", "0.4", "0.4.0.1", "0.4.0-rc1", "", "0..0", "+1.2.3"] {
            let error = text.parse::<Version>().unwrap_err();
            assert!(matches!(error, UpdateError::Version(_)), "{text}: {error}");
        }
    }

    #[test]
    fn versions_order_numerically_not_lexically() {
        assert!(Version(0, 4, 0) < Version(0, 10, 0));
        assert!(Version(0, 10, 0) < Version(1, 0, 0));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
    }

    #[test]
    fn asset_names_follow_the_release_workflow() {
        let version = Version(0, 5, 0);
        assert_eq!(
            release_dir_name(version),
            format!("brp-0.5.0-{PLATFORM}")
        );
        assert_eq!(
            asset_name(version),
            format!("brp-0.5.0-{PLATFORM}{ARCHIVE_EXTENSION}")
        );
    }
}
```

- [ ] **Step 4: Write `lib.rs` and `error.rs`**

`crates/update/src/lib.rs`:

```rust
//! Self-update against GitHub Releases: learn the latest version, download and verify the
//! platform archive, and swap it into the directory the running binary was started from.
//!
//! Nothing here knows about windows or settings. The app decides when to check, when to
//! download, and how to relaunch.

use std::time::Duration;

pub mod error;
pub mod release;

pub use error::UpdateError;
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
```

Tasks 3 to 6 each add their own `pub mod` line (alphabetical order: `apply`, `archive`, `error`, `fetch`, `install`, `release`) and re-export: `pub use apply::{Staged, apply};`, `pub use fetch::{check, download};`, `pub use install::{Install, cleanup_stale};`. Nothing is declared before its file exists, so the crate compiles at every commit.

`crates/update/src/error.rs`:

```rust
use std::path::PathBuf;

use thiserror::Error;

use crate::RELEASES_URL;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("github: {0}")]
    Http(#[from] reqwest::Error),
    #[error("not a release tag: {0:?}")]
    Version(String),
    #[error("checksum: {0}")]
    Checksum(String),
    #[error("archive rejected: {0}")]
    Archive(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("install directory {} is not writable: {source}", .dir.display())]
    NotWritable {
        dir: PathBuf,
        source: std::io::Error,
    },
    #[error("could not replace the installed files: {source}; {}", rollback_note(*.rolled_back))]
    Apply {
        source: std::io::Error,
        rolled_back: bool,
    },
}

fn rollback_note(rolled_back: bool) -> String {
    if rolled_back {
        "the previous version was restored".to_string()
    } else {
        format!("the previous version could not be restored, reinstall from {RELEASES_URL}")
    }
}
```

- [ ] **Step 5: Write `release.rs` above the tests**

```rust
//! What a release is called: the version, its tag, and the asset and directory names the release
//! workflow derives from it.

use std::fmt;
use std::str::FromStr;

use crate::error::UpdateError;
use crate::{ARCHIVE_EXTENSION, PLATFORM};

/// A plain `X.Y.Z`, the only shape the release workflow produces. Derived `Ord` compares the
/// three numbers in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl FromStr for Version {
    type Err = UpdateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reject = || UpdateError::Version(text.to_string());
        let mut parts = text.split('.');
        let (major, minor, patch) = (parts.next(), parts.next(), parts.next());
        if parts.next().is_some() {
            return Err(reject());
        }
        match (major, minor, patch) {
            (Some(major), Some(minor), Some(patch)) => Ok(Self(
                number(major).ok_or_else(reject)?,
                number(minor).ok_or_else(reject)?,
                number(patch).ok_or_else(reject)?,
            )),
            _ => Err(reject()),
        }
    }
}

/// Digits only: `u64::from_str` would also accept a leading `+`.
fn number(part: &str) -> Option<u64> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .then(|| part.parse().ok())
        .flatten()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A published release: the version and the tag its assets live under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
}

/// The archive's top-level directory, `brp-<version>-<platform>`.
pub fn release_dir_name(version: Version) -> String {
    format!("brp-{version}-{PLATFORM}")
}

/// The archive's file name on the release page.
pub fn asset_name(version: Version) -> String {
    format!("{}{ARCHIVE_EXTENSION}", release_dir_name(version))
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p brp-update`
Expected: 4 passed.

- [ ] **Step 7: Build and lint the workspace**

Run: `cargo build -p brp-update && cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean. This proves the reqwest feature set unifies with iroh's; the client itself is first constructed in Task 6.

- [ ] **Step 8: Commit**

```bash
git status --short
git add Cargo.toml Cargo.lock crates/update/Cargo.toml crates/update/src/lib.rs crates/update/src/error.rs crates/update/src/release.rs
git commit -m "feat(update): add the brp-update crate with versions and release names"
```

---

### Task 2: `newer_release` and `checksum_for`

**Files:**
- Modify: `crates/update/src/release.rs`

**Interfaces:**
- Produces: `release::newer_release(current: Version, final_url: &str) -> Result<Option<Release>, UpdateError>`, `release::checksum_for(sums: &str, asset: &str) -> Result<String, UpdateError>` (the lowercase hex digest).

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
    #[test]
    fn the_tag_page_url_yields_a_release_only_when_newer() {
        let url = "https://github.com/gtkacz/openstream/releases/tag/v0.5.0";
        assert_eq!(
            newer_release(Version(0, 4, 0), url).unwrap(),
            Some(Release {
                version: Version(0, 5, 0),
                tag: "v0.5.0".into(),
            })
        );
        assert_eq!(newer_release(Version(0, 5, 0), url).unwrap(), None);
        assert_eq!(newer_release(Version(0, 6, 0), url).unwrap(), None);
        // A trailing slash is how some redirects arrive.
        assert!(newer_release(Version(0, 4, 0), &format!("{url}/")).unwrap().is_some());
    }

    #[test]
    fn a_url_that_does_not_end_in_a_tag_is_a_version_error() {
        for url in [
            "https://github.com/gtkacz/openstream/releases",
            "https://github.com/gtkacz/openstream/releases/tag/0.5.0",
            "https://github.com/gtkacz/openstream/releases/tag/nightly",
            "",
        ] {
            let error = newer_release(Version(0, 4, 0), url).unwrap_err();
            assert!(matches!(error, UpdateError::Version(_)), "{url}: {error}");
        }
    }

    const SUMS: &str = "\
f6c4cab76618ef2b810f99c352684851226d7b0abd43ee9d4e62d934063a33a1  brp-0.4.0-linux-x86_64.tar.gz
E7DA9DC799608DB8A324DBA72CFC37B8B76B375B927A7AEF9A90CE758915A7F7  brp-0.4.0-windows-x86_64.zip
";

    #[test]
    fn the_digest_of_the_named_asset_is_found_and_lowercased() {
        assert_eq!(
            checksum_for(SUMS, "brp-0.4.0-linux-x86_64.tar.gz").unwrap(),
            "f6c4cab76618ef2b810f99c352684851226d7b0abd43ee9d4e62d934063a33a1"
        );
        assert_eq!(
            checksum_for(SUMS, "brp-0.4.0-windows-x86_64.zip").unwrap(),
            "e7da9dc799608db8a324dba72cfc37b8b76b375b927a7aef9a90ce758915a7f7"
        );
    }

    #[test]
    fn a_missing_asset_or_a_malformed_digest_is_a_checksum_error() {
        let missing = checksum_for(SUMS, "brp-0.4.0-macos.tar.gz").unwrap_err();
        assert!(matches!(missing, UpdateError::Checksum(_)), "{missing}");
        let short = checksum_for("abc123  brp.tar.gz\n", "brp.tar.gz").unwrap_err();
        assert!(matches!(short, UpdateError::Checksum(_)), "{short}");
        let not_hex = checksum_for(&format!("{}  brp.tar.gz\n", "g".repeat(64)), "brp.tar.gz")
            .unwrap_err();
        assert!(matches!(not_hex, UpdateError::Checksum(_)), "{not_hex}");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p brp-update`
Expected: compile errors naming `newer_release` and `checksum_for`.

- [ ] **Step 3: Implement**

Add to `release.rs` below `asset_name`:

```rust
/// The release the latest-release page led to, when it is newer than `current`. `final_url` is
/// the URL after every redirect, ending in `/releases/tag/vX.Y.Z`.
pub fn newer_release(current: Version, final_url: &str) -> Result<Option<Release>, UpdateError> {
    let tag = final_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let version: Version = tag
        .strip_prefix('v')
        .ok_or_else(|| UpdateError::Version(tag.to_string()))?
        .parse()?;
    Ok((version > current).then(|| Release {
        version,
        tag: tag.to_string(),
    }))
}

/// Length of a SHA-256 digest in hex characters.
const SHA256_HEX_LEN: usize = 64;

/// The digest listed for `asset` in a `sha256sum` output, lowercased for comparison.
pub fn checksum_for(sums: &str, asset: &str) -> Result<String, UpdateError> {
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name != asset {
            continue;
        }
        let well_formed =
            digest.len() == SHA256_HEX_LEN && digest.bytes().all(|b| b.is_ascii_hexdigit());
        return if well_formed {
            Ok(digest.to_ascii_lowercase())
        } else {
            Err(UpdateError::Checksum(format!(
                "malformed digest for {asset} in {}",
                crate::CHECKSUMS_FILE
            )))
        };
    }
    Err(UpdateError::Checksum(format!(
        "{asset} is not listed in {}",
        crate::CHECKSUMS_FILE
    )))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p brp-update`
Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/update/src/release.rs
git commit -m "feat(update): read the release tag from the redirect and the digest from SHA256SUMS"
```

---

### Task 3: `Install` and `cleanup_stale`

**Files:**
- Create: `crates/update/src/install.rs`
- Modify: `crates/update/src/lib.rs` (activate `pub mod install;` and `pub use install::{Install, cleanup_stale};`)

**Interfaces:**
- Produces: `Install { pub dir: PathBuf, pub exe: PathBuf }` with `Install::current()`, `Install::at(exe: PathBuf)`, `Install::is_release_layout(&self) -> bool`; `cleanup_stale(install: &Install)`.

- [ ] **Step 1: Write the failing tests**

`crates/update/src/install.rs`:

```rust
//! Where the running binary lives and whether that directory is an extracted release.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::error::UpdateError;
use crate::{OLD_SUFFIX, RELEASE_MARKER, STAGING_PREFIX};

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_install(test: &str) -> Install {
        let dir = std::env::temp_dir().join(format!("brp-install-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Install::at(dir.join("brp")).unwrap()
    }

    #[test]
    fn the_install_is_the_directory_of_the_exe() {
        let install = Install::at(PathBuf::from("/opt/brp-0.5.0/brp")).unwrap();
        assert_eq!(install.dir, PathBuf::from("/opt/brp-0.5.0"));
        assert_eq!(install.exe, PathBuf::from("/opt/brp-0.5.0/brp"));
        assert!(Install::at(PathBuf::from("/")).is_err());
    }

    #[test]
    fn the_ffmpeg_licence_beside_the_exe_marks_a_release_layout() {
        let install = temp_install("layout");
        assert!(!install.is_release_layout());
        fs::write(install.dir.join(RELEASE_MARKER), "LGPL").unwrap();
        assert!(install.is_release_layout());
    }

    #[test]
    fn cleanup_removes_old_files_and_staging_directories_and_nothing_else() {
        let install = temp_install("cleanup");
        fs::write(install.dir.join("brp"), "new").unwrap();
        fs::write(install.dir.join(format!("brp{OLD_SUFFIX}")), "old").unwrap();
        fs::write(install.dir.join(format!("libavcodec.so.62{OLD_SUFFIX}")), "old").unwrap();
        let staging = install.dir.join(format!("{STAGING_PREFIX}0.6.0"));
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("partial"), "x").unwrap();
        fs::write(install.dir.join("LICENSE"), "MIT").unwrap();

        cleanup_stale(&install);

        let mut names: Vec<String> = fs::read_dir(&install.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["LICENSE", "brp"]);
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp-update install`
Expected: compile errors, `Install` not found.

- [ ] **Step 3: Implement above the tests**

```rust
/// The directory the running binary was started from and the binary's path, taken once at startup:
/// after an update has renamed the running file, `current_exe` on Linux resolves to the `.old`
/// inode, so the path must be captured before any swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl Install {
    pub fn current() -> Result<Self, UpdateError> {
        Self::at(std::env::current_exe()?)
    }

    /// The install is the directory `exe` sits in.
    pub fn at(exe: PathBuf) -> Result<Self, UpdateError> {
        let dir = exe
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                UpdateError::Io(io::Error::other(format!(
                    "{} has no parent directory",
                    exe.display()
                )))
            })?
            .to_path_buf();
        Ok(Self { dir, exe })
    }

    /// True when the directory was produced by a staging script, so replacing its files is what
    /// an update means. A `cargo build` output has no such marker and is never touched.
    pub fn is_release_layout(&self) -> bool {
        self.dir.join(RELEASE_MARKER).is_file()
    }
}

/// Removes the previous version's `.old` files and any staging directory an earlier run left.
/// Best effort: on Windows the process that was just replaced may still be exiting, in which
/// case its files are removed by the launch after this one.
pub fn cleanup_stale(install: &Install) {
    let entries = match fs::read_dir(&install.dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(%error, dir = %install.dir.display(), "install directory not listed");
            return;
        }
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let path = entry.path();
        let result = if name.ends_with(OLD_SUFFIX) && path.is_file() {
            fs::remove_file(&path)
        } else if name.starts_with(STAGING_PREFIX) && path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            continue;
        };
        if let Err(error) = result {
            tracing::debug!(%error, path = %path.display(), "leftover from an earlier update kept");
        }
    }
}
```

Note the `Install::at(PathBuf::from("/"))` test: `Path::new("/").parent()` is `None`, so the `filter` on an empty parent only matters for relative names such as `brp`, whose parent is `""`.

- [ ] **Step 4: Activate the module and run the tests**

In `lib.rs` add `pub mod install;` and `pub use install::{Install, cleanup_stale};`.

Run: `cargo test -p brp-update`
Expected: 11 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/update/src/install.rs crates/update/src/lib.rs
git commit -m "feat(update): capture the install directory and clean up leftovers"
```

---

### Task 4: Archive extraction with path sanitisation

**Files:**
- Create: `crates/update/src/archive.rs`
- Modify: `crates/update/src/lib.rs` (activate `pub mod archive;`)

**Interfaces:**
- Produces: `archive::extract(archive: &Path, top_level: &str, into: &Path) -> Result<Vec<OsString>, UpdateError>` (the platform's format), `archive::extract_tar_gz` and `archive::extract_zip` with the same signature, `archive::member_name(path: &Path, top_level: &str) -> Result<OsString, UpdateError>`.

- [ ] **Step 1: Write the failing tests**

`crates/update/src/archive.rs`:

```rust
//! Unpacking a release archive into the staging directory. Both formats are compiled on every
//! platform so the Windows extractor is exercised by the Linux suite.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Component, Path};

use flate2::read::GzDecoder;

use crate::error::UpdateError;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const TOP: &str = "brp-0.6.0-test";

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-archive-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tar header whose name is written byte for byte, because `Header::set_path` refuses `..`
    /// and absolute paths and the tests need exactly those.
    fn raw_header(name: &str, size: u64, mode: u32) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_size(size);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        header
    }

    fn write_tar_gz(path: &Path, entries: &[(&str, &[u8], u32)], with_dir: bool) {
        let file = File::create(path).unwrap();
        let mut builder = tar::Builder::new(GzEncoder::new(file, Compression::fast()));
        if with_dir {
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{TOP}/")).unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder.append(&header, io::empty()).unwrap();
        }
        for (name, data, mode) in entries {
            builder
                .append(&raw_header(name, data.len() as u64, *mode), *data)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn member_names_are_one_plain_name_under_the_top_level_directory() {
        assert_eq!(
            member_name(Path::new("top/brp"), "top").unwrap(),
            OsString::from("brp")
        );
        for bad in [
            "other/brp",
            "top/sub/brp",
            "top/../escape",
            "/top/brp",
            "top",
            "top/",
            "./top/brp",
        ] {
            let error = member_name(Path::new(bad), "top").unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{bad}: {error}");
        }
    }

    #[test]
    fn a_tarball_extracts_its_files_with_their_modes_and_skips_the_directory_entry() {
        let dir = temp_dir("tar_ok");
        let archive = dir.join("release.tar.gz");
        write_tar_gz(
            &archive,
            &[
                (&format!("{TOP}/brp"), b"binary", 0o755),
                (&format!("{TOP}/LICENSE"), b"MIT", 0o644),
            ],
            true,
        );
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        let mut files = extract_tar_gz(&archive, TOP, &out).unwrap();
        files.sort();
        assert_eq!(files, [OsString::from("LICENSE"), OsString::from("brp")]);
        assert_eq!(fs::read(out.join("brp")).unwrap(), b"binary");
        assert_eq!(fs::read(out.join("LICENSE")).unwrap(), b"MIT");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(out.join("brp")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "{mode:o}");
        }
    }

    #[test]
    fn a_tarball_with_an_escaping_nested_or_foreign_entry_is_rejected_before_writing() {
        for (case, name) in [
            ("parent", format!("{TOP}/../escape")),
            ("absolute", "/tmp/escape".to_string()),
            ("nested", format!("{TOP}/sub/brp")),
            ("wrong_top", "brp-9.9.9-other/brp".to_string()),
        ] {
            let dir = temp_dir(&format!("tar_bad_{case}"));
            let archive = dir.join("release.tar.gz");
            write_tar_gz(
                &archive,
                &[(&format!("{TOP}/brp"), b"binary", 0o755), (&name, b"x", 0o644)],
                false,
            );
            let out = dir.join("out");
            fs::create_dir(&out).unwrap();
            let error = extract_tar_gz(&archive, TOP, &out).unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{case}: {error}");
            // The good entry before the bad one may have been written; the bad one never is.
            assert!(!dir.join("escape").exists(), "{case}");
            assert!(!out.join("sub").exists(), "{case}");
            assert!(!Path::new("/tmp/escape").exists(), "{case}");
        }
    }

    #[test]
    fn a_zip_extracts_forward_and_backslash_entries_alike_and_skips_directories() {
        let dir = temp_dir("zip_ok");
        let archive = dir.join("release.zip");
        write_zip(
            &archive,
            &[
                (&format!("{TOP}/"), b""),
                (&format!("{TOP}/brp.exe"), b"binary"),
                (&format!("{TOP}\\avcodec-62.dll"), b"codec"),
            ],
        );
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        let mut files = extract_zip(&archive, TOP, &out).unwrap();
        files.sort();
        assert_eq!(
            files,
            [OsString::from("avcodec-62.dll"), OsString::from("brp.exe")]
        );
        assert_eq!(listing(&out), ["avcodec-62.dll", "brp.exe"]);
        assert_eq!(fs::read(out.join("brp.exe")).unwrap(), b"binary");
    }

    #[test]
    fn a_zip_with_an_escaping_or_nested_entry_is_rejected() {
        for (case, name) in [
            ("parent", format!("{TOP}/../escape")),
            ("nested", format!("{TOP}/sub/brp.exe")),
            ("wrong_top", "other/brp.exe".to_string()),
        ] {
            let dir = temp_dir(&format!("zip_bad_{case}"));
            let archive = dir.join("release.zip");
            write_zip(&archive, &[(&name, b"x")]);
            let out = dir.join("out");
            fs::create_dir(&out).unwrap();
            let error = extract_zip(&archive, TOP, &out).unwrap_err();
            assert!(matches!(error, UpdateError::Archive(_)), "{case}: {error}");
            assert!(listing(&out).is_empty(), "{case}");
        }
    }

    #[test]
    fn extract_uses_the_platform_format() {
        let dir = temp_dir("platform");
        let archive = dir.join(format!("release{}", crate::ARCHIVE_EXTENSION));
        let entry = format!("{TOP}/file");
        if cfg!(windows) {
            write_zip(&archive, &[(&entry, b"x")]);
        } else {
            write_tar_gz(&archive, &[(&entry, b"x", 0o644)], false);
        }
        let out = dir.join("out");
        fs::create_dir(&out).unwrap();
        assert_eq!(
            extract(&archive, TOP, &out).unwrap(),
            [OsString::from("file")]
        );
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp-update archive`
Expected: compile errors, `member_name`, `extract_tar_gz`, `extract_zip`, `extract` not found.

- [ ] **Step 3: Implement above the tests**

```rust
/// Unpacks the platform's archive format. Returns the file names written into `into`.
pub fn extract(archive: &Path, top_level: &str, into: &Path) -> Result<Vec<OsString>, UpdateError> {
    if cfg!(windows) {
        extract_zip(archive, top_level, into)
    } else {
        extract_tar_gz(archive, top_level, into)
    }
}

/// The Linux release: one top-level directory of regular files whose Unix modes matter, since
/// `brp` and the shared libraries must stay executable.
pub fn extract_tar_gz(
    archive: &Path,
    top_level: &str,
    into: &Path,
) -> Result<Vec<OsString>, UpdateError> {
    let file = File::open(archive)?;
    let mut tar = tar::Archive::new(GzDecoder::new(BufReader::new(file)));
    let mut files = Vec::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        let path = entry.path()?.into_owned();
        if !kind.is_file() {
            return Err(UpdateError::Archive(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        let name = member_name(&path, top_level)?;
        // `unpack` applies the header's mode on Unix and refuses to follow links; the type and
        // path checks above already rejected anything that is not a plain file.
        entry.unpack(into.join(&name))?;
        files.push(name);
    }
    Ok(files)
}

/// The Windows release. Entry names are normalised to forward slashes first because
/// `Compress-Archive` has written backslashes in some PowerShell versions.
pub fn extract_zip(
    archive: &Path,
    top_level: &str,
    into: &Path,
) -> Result<Vec<OsString>, UpdateError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).map_err(archive_error)?;
    let mut files = Vec::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index).map_err(archive_error)?;
        if member.is_dir() {
            continue;
        }
        if member.is_symlink() {
            return Err(UpdateError::Archive(format!("{} is a link", member.name())));
        }
        let normalised = member.name().replace('\\', "/");
        let name = member_name(Path::new(&normalised), top_level)?;
        let mut out = File::create(into.join(&name))?;
        io::copy(&mut member, &mut out)?;
        files.push(name);
    }
    Ok(files)
}

fn archive_error(error: zip::result::ZipError) -> UpdateError {
    UpdateError::Archive(error.to_string())
}

/// The file name of an entry that is exactly `<top_level>/<name>`. Anything else, including
/// `..`, an absolute path, a nested directory, or another top level, is rejected so a crafted
/// archive cannot write outside the staging directory.
pub fn member_name(path: &Path, top_level: &str) -> Result<OsString, UpdateError> {
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::Normal(top)), Some(Component::Normal(name)), None)
            if top == top_level && !name.is_empty() =>
        {
            Ok(name.to_os_string())
        }
        _ => Err(UpdateError::Archive(format!(
            "unexpected entry {}",
            path.display()
        ))),
    }
}
```

If `ZipFile::is_symlink` does not exist in the resolved zip 8 version, replace that check with `member.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000)` and add a one-line comment naming the S_IFLNK constant. Check with `cargo doc -p zip --no-deps` before choosing.

- [ ] **Step 4: Activate the module and run the tests**

In `lib.rs` add `pub mod archive;`.

Run: `cargo test -p brp-update`
Expected: 17 passed. The tar mode assertion runs on Linux; the whole zip path runs on Linux too.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/update/src/archive.rs crates/update/src/lib.rs
git commit -m "feat(update): extract release archives with entry sanitisation"
```

---

### Task 5: `apply` with rollback

**Files:**
- Create: `crates/update/src/apply.rs`
- Modify: `crates/update/src/lib.rs` (activate `pub mod apply;` and `pub use apply::{Staged, apply};`)

**Interfaces:**
- Consumes: `Install`, `OLD_SUFFIX`.
- Produces: `pub struct Staged { pub(crate) dir: PathBuf, pub(crate) files: Vec<OsString> }` and `pub fn apply(staged: Staged, install: &Install) -> Result<(), UpdateError>`.

- [ ] **Step 1: Write the failing tests**

`crates/update/src/apply.rs`:

```rust
//! Swapping the staged release into the install directory: the previous file is renamed aside,
//! never deleted, because Windows lets a running exe and its loaded DLLs be renamed but not
//! removed. A failure part-way puts every rename back.

use std::ffi::OsString;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

use crate::OLD_SUFFIX;
use crate::error::UpdateError;
use crate::install::Install;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_install(test: &str) -> Install {
        let dir = std::env::temp_dir().join(format!("brp-apply-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Install::at(dir.join("brp")).unwrap()
    }

    fn stage(install: &Install, files: &[(&str, &[u8])], listed: &[&str]) -> Staged {
        let dir = install.dir.join(".brp-update-test");
        fs::create_dir(&dir).unwrap();
        for (name, data) in files {
            fs::write(dir.join(name), data).unwrap();
        }
        Staged {
            dir,
            files: listed.iter().map(OsString::from).collect(),
        }
    }

    fn listing(install: &Install) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&install.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_staged_file_replaces_its_predecessor_which_survives_as_old() {
        let install = temp_install("swap");
        fs::write(install.dir.join("brp"), "v1").unwrap();
        fs::write(install.dir.join("LICENSE"), "MIT").unwrap();
        let staged = stage(
            &install,
            &[("brp", b"v2"), ("LICENSE", b"MIT"), ("NEW.txt", b"new")],
            &["brp", "LICENSE", "NEW.txt"],
        );

        apply(staged, &install).unwrap();

        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v2");
        assert_eq!(fs::read(install.dir.join("brp.old")).unwrap(), b"v1");
        assert_eq!(fs::read(install.dir.join("NEW.txt")).unwrap(), b"new");
        assert_eq!(
            listing(&install),
            ["LICENSE", "LICENSE.old", "NEW.txt", "brp", "brp.old"]
        );
    }

    #[test]
    fn a_stale_old_file_is_replaced_by_the_current_one() {
        let install = temp_install("stale");
        fs::write(install.dir.join("brp"), "v2").unwrap();
        fs::write(install.dir.join("brp.old"), "v1").unwrap();
        let staged = stage(&install, &[("brp", b"v3")], &["brp"]);

        apply(staged, &install).unwrap();

        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v3");
        assert_eq!(fs::read(install.dir.join("brp.old")).unwrap(), b"v2");
    }

    #[test]
    fn a_failure_part_way_restores_every_file_and_reports_the_rollback() {
        let install = temp_install("rollback");
        fs::write(install.dir.join("brp"), "v1").unwrap();
        fs::write(install.dir.join("libx.so"), "lib1").unwrap();
        // `libx.so` is listed but never staged, so its rename fails after `brp` was swapped.
        let staged = stage(&install, &[("brp", b"v2"), ("NEW.txt", b"n")], &["brp", "NEW.txt", "libx.so"]);

        let error = apply(staged, &install).unwrap_err();

        assert!(
            matches!(error, UpdateError::Apply { rolled_back: true, .. }),
            "{error}"
        );
        assert_eq!(fs::read(install.dir.join("brp")).unwrap(), b"v1");
        assert_eq!(fs::read(install.dir.join("libx.so")).unwrap(), b"lib1");
        assert_eq!(listing(&install), ["brp", "libx.so"]);
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp-update apply`
Expected: compile errors, `Staged` and `apply` not found.

- [ ] **Step 3: Implement above the tests**

```rust
/// A verified, extracted release waiting in its staging directory.
#[derive(Debug)]
pub struct Staged {
    pub(crate) dir: PathBuf,
    pub(crate) files: Vec<OsString>,
}

/// One completed step: `current` now holds the new file, and `previous` is where the old one went
/// (`None` when there was no old one).
struct Swap {
    current: PathBuf,
    previous: Option<PathBuf>,
}

/// Moves every staged file into place. The staging directory is removed whatever happens.
pub fn apply(staged: Staged, install: &Install) -> Result<(), UpdateError> {
    let mut swaps = Vec::new();
    let outcome = swap_all(&staged, install, &mut swaps);
    let _ = fs::remove_dir_all(&staged.dir);
    match outcome {
        Ok(()) => Ok(()),
        Err(source) => {
            let rolled_back = roll_back(swaps);
            Err(UpdateError::Apply {
                source,
                rolled_back,
            })
        }
    }
}

fn swap_all(staged: &Staged, install: &Install, swaps: &mut Vec<Swap>) -> io::Result<()> {
    for name in &staged.files {
        let current = install.dir.join(name);
        let mut previous_name = name.clone();
        previous_name.push(OLD_SUFFIX);
        let previous = install.dir.join(previous_name);
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        let had_current = current.exists();
        if had_current {
            fs::rename(&current, &previous)?;
        }
        // Recorded before the second rename so a failure there still puts `previous` back.
        swaps.push(Swap {
            current: current.clone(),
            previous: had_current.then_some(previous),
        });
        fs::rename(staged.dir.join(name), &current)?;
    }
    Ok(())
}

/// Reverses the completed swaps, newest first. Returns whether every one went back.
fn roll_back(swaps: Vec<Swap>) -> bool {
    let mut restored = true;
    for swap in swaps.into_iter().rev() {
        let result = match &swap.previous {
            Some(previous) => fs::rename(previous, &swap.current),
            None => match fs::remove_file(&swap.current) {
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        if let Err(error) = result {
            tracing::error!(%error, path = %swap.current.display(), "rollback failed");
            restored = false;
        }
    }
    restored
}
```

- [ ] **Step 4: Activate the module and run the tests**

In `lib.rs` add `pub mod apply;` and `pub use apply::{Staged, apply};`.

Run: `cargo test -p brp-update`
Expected: 20 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/update/src/apply.rs crates/update/src/lib.rs
git commit -m "feat(update): swap the staged release into place with rollback"
```

---

### Task 6: `check` and `download`

**Files:**
- Create: `crates/update/src/fetch.rs`
- Modify: `crates/update/src/lib.rs` (activate `pub mod fetch;` and `pub use fetch::{check, download};`)

**Interfaces:**
- Consumes: `newer_release`, `checksum_for`, `asset_name`, `release_dir_name`, `extract`, `Staged`, `Install`, the constants.
- Produces: `pub async fn check(current: Version) -> Result<Option<Release>, UpdateError>` and `pub async fn download(release: &Release, install: &Install, progress: impl FnMut(u64, Option<u64>) + Send) -> Result<Staged, UpdateError>`.

- [ ] **Step 1: Write the one test this module can have without a network**

`crates/update/src/fetch.rs`:

```rust
//! The two network operations, built from the pure functions in `release` and `archive`.

use std::fs;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::apply::Staged;
use crate::archive::extract;
use crate::error::UpdateError;
use crate::install::Install;
use crate::release::{Release, Version, asset_name, checksum_for, newer_release, release_dir_name};
use crate::{CHECKSUMS_FILE, RELEASES_URL, STAGING_PREFIX, UPDATE_CHECK_TIMEOUT, USER_AGENT};

#[cfg(test)]
mod tests {
    use super::*;

    /// Building the client resolves the TLS stack. With iroh's `tls-ring` the only provider in
    /// the tree, rustls installs it; a second provider in the tree would make this panic.
    #[test]
    fn the_http_client_builds_with_the_tls_stack_in_the_tree() {
        client().unwrap();
    }

    #[test]
    fn digests_print_as_lowercase_hex() {
        let digest = Sha256::digest(b"");
        assert_eq!(
            hex(&digest),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp-update fetch`
Expected: compile errors, `client` and `hex` not found.

- [ ] **Step 3: Implement above the tests**

```rust
/// Asks GitHub which release is the latest. `Some` only when it is newer than `current`.
///
/// A HEAD request that follows redirects: the old repository name redirects to the new one, and
/// the latest-release page redirects to the tag page, whose URL names the version. The tag page
/// itself is never downloaded.
pub async fn check(current: Version) -> Result<Option<Release>, UpdateError> {
    let response = client()?
        .head(format!("{RELEASES_URL}/latest"))
        .send()
        .await?
        .error_for_status()?;
    newer_release(current, response.url().as_str())
}

/// Downloads the platform archive for `release` into a staging directory inside `install.dir`,
/// checks it against the release's `SHA256SUMS`, and extracts it there. `progress` is called
/// with the bytes received so far and the total when the server states one.
pub async fn download(
    release: &Release,
    install: &Install,
    mut progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<Staged, UpdateError> {
    let staging = install
        .dir
        .join(format!("{STAGING_PREFIX}{}", release.version));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging).map_err(|source| UpdateError::NotWritable {
        dir: install.dir.clone(),
        source,
    })?;
    let outcome = fetch_into(release, &staging, &mut progress).await;
    if outcome.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    outcome
}

async fn fetch_into(
    release: &Release,
    staging: &Path,
    progress: &mut (impl FnMut(u64, Option<u64>) + Send),
) -> Result<Staged, UpdateError> {
    let client = client()?;
    let base = format!("{RELEASES_URL}/download/{}", release.tag);
    let sums = client
        .get(format!("{base}/{CHECKSUMS_FILE}"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let asset = asset_name(release.version);
    let expected = checksum_for(&sums, &asset)?;

    let archive = staging.join(&asset);
    let mut response = client
        .get(format!("{base}/{asset}"))
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = response.chunk().await? {
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        progress(received, total);
    }
    file.flush().await?;
    drop(file);
    if hex(&hasher.finalize()) != expected {
        return Err(UpdateError::Checksum(format!("{asset} does not match {CHECKSUMS_FILE}")));
    }

    let top_level = release_dir_name(release.version);
    let files = tokio::task::spawn_blocking({
        let archive = archive.clone();
        let staging = staging.to_path_buf();
        move || extract(&archive, &top_level, &staging)
    })
    .await
    .map_err(|join| UpdateError::Io(io::Error::other(join)))??;
    fs::remove_file(&archive)?;
    Ok(Staged {
        dir: staging.to_path_buf(),
        files,
    })
}

/// One client for both operations: redirects followed (assets live on GitHub's object store),
/// a bounded connect and per-read wait so a dead network fails rather than hangs.
fn client() -> Result<reqwest::Client, UpdateError> {
    Ok(reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(UPDATE_CHECK_TIMEOUT)
        .read_timeout(UPDATE_CHECK_TIMEOUT)
        .build()?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
```

- [ ] **Step 4: Activate the module and run everything**

In `lib.rs` add `pub mod fetch;` and `pub use fetch::{check, download};`. Every module of the crate is now declared.

Run: `cargo test -p brp-update`
Expected: 22 passed.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean. If clippy flags `let _ = fs::remove_dir_all(..)`, it is intentional (a failed cleanup of a directory we are about to report an error for changes nothing) and the calls stay.

- [ ] **Step 5: One manual probe, then commit**

Run once, from a scratch example, to confirm the redirect chain resolves against the real repository (this is the only network use in the plan and is not a test):

```bash
cat > /tmp/probe.rs <<'EOF'
fn main() {
    // Current-thread: brp-update enables only tokio's `rt`, not `rt-multi-thread`.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let found = rt.block_on(brp_update::check(brp_update::Version(0, 0, 0))).unwrap();
    println!("{found:?}");
}
EOF
mkdir -p crates/update/examples && cp /tmp/probe.rs crates/update/examples/probe.rs
cargo run -p brp-update --example probe
rm -r crates/update/examples
```

Expected output names the current latest tag, for example `Some(Release { version: Version(0, 5, 0), tag: "v0.5.0" })`. The example is deleted before committing.

```bash
cargo fmt --all
git status --short
git add crates/update/src/fetch.rs crates/update/src/lib.rs
git commit -m "feat(update): check the latest release and download the verified archive"
```

---

### Task 7: The `check_updates` setting and its checkbox

**Files:**
- Modify: `crates/app/src/settings.rs`
- Modify: `crates/app/src/launch.rs` (the `saved()` test fixture)
- Modify: `crates/app/src/ui/settings.rs`

**Interfaces:**
- Produces: `Settings.check_updates: bool`, default `true`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/settings.rs` `mod tests`, change `full()` to include `check_updates: false,` after `recent_rooms: vec![..],`, add to `the_file_shape_matches_the_spec` after the `[[recent_rooms]]` assertion:

```rust
        assert!(text.contains("check_updates = false"), "{text}");
```

and add a test:

```rust
    #[test]
    fn a_file_from_before_the_updater_checks_for_updates() {
        let settings: Settings = toml::from_str("fps = 24\n").unwrap();
        assert!(settings.check_updates);
        assert!(Settings::default().check_updates);
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp settings`
Expected: compile error, no field `check_updates`.

- [ ] **Step 3: Add the field**

In `Settings`:

```rust
    pub recent_rooms: Vec<RecentRoom>,
    /// Ask GitHub for the latest release at launch. On by default and documented as a network
    /// request, which is why it can be turned off.
    pub check_updates: bool,
```

In `Default for Settings`: `check_updates: true,`. In `crates/app/src/launch.rs` test fixture `saved()`, add `check_updates: true,` after `recent_rooms: Vec::new(),`.

- [ ] **Step 4: Add the checkbox row**

In `crates/app/src/ui/settings.rs` `draw`, after the `Audio output` row's `ui.end_row();` and before the grid closure closes:

```rust
                    ui.label("Updates");
                    ui.checkbox(
                        &mut dialog.draft.check_updates,
                        "Check for updates at launch (from the next launch)",
                    );
                    ui.end_row();
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p brp`
Expected: all pass, including the two new assertions.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/src/settings.rs crates/app/src/launch.rs crates/app/src/ui/settings.rs
git commit -m "feat(app): add the check-for-updates setting"
```

---

### Task 8: `ui::update` state machine and widget

**Files:**
- Modify: `crates/app/Cargo.toml` (add `brp-update.workspace = true` after `brp-audio.workspace = true`)
- Create: `crates/app/src/ui/update.rs`
- Modify: `crates/app/src/ui/mod.rs` (add `pub mod update;` in alphabetical order, after `pub mod tiles;`)

**Interfaces:**
- Consumes: `brp_update::{Release, RELEASES_URL}`.
- Produces: `UpdatePhase`, `UpdateState { pub available: Option<Release>, pub can_apply: bool, pub phase: UpdatePhase }`, `UpdateState::new(can_apply: bool)`, `UpdateState::start(&mut self) -> Option<Release>`, `UpdateState::progress(&mut self, received: u64, total: Option<u64>)`, `UpdateState::failed(&mut self, message: String)`, `notice(release: &Release, can_apply: bool) -> String`, `progress_text(received: u64, total: Option<u64>) -> String`, `draw(ui: &mut egui::Ui, state: &UpdateState) -> bool`, and the constants `BYTES_PER_MB`, `PROGRESS_STEP_BYTES`.

- [ ] **Step 1: Write the failing tests**

`crates/app/src/ui/update.rs`:

```rust
//! The update notice and button, drawn on the start screen and in the status bar. Kept apart from
//! `UiState`, which a room opening resets: a download in flight must survive that.

use brp_update::{RELEASES_URL, Release};

/// Decimal megabytes, the unit the progress line shows.
pub const BYTES_PER_MB: u64 = 1_000_000;
/// One progress event per this many bytes, so a 50 MB download does not wake the event loop
/// once per network chunk.
pub const PROGRESS_STEP_BYTES: u64 = BYTES_PER_MB;

#[cfg(test)]
mod tests {
    use brp_update::Version;

    use super::*;

    fn release() -> Release {
        Release {
            version: Version(0, 6, 0),
            tag: "v0.6.0".into(),
        }
    }

    #[test]
    fn start_hands_out_the_release_once_until_the_download_settles() {
        let mut state = UpdateState::new(true);
        assert_eq!(state.start(), None, "nothing available yet");
        state.available = Some(release());
        assert_eq!(state.start(), Some(release()));
        assert!(matches!(state.phase, UpdatePhase::Downloading { .. }));
        assert_eq!(state.start(), None, "one download at a time");
        state.progress(5, Some(10));
        assert_eq!(
            state.phase,
            UpdatePhase::Downloading {
                received: 5,
                total: Some(10)
            }
        );
        state.failed("network".into());
        assert_eq!(state.phase, UpdatePhase::Failed("network".into()));
        assert_eq!(state.start(), Some(release()), "a failure allows a retry");
    }

    #[test]
    fn without_a_release_layout_nothing_can_be_started() {
        let mut state = UpdateState::new(false);
        state.available = Some(release());
        assert_eq!(state.start(), None);
        assert_eq!(state.phase, UpdatePhase::Idle);
    }

    #[test]
    fn progress_outside_a_download_is_ignored() {
        let mut state = UpdateState::new(true);
        state.progress(1, None);
        assert_eq!(state.phase, UpdatePhase::Idle);
    }

    #[test]
    fn the_notice_names_the_version_and_points_at_the_page_when_nothing_can_be_applied() {
        assert_eq!(notice(&release(), true), "v0.6.0 is available");
        assert_eq!(
            notice(&release(), false),
            format!("v0.6.0 is available at {RELEASES_URL}")
        );
    }

    #[test]
    fn progress_text_truncates_to_whole_megabytes() {
        assert_eq!(
            progress_text(12_999_999, Some(41_000_000)),
            "downloading 12 MB / 41 MB"
        );
        assert_eq!(progress_text(999_999, None), "downloading 0 MB");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp ui::update`
Expected: compile errors, `UpdateState` not found.

- [ ] **Step 3: Implement between the constants and the tests**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Downloading { received: u64, total: Option<u64> },
    Failed(String),
}

/// What the window knows about updates: the release the launch check found, whether this install
/// can be replaced, and how far a download has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateState {
    pub available: Option<Release>,
    /// The install is a release layout, so the button is offered; a dev build only gets the notice.
    pub can_apply: bool,
    pub phase: UpdatePhase,
}

impl UpdateState {
    pub fn new(can_apply: bool) -> Self {
        Self {
            available: None,
            can_apply,
            phase: UpdatePhase::Idle,
        }
    }

    /// The button was clicked: the release to download, or `None` while one is already in flight
    /// or nothing can be applied.
    pub fn start(&mut self) -> Option<Release> {
        if !self.can_apply || matches!(self.phase, UpdatePhase::Downloading { .. }) {
            return None;
        }
        let release = self.available.clone()?;
        self.phase = UpdatePhase::Downloading {
            received: 0,
            total: None,
        };
        Some(release)
    }

    pub fn progress(&mut self, received: u64, total: Option<u64>) {
        if matches!(self.phase, UpdatePhase::Downloading { .. }) {
            self.phase = UpdatePhase::Downloading { received, total };
        }
    }

    pub fn failed(&mut self, message: String) {
        self.phase = UpdatePhase::Failed(message);
    }
}

pub fn notice(release: &Release, can_apply: bool) -> String {
    if can_apply {
        format!("v{} is available", release.version)
    } else {
        format!("v{} is available at {RELEASES_URL}", release.version)
    }
}

pub fn progress_text(received: u64, total: Option<u64>) -> String {
    let received = received / BYTES_PER_MB;
    match total {
        Some(total) => format!("downloading {received} MB / {} MB", total / BYTES_PER_MB),
        None => format!("downloading {received} MB"),
    }
}

/// Draws the notice with the button, the progress line, or nothing when no release is known.
/// Returns true when the button was clicked.
pub fn draw(ui: &mut egui::Ui, state: &UpdateState) -> bool {
    let Some(release) = &state.available else {
        return false;
    };
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.weak(notice(release, state.can_apply));
        if !state.can_apply {
            return;
        }
        match &state.phase {
            UpdatePhase::Downloading { received, total } => {
                ui.weak(progress_text(*received, *total));
            }
            UpdatePhase::Idle | UpdatePhase::Failed(_) => {
                if ui.button("Update and restart").clicked() {
                    clicked = true;
                }
            }
        }
    });
    if let UpdatePhase::Failed(message) = &state.phase {
        ui.colored_label(egui::Color32::LIGHT_RED, message);
    }
    clicked
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p brp ui::update`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/Cargo.toml Cargo.lock crates/app/src/ui/update.rs crates/app/src/ui/mod.rs
git commit -m "feat(app): add the update notice state and widget"
```

---

### Task 9: `relaunch` module

**Files:**
- Create: `crates/app/src/relaunch.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod relaunch;` between `pub mod publish;` and `pub mod render;`)

**Interfaces:**
- Consumes: `brp_update::Install`, `crate::cli::WindowArgs`.
- Produces: `Relaunch { pub exe: PathBuf, pub args: Vec<OsString> }`, `Relaunch::new(install: &Install, ticket: Option<&str>, args: &WindowArgs) -> Self`, `Relaunch::spawn(&self)`, `relaunch_args(ticket: Option<&str>, args: &WindowArgs) -> Vec<OsString>`.

- [ ] **Step 1: Write the failing tests**

`crates/app/src/relaunch.rs`:

```rust
//! Starting the updated binary after this process leaves: back into the same room, with the
//! flags this launch was given.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use brp_update::Install;

use crate::cli::WindowArgs;

#[cfg(test)]
mod tests {
    use super::*;

    fn os(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn in_a_room_the_new_process_joins_it_with_each_present_flag() {
        let args = WindowArgs {
            nickname: Some("alice".into()),
            fps: Some(30),
            no_relay: true,
        };
        assert_eq!(
            relaunch_args(Some("brpticket"), &args),
            os(&["join", "brpticket", "--nickname", "alice", "--fps", "30", "--no-relay"])
        );
        assert_eq!(
            relaunch_args(Some("brpticket"), &WindowArgs::default()),
            os(&["join", "brpticket"])
        );
    }

    #[test]
    fn from_the_start_screen_the_new_process_takes_no_arguments() {
        let args = WindowArgs {
            nickname: Some("alice".into()),
            fps: Some(30),
            no_relay: true,
        };
        assert!(relaunch_args(None, &args).is_empty());
    }

    #[test]
    fn the_relaunch_runs_the_exe_of_the_install_captured_at_startup() {
        let install = Install::at(PathBuf::from("/opt/brp/brp")).unwrap();
        let relaunch = Relaunch::new(&install, Some("t"), &WindowArgs::default());
        assert_eq!(relaunch.exe, PathBuf::from("/opt/brp/brp"));
        assert_eq!(relaunch.args, os(&["join", "t"]));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p brp relaunch`
Expected: compile errors.

- [ ] **Step 3: Implement above the tests**

```rust
/// The process to start once this one has left the room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relaunch {
    pub exe: PathBuf,
    pub args: Vec<OsString>,
}

impl Relaunch {
    /// `install.exe` is the path captured at startup, which the swap has just repointed at the
    /// new binary; `current_exe()` here would name the renamed `.old` file on Linux.
    pub fn new(install: &Install, ticket: Option<&str>, args: &WindowArgs) -> Self {
        Self {
            exe: install.exe.clone(),
            args: relaunch_args(ticket, args),
        }
    }

    /// Starts the new process and returns at once. A failure can only be logged: the window is
    /// gone, and the update is already applied, so the next manual launch runs the new version.
    pub fn spawn(&self) {
        match Command::new(&self.exe).args(&self.args).spawn() {
            Ok(child) => {
                tracing::info!(pid = child.id(), exe = %self.exe.display(), "relaunched after the update");
            }
            Err(error) => {
                tracing::error!(%error, exe = %self.exe.display(), "could not relaunch after the update");
                eprintln!("error: could not relaunch {}: {error}", self.exe.display());
            }
        }
    }
}

/// `join <ticket>` with this launch's flags when a room was open. A bare `brp` takes no flags,
/// so the start screen relaunches without any.
pub fn relaunch_args(ticket: Option<&str>, args: &WindowArgs) -> Vec<OsString> {
    let Some(ticket) = ticket else {
        return Vec::new();
    };
    let mut out = vec![OsString::from("join"), OsString::from(ticket)];
    if let Some(nickname) = &args.nickname {
        out.push("--nickname".into());
        out.push(nickname.into());
    }
    if let Some(fps) = args.fps {
        out.push("--fps".into());
        out.push(fps.to_string().into());
    }
    if args.no_relay {
        out.push("--no-relay".into());
    }
    out
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p brp relaunch`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/src/relaunch.rs crates/app/src/lib.rs
git commit -m "feat(app): build the relaunch command for after an update"
```

---

### Task 10: Wire the window, the start screen, the status bar, and the launch

**Files:**
- Modify: `crates/app/src/ui/start.rs`
- Modify: `crates/app/src/ui/status.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/window.rs`
- Modify: `crates/app/src/participant.rs`

**Interfaces:**
- Consumes: everything from Tasks 3, 6, 8, 9.
- Produces: `AppEvent::{UpdateAvailable(Release), UpdateProgress { received: u64, total: Option<u64> }, UpdateReady(Result<(), String>)}`, `App::new(.., install: Option<Install>)`, `Shutdown.relaunch: Option<Relaunch>`, `UiOutput.update_clicked: bool`, `StartAction::Update`.

- [ ] **Step 1: Write the failing start-screen test**

In `crates/app/src/ui/start.rs` `mod tests`, extend `open_settings_is_never_an_intent_and_leaves_the_form_alone`:

```rust
    #[test]
    fn open_settings_and_update_are_never_intents_and_leave_the_form_alone() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::OpenSettings), None);
        assert_eq!(state.submit(StartAction::Update), None);
        assert!(!state.connecting);
    }
```

(Rename the existing test to this name and body.)

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p brp start`
Expected: compile error, no variant `Update`.

- [ ] **Step 3: Start screen**

In `crates/app/src/ui/start.rs`:

```rust
use super::update::{self, UpdateState};
```

`StartAction` gains `Update,` after `OpenSettings,`. In `submit`, replace the first guard and the unreachable arm:

```rust
        if matches!(action, StartAction::OpenSettings | StartAction::Update) {
            return None;
        }
        ...
            // Returned above before this match is reached.
            StartAction::OpenSettings | StartAction::Update => unreachable!(),
```

`draw` gains a parameter and draws the widget under the heading:

```rust
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
    update_state: &UpdateState,
) -> Option<StartAction> {
    ...
            ui.heading("brp");
            ui.add_space(8.0);
            if update::draw(ui, update_state) {
                action = Some(StartAction::Update);
            }
            ui.add_space(16.0);
```

(The existing `ui.add_space(16.0)` after the heading becomes the second `add_space`; the widget draws nothing when no release is known, so the layout of a quiet start screen is unchanged apart from eight points.)

- [ ] **Step 4: Status bar and `ui::draw`**

`crates/app/src/ui/status.rs`: import `use super::update::{self, UpdateState};`, extend the doc comment with "the Update button sets `update_clicked`", and add two parameters after `open_settings: &mut bool`:

```rust
    update_state: &UpdateState,
    update_clicked: &mut bool,
```

After the Settings button block and before the audio-output error:

```rust
            if update_state.available.is_some() {
                ui.separator();
                if update::draw(ui, update_state) {
                    *update_clicked = true;
                }
            }
```

`crates/app/src/ui/mod.rs`: `UiOutput` gains

```rust
    /// The status bar's Update button was clicked.
    pub update_clicked: bool,
```

`draw` gains `update_state: &update::UpdateState` as its last parameter, declares `let mut update_clicked = false;`, passes `update_state, &mut update_clicked` to `status::draw`, and sets `update_clicked` in the returned `UiOutput`.

- [ ] **Step 5: The window**

In `crates/app/src/window.rs`:

Imports:

```rust
use brp_update::{Install, Release};
use crate::relaunch::Relaunch;
use crate::ui::update::{PROGRESS_STEP_BYTES, UpdateState};
```

`AppEvent` gains, after `RoomOpened`:

```rust
    /// The launch check found a newer release.
    UpdateAvailable(Release),
    /// Bytes of the release archive received so far, throttled by the download task.
    UpdateProgress { received: u64, total: Option<u64> },
    /// The download and swap finished: relaunch, or the error to show.
    UpdateReady(Result<(), String>),
```

`Shutdown` gains:

```rust
    /// The updated binary to start once the room has been left.
    pub relaunch: Option<Relaunch>,
```

`App` gains, after `pending_intent`:

```rust
    /// Where this binary runs from, captured before any swap; `None` when it could not be
    /// determined, in which case updates are noticed but never applied.
    install: Option<Install>,
    update: UpdateState,
    relaunch: Option<Relaunch>,
```

`App::new` takes `install: Option<Install>` as its last parameter and initialises:

```rust
            update: UpdateState::new(install.as_ref().is_some_and(Install::is_release_layout)),
            install,
            relaunch: None,
```

`finish` adds `relaunch: self.relaunch,` to the `Shutdown`.

New method after `open`:

```rust
    /// Downloads and swaps in the release the check found, reporting progress and the outcome
    /// through events. The relaunch arguments are decided when the outcome arrives, not now, so a
    /// room opened during the download is the one rejoined.
    fn start_update(&mut self) {
        let Some(release) = self.update.start() else {
            return;
        };
        let Some(install) = self.install.clone() else {
            return;
        };
        let progress_events = self.proxy.clone();
        let done = self.proxy.clone();
        self.runtime.spawn(async move {
            let mut last_reported = 0u64;
            let progress = move |received: u64, total: Option<u64>| {
                let step = received - last_reported >= PROGRESS_STEP_BYTES;
                if step || Some(received) == total {
                    last_reported = received;
                    let _ = progress_events.send_event(AppEvent::UpdateProgress { received, total });
                }
            };
            let outcome = async {
                let staged = brp_update::download(&release, &install, progress).await?;
                brp_update::apply(staged, &install)
            }
            .await
            .map_err(|error| error.to_string());
            let _ = done.send_event(AppEvent::UpdateReady(outcome));
        });
    }
```

In `redraw_main`: pass `&self.update` to `start::draw(root, &mut self.start, &self.store.settings.recent_rooms, now, &self.update)` and to `ui::draw(root, &view.snapshot, &view.ticket, &mut self.state, &popped, &self.update)`. After the block that handles `start_action` and `self.open(intent)`, add:

```rust
        if start_action == Some(StartAction::Update) || output.update_clicked {
            self.start_update();
        }
```

`user_event` renames its `_` parameter to `event_loop` and gains arms before the catch-all:

```rust
            AppEvent::UpdateAvailable(release) => {
                self.update.available = Some(release);
            }
            AppEvent::UpdateProgress { received, total } => {
                self.update.progress(received, total);
            }
            AppEvent::UpdateReady(Ok(())) => {
                if let Some(install) = &self.install {
                    let ticket = match &self.phase {
                        Phase::Room(view) => Some(view.ticket.as_str()),
                        Phase::Start => None,
                    };
                    self.relaunch = Some(Relaunch::new(install, ticket, &self.args));
                }
                event_loop.exit();
            }
            AppEvent::UpdateReady(Err(message)) => {
                self.update.failed(message);
            }
```

- [ ] **Step 6: The launch and the relaunch**

In `crates/app/src/participant.rs`:

```rust
use std::str::FromStr;

use brp_update::{Install, Version};
```

After `let nickname = launch::default_nickname(&launch, &secret);`:

```rust
    let install = match Install::current() {
        Ok(install) => {
            brp_update::cleanup_stale(&install);
            Some(install)
        }
        Err(error) => {
            tracing::warn!(%error, "install path unknown; updates can be noticed but not applied");
            None
        }
    };
```

After the ticker is spawned and before `App::new`:

```rust
    if store.settings.check_updates {
        spawn_update_check(runtime, proxy.clone());
    }
```

Pass `install` as the last argument of `App::new`. After the `for room in rooms { .. }` loop and before `outcome`:

```rust
    if let Some(relaunch) = shutdown.relaunch {
        relaunch.spawn();
    }
```

New function at the bottom of the file, before any tests:

```rust
/// Asks GitHub once whether a newer release exists. Offline is normal, so a failure is a log
/// line and nothing in the window.
fn spawn_update_check(runtime: &Runtime, proxy: EventLoopProxy<AppEvent>) {
    let current = match Version::from_str(env!("CARGO_PKG_VERSION")) {
        Ok(current) => current,
        Err(error) => {
            tracing::warn!(%error, "not checking for updates: this build's version is not a release version");
            return;
        }
    };
    runtime.spawn(async move {
        match brp_update::check(current).await {
            Ok(Some(release)) => {
                let _ = proxy.send_event(AppEvent::UpdateAvailable(release));
            }
            Ok(None) => tracing::debug!("this is the latest release"),
            Err(error) => tracing::warn!(%error, "update check failed"),
        }
    });
}
```

Add `use winit::event_loop::EventLoopProxy;` to the imports.

- [ ] **Step 7: Build, test, lint**

Run: `cargo test --workspace`
Expected: all pass, including the renamed start-screen test.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean. If clippy asks for `too_many_arguments` on `status::draw` (it now has eight), add `#[allow(clippy::too_many_arguments)]` on that function only, with the comment `// One parameter per thing the bar shows; a struct would be built and unpacked in one place.`

- [ ] **Step 8: Manual smoke run on the dev machine**

```bash
cargo run --release -p brp 2>&1 | head -5
```

Expected: the window opens; within a few seconds the start screen shows `v<latest> is available at https://github.com/gtkacz/openstream/releases` when the workspace version is older than the latest release, or nothing when it is equal. `~/.config/brp/brp.log` contains either `this is the latest release` or `update check failed` at warn when offline. No button appears, since `target/release` has no `FFMPEG-LICENSE.txt`. Then confirm the button path renders: copy `FFMPEG-LICENSE.txt` from a release tarball into `target/release/`, run again, and confirm `Update and restart` appears; do **not** click it against `target/release` unless you want the release files there. Remove the marker afterwards. Open Settings and confirm the `Updates` row with its checkbox; untick, Save, relaunch, and confirm the log has no update line.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git status --short
git add crates/app/src/ui/start.rs crates/app/src/ui/status.rs crates/app/src/ui/mod.rs crates/app/src/window.rs crates/app/src/participant.rs
git commit -m "feat(app): notice new releases and update in place with a relaunch"
```

---

### Task 11: README and spec amendments

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-10-phase8-self-update-design.md`

- [ ] **Step 1: README, Usage**

After the paragraph that ends "Command line flags override the saved settings for that launch and are not saved.", add:

```markdown
At launch brp asks GitHub for the latest release and, when it is newer, shows
"vX.Y.Z is available" on the start screen and in the status bar with an
"Update and restart" button. The button downloads the release for this
platform, checks it against the release's `SHA256SUMS`, replaces every file
of the directory brp runs from, and relaunches: from a room, straight back into
that room; from the start screen, to the start screen. The check can be turned
off in Settings and applies from the next launch. A build from source sees the
notice with a link and no button, since only an extracted release directory is
replaced.
```

- [ ] **Step 2: README, Linux and Windows sections**

In the Linux section, after "The binary finds them beside itself wherever the extracted directory sits, so no FFmpeg needs to be installed.", add the sentence: "An in-app update replaces every file of that directory in place, keeping the previous ones as `.old` until the next launch removes them."

In the Windows section, after "extract, and run `brp.exe` from that directory.", add: "An in-app update replaces the files of that directory in place; the running `brp.exe` and its DLLs are renamed to `.old` and removed at the launch after the next, since Windows will not delete a file a process still maps."

- [ ] **Step 3: README, Identity and privacy**

Extend the settings list in "Settings are saved beside the identity key in `brp/settings.toml`: nickname, relay choice, frame rate ceiling, audio output device, and the tickets of recent rooms." to end "…, the tickets of recent rooms, and whether to check for updates." Then add after that paragraph:

```markdown
The launch update check is one HTTPS request to github.com carrying only a
`brp/<version>` user agent; it tells GitHub that a brp of that version started
from your address, and the Settings checkbox turns it off. When an update
relaunches brp into a room, the ticket is passed on the new process's command
line, exactly as `brp join <ticket>` does, and command lines are readable by
other users of the same machine on most systems.
```

- [ ] **Step 4: README, crate table, Development, Roadmap**

Add a row before the `brp` row:

```markdown
| `brp-update` | The release check against GitHub, the verified download, archive extraction, and the in-place swap with rollback |
```

In Development's test listing add `cargo test -p brp-update            # versions, checksums, archives, the swap and its rollback`.

Append a roadmap entry after the last numbered item present when this task runs (item 6 at the time of writing; use the next number, and renumber nothing):

```markdown
7. **Self-update** — done pending the first release pair: the launch check,
   the in-app update with checksum verification and rollback, and the relaunch
   into the same room.
```

- [ ] **Step 5: Spec amendments**

Append to the spec:

```markdown
## 15. Amendments from the implementation run

- **Repository name and redirect handling (3, 5.1, 12).** The repository is now `gtkacz/openstream`; `RELEASES_URL` names it. `check` is a HEAD request that follows redirects and reads the final URL, because the old name redirects to the new one before the latest-release page redirects to the tag, and a future rename would add another hop. `UpdateError::NoRedirect` does not exist: a bad final status is `Http`, a final URL without a tag is `Version`.
- **Archive crates on both platforms (5.1).** `tar`, `flate2`, and `zip` are unconditional so the Windows extractor is tested by the Linux suite; `extract` picks the format by `cfg`. Zip entry names are normalised from backslashes before sanitisation.
- **Constants (12).** `ASSET_SUFFIX` is `PLATFORM` plus `ARCHIVE_EXTENSION`, since the top-level directory `brp-<version>-<PLATFORM>` is needed separately. `UPDATE_CHECK_TIMEOUT` is also the connect and read timeout of the download.
- **Progress throttling (5.4).** `ui::update::PROGRESS_STEP_BYTES` (1 MB) bounds `UpdateProgress` events; `BYTES_PER_MB` is the display unit.
- **Relaunch (5.4).** `Relaunch` lives in `app::relaunch` and takes the ticket as `Option<&str>`, so `window::Phase` stays private and the argument builder is a pure function.
- **Settings checkbox copy (6).** `Check for updates at launch (from the next launch)`.
```

- [ ] **Step 6: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-10-phase8-self-update-design.md
git commit -m "docs: describe the in-app update and record the phase 8 amendments"
```

---

## Self-review against the spec

- **Spec 5.1 `brp-update` API** → Tasks 1 to 6. `check`, `Install::current/is_release_layout`, `download`, `apply`, `cleanup_stale`, `newer_release`, `checksum_for`, `extract`, `asset_name` all present; `NoRedirect` dropped and recorded.
- **5.2 settings** → Task 7. **5.3 `ui::update`** → Task 8. **5.4 window and participant** → Tasks 9 and 10, including capturing `Install` before the loop, `cleanup_stale`, the check task, relaunch after the leave, and relaunch arguments computed at `UpdateReady`.
- **6 UI** → Tasks 8 and 10: start screen under the heading, status bar after Settings, progress replaces the button, error in red, button back after failure, notice with URL when `can_apply` is false.
- **7 data flow** → Task 10. **8 errors** → `UpdateError` in Task 1; silent failed check, red message, rollback wording, spawn failure logged: Tasks 5, 6, 9, 10.
- **9 security** → sanitisation in Task 4, checksum in Task 6, ticket-on-argv sentence in Task 11.
- **11 testing** → every listed unit test has a task: Version and ordering (1), `newer_release` and `checksum_for` (2), extraction with traversal, absolute, nested, wrong-top entries (4), apply with `.old`, stale `.old`, rollback (5), cleanup and layout marker (3), `UpdateState` (8), `relaunch_args` (9), settings default and round trip (7). The spec's "a staged name that is a directory in the install" failure injection is replaced by a listed-but-unstaged name, which reliably fails the second rename on both platforms.
- **13 documentation** → Task 11.
- **Type consistency.** `Version(u64, u64, u64)`, `Release { version, tag }`, `Install { dir, exe }`, `Staged { dir, files: Vec<OsString> }`, `UpdateState::{new, start, progress, failed}`, `Relaunch::{new, spawn}`, `relaunch_args(Option<&str>, &WindowArgs)`, `AppEvent::UpdateProgress { received, total }` are spelled the same in every task that names them.
