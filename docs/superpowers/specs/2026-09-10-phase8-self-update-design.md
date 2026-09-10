# Phase 8: Self-update

Status: approved design, 2026-09-10. Adds a phase to `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies. Builds on the release workflow of `2026-09-06-phase5-windows-settings-release-design.md` and is compatible with the `join` entry point as phase 7 (`2026-09-10-phase7-join-links-design.md`) redefines it.

## 1. Goals

- A participant learns inside the window that a newer release exists, on the start screen and in a room.
- One click downloads the release, verifies it, replaces the installed files, and relaunches brp. A participant who was in a room is back in the same room; one who was on the start screen is back on the start screen.
- The installed directory keeps its path. Shortcuts, `PATH` entries, and the `brp://` handler registration of phase 7 keep pointing at a binary that is now the new version.
- The check is on by default and can be turned off in Settings, because it is a network request to GitHub on every launch.
- Works from the Linux tarball and the Windows zip as shipped, with no installer, and needs no change to the release workflow.
- Everything above the network and the file system is unit-tested without a display or a connection.

## 2. Non-goals

- Signed releases. The download is verified against the release's `SHA256SUMS`, which protects against a corrupt or truncated download and not against a compromised GitHub account. Signing would need a key stored as a repository secret that must never be lost; it can be added later without changing the flow here.
- Downloading or installing without a click. The check is automatic; the update is not.
- Delta updates, update channels, rollback to a previous version from the UI, or showing release notes.
- Updating the headless `publish` command. It has no window to show a notice in.
- A hook to point the updater at another URL for testing. The first end-to-end run happens against the release after this one; see section 10.
- Runtime verification on Windows hardware, deferred as in phases 3 to 7.
- macOS, which has no release artefact yet.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| The check follows `releases/latest` and reads the tag from the redirect, rather than calling the GitHub API | The redirect is one request with no rate limit, no JSON, and no token. The API's unauthenticated limit of sixty requests an hour per address is shared by everyone behind a NAT, and `serde_json` is not in the dependency tree. The tag is all the check needs. |
| Only a strictly newer version is a notice | The `chore(release)` commit leaves `main` at the released version, so a build from `main` equals the latest release and stays quiet. |
| The whole release directory is replaced, not only the binary | The binary loads the four FFmpeg libraries beside itself through an `$ORIGIN` rpath on Linux and the DLL search path on Windows. Their sonames change when the pinned FFmpeg build changes, so a binary alone can be unloadable. |
| Files are swapped in place by renaming the current file to `<name>.old` and moving the new file in | Windows forbids deleting or overwriting a running executable and loaded DLLs, but allows renaming them. On Linux the running process keeps its mapped inodes whatever happens to the names. Renaming inside one directory is atomic on both platforms, and the install keeps its path. |
| The download is staged inside the install directory | The final step is a rename, which is only atomic within one file system. A temp directory elsewhere could be on another mount. |
| Verification is against the release's `SHA256SUMS` | It exists already, costs nothing in CI, and turns a truncated or corrupted download into an error instead of a broken install. |
| `.old` files are deleted at the next launch, best effort | On Windows the old process may still be exiting when the new one starts, so a deletion can fail with a sharing violation. The launch after that succeeds. Nothing is worse than a stray file for a while. |
| Whether the Update button is offered is decided by the presence of `FFMPEG-LICENSE.txt` beside the exe | The staging scripts write it and no `cargo build` does, so it marks a release layout without listing library sonames that change with the pinned FFmpeg build. A dev build sees the notice and the releases URL, never a button that would drop release files into `target/`. |
| The relaunch goes through `brp join <ticket>` with this launch's flags | It is the documented entry point, phase 7 keeps it accepting a raw ticket, and it needs no new command. Flags belong to `create` and `join`, so a relaunch from the start screen is a bare `brp`. |
| The relaunch is spawned after the orderly leave, by the same code path that leaves | The room sees one leave and one join. Spawning before the leave would put two identities from one machine in the room for a moment, and a spawn failure after the window is gone can only be logged anyway. |
| A failed check is silent in the UI | Being offline is normal and the check is a convenience. It is logged at warn for the diagnostic log. |
| The relaunch arguments are computed when the download completes, not when the button is clicked | A room opened or closed during the download must be reflected. The download does not block the rest of the window. |
| One new crate, `brp-update`, with no winit or egui | The network, archive, and file-swap code has nothing to do with the window and is the part that most needs testing in isolation. The app crate keeps the state machine, the widget, and the relaunch. |

## 4. Product model additions

- **Release.** A version `X.Y.Z` with its tag `vX.Y.Z`, learned from the redirect of the latest-release page.
- **Install.** The directory the running exe was started from, captured once at startup, and whether it is a release layout.
- **Update notice.** "vX.Y.Z is available", shown on the start screen under the heading and in the status bar beside Settings.
- **Update button.** "Update and restart", shown with the notice when the install is a release layout. Replaced by a progress line while downloading and joined by the error in red after a failure.
- **Check for updates at launch.** A boolean setting, on by default, with a checkbox in the Settings dialog.

## 5. Architecture

### 5.1 `brp-update` (new crate, `crates/update`)

Dependencies: `reqwest` (already in the tree through iroh, with its `rustls-no-provider` and `stream` features; iroh supplies the ring provider), `sha2` (in the tree), `tokio`, `thiserror`, `tracing`; on Unix `tar` and `flate2`; on Windows `zip` with default features off and `deflate` on. The three archive crates are the only additions to the lock file.

```
/// A plain `X.Y.Z`, which is the only shape the release workflow produces.
pub struct Version(pub u64, pub u64, pub u64);   // FromStr, Ord, Display
pub struct Release { pub version: Version, pub tag: String }

/// The directory the running binary lives in, taken from `current_exe` once at startup.
pub struct Install { pub dir: PathBuf, pub exe: PathBuf }
impl Install {
    pub fn current() -> Result<Self, UpdateError>;
    /// `FFMPEG-LICENSE.txt` sits beside the exe: the staging scripts wrote this directory.
    pub fn is_release_layout(&self) -> bool;
}

/// Asks GitHub for the latest release; `Some` only when it is newer than `current`.
pub async fn check(current: Version) -> Result<Option<Release>, UpdateError>;

/// The extracted release, ready to swap in.
pub struct Staged { dir: PathBuf, files: Vec<OsString> }

/// Downloads the platform archive and SHA256SUMS into a staging directory inside `install.dir`,
/// verifies, and extracts. `progress` sees bytes received and the total when known.
pub async fn download(
    release: &Release,
    install: &Install,
    progress: impl FnMut(u64, Option<u64>) + Send,
) -> Result<Staged, UpdateError>;

/// Renames each current file to `<name>.old` and moves the staged file in. On failure the
/// completed renames are reversed and the staging directory removed.
pub fn apply(staged: Staged, install: &Install) -> Result<(), UpdateError>;

/// Deletes `*.old` and leftover staging directories beside the exe. Failures are logged.
pub fn cleanup_stale(install: &Install);
```

Pure functions the two async entry points are built from, and which the tests exercise:

- `newer_release(current: Version, location: &str) -> Result<Option<Release>, UpdateError>`: the last path segment of the redirect target must be `vX.Y.Z`.
- `checksum_for(sums: &str, asset: &str) -> Result<[u8; 32], UpdateError>`: the `<hex>  <name>` lines of `SHA256SUMS`.
- `extract(archive: &Path, top_level: &str, into: &Path) -> Result<Vec<OsString>, UpdateError>`: every entry must be a regular file whose path is exactly `<top_level>/<name>`; directory entries are skipped; anything else (nested paths, `..`, absolute paths, symlinks, hard links) is `UpdateError::Archive`. Unix modes are preserved so `brp` stays executable.
- `asset_name(version) -> String`: `brp-{version}-{ASSET_SUFFIX}`.

`check` builds a client with redirects disabled, GETs `{RELEASES_URL}/latest` with `UPDATE_CHECK_TIMEOUT`, and hands the `Location` header to `newer_release`. A non-redirect status or a missing header is `UpdateError::NoRedirect`.

`download` builds a client that follows redirects (release assets redirect to GitHub's object store), creates `{install.dir}/{STAGING_PREFIX}{version}` (a failure here is `UpdateError::NotWritable`), fetches `SHA256SUMS`, then streams the archive to disk with `Response::chunk`, hashing as it writes and reporting progress from `Content-Length`. A digest mismatch is `UpdateError::Checksum` and removes the staging directory. Extraction then runs in `spawn_blocking`.

`apply` first removes any `<name>.old` left from an earlier update, then renames in archive order, recording each; a failure reverses the recorded renames and removes the staging directory, and the error says whether the rollback succeeded. The staging directory is removed on success.

### 5.2 `app::settings`

`Settings` gains `check_updates: bool`, default `true`, under `#[serde(default)]` like every other field, so an older file reads with the check on. Nothing else in the file changes.

### 5.3 `app::ui::update` (new module)

```
pub enum UpdatePhase { Idle, Downloading { received: u64, total: Option<u64> }, Failed(String) }

pub struct UpdateState {
    pub available: Option<Release>,
    /// The install is a release layout, so the button is offered.
    pub can_apply: bool,
    pub phase: UpdatePhase,
}
impl UpdateState {
    /// The button was clicked: the release to download, or `None` while one is in flight or when
    /// nothing can be applied.
    pub fn start(&mut self) -> Option<Release>;
    pub fn progress(&mut self, received: u64, total: Option<u64>);
    pub fn failed(&mut self, message: String);
}

/// The notice, the button or the progress line, and the error. Returns `true` when clicked.
/// Draws nothing when no release is available.
pub fn draw(ui: &mut egui::Ui, state: &UpdateState) -> bool;
```

Lives in `ui/` because it is drawn, but it is separate from `UiState`, which `RoomOpened` resets: an update in flight must survive a room opening.

### 5.4 `app::window` and `app::participant`

- `AppEvent` gains `UpdateAvailable(Release)`, `UpdateProgress { received, total }`, and `UpdateReady(Result<(), String>)`.
- `App` gains `install: Option<Install>`, `update: UpdateState`, and `relaunch: Option<Relaunch>`. `Shutdown` gains `relaunch`.
- `start::draw` and `status::draw` each call `ui::update::draw`; a click reaches `App::start_update`, which takes the release from `UpdateState::start`, and spawns `download` with a progress closure that sends `UpdateProgress`, then `apply`, then `UpdateReady`.
- `UpdateReady(Ok(()))` sets `relaunch = Some(relaunch_for(&self.phase, &self.args, &install))` and calls `event_loop.exit()`. `UpdateReady(Err(message))` calls `update.failed(message)`.
- `participant::run` captures `Install::current()` before the event loop is built (after the swap, `/proc/self/exe` resolves to the renamed `.old` inode, so the path must be taken early), calls `cleanup_stale`, and when `settings.check_updates` spawns `check` with `env!("CARGO_PKG_VERSION")`, sending `UpdateAvailable` on `Some` and logging at warn on error. After the orderly leave it spawns the relaunch if there is one.

```
pub struct Relaunch { pub exe: PathBuf, pub args: Vec<OsString> }

/// In a room: `join <ticket>` with this launch's `--nickname`, `--fps`, and `--no-relay` when
/// present. On the start screen: no arguments.
pub fn relaunch_for(phase: &Phase, args: &WindowArgs, install: &Install) -> Relaunch;
```

The relaunch uses `std::process::Command` with `install.exe`; a spawn failure is logged at error with the path and printed to stderr, which on Windows reaches the parent console when there is one.

## 6. User interface

- **Start screen.** Under the "brp" heading: `v0.5.0 is available` in weak text with the `Update and restart` button beside it. While downloading, the button is replaced by `downloading 12 MB / 41 MB` (or `downloading 12 MB` without a total). After a failure the error is shown in red under the notice and the button returns. Without a release layout the notice reads `v0.5.0 is available at github.com/gtkacz/brp_sharing/releases` and there is no button.
- **Status bar.** The same notice and button after the Settings button, so the room panels are untouched.
- **Settings dialog.** A `Check for updates at launch` checkbox. Unlike the other settings it applies at the next launch, not the next room, and its label says so.
- Create, Join, sharing, and watching stay enabled during a download. The button ignores clicks while a download is in flight.

## 7. Data flow

1. Launch: `Install::current()`, `cleanup_stale`, then `check` on the runtime if enabled.
2. `check` → `UpdateAvailable(release)` → `update.available = Some(release)`, `update.can_apply = install.is_release_layout()` → the notice appears on the next redraw.
3. Click → `UpdateState::start` → task: `download` (progress events) → `apply` → `UpdateReady`.
4. `UpdateReady(Ok)` → relaunch arguments from the current phase → event loop exits → `participant::run` leaves the room, aborts the tasks, spawns `install.exe` with the arguments → process exits.
5. The new process starts, `cleanup_stale` deletes the `.old` files it can, and `join <ticket>` opens the room.

## 8. Error handling

One `UpdateError` (`thiserror`): `Http`, `NoRedirect`, `Version`, `Checksum`, `Archive`, `Io`, `NotWritable`, `Apply { rolled_back: bool }`. `AppError` does not gain a variant: the updater never fails a launch, and its errors reach the UI as strings the way share and open errors do.

- A failed check: `tracing::warn!`, nothing in the UI.
- A failed download or apply: the message in red beside the notice, the button back for a retry. A read-only install directory fails at staging with a message naming the directory.
- An apply that fails mid-way: the message says the install was restored, or, when the rollback also failed, names the directory and the releases URL so the user can repair by hand.
- A relaunch spawn failure: logged at error with the exe path. The update is already applied, so the next manual launch runs the new version.
- `cleanup_stale` failures: logged at debug. Expected on Windows for one launch.

## 9. Security

- Transport is HTTPS to `github.com` and its object store through the rustls stack already used for relays. No token, no cookie; the user agent is `brp/<version>`.
- The checksum guards integrity of the download, not authenticity of the release. Recorded as a non-goal above.
- Archive entries are rejected unless they are regular files directly under the expected top-level directory, so a crafted archive cannot write outside the staging directory.
- The relaunch passes the ticket as a process argument, exactly as `brp join` does today, and `/proc/<pid>/cmdline` is readable by other local users on most Linux systems. The README's ticket-sensitivity paragraph gains a sentence saying so.
- The check tells GitHub that some brp of some version started, from the user's address. This is why the setting exists and why the README documents the check.

## 10. Known limitations

- The first end-to-end run of the updater can only happen once two releases carry it: the release built from this phase installs by hand, and the release after it is the first one it can fetch. Until then the flow is exercised by unit tests and by reading.
- On Windows, `.old` files may survive one launch; they are gone the launch after.
- A file in the old release that the new release no longer ships stays in the directory untouched. Harmless, since nothing loads it.
- A relaunch from the start screen drops the launch's flags, because a bare `brp` takes none.
- A dev build at a version older than the latest release sees the notice with the URL and no button.
- Two updates in a row while the process from the first is still exiting on Windows fail at removing the earlier `.old`, with the error shown; retrying a moment later succeeds.

## 11. Testing

Unit, in the hardware-free suite, with no network:

- `Version`: parses `0.4.0` and `12.3.45`, rejects `v0.4.0`, `0.4`, `0.4.0-rc1`, and empty; orders `0.4.0 < 0.10.0 < 1.0.0`.
- `newer_release`: a `Location` ending in `/releases/tag/v0.5.0` against current `0.4.0` is `Some`; the same tag against `0.5.0` and `0.6.0` is `None`; a location without a tag segment is `UpdateError::Version`.
- `checksum_for`: finds the line for the asset among others, rejects a missing asset and a malformed digest.
- `extract`: a tar.gz and a zip built in a temp directory with `<top>/brp`, `<top>/lib.so`, and a directory entry extract those two files with modes kept; an entry `<top>/../escape`, an absolute path, a nested `<top>/sub/file`, or a wrong top level is `UpdateError::Archive` and writes nothing.
- `apply`: with a populated install directory and a staged set, every staged name is swapped in and every previous file is `<name>.old` with the old bytes; a stale `<name>.old` is removed first; a rename made to fail (a staged name that is a directory in the install) reverses the completed renames and reports `rolled_back: true`.
- `cleanup_stale`: removes `*.old` and `.brp-update-*` directories, leaves everything else.
- `Install::is_release_layout`: true with `FFMPEG-LICENSE.txt` beside the exe path, false without.
- `UpdateState`: `start` returns the release once and `None` while downloading; `failed` returns to a state where `start` works again; `start` is `None` when `can_apply` is false.
- `relaunch_for`: in a room yields `join <ticket>` plus each present flag and nothing for absent ones; on the start screen yields no arguments; the exe is `install.exe`.
- `Settings`: a file without `check_updates` loads with it `true`; a round trip keeps `false`.

Not unit-tested: `check` and `download` against GitHub, the spawn, and the running-exe rename on Windows. Manual: after the next release, launch the installed previous release, see the notice, click, watch the progress, and confirm the new window rejoins the room the old one was in, with `.old` files gone after the following launch.

## 12. Constants added in this phase

In `brp-update`:

| Constant | Value | Rationale |
|---|---|---|
| `RELEASES_URL` | `https://github.com/gtkacz/brp_sharing/releases` | GitHub redirects a renamed repository's URLs, so a rename does not strand installed versions. |
| `ASSET_SUFFIX` | `linux-x86_64.tar.gz` / `windows-x86_64.zip` per target | Matches the release workflow's asset names. |
| `CHECKSUMS_FILE` | `SHA256SUMS` | The file the publish job writes. |
| `RELEASE_MARKER` | `FFMPEG-LICENSE.txt` | Written by both staging scripts, absent from any `cargo build`. |
| `STAGING_PREFIX` | `.brp-update-` | Hidden on Linux, recognisable for cleanup. |
| `OLD_SUFFIX` | `.old` | The renamed previous files. |
| `UPDATE_CHECK_TIMEOUT` | 10 s | One redirect; longer means the network is unusable and the check should stop bothering. |
| `USER_AGENT` | `brp/<CARGO_PKG_VERSION>` | GitHub requires a user agent; the version is the only detail sent. |

## 13. Documentation

README: the Usage section describes the notice, the button, the rejoin, and the Settings checkbox; the Identity and privacy section says that the launch check contacts GitHub, how to turn it off, and that a relaunch passes the ticket on the command line; the Linux and Windows sections say an update replaces every file of the extracted directory in place. The phase 5 release paragraph is unchanged.

## 14. References

- GitHub: `https://github.com/<owner>/<repo>/releases/latest` answers 302 to `/releases/tag/<tag>`; release assets under `/releases/download/<tag>/<asset>` redirect to the object store.
- reqwest 0.13 `redirect::Policy::none()` and `Response::chunk` for streaming without the `stream` adapter.
- Windows `MoveFileExW` semantics, which Rust's `std::fs::rename` uses: an open executable or DLL can be renamed within its volume; it cannot be deleted or overwritten while mapped.
- `tar` and `flate2` for the Linux tarball; `zip` for the Windows archive.
