# Plan 5b: Settings, Persistence, and Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Nickname, relay choice, capture frame rate ceiling, audio output device, and recent rooms persist in one TOML file edited from an in-app Settings dialog, and a `v*` tag publishes a GitHub Release with a Windows zip and a Linux tarball.

**Architecture:** `brp-net` gains a custom relay URL and `brp-audio` gains output device listing and selection, both small and hardware-free to test. The app gains a `settings` module (pure data, TOML round trip, atomic save, recent-room upsert, validation) and a `SettingsStore` that owns the file path and the load error. `Launch` is built from settings then command line flags. A `SettingsDialog` edits a draft copy; the start screen lists recent rooms; the status bar and the start screen open the dialog. A composite action and a staging script make the Windows FFmpeg steps shared between CI and a new tag-triggered release workflow with a version guard.

**Tech Stack:** Rust 2024, serde 1.0 with derive, toml 1.1, directories 6, iroh 1.1 (`RelayUrl`, `RelayMode::Custom`, `RelayMap`), cpal 0.18.2 (`DeviceId`, `device_by_id`, `output_devices`), egui 0.36.1, GitHub Actions (`softprops/action-gh-release@v2`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`).

**Spec:** `docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md`, sections 5.4 to 5.7, 7, 8.2, 9.2, 10.2, 11, refining `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md` sections 5.7 and 8. Read both. Plan 5a (`docs/superpowers/plans/2026-09-06-plan5a-popouts.md`) must be complete first: this plan edits the `window.rs`, `ui/mod.rs`, and `ui/start.rs` that 5a leaves behind.

## Global Constraints

- **Start gate.** Plan 5a is merged and the user's click-through of spec section 10.1 reported no open bugs. Run `git status --short` before Task 1; if unrelated files are modified or staged, stop and ask. If told to proceed regardless, every commit uses explicit pathspecs: `git add <files> && git commit -m "..." -- <files>`.
- Command line flags override the file for one launch and are never written back. The file is written only after a successful room open (recent room, and the start-screen nickname when no `--nickname` flag was given) and when the dialog's Save succeeds.
- A corrupt settings file is never overwritten silently: load fails, defaults apply, the start screen shows the error, and only an explicit Save writes.
- The identity key file is untouched by this plan. Settings live in `settings.toml` beside it.
- Settings apply when the next room opens. Nothing in this plan changes an open room.
- No wire changes. Nothing under `crates/room`, `crates/pipeline`, `crates/proto`, `crates/codec`, `crates/capture` changes. `crates/net` changes only `src/endpoint.rs` and `tests/loopback.rs`; `crates/audio` changes only `src/cpal_output.rs` and `src/lib.rs`.
- Comments explain why. Doc comments state contracts on new public items. No task ids in code.
- One Conventional Commit per task, imperative subject, no co-author lines. The `Claude-Session:` trailer the harness requires is expected; no other trailers. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass before each commit. Before committing, run `git status --short`; if `.vscode/` files appear staged, run `git rm --cached -r .vscode` first (recurring environment quirk).
- Tests run with `cargo test --workspace`. Tests that touch the file system use a directory under `std::env::temp_dir()` named after the process id and the test; no `tempfile` crate.
- Verified library facts this plan relies on. iroh 1.1.0: `iroh::RelayUrl` implements `FromStr` (error `RelayUrlParseError`), `PartialEq`, `Eq`, `Clone`; `iroh::RelayMap: From<RelayUrl>`; `iroh::RelayMode::Custom(RelayMap)`; `Endpoint::builder(presets::N0).relay_mode(..)`. cpal 0.18.2: `cpal::DeviceId` implements `Display` and `FromStr`; `HostTrait::device_by_id(&DeviceId) -> Option<Device>`; `HostTrait::output_devices() -> Result<impl Iterator<Item = Device>, _>`; `DeviceTrait::id() -> Result<DeviceId, _>`; `DeviceTrait::description() -> Result<DeviceDescription, _>` with `DeviceDescription::name() -> &str`. toml 1.1.5: `toml::from_str`, `toml::to_string_pretty`, both returning `toml::de::Error` / `toml::ser::Error` that implement `Display`; serde adjacently tagged enums (`#[serde(tag = "mode", content = "url")]`) serialize unit variants as a table with only the tag. egui 0.36.1: `egui::Window::new(..).open(&mut bool).collapsible(false).resizable(false).show(ctx, ..)`, `egui::Grid::new(..).num_columns(2)`, `egui::DragValue::new(&mut u32).range(..)`, `egui::ComboBox::from_id_salt(..)`, `ui.radio_value`, `ui.selectable_value`. GitHub Actions: composite actions write `$GITHUB_ENV` and `$GITHUB_PATH` like steps do; `actions/download-artifact@v4` with `merge-multiple: true` flattens several artifacts into one directory; `softprops/action-gh-release@v2` takes `files:` (glob) and `body_path:`; `jq` is present on `ubuntu-latest`.

## File Structure

```
Cargo.toml                                   + toml workspace dependency
crates/app/Cargo.toml                        + serde, toml

crates/net/src/endpoint.rs                   RelaySetting::Custom(RelayUrl); bind maps it; loses Copy
crates/net/tests/loopback.rs                 + bind with a custom relay URL

crates/audio/src/cpal_output.rs              CpalOutput::new(Option<String>), output_devices, parse_device_id
crates/audio/src/lib.rs                      re-export OutputDevice, output_devices

crates/app/src/error.rs                      + AppError::Settings(String)
crates/app/src/settings.rs                   new: Settings, RelayChoice, AudioSettings, RecentRoom, SettingsStore, constants
crates/app/src/lib.rs                        + pub mod settings
crates/app/src/cli.rs                        WindowArgs.fps becomes Option<u32>
crates/app/src/launch.rs                     Launch::from_settings; audio_output; nickname_from_flag
crates/app/src/publish.rs                    CpalOutput::new(None); relay clone
crates/app/src/participant.rs                loads the store, builds Launch from it
crates/app/src/ui/settings.rs                new: SettingsDialog and its draw (pure helpers tested)
crates/app/src/ui/start.rs                   Settings button, recent rooms, OpenSettings action, age and ticket helpers
crates/app/src/ui/status.rs                  Settings button
crates/app/src/ui/mod.rs                     UiOutput.open_settings; pub mod settings
crates/app/src/window.rs                     store, dialog, pending intent, save after open, apply Save

.github/actions/windows-ffmpeg/action.yml    new: composite LLVM + pinned FFmpeg install
ci/stage-windows.ps1                         new: exe + DLLs + licenses into a directory
.github/workflows/ci.yml                     Windows job uses both
.github/workflows/release.yml                new: version guard, linux tarball, windows zip, publish

README.md                                    settings, recent rooms privacy, releases, release procedure
docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md   + 5b amendments
```

---

### Task 1: Custom relay URL in `brp-net`

**Files:**
- Modify: `crates/net/src/endpoint.rs`
- Modify: `crates/net/tests/loopback.rs`
- Modify: `crates/app/src/launch.rs` (two lines)
- Modify: `crates/app/src/publish.rs` (two lines)

**Interfaces:**
- Consumes: `iroh::{RelayMap, RelayMode, RelayUrl}`, `iroh::endpoint::presets`.
- Produces: `RelaySetting::{Default, Custom(RelayUrl), Disabled}` deriving `Debug, Clone, PartialEq, Eq` (no longer `Copy`); `bind_endpoint(secret, relay: RelaySetting, known_peers)` unchanged in shape.

- [ ] **Step 1: Write the failing bind test**

Append to `crates/net/tests/loopback.rs` (it already imports `RelaySetting`, `bind_endpoint`, `SecretKey`, and uses `#[tokio::test]`):

```rust
#[tokio::test]
async fn an_endpoint_binds_with_a_custom_relay_url() {
    // Binding does not contact the relay, so an unreachable host is fine here.
    let url: iroh::RelayUrl = "https://relay.example.invalid/".parse().unwrap();
    let endpoint = bind_endpoint(
        SecretKey::generate(),
        RelaySetting::Custom(url),
        vec![],
    )
    .await
    .expect("bind with a custom relay map");
    endpoint.close().await;
}
```

Run: `cargo test -p brp-net an_endpoint_binds_with_a_custom_relay_url`
Expected: compile error, no variant `Custom`.

- [ ] **Step 2: Add the variant and the mapping**

In `crates/net/src/endpoint.rs`, replace the enum and the `builder` match:

```rust
use iroh::{Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey};

/// Controls which relay infrastructure the endpoint uses for hole punching and fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaySetting {
    /// The library's public relays.
    Default,
    /// One self-hosted relay instead of the public ones.
    Custom(RelayUrl),
    /// No relays: LAN and directly reachable peers only.
    Disabled,
}
```

```rust
    let builder = match relay {
        RelaySetting::Default => Endpoint::builder(presets::N0),
        RelaySetting::Custom(url) => {
            Endpoint::builder(presets::N0).relay_mode(RelayMode::Custom(RelayMap::from(url)))
        }
        RelaySetting::Disabled => {
            Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
        }
    };
```

`presets::N0` keeps the public address lookup services alongside the custom relay, which is what a self-hosted relay user wants; `Minimal` would also drop discovery.

- [ ] **Step 3: Fix the two call sites that relied on `Copy`**

`crates/app/src/launch.rs`: in `open_room`, `relay: launch.relay,` becomes `relay: launch.relay.clone(),`. The later `launch.relay == RelaySetting::Default` compiles as is.

`crates/app/src/publish.rs`: `relay,` in the `RoomConfig` literal becomes `relay: relay.clone(),`.

`crates/room/tests/two_rooms.rs` constructs `RelaySetting::Disabled` inline and compiles unchanged.

- [ ] **Step 4: Test, lint, commit**

Run: `cargo test -p brp-net && cargo clippy --workspace --all-targets -- -D warnings`
Expected: the new test passes with the existing loopback tests.

```bash
cargo fmt --all
git add crates/net/src/endpoint.rs crates/net/tests/loopback.rs crates/app/src/launch.rs crates/app/src/publish.rs
git commit -m "feat: allow a custom relay URL in the transport settings"
```

---

### Task 2: Output device listing and selection in `brp-audio`

**Files:**
- Modify: `crates/audio/src/cpal_output.rs`
- Modify: `crates/audio/src/lib.rs`
- Modify: `crates/app/src/launch.rs` (one line)
- Modify: `crates/app/src/publish.rs` (one line)

**Interfaces:**
- Consumes: cpal 0.18 `DeviceId`, `HostTrait::{device_by_id, output_devices, default_output_device}`, `DeviceTrait::{id, description}`.
- Produces: `brp_audio::OutputDevice { pub id: String, pub name: String }`; `brp_audio::output_devices() -> Result<Vec<OutputDevice>, AudioError>`; `CpalOutput::new(device: Option<String>) -> CpalOutput`; `brp_audio::cpal_output::parse_device_id(&str) -> Result<cpal::DeviceId, AudioError>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/audio/src/cpal_output.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_device_id_is_refused_by_name_without_touching_a_device() {
        let error = parse_device_id("nonsense").unwrap_err();
        assert!(matches!(error, AudioError::Device(_)), "{error}");
        assert!(error.to_string().contains("nonsense"), "{error}");
    }

    #[test]
    fn a_missing_device_fails_the_start_with_its_id_in_the_message() {
        let output = CpalOutput::new(Some("nonsense".into()));
        let error = output.start(Box::new(|_| {})).err().expect("start must fail");
        assert!(error.to_string().contains("nonsense"), "{error}");
    }

    #[test]
    fn listing_devices_does_not_fail_without_hardware() {
        // CI has no audio devices; an empty list is fine, an error is not.
        let devices = output_devices().expect("listing must succeed");
        for device in devices {
            assert!(!device.id.is_empty());
        }
    }
}
```

Run: `cargo test -p brp-audio cpal_output`
Expected: compile errors (`parse_device_id`, `CpalOutput::new`, `output_devices` missing).

- [ ] **Step 2: Implement**

In `crates/audio/src/cpal_output.rs`, replace the struct and `open_stream`, and add the listing:

```rust
/// Playback through one output device: the system default, or the device whose cpal id was
/// saved in settings.
#[derive(Debug, Default, Clone)]
pub struct CpalOutput {
    device: Option<String>,
}

impl CpalOutput {
    /// `None` plays through the default device. `Some(id)` is a [`cpal::DeviceId`] in its
    /// `Display` form; a device that is absent or an id that does not parse fails `start`, so
    /// audio never silently moves to a device the user did not choose.
    pub fn new(device: Option<String>) -> Self {
        Self { device }
    }
}

/// An output device as the settings dialog lists it: the stable id to save and the name to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// Every output device of the default host. Devices whose id or description cannot be read are
/// skipped rather than failing the list.
pub fn output_devices() -> Result<Vec<OutputDevice>, AudioError> {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .map_err(|e| AudioError::Device(format!("could not list output devices: {e}")))?;
    Ok(devices
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            let name = device.description().ok()?.name().to_string();
            Some(OutputDevice { id, name })
        })
        .collect())
}

pub fn parse_device_id(id: &str) -> Result<cpal::DeviceId, AudioError> {
    id.parse()
        .map_err(|e| AudioError::Device(format!("output device {id:?}: {e}")))
}
```

Change `impl AudioOutput for CpalOutput` so the thread receives the device choice: before `thread::Builder`, add `let device = self.device.clone();` and pass it into the closure's `open_stream(device.as_deref(), &mut render)`. Then:

```rust
fn open_stream(device: Option<&str>, render: &mut RenderFn) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = match device {
        None => host
            .default_output_device()
            .ok_or_else(|| AudioError::Device("no default output device".into()))?,
        Some(id) => {
            let parsed = parse_device_id(id)?;
            host.device_by_id(&parsed)
                .ok_or_else(|| AudioError::Device(format!("output device {id:?} not found")))?
        }
    };
    let config = cpal::StreamConfig {
        channels: u16::from(AUDIO_CHANNELS),
        sample_rate: AUDIO_SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };
    let mut render = std::mem::replace(render, Box::new(|_| {}));
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| render(data),
            |error| tracing::warn!(%error, "audio output stream error"),
            None,
        )
        .map_err(|e| AudioError::Format(format!("48 kHz stereo float output refused: {e}")))?;
    stream
        .play()
        .map_err(|e| AudioError::Device(format!("could not start playback: {e}")))?;
    Ok(stream)
}
```

Update the module doc comment's first line to "Playback through the default output device or a saved one."

In `crates/audio/src/lib.rs`, change the re-export to `pub use cpal_output::{CpalOutput, OutputDevice, output_devices};`.

`crates/app/src/launch.rs` and `crates/app/src/publish.rs`: `Arc::new(CpalOutput)` becomes `Arc::new(CpalOutput::new(None))` in both. Task 4 passes the saved device in `launch.rs`.

- [ ] **Step 3: Test, lint, commit**

Run: `cargo test -p brp-audio && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 3 new tests pass. If `listing_devices_does_not_fail_without_hardware` fails on the CI container because cpal's ALSA host errors when no card exists, keep the test but accept that outcome explicitly: change `output_devices()` so an `output_devices` error from the host yields `Ok(Vec::new())` with a `tracing::warn!`, and update the doc comment to say so. Record the choice in the spec amendments in Task 6.

```bash
cargo fmt --all
git add crates/audio/src/cpal_output.rs crates/audio/src/lib.rs crates/app/src/launch.rs crates/app/src/publish.rs
git commit -m "feat: list output devices and play through a chosen one"
```

---

### Task 3: The settings module

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/error.rs`
- Create: `crates/app/src/settings.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `RelaySetting` (Task 1), `DEFAULT_FPS` from `cli.rs`, `AppError::Io`.
- Produces: `settings::{SETTINGS_FILE, RECENT_ROOMS_MAX}`; `Settings { nickname: Option<String>, fps: u32, relay: RelayChoice, audio: AudioSettings, recent_rooms: Vec<RecentRoom> }` with `Default`, `load(&Path) -> Result<Settings, AppError>`, `save(&self, &Path) -> Result<(), AppError>`, `remember_room(&mut self, ticket: &str, now_unix: u64)`, `validate(&self) -> Result<(), String>`; `RelayChoice::{Default, Custom(String), Disabled}` with `to_relay_setting(&self) -> Result<RelaySetting, AppError>`; `AudioSettings { output_device: Option<String> }`; `RecentRoom { ticket: String, last_joined_unix: u64 }`; `SettingsStore { settings, path, load_error: Option<String> }` with `SettingsStore::load() -> Result<Self, AppError>`, `load_at(PathBuf) -> Self`, `save(&mut self) -> Result<(), AppError>`; `settings::now_unix() -> u64`; `AppError::Settings(String)`.

- [ ] **Step 1: Dependencies and the error variant**

Workspace `Cargo.toml`, under `[workspace.dependencies]`, after the `serde` line: `toml = "1.1"`.

`crates/app/Cargo.toml`, under `[dependencies]`: add `serde.workspace = true` and `toml.workspace = true`.

`crates/app/src/error.rs`: add before `Io`:

```rust
    #[error("settings: {0}")]
    Settings(String),
```

- [ ] **Step 2: Write the module with its tests**

Create `crates/app/src/settings.rs`:

```rust
//! What persists across launches besides the identity key: one TOML file in the platform config
//! directory, read once at startup and written whole after a room opens or the dialog saves.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use brp_net::RelaySetting;
use directories::ProjectDirs;
use iroh::RelayUrl;
use serde::{Deserialize, Serialize};

use crate::cli::DEFAULT_FPS;
use crate::error::AppError;

/// Beside `identity.key` in the platform config directory.
pub const SETTINGS_FILE: &str = "settings.toml";
/// Enough to cover a week of rooms for a small group without scrolling the start screen.
pub const RECENT_ROOMS_MAX: usize = 8;

/// Everything the file holds. Unknown keys are ignored on load so an older binary reads a newer
/// file; missing keys take their defaults so a newer binary reads an older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub nickname: Option<String>,
    pub fps: u32,
    pub relay: RelayChoice,
    pub audio: AudioSettings,
    pub recent_rooms: Vec<RecentRoom>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            nickname: None,
            fps: DEFAULT_FPS,
            relay: RelayChoice::Default,
            audio: AudioSettings::default(),
            recent_rooms: Vec::new(),
        }
    }
}

/// Serialised as `[relay] mode = "default" | "custom" | "disabled"`, with `url` for custom.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "url", rename_all = "lowercase")]
pub enum RelayChoice {
    #[default]
    Default,
    Custom(String),
    Disabled,
}

impl RelayChoice {
    /// The transport setting, or the parse error of a custom URL.
    pub fn to_relay_setting(&self) -> Result<RelaySetting, AppError> {
        Ok(match self {
            RelayChoice::Default => RelaySetting::Default,
            RelayChoice::Disabled => RelaySetting::Disabled,
            RelayChoice::Custom(url) => RelaySetting::Custom(
                url.parse::<RelayUrl>()
                    .map_err(|e| AppError::Settings(format!("relay URL {url:?}: {e}")))?,
            ),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// A cpal device id in its `Display` form; `None` is the system default.
    pub output_device: Option<String>,
}

/// A room the user created or joined: the ticket that got them in, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRoom {
    pub ticket: String,
    pub last_joined_unix: u64,
}

impl Settings {
    pub fn path() -> Result<PathBuf, AppError> {
        let dirs = ProjectDirs::from("", "", "brp").ok_or_else(|| {
            AppError::Settings("no home directory to store settings in".into())
        })?;
        Ok(dirs.config_dir().join(SETTINGS_FILE))
    }

    /// A missing file is the defaults. Any other failure is an error and the file is left as is.
    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(AppError::Settings(format!("{}: {e}", path.display()))),
        };
        toml::from_str(&text).map_err(|e| AppError::Settings(format!("{}: {e}", path.display())))
    }

    /// Writes the whole file through a temporary sibling and a rename, so a crash mid-write
    /// leaves the previous file intact.
    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        let text = toml::to_string_pretty(self).map_err(|e| AppError::Settings(e.to_string()))?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Puts `ticket` first with `now_unix`, dropping an older entry for the same ticket and
    /// anything beyond [`RECENT_ROOMS_MAX`].
    pub fn remember_room(&mut self, ticket: &str, now_unix: u64) {
        self.recent_rooms.retain(|room| room.ticket != ticket);
        self.recent_rooms.insert(
            0,
            RecentRoom {
                ticket: ticket.to_string(),
                last_joined_unix: now_unix,
            },
        );
        self.recent_rooms.truncate(RECENT_ROOMS_MAX);
    }

    /// What Save refuses: a zero frame rate, or a custom relay URL that does not parse.
    pub fn validate(&self) -> Result<(), String> {
        if self.fps == 0 {
            return Err("frame rate ceiling must be at least 1".into());
        }
        self.relay
            .to_relay_setting()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The settings together with where they live and whether loading them failed; the app owns one.
#[derive(Debug)]
pub struct SettingsStore {
    pub settings: Settings,
    pub path: PathBuf,
    /// Set when the file exists but could not be read or parsed. Shown on the start screen; the
    /// defaults are in use and nothing is written until the user saves.
    pub load_error: Option<String>,
}

impl SettingsStore {
    pub fn load() -> Result<Self, AppError> {
        Ok(Self::load_at(Settings::path()?))
    }

    pub fn load_at(path: PathBuf) -> Self {
        let (settings, load_error) = match Settings::load(&path) {
            Ok(settings) => (settings, None),
            Err(error) => (Settings::default(), Some(error.to_string())),
        };
        Self {
            settings,
            path,
            load_error,
        }
    }

    /// Saves and, on success, clears a load error: the file is now what we wrote.
    pub fn save(&mut self) -> Result<(), AppError> {
        self.settings.save(&self.path)?;
        self.load_error = None;
        Ok(())
    }
}

/// Seconds since the Unix epoch, or zero if the clock is before it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brp-settings-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(SETTINGS_FILE)
    }

    fn full() -> Settings {
        Settings {
            nickname: Some("alice".into()),
            fps: 30,
            relay: RelayChoice::Custom("https://relay.example.com/".into()),
            audio: AudioSettings {
                output_device: Some("PipeWire:alsa_output.usb".into()),
            },
            recent_rooms: vec![RecentRoom {
                ticket: "brpticket".into(),
                last_joined_unix: 1_788_000_000,
            }],
        }
    }

    #[test]
    fn defaults_and_every_field_round_trip_through_toml() {
        for settings in [Settings::default(), full()] {
            let text = toml::to_string_pretty(&settings).unwrap();
            let back: Settings = toml::from_str(&text).unwrap();
            assert_eq!(back, settings, "{text}");
        }
    }

    #[test]
    fn the_file_shape_matches_the_spec() {
        let text = toml::to_string_pretty(&full()).unwrap();
        assert!(text.contains("nickname = \"alice\""), "{text}");
        assert!(text.contains("[relay]"), "{text}");
        assert!(text.contains("mode = \"custom\""), "{text}");
        assert!(text.contains("url = \"https://relay.example.com/\""), "{text}");
        assert!(text.contains("[audio]"), "{text}");
        assert!(text.contains("[[recent_rooms]]"), "{text}");
        let disabled = toml::to_string_pretty(&Settings {
            relay: RelayChoice::Disabled,
            ..Settings::default()
        })
        .unwrap();
        assert!(disabled.contains("mode = \"disabled\""), "{disabled}");
    }

    #[test]
    fn unknown_and_missing_keys_are_tolerated() {
        let settings: Settings = toml::from_str("fps = 24\nfuture_key = 1\n[relay]\nmode = \"disabled\"\n").unwrap();
        assert_eq!(settings.fps, 24);
        assert_eq!(settings.relay, RelayChoice::Disabled);
        assert_eq!(settings.nickname, None);
        assert!(settings.recent_rooms.is_empty());
    }

    #[test]
    fn a_missing_file_loads_the_defaults() {
        let path = temp_path("missing");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    }

    #[test]
    fn an_invalid_file_is_an_error_and_is_left_untouched() {
        let path = temp_path("invalid");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "this = = is not toml").unwrap();
        let error = Settings::load(&path).unwrap_err();
        assert!(matches!(error, AppError::Settings(_)), "{error}");
        assert!(error.to_string().contains(&path.display().to_string()), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "this = = is not toml");
        let store = SettingsStore::load_at(path);
        assert_eq!(store.settings, Settings::default());
        assert!(store.load_error.is_some());
    }

    #[test]
    fn save_creates_the_directory_replaces_the_file_and_leaves_no_temp_file() {
        let path = temp_path("save");
        Settings::default().save(&path).unwrap();
        full().save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), full());
        let siblings: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(siblings, [SETTINGS_FILE]);
        let mut store = SettingsStore::load_at(path.clone());
        store.load_error = Some("stale".into());
        store.save().unwrap();
        assert_eq!(store.load_error, None);
    }

    #[test]
    fn remember_room_moves_a_repeat_to_the_front_and_caps_the_list() {
        let mut settings = Settings::default();
        for i in 0..RECENT_ROOMS_MAX as u64 + 2 {
            settings.remember_room(&format!("t{i}"), i);
        }
        assert_eq!(settings.recent_rooms.len(), RECENT_ROOMS_MAX);
        assert_eq!(settings.recent_rooms[0].ticket, "t9");
        assert!(settings.recent_rooms.iter().all(|r| r.ticket != "t0" && r.ticket != "t1"));
        settings.remember_room("t5", 100);
        assert_eq!(settings.recent_rooms.len(), RECENT_ROOMS_MAX);
        assert_eq!(settings.recent_rooms[0], RecentRoom { ticket: "t5".into(), last_joined_unix: 100 });
        assert_eq!(settings.recent_rooms.iter().filter(|r| r.ticket == "t5").count(), 1);
    }

    #[test]
    fn validate_rejects_a_zero_frame_rate_and_a_bad_relay_url() {
        let mut settings = Settings::default();
        assert_eq!(settings.validate(), Ok(()));
        settings.fps = 0;
        assert!(settings.validate().unwrap_err().contains("frame rate"));
        settings.fps = 60;
        settings.relay = RelayChoice::Custom("not a url".into());
        assert!(settings.validate().unwrap_err().contains("relay URL"));
        settings.relay = RelayChoice::Custom("https://relay.example.com/".into());
        assert_eq!(settings.validate(), Ok(()));
    }

    #[test]
    fn relay_choice_maps_onto_the_transport_setting() {
        assert_eq!(
            RelayChoice::Default.to_relay_setting().unwrap(),
            RelaySetting::Default
        );
        assert_eq!(
            RelayChoice::Disabled.to_relay_setting().unwrap(),
            RelaySetting::Disabled
        );
        let url: RelayUrl = "https://relay.example.com/".parse().unwrap();
        assert_eq!(
            RelayChoice::Custom("https://relay.example.com/".into())
                .to_relay_setting()
                .unwrap(),
            RelaySetting::Custom(url)
        );
        assert!(RelayChoice::Custom("nope".into()).to_relay_setting().is_err());
    }
}
```

Add `pub mod settings;` to `crates/app/src/lib.rs` (alphabetical, after `room_view`).

- [ ] **Step 3: Test, lint, commit**

Run: `cargo test -p brp settings && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 9 tests pass. If `the_file_shape_matches_the_spec` fails on how toml 1.1 formats the adjacently tagged unit variant, read the actual output from the assertion message and correct the assertion to the produced form, then record the produced shape in the spec amendments (Task 6).

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/src/error.rs crates/app/src/settings.rs crates/app/src/lib.rs
git commit -m "feat: add the settings file with atomic saves and recent rooms"
```

---

### Task 4: Launch from settings and flags; the app saves after an open

**Files:**
- Modify: `crates/app/src/cli.rs`
- Modify: `crates/app/src/launch.rs`
- Modify: `crates/app/src/participant.rs`
- Modify: `crates/app/src/window.rs`

**Interfaces:**
- Consumes: `Settings`, `SettingsStore`, `now_unix` (Task 3); `CpalOutput::new` (Task 2); `App` as plan 5a left it.
- Produces: `WindowArgs { nickname: Option<String>, fps: Option<u32>, no_relay: bool }`; `Launch { nickname: Option<String>, nickname_from_flag: bool, fps: u32, relay: RelaySetting, audio_output: Option<String> }` with `Launch::from_settings(&Settings, &WindowArgs) -> Result<Launch, AppError>`; `App::new(runtime, proxy, launch, secret, nickname, intent, store: SettingsStore)`.

- [ ] **Step 1: Flags become optional overrides**

In `crates/app/src/cli.rs`, `WindowArgs`:

```rust
#[derive(Args, Debug)]
pub struct WindowArgs {
    /// Shown to other participants. Overrides the saved nickname for this launch.
    #[arg(long)]
    pub nickname: Option<String>,
    /// Capture ceiling for lives shared from the window; each live's presets can go lower.
    /// Overrides the saved setting for this launch.
    #[arg(long)]
    pub fps: Option<u32>,
    /// Disables relays for this launch whatever the saved relay setting is.
    #[arg(long)]
    pub no_relay: bool,
}
impl Default for WindowArgs {
    fn default() -> Self {
        Self {
            nickname: None,
            fps: None,
            no_relay: false,
        }
    }
}
```

`PublishArgs.fps` keeps its `default_value_t = DEFAULT_FPS`; the terminal path has no settings file.

- [ ] **Step 2: Write the failing precedence tests and `Launch::from_settings`**

In `crates/app/src/launch.rs`, replace the `Launch` struct and the `From<WindowArgs>` impl with:

```rust
/// Settings that apply to any room this window opens: the saved settings, with each command line
/// flag that is present replacing its field for this launch only.
#[derive(Debug, Clone)]
pub struct Launch {
    pub nickname: Option<String>,
    /// True when `--nickname` was given: the start screen's nickname is then not saved.
    pub nickname_from_flag: bool,
    pub fps: u32,
    pub relay: RelaySetting,
    /// cpal device id to play through; `None` is the system default.
    pub audio_output: Option<String>,
}

impl Launch {
    pub fn from_settings(settings: &Settings, args: &WindowArgs) -> Result<Self, AppError> {
        let relay = if args.no_relay {
            RelaySetting::Disabled
        } else {
            settings.relay.to_relay_setting()?
        };
        Ok(Self {
            nickname: args.nickname.clone().or_else(|| settings.nickname.clone()),
            nickname_from_flag: args.nickname.is_some(),
            fps: args.fps.unwrap_or(settings.fps),
            relay,
            audio_output: settings.audio.output_device.clone(),
        })
    }
}
```

Add `use crate::settings::Settings;`. In `open_room`, `audio_output: Arc::new(CpalOutput::new(None)),` becomes `audio_output: Arc::new(CpalOutput::new(launch.audio_output.clone())),`.

Append tests to `launch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AudioSettings, RelayChoice};

    fn saved() -> Settings {
        Settings {
            nickname: Some("saved".into()),
            fps: 30,
            relay: RelayChoice::Custom("https://relay.example.com/".into()),
            audio: AudioSettings {
                output_device: Some("dev".into()),
            },
            recent_rooms: Vec::new(),
        }
    }

    #[test]
    fn without_flags_the_saved_settings_apply() {
        let launch = Launch::from_settings(&saved(), &WindowArgs::default()).unwrap();
        assert_eq!(launch.nickname.as_deref(), Some("saved"));
        assert!(!launch.nickname_from_flag);
        assert_eq!(launch.fps, 30);
        assert!(matches!(launch.relay, RelaySetting::Custom(_)));
        assert_eq!(launch.audio_output.as_deref(), Some("dev"));
    }

    #[test]
    fn each_flag_overrides_only_its_field() {
        let args = WindowArgs {
            nickname: Some("flag".into()),
            fps: Some(120),
            no_relay: true,
        };
        let launch = Launch::from_settings(&saved(), &args).unwrap();
        assert_eq!(launch.nickname.as_deref(), Some("flag"));
        assert!(launch.nickname_from_flag);
        assert_eq!(launch.fps, 120);
        assert_eq!(launch.relay, RelaySetting::Disabled);
        assert_eq!(launch.audio_output.as_deref(), Some("dev"));
    }

    #[test]
    fn a_bad_saved_relay_url_is_an_error_unless_relays_are_off() {
        let mut settings = saved();
        settings.relay = RelayChoice::Custom("nope".into());
        assert!(Launch::from_settings(&settings, &WindowArgs::default()).is_err());
        let args = WindowArgs {
            no_relay: true,
            ..WindowArgs::default()
        };
        assert_eq!(
            Launch::from_settings(&settings, &args).unwrap().relay,
            RelaySetting::Disabled
        );
    }
}
```

Run: `cargo test -p brp launch`
Expected: 3 pass once `participant.rs` and `window.rs` compile (Steps 3 and 4); until then the crate does not build, so do Steps 3 and 4 before running.

- [ ] **Step 3: `participant::run` loads the store**

In `crates/app/src/participant.rs`, replace `let launch = Launch::from(args);` with:

```rust
    let store = SettingsStore::load()?;
    let launch = Launch::from_settings(&store.settings, &args)?;
```

with `use crate::settings::SettingsStore;`, and pass `store` as the new last argument of `App::new(..)`.

- [ ] **Step 4: `App` owns the store and saves after an open**

In `crates/app/src/window.rs`:

Imports: add `use crate::settings::{SettingsStore, now_unix};`.

Fields on `App`: add

```rust
    store: SettingsStore,
    /// The intent an open in flight was started with; a join remembers the ticket that got us in,
    /// a create remembers the room's own ticket.
    pending_intent: Option<Intent>,
```

`App::new` gains `store: SettingsStore` as its last parameter and sets `store, pending_intent: None,`. When `store.load_error` is `Some(message)`, set `app.start.error = format!("settings not loaded, defaults in use: {message}")` right after constructing `app` and before the `intent` handling, so the start screen shows it.

In `open`, first line: `self.pending_intent = Some(intent.clone());`.

In `user_event`, the `RoomOpened(Ok(room))` arm gains, after `self.phase = Phase::Room(..)`:

```rust
                self.remember_open(&room);
```

and add the method to `impl App`:

```rust
    /// Persists what a successful open teaches us: the ticket to list under recent rooms, and the
    /// nickname typed on the start screen unless a flag chose it. Failures are shown, not fatal.
    fn remember_open(&mut self, room: &Room) {
        let ticket = match self.pending_intent.take() {
            Some(Intent::Join(ticket)) => ticket.to_string(),
            Some(Intent::Create) | None => room.ticket().to_string(),
        };
        self.store.settings.remember_room(&ticket, now_unix());
        if !self.launch.nickname_from_flag {
            let nickname = self.start.nickname.trim();
            if !nickname.is_empty() {
                self.store.settings.nickname = Some(nickname.to_string());
            }
        }
        if let Err(error) = self.store.save() {
            self.state.status = format!("settings not saved: {error}");
        }
    }
```

Note the arm holds `room: Arc<Room>`; call `self.remember_open(&room)` before `room` is moved into `RoomView::new(room)`, so the order in the arm is: `self.pending_open = None; self.state = UiState::new(); set title; self.remember_open(&room); self.phase = Phase::Room(Box::new(RoomView::new(room)));`.

- [ ] **Step 5: Build, test, lint, commit**

Run: `cargo test -p brp && cargo clippy --workspace --all-targets -- -D warnings`
Expected: the 3 launch tests and every existing test pass.

Run `cargo run -p brp -- create --no-relay`, then close the window, and `cat ~/.config/brp/settings.toml`: it holds one `[[recent_rooms]]` entry and the nickname shown on the start screen.

```bash
cargo fmt --all
git add crates/app/src/cli.rs crates/app/src/launch.rs crates/app/src/participant.rs crates/app/src/window.rs
git commit -m "feat: build the launch from saved settings and remember rooms after an open"
```

---

### Task 5: Settings dialog, recent rooms on the start screen, Settings buttons

**Files:**
- Create: `crates/app/src/ui/settings.rs`
- Modify: `crates/app/src/ui/start.rs`
- Modify: `crates/app/src/ui/status.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/window.rs`

**Interfaces:**
- Consumes: `Settings`, `RelayChoice`, `RecentRoom` (Task 3); `brp_audio::{OutputDevice, output_devices}` (Task 2); `App.store` (Task 4).
- Produces: `ui::settings::{SettingsDialog, RelayKind, switch_relay, draw}`; `ui::start::StartAction::OpenSettings`; `ui::start::draw(ui, state, recent: &[RecentRoom], now_unix: u64) -> Option<StartAction>`; `ui::start::{relative_age(then_unix, now_unix) -> String, abbreviate_ticket(&str) -> String}`; `ui::status::draw(.., open_settings: &mut bool)`; `UiOutput.open_settings: bool`.

- [ ] **Step 1: Write the failing pure tests for the dialog and the start screen**

Create `crates/app/src/ui/settings.rs`:

```rust
//! The Settings dialog: edits a draft copy of the saved settings and hands it back on Save.
//! Changes apply when the next room opens; the dialog says so while a room is open.

use brp_audio::OutputDevice;

use crate::settings::{RelayChoice, Settings};

/// Dialog state: whether it is open, the draft being edited, the devices listed when it opened,
/// and the last validation or save error.
#[derive(Debug, Default)]
pub struct SettingsDialog {
    pub open: bool,
    pub draft: Settings,
    pub devices: Vec<OutputDevice>,
    pub devices_error: Option<String>,
    pub error: String,
}

impl SettingsDialog {
    /// Opens with a fresh draft of `settings` and the device list as enumerated now.
    pub fn open_with(&mut self, settings: &Settings, devices: Result<Vec<OutputDevice>, String>) {
        self.draft = settings.clone();
        match devices {
            Ok(devices) => {
                self.devices = devices;
                self.devices_error = None;
            }
            Err(error) => {
                self.devices = Vec::new();
                self.devices_error = Some(error);
            }
        }
        self.error.clear();
        self.open = true;
    }
}

/// The three radio buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayKind {
    Default,
    Custom,
    Disabled,
}

impl RelayKind {
    pub fn of(choice: &RelayChoice) -> Self {
        match choice {
            RelayChoice::Default => Self::Default,
            RelayChoice::Custom(_) => Self::Custom,
            RelayChoice::Disabled => Self::Disabled,
        }
    }
}

/// The choice after a radio click. Switching to custom keeps a URL that was already typed and
/// starts empty otherwise; switching away keeps nothing, since the draft is discarded on Cancel.
pub fn switch_relay(current: &RelayChoice, kind: RelayKind) -> RelayChoice {
    match (kind, current) {
        (RelayKind::Custom, RelayChoice::Custom(url)) => RelayChoice::Custom(url.clone()),
        (RelayKind::Custom, _) => RelayChoice::Custom(String::new()),
        (RelayKind::Default, _) => RelayChoice::Default,
        (RelayKind::Disabled, _) => RelayChoice::Disabled,
    }
}

/// What the combo box shows for the draft's device: its name when listed, the id with a note when
/// it is saved but absent, or the default.
pub fn device_label(selected: Option<&str>, devices: &[OutputDevice]) -> String {
    match selected {
        None => "System default".to_string(),
        Some(id) => devices
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| format!("{id} (not present)")),
    }
}

/// Draws the dialog when open. Returns the settings to persist when Save was clicked and the
/// draft validates; the caller saves and reopens the dialog with the error if that fails.
pub fn draw(ctx: &egui::Context, dialog: &mut SettingsDialog, room_open: bool) -> Option<Settings> {
    if !dialog.open {
        return None;
    }
    let mut saved = None;
    let mut open = true;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Nickname");
                    let mut nickname = dialog.draft.nickname.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut nickname).changed() {
                        dialog.draft.nickname =
                            (!nickname.trim().is_empty()).then(|| nickname.trim().to_string());
                    }
                    ui.end_row();

                    ui.label("Relay");
                    ui.vertical(|ui| {
                        let mut kind = RelayKind::of(&dialog.draft.relay);
                        let before = kind;
                        ui.radio_value(&mut kind, RelayKind::Default, "Public relays");
                        ui.radio_value(&mut kind, RelayKind::Custom, "Custom relay URL");
                        ui.radio_value(&mut kind, RelayKind::Disabled, "No relay (LAN only)");
                        if kind != before {
                            dialog.draft.relay = switch_relay(&dialog.draft.relay, kind);
                        }
                        if let RelayChoice::Custom(url) = &mut dialog.draft.relay {
                            ui.add(
                                egui::TextEdit::singleline(url)
                                    .hint_text("https://relay.example.com/"),
                            );
                        }
                    });
                    ui.end_row();

                    ui.label("Frame rate ceiling");
                    ui.add(egui::DragValue::new(&mut dialog.draft.fps).range(1..=u32::MAX));
                    ui.end_row();

                    ui.label("Audio output");
                    let label =
                        device_label(dialog.draft.audio.output_device.as_deref(), &dialog.devices);
                    egui::ComboBox::from_id_salt("settings-output-device")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut dialog.draft.audio.output_device,
                                None,
                                "System default",
                            );
                            for device in &dialog.devices {
                                ui.selectable_value(
                                    &mut dialog.draft.audio.output_device,
                                    Some(device.id.clone()),
                                    &device.name,
                                );
                            }
                        });
                    ui.end_row();
                });
            if let Some(error) = &dialog.devices_error {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("could not list output devices: {error}"),
                );
            }
            if room_open {
                ui.weak("Applies to the next room you open.");
            }
            if !dialog.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &dialog.error);
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    match dialog.draft.validate() {
                        Ok(()) => {
                            saved = Some(dialog.draft.clone());
                            dialog.open = false;
                        }
                        Err(error) => dialog.error = error,
                    }
                }
                if ui.button("Cancel").clicked() {
                    dialog.open = false;
                }
            });
        });
    // The title bar's close button behaves like Cancel.
    dialog.open &= open;
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_relay_kinds_keeps_a_typed_url_only_while_custom() {
        let typed = RelayChoice::Custom("https://r.example/".into());
        assert_eq!(switch_relay(&typed, RelayKind::Custom), typed);
        assert_eq!(switch_relay(&typed, RelayKind::Disabled), RelayChoice::Disabled);
        assert_eq!(
            switch_relay(&RelayChoice::Default, RelayKind::Custom),
            RelayChoice::Custom(String::new())
        );
        assert_eq!(RelayKind::of(&typed), RelayKind::Custom);
    }

    #[test]
    fn the_device_label_names_listed_devices_and_flags_absent_ones() {
        let devices = vec![OutputDevice {
            id: "host:abc".into(),
            name: "Speakers".into(),
        }];
        assert_eq!(device_label(None, &devices), "System default");
        assert_eq!(device_label(Some("host:abc"), &devices), "Speakers");
        assert_eq!(device_label(Some("host:gone"), &devices), "host:gone (not present)");
    }

    #[test]
    fn opening_takes_a_fresh_draft_and_clears_old_errors() {
        let mut dialog = SettingsDialog::default();
        dialog.error = "old".into();
        let settings = Settings {
            fps: 24,
            ..Settings::default()
        };
        dialog.open_with(&settings, Err("no host".into()));
        assert!(dialog.open);
        assert_eq!(dialog.draft, settings);
        assert!(dialog.error.is_empty());
        assert_eq!(dialog.devices_error.as_deref(), Some("no host"));
        assert!(dialog.devices.is_empty());
    }
}
```

In `crates/app/src/ui/start.rs`, add `use crate::settings::RecentRoom;`, add the `OpenSettings` variant, the helpers, and change `draw`:

```rust
/// Which button the user clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Create,
    Join,
    OpenSettings,
}
```

At the top of `submit`, before the `connecting` check:

```rust
        if action == StartAction::OpenSettings {
            return None;
        }
```

Add the helpers before `draw`:

```rust
/// Characters of a ticket shown at each end on the start screen; the middle is elided.
const TICKET_HEAD: usize = 8;
const TICKET_TAIL: usize = 6;

/// "just now", then minutes, hours, days.
pub fn relative_age(then_unix: u64, now_unix: u64) -> String {
    let seconds = now_unix.saturating_sub(then_unix);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes == 0 {
        "just now".to_string()
    } else if hours == 0 {
        format!("{minutes} min ago")
    } else if days == 0 {
        format!("{hours} h ago")
    } else {
        format!("{days} d ago")
    }
}

/// The ticket's ends with the middle elided, so a row fits and two tickets stay tellable apart.
pub fn abbreviate_ticket(ticket: &str) -> String {
    let chars: Vec<char> = ticket.chars().collect();
    if chars.len() <= TICKET_HEAD + TICKET_TAIL + 1 {
        return ticket.to_string();
    }
    let head: String = chars[..TICKET_HEAD].iter().collect();
    let tail: String = chars[chars.len() - TICKET_TAIL..].iter().collect();
    format!("{head}…{tail}")
}
```

Replace `draw` with:

```rust
/// Draws the start screen and returns the button clicked, if any. Clicking a recent room fills
/// the ticket field and joins it.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut StartState,
    recent: &[RecentRoom],
    now_unix: u64,
) -> Option<StartAction> {
    let mut action = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.button("Settings").clicked() {
                action = Some(StartAction::OpenSettings);
            }
        });
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.heading("brp");
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label("Nickname");
                ui.add_enabled(
                    !state.connecting,
                    egui::TextEdit::singleline(&mut state.nickname).desired_width(240.0),
                );
            });
            ui.add_space(8.0);
            if ui
                .add_enabled(!state.connecting, egui::Button::new("Create room"))
                .clicked()
            {
                action = Some(StartAction::Create);
            }
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.add_enabled(
                !state.connecting,
                egui::TextEdit::singleline(&mut state.ticket)
                    .hint_text("paste a ticket")
                    .desired_width(480.0),
            );
            let can_join = !state.connecting && !state.ticket.trim().is_empty();
            if ui
                .add_enabled(can_join, egui::Button::new("Join room"))
                .clicked()
            {
                action = Some(StartAction::Join);
            }
            if !recent.is_empty() {
                ui.add_space(16.0);
                ui.label("Recent rooms");
                for room in recent {
                    ui.horizontal(|ui| {
                        ui.monospace(abbreviate_ticket(&room.ticket));
                        ui.weak(relative_age(room.last_joined_unix, now_unix));
                        if ui
                            .add_enabled(!state.connecting, egui::Button::new("Join"))
                            .clicked()
                        {
                            state.ticket = room.ticket.clone();
                            action = Some(StartAction::Join);
                        }
                    });
                }
            }
            ui.add_space(16.0);
            if state.connecting {
                ui.weak("connecting");
            }
            if !state.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &state.error);
            }
        });
    });
    action
}
```

Add to the `tests` module in `start.rs`:

```rust
    #[test]
    fn open_settings_is_never_an_intent_and_leaves_the_form_alone() {
        let mut state = StartState::new("alice".into());
        assert_eq!(state.submit(StartAction::OpenSettings), None);
        assert!(!state.connecting);
    }

    #[test]
    fn relative_age_rounds_down_through_the_units() {
        assert_eq!(relative_age(1_000, 1_030), "just now");
        assert_eq!(relative_age(1_000, 1_000 + 5 * 60), "5 min ago");
        assert_eq!(relative_age(1_000, 1_000 + 3 * 3_600 + 59), "3 h ago");
        assert_eq!(relative_age(1_000, 1_000 + 2 * 86_400), "2 d ago");
        assert_eq!(relative_age(2_000, 1_000), "just now");
    }

    #[test]
    fn short_tickets_are_shown_whole_and_long_ones_keep_both_ends() {
        assert_eq!(abbreviate_ticket("brpshort"), "brpshort");
        let long = "brp".to_string() + &"x".repeat(40) + "tail42";
        let shown = abbreviate_ticket(&long);
        assert!(shown.starts_with("brpxxxxx"), "{shown}");
        assert!(shown.ends_with("tail42"), "{shown}");
        assert!(shown.contains('…'), "{shown}");
        assert_eq!(shown.chars().count(), TICKET_HEAD + TICKET_TAIL + 1);
    }
```

- [ ] **Step 2: Status bar button and the UI output flag**

`crates/app/src/ui/status.rs`: `draw` gains a last parameter `open_settings: &mut bool`, and after the master mute toggle (before the audio output error) add:

```rust
            ui.separator();
            if ui.button("Settings").clicked() {
                *open_settings = true;
            }
```

Update its doc comment to say the Settings button sets the flag.

`crates/app/src/ui/mod.rs`: add `pub mod settings;`, add `pub open_settings: bool,` to `UiOutput` with the doc `/// The status bar's Settings button was clicked.`, and in `draw`:

```rust
    let mut open_settings = false;
    status::draw(ui, snapshot, ticket, state, &mut commands, &mut open_settings);
    ...
    UiOutput {
        commands,
        window_commands,
        open_settings,
        tile_rects,
    }
```

- [ ] **Step 3: Wire the dialog into `App`**

In `crates/app/src/window.rs`:

Imports: add `use crate::ui::settings::{self as settings_ui, SettingsDialog};` and `use crate::ui::start::StartAction;`.

Field on `App`: `settings_dialog: SettingsDialog,` initialised with `SettingsDialog::default()`.

In `redraw_main`, the egui closure becomes:

```rust
        let room_open = matches!(self.phase, Phase::Room(_));
        let now = now_unix();
        let mut saved = None;
        let mut ui_frame = main.ui.run(&main.window, [size.0, size.1], |root| {
            match &self.phase {
                Phase::Start => {
                    start_action = start::draw(
                        root,
                        &mut self.start,
                        &self.store.settings.recent_rooms,
                        now,
                    );
                }
                Phase::Room(view) => {
                    output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state, &popped);
                }
            }
            saved = settings_ui::draw(root.ctx(), &mut self.settings_dialog, room_open);
        });
```

After the existing command handling at the end of `redraw_main`, add:

```rust
        if output.open_settings || start_action == Some(StartAction::OpenSettings) {
            let devices = brp_audio::output_devices().map_err(|e| e.to_string());
            self.settings_dialog
                .open_with(&self.store.settings, devices);
        }
        if let Some(settings) = saved {
            self.store.settings = settings;
            if let Err(error) = self.store.save() {
                self.settings_dialog.error = format!("could not save: {error}");
                self.settings_dialog.open = true;
            }
        }
        if let Some(main) = &self.main
            && (saved.is_some() || output.open_settings)
        {
            main.window.request_redraw();
        }
```

`saved` is moved by the `if let Some(settings) = saved` above, so keep a `let saved_any = saved.is_some();` before it and use `saved_any` in the last condition. `start_action` is `Option<StartAction>`; the `submit` call in the existing code returns `None` for `OpenSettings`, so nothing else changes there.

- [ ] **Step 4: Test, lint, commit**

Run: `cargo test -p brp && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 3 dialog tests and 3 start tests pass with everything else.

Run `cargo run -p brp`: click Settings on the start screen, set a nickname and pick "No relay", Save, close the app, reopen: the nickname field shows the saved name. Open a room, click Settings in the status bar, confirm the "Applies to the next room" note.

```bash
cargo fmt --all
git add crates/app/src/ui/settings.rs crates/app/src/ui/start.rs crates/app/src/ui/status.rs crates/app/src/ui/mod.rs crates/app/src/window.rs
git commit -m "feat: add the settings dialog and recent rooms on the start screen"
```

---

### Task 6: Release workflow, shared Windows steps, documentation

**Files:**
- Create: `.github/actions/windows-ffmpeg/action.yml`
- Create: `ci/stage-windows.ps1`
- Modify: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md`

**Interfaces:** none in code. The composite action exports `FFMPEG_DIR` and `LIBCLANG_PATH` into the job environment and adds the FFmpeg `bin` directory to `PATH`. The script takes `-Destination <dir>`.

- [ ] **Step 1: The composite action**

Create `.github/actions/windows-ffmpeg/action.yml`:

```yaml
name: Windows FFmpeg
description: LLVM for bindgen and the pinned BtbN LGPL shared FFmpeg build. Exports FFMPEG_DIR and LIBCLANG_PATH and puts the DLLs on PATH.
runs:
  using: composite
  steps:
    - name: Ensure LLVM for bindgen
      shell: pwsh
      run: |
        $ErrorActionPreference = "Stop"
        $libclang = "C:\Program Files\LLVM\bin"
        if (-not (Test-Path "$libclang\libclang.dll")) {
          # choco is a native command: the error preference does not see its exit code.
          choco install llvm -y --no-progress
          if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }
        Add-Content $env:GITHUB_ENV "LIBCLANG_PATH=$libclang"
    - name: Install FFmpeg (BtbN LGPL shared, pinned)
      shell: pwsh
      run: |
        $ErrorActionPreference = "Stop"
        $release = "autobuild-2026-09-05-13-10"
        $asset = "ffmpeg-n8.1.2-50-g1a748fe2cd-win64-lgpl-shared-8.1.zip"
        $dir = Join-Path $env:GITHUB_WORKSPACE "ffmpeg"
        Invoke-WebRequest -Uri "https://github.com/BtbN/FFmpeg-Builds/releases/download/$release/$asset" -OutFile ffmpeg.zip
        Expand-Archive ffmpeg.zip -DestinationPath ffmpeg-extract
        Move-Item (Get-ChildItem ffmpeg-extract | Select-Object -First 1).FullName $dir
        Add-Content $env:GITHUB_ENV "FFMPEG_DIR=$dir"
        Add-Content $env:GITHUB_PATH "$dir\bin"
```

- [ ] **Step 2: The staging script**

Create `ci/stage-windows.ps1`:

```powershell
# Stages the Windows build into one directory: the exe, the FFmpeg DLLs it loads, and licenses.
param(
  [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Destination | Out-Null
Copy-Item target\release\brp.exe $Destination
foreach ($dll in "avcodec-62.dll", "avutil-60.dll", "swscale-9.dll", "swresample-6.dll") {
  Copy-Item (Join-Path $env:FFMPEG_DIR "bin\$dll") $Destination
}
Copy-Item (Join-Path $env:FFMPEG_DIR "LICENSE.txt") (Join-Path $Destination "FFMPEG-LICENSE.txt")
Copy-Item LICENSE (Join-Path $Destination "LICENSE")
```

- [ ] **Step 3: CI uses both**

Replace the `windows` job in `.github/workflows/ci.yml` with:

```yaml
  windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - uses: ./.github/actions/windows-ffmpeg
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo build --release -p brp
      - name: Stage the artifact
        shell: pwsh
        run: ./ci/stage-windows.ps1 -Destination dist
      - uses: actions/upload-artifact@v4
        with:
          name: brp-windows-x86_64
          path: dist
```

The job-level `env:` block is removed; the action exports both variables.

- [ ] **Step 4: The release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release
on:
  push:
    tags: ['v*']
permissions:
  contents: write
jobs:
  version:
    runs-on: ubuntu-latest
    outputs:
      version: ${{ steps.check.outputs.version }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - id: check
        name: Tag must equal the workspace version
        run: |
          set -eu
          tag="${GITHUB_REF_NAME#v}"
          version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "brp") | .version')"
          if [ "$tag" != "$version" ]; then
            echo "tag v$tag does not match the workspace version $version" >&2
            exit 1
          fi
          echo "version=$version" >> "$GITHUB_OUTPUT"
  linux:
    needs: version
    runs-on: ubuntu-latest
    container: fedora:44
    steps:
      - name: Install build dependencies
        run: >
          dnf install -y --setopt=install_weak_deps=False git gcc gcc-c++ clang clang-devel
          pkgconf-pkg-config ffmpeg-free-devel pipewire-devel libxkbcommon-devel wayland-devel libX11-devel
          alsa-lib-devel
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --release -p brp
      - name: Stage the tarball
        env:
          VERSION: ${{ needs.version.outputs.version }}
        run: |
          set -eu
          name="brp-$VERSION-linux-x86_64"
          mkdir -p "dist/$name"
          cp target/release/brp LICENSE README.md "dist/$name/"
          tar -C dist -czf "dist/$name.tar.gz" "$name"
      - uses: actions/upload-artifact@v4
        with:
          name: linux
          path: dist/*.tar.gz
  windows:
    needs: version
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: ./.github/actions/windows-ffmpeg
      - run: cargo build --release -p brp
      - name: Stage the zip
        shell: pwsh
        env:
          VERSION: ${{ needs.version.outputs.version }}
        run: |
          $ErrorActionPreference = "Stop"
          $name = "brp-$env:VERSION-windows-x86_64"
          ./ci/stage-windows.ps1 -Destination "dist\$name"
          Compress-Archive -Path "dist\$name" -DestinationPath "dist\$name.zip"
      - uses: actions/upload-artifact@v4
        with:
          name: windows
          path: dist/*.zip
  publish:
    needs: [version, linux, windows]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true
      - name: Checksums and release notes from the tag annotation
        run: |
          set -eu
          (cd dist && sha256sum *.tar.gz *.zip > SHA256SUMS)
          git fetch --no-tags origin "refs/tags/$GITHUB_REF_NAME:refs/tags/$GITHUB_REF_NAME" || true
          git tag -l --format='%(contents)' "$GITHUB_REF_NAME" > notes.md
      - uses: softprops/action-gh-release@v2
        with:
          files: dist/*
          body_path: notes.md
```

- [ ] **Step 5: README**

In `README.md`:

Under `## Usage`, after the first paragraph, add:

```markdown
The Settings button on the start screen and in the status bar opens a dialog
for the nickname, the relay choice (public relays, a custom relay URL, or no
relay), the capture frame rate ceiling, and the audio output device. Settings
apply when the next room opens. The start screen lists recent rooms with a Join
button. Command line flags override the saved settings for that launch and are
not saved.
```

Under `## Identity and privacy`, replace the last sentence ("Nothing else is persisted yet: nickname, relay setting, and recent rooms are entered each launch until the settings phase.") with:

```markdown
Settings are saved beside the identity key in `brp/settings.toml`: nickname,
relay choice, frame rate ceiling, audio output device, and the tickets of recent
rooms. A stored ticket contains the addresses of the peer that issued it, so
treat the file as you would the tickets themselves.
```

Under `## Windows`, where the per-push CI artifact is described, add a sentence: "Tagged releases publish a zip of the same layout and a Linux tarball on the GitHub Releases page, with a `SHA256SUMS` file."

Under `## Linux prerequisites`, add at the end:

```markdown
The Linux release tarball is built on Fedora 44 and links the distribution's
FFmpeg 8 (`libavcodec.so.62`), PipeWire, and portal libraries; it runs on
distributions shipping FFmpeg 8. Elsewhere, build from source.
```

Under `## Development`, add a subsection:

```markdown
### Releasing

Bump `version` in the workspace `Cargo.toml`, commit, and push an annotated tag
whose name is `v` plus that version (`git tag -a v0.1.0 -m "..."`). The release
workflow refuses a tag that does not match the workspace version, builds the
Linux tarball and the Windows zip, and publishes them with checksums and the
tag's annotation as the release notes.
```

In `## Roadmap`, item 5 becomes:

```markdown
5. **Window management and polish** — done: pop-outs and fullscreen, settings
   UI and persistence, tagged releases for Windows and Linux.
```

and after the plan 5a sentence add "settings, persistence, and releases are implemented by [`2026-09-06-plan5b-settings-release.md`](docs/superpowers/plans/2026-09-06-plan5b-settings-release.md)." In the status blockquote near the top, replace "Persistent settings and release packaging are the next steps" with "Settings persist and tagged releases publish Windows and Linux builds; macOS is on the backlog." (keep the rest of the sentence coherent).

- [ ] **Step 6: Spec amendments**

Append to section 13 of the spec (created in plan 5a; create the section if it does not exist):

```markdown
- **Audio settings shape (5.4).** The device lives under `[audio] output_device` as a nested `AudioSettings` struct so the TOML matches the documented shape; the spec's flat `audio_output` field name is not used.
- **Store type (5.4).** `SettingsStore { settings, path, load_error }` carries the path and the load error the start screen shows; `Settings` stays pure.
- **fps flag (5.4).** `WindowArgs.fps` became `Option<u32>` so an absent flag can defer to the file; `brp publish --fps` keeps its default of 60.
- **Start screen (8.2).** Clicking a recent room fills the ticket field and submits Join; a new `StartAction::OpenSettings` opens the dialog and never becomes an intent.
- **Release notes (5.7).** The publish job reads the annotated tag's message with `git tag -l --format='%(contents)'` into the release body; `softprops/action-gh-release` does not read tag annotations by itself.
```

Add any further divergences from Tasks 1 to 5 (for example the device-listing fallback of Task 2 Step 3 or a different TOML rendering of unit variants from Task 3 Step 3).

- [ ] **Step 7: Validate the YAML, commit, push, watch CI**

Run: `python3 -c "import yaml,sys; [yaml.safe_load(open(p)) for p in sys.argv[1:]]; print('ok')" .github/workflows/ci.yml .github/workflows/release.yml .github/actions/windows-ffmpeg/action.yml` (if PyYAML is missing, `ruby -ryaml -e 'ARGV.each { |p| YAML.load_file(p) }; puts "ok"'` with the same paths; if neither exists, skip and rely on CI).

```bash
git add .github/actions/windows-ffmpeg/action.yml ci/stage-windows.ps1 .github/workflows/ci.yml .github/workflows/release.yml README.md docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md
git commit -m "ci: add the tagged release workflow and share the Windows FFmpeg steps"
git push
```

Watch the `ci` run with `gh run watch` (or `gh run list --workflow ci --limit 1`). Both jobs must pass; the Windows job proves the composite action and the staging script. The release workflow itself runs only on a tag: the first release is the user's `v0.1.0`, made after they have run the manual check below.

- [ ] **Step 8: Hand the click-through to the user**

Ask the user to run spec section 10.2's manual check:

1. Open Settings from the start screen; set a nickname, a custom relay URL, and a device; Save; restart; confirm the file and the start screen.
2. Create a room and see it under recent rooms after a restart; join it from the row.
3. Set a device, unplug it, open a room, read the status bar's output error.
4. Optionally push a mismatched tag on a fork or scratch branch (for example `v9.9.9`) to see the version guard fail, then delete the tag.
5. Tag `v0.1.0` when satisfied and confirm the GitHub Release holds the zip, the tarball, and `SHA256SUMS`.

Report anything that fails as a bug before phase 5 is recorded as done.
