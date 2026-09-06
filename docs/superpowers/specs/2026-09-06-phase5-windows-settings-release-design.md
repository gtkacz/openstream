# Phase 5: Window management, settings, and release

Status: approved design, 2026-09-06. Refines phase 5 of `2026-09-04-p2p-screen-sharing-design.md`, which remains the master spec. Where this document is silent, the master spec applies. Where the two differ, this document wins for pop-outs, fullscreen, settings, and packaging.

This phase is implemented by two plans: **5a** covers pop-outs and fullscreen (sections 5.1 to 5.3, 8.1, 9.1, 10.1); **5b** covers settings, persistence, and release packaging (sections 5.4 to 5.7, 8.2, 9.2, 10.2). Phase 4 is in final review on `main` while this document is written; plan 5a starts only after that review is committed. Neither plan touches the files under phase 4 review.

## 1. Goals

- A viewer moves any watched live into its own native window, on any monitor, and makes it borderless fullscreen. Closing the window returns the live to the grid. A live is decoded once whichever window shows it.
- Nickname, relay choice, capture frame rate ceiling, audio output device, and recent rooms persist across launches in one TOML file, edited from an in-app Settings dialog. The identity key stays where phase 3 put it.
- A tagged push publishes a GitHub Release with a Windows zip and a Linux tarball, built from the same steps CI already runs.
- Everything above winit, the file system, and GitHub Actions is unit-tested without a display.

## 2. Non-goals for this phase

- Mirroring a live in two windows at once. Popping out moves it.
- Fullscreen for the main window. The tile's fullscreen button pops the live out and makes that window fullscreen.
- Remembering window positions and sizes, or which lives were popped out, across launches.
- Applying a settings change to an open room. Changes take effect when the next room opens.
- Default bitrate scaling in Settings. The per-live preset editor covers bitrate.
- Installers, Flatpak, AppImage, code signing, auto-update. The Linux tarball links the distribution's FFmpeg.
- Routing frame wake-ups to only the window that shows the live. See section 7.
- Runtime verification on Windows hardware, deferred as in phases 3 and 4.

## 3. Decisions and rationale

| Decision | Rationale |
|---|---|
| Pop-outs are native winit windows sharing one wgpu device, queue, and `TileRenderer`, each with its own surface and egui context | This is the master spec's section 5.6, confirmed against the current code. egui viewports would need the viewport protocol re-implemented on top of egui-winit and still require a surface per OS window under the video. A single-window focus mode would not put a live on a second monitor while the grid stays visible. |
| Popping out moves the live, it never mirrors | Each tile owns one letterbox uniform written once per redraw. Drawing a tile in two windows of different sizes in one frame would need a uniform per placement. Moving keeps the renderer as it is and matches the master spec, which says closing a pop-out returns the live to the grid. |
| Pop-out, fullscreen, and return are `WindowCommand`s, separate from `RoomCommand` | They change windows, not the room. Keeping them apart preserves the rule that panels only describe what they want and never hold room or window handles. |
| Pop-out bookkeeping is a pure module generic over the window id | The mapping between windows and tiles, and what to close when a watch ends, is the part that can go wrong. Generic over the id, it is tested with integers, without winit. |
| Settings are one TOML file beside the identity key, written atomically | The master spec names TOML in the platform config directory. The identity key keeps its own 0600 file so a settings rewrite can never touch it. Temp file plus rename means a crash mid-write leaves the previous file intact. |
| Command line flags override the file for that launch and are never written back | A flag is an instruction for one run; the file is what the user chose in the dialog. Writing flags back would make a one-off `--no-relay` permanent. |
| A corrupt settings file yields defaults and an error on the start screen; nothing is written until the user saves | Silently replacing the file would destroy what the user wrote. A visible error plus an explicit Save is the recovery path. |
| A named audio output device that no longer exists fails the output start | Falling back to the default device silently would play audio somewhere the user did not choose. The status bar already shows the output error; the user fixes it in Settings. |
| Stored device is cpal's `DeviceId`, shown by its description name | cpal 0.18 gives every device a stable id with `Display` and `FromStr` for exactly this purpose, and `device_by_id` to reopen it. Names alone collide and change. |
| Recent rooms store the ticket and a unix timestamp | The ticket is what a rejoin needs. Unix seconds serialize with serde and need no date crate. |
| Release workflow on `v*` tags, with a version guard | A release must match the version the binary reports. Failing when the tag and the workspace version differ is cheaper than an incorrect release. |
| Windows FFmpeg install lives in one composite action and staging in one script, used by CI and release | The pinned BtbN build, the DLL list, and the license copy are defined once. Two workflows carrying the same PowerShell would drift. |
| Linux tarball is built in the Fedora 44 container CI already uses | Same toolchain and headers as every test run. It links `libavcodec.so.62`, so it runs on distributions shipping FFmpeg 8; the README says so. |

## 4. Product model additions

- **Pop-out.** A native window showing exactly one watched live, with the same hover overlay as a grid tile plus fullscreen and return controls. Popping out removes the live from the grid; closing the pop-out puts it back.
- **Fullscreen.** A pop-out in borderless fullscreen on its current monitor. F11 toggles it, Esc leaves it. From a grid tile, the fullscreen button pops out and goes fullscreen in one step.
- **Settings.** Nickname, relay choice (default, custom URL, disabled), capture fps ceiling, audio output device. Saved to disk; applied when the next room opens.
- **Recent rooms.** The last rooms created or joined, newest first, listed on the start screen with a Join button. Tickets embed peer addresses; the README notes it.
- **Release.** A GitHub Release per `v*` tag with `brp-<version>-windows-x86_64.zip`, `brp-<version>-linux-x86_64.tar.gz`, and `SHA256SUMS`.

## 5. Architecture

### 5.1 `render` (plan 5a)

`GpuContext` becomes the shared half only: `instance`, `adapter`, `device`, `queue`, and `format`, the main window's surface format. It is created once with the main window as the compatible surface.

A new `render/surface.rs` holds the per-window half:

```
WindowSurface {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ui: EguiLayer,
}
WindowSurface::new(gpu: &GpuContext, window: Arc<Window>) -> Result<Self, AppError>
WindowSurface::resize(&mut self, gpu: &GpuContext, width: u32, height: u32)
```

`new` configures the surface with `gpu.format`; if the surface's capabilities do not list that format, it returns `AppError::Window` and the caller shows the message in the status line. `TileRenderer` and `EguiLayer` are unchanged; `TileRenderer` is built once from `gpu.format` and shared by every window.

### 5.2 Pop-out bookkeeping (plan 5a)

`crates/app/src/popouts.rs`, pure and generic over the window id:

```
PopOuts<Id: Copy + Eq + Hash> { by_window: HashMap<Id, TileKey> }

fn insert(&mut self, id: Id, key: TileKey)
fn key_of(&self, id: Id) -> Option<TileKey>
fn window_of(&self, key: TileKey) -> Option<Id>
fn is_popped(&self, key: TileKey) -> bool
fn remove_window(&mut self, id: Id) -> Option<TileKey>
fn remove_key(&mut self, key: TileKey) -> Option<Id>
/// Windows whose live is no longer watched; the caller closes them.
fn retain_watched(&mut self, watched: &HashSet<TileKey>) -> Vec<Id>
fn popped(&self) -> HashSet<TileKey>
```

A key is in at most one window; `insert` on an already popped key is a no-op returning the existing window through `window_of`.

### 5.3 `window.rs` and `room_view.rs` (plan 5a)

`App` holds `main: Option<WindowSurface>`, `popouts: PopOuts<WindowId>`, `popout_surfaces: HashMap<WindowId, WindowSurface>`, and `fullscreen: HashSet<WindowId>` (which pop-outs are fullscreen, read by the overlay to label its toggle). `gpu` and `tiles` stay single.

- `window_event` looks the id up: the main window keeps today's path; a pop-out routes the event to its own egui layer, then handles `CloseRequested` (return the live to the grid and drop the surface), `Resized`, and `RedrawRequested`.
- `redraw(window_id)` renders one window. The main window's egui pass receives the popped set and the tile grid skips those keys. A pop-out's pass draws `ui::popout::draw`, which returns one placement covering the central panel. Both passes end with the same encoder, tiles, and egui paint sequence as today, against that window's surface.
- `UiOutput` gains `window_commands: Vec<WindowCommand>`:

```
enum WindowCommand {
    PopOut(TileKey),
    PopOutFullscreen(TileKey),
    ToggleFullscreen(TileKey),
    ReturnToGrid(TileKey),
}
```

  `App` applies them after the egui pass with the `ActiveEventLoop` the redraw handler already has. `PopOut` creates a window titled `brp: <live title>` at the main window's default size, builds its `WindowSurface`, and records it; on failure the status line gets the error and the live stays in the grid. `PopOutFullscreen` does the same and then sets `Fullscreen::Borderless(None)`. `ToggleFullscreen` flips between borderless and none. `ReturnToGrid` drops the surface and the bookkeeping entry; winit closes the window when its last `Arc` drops.
- `RoomView::refresh` continues to compute the live watch set; `App` passes it to `popouts.retain_watched` and closes the returned windows. An `Unwatch` for a popped key is applied to the room as today and the pop-out closes on the next refresh.
- `AppEvent::NewFrame`, `RoomChanged`, and `Tick` request a redraw of every window. `RoomOpened` and `ShareFinished` are main-window concerns and unchanged. Leaving the room phase (not possible today without closing the app) is out of scope.
- `App::finish` is unchanged: pop-out surfaces drop with the `App`.

### 5.4 `settings.rs` (plan 5b)

```
Settings {
    nickname: Option<String>,
    relay: RelayChoice,          // Default | Custom(String) | Disabled
    fps: u32,                    // default DEFAULT_FPS
    audio_output: Option<String>,// cpal DeviceId in Display form
    recent_rooms: Vec<RecentRoom>,
}
RecentRoom { ticket: String, last_joined_unix: u64 }

Settings::path() -> Result<PathBuf, AppError>            // config_dir/settings.toml
Settings::load(path) -> Result<Settings, AppError>       // missing file is defaults; AppError::Settings otherwise
Settings::save(&self, path) -> Result<(), AppError>       // temp file + rename
Settings::remember_room(&mut self, ticket: &str, now_unix: u64)   // upsert to front, cap RECENT_ROOMS_MAX
Settings::validate(&self) -> Result<(), String>          // fps >= 1, custom relay URL parses as iroh::RelayUrl
```

TOML shape:

```toml
nickname = "alice"
fps = 60

[relay]
mode = "custom"            # "default" | "custom" | "disabled"
url = "https://relay.example.com/"

[audio]
output_device = "PipeWire:alsa_output.usb-…"

[[recent_rooms]]
ticket = "brp…"
last_joined_unix = 1788000000
```

Unknown keys are ignored so older binaries can read newer files. `toml` 1.1 and `serde` are already in the lock file; `toml` is added to the workspace and app manifests.

`Launch` is built from `Settings` then `WindowArgs`: a flag that is present replaces the file's value for this run. `RelaySetting` in `brp-net` gains `Custom(RelayUrl)` and loses `Copy`; `bind_endpoint` maps it to `Endpoint::builder(presets::N0).relay_mode(RelayMode::Custom(RelayMap::from(url)))`. Call sites that compared by value clone instead.

`App` holds `settings: Settings` and `settings_path: PathBuf`. It saves after a successful open (recent room, and the nickname typed on the start screen when no `--nickname` flag was given) and when the dialog's Save succeeds. Save failures land in the status line or the dialog's error text; they never abort the open.

### 5.5 `audio` (plan 5b)

```
pub struct OutputDevice { pub id: String, pub name: String }
pub fn output_devices() -> Result<Vec<OutputDevice>, AudioError>
impl CpalOutput { pub fn new(device: Option<String>) -> Self }
```

`output_devices` lists the default host's output devices by `DeviceId` display string and description name. `CpalOutput::new(None)` keeps today's default-device behaviour; `Some(id)` parses the id and opens that device through `device_by_id`, returning `AudioError::Device("output device <id> not found")` when absent or unparsable. `CpalOutput` stops being a unit struct; the `publish` path constructs it with `None`.

### 5.6 `ui` (plan 5b)

`ui/settings.rs` holds `SettingsDialog { open: bool, draft: Settings, devices: Vec<OutputDevice>, error: String }` and `draw(ctx, dialog, room_open: bool) -> Option<Settings>`, returning the settings to persist when Save is clicked and validation passes. Devices are enumerated when the dialog opens. The start screen and the status bar each gain a Settings button that opens it with a fresh draft copied from the saved settings.

`ui/start.rs` lists recent rooms below the ticket field: relative age and a Join button that submits that ticket through the existing `StartAction::Join` path.

### 5.7 Release workflow (plan 5b)

- `.github/actions/windows-ffmpeg/action.yml`: composite action holding today's "Ensure LLVM" and "Install FFmpeg" steps, exporting `FFMPEG_DIR` and extending `PATH`. `ci/stage-windows.ps1`: today's staging step, taking the destination directory as a parameter. `ci.yml` switches to both.
- `.github/workflows/release.yml`, `on: push: tags: ['v*']`:
  - `version`: reads the workspace version with `cargo metadata --no-deps` and fails unless it equals the tag without its `v`; outputs the version.
  - `linux`: the Fedora 44 container and package list from `ci.yml`, `cargo build --release -p brp`, stages `brp`, `LICENSE`, `README.md` into `brp-<version>-linux-x86_64/`, uploads the tarball as an artifact.
  - `windows`: the composite action, `cargo build --release -p brp`, the staging script, uploads `brp-<version>-windows-x86_64.zip`.
  - `publish`: downloads both, writes `SHA256SUMS`, creates the release with `softprops/action-gh-release@v2` attaching the three files, with the tag's annotation as the body.
- `brp --version` prints the workspace version through clap, so the guard covers what the binary reports.

## 6. Protocol

No wire changes. `PROTOCOL_VERSION` stays 1.

## 7. Data flow

Frames flow as in phase 2: the decoder thread fills the watch's slot and sends `NewFrame` through the proxy. `App` requests a redraw of every window; at redraw, `RoomView::upload_frames` uploads each slot's newest frame once into the shared `TileRenderer`, and each window draws only the tiles it places. A pop-out therefore redraws the main window at the pop-out's frame rate. The main window already redraws at the highest watched rate when it holds tiles, so the added cost is one egui pass per extra window per frame. Carrying the live's key in the frame notification would let `App` wake only the window that shows it; that needs a change to the room's `FrameNotify`, which is under phase 4 review, and is left as a follow-up.

Settings flow: `participant::run` loads the file, builds `Launch` from settings and flags, and hands both to `App`. The dialog edits a draft; Save validates, persists, and replaces `App.settings`. `open_room` reads relay, fps, and device from `Launch` at open time only.

## 8. User interface

### 8.1 Windows (plan 5a)

- **Tile overlay.** Two buttons after `stats`: `pop out` and `fullscreen`.
- **Pop-out.** The video fills the window. On hover, the same bar as a tile: title, preset selector, volume when the live carries audio, stats toggle, then `fullscreen` or `windowed`, and `back to grid`. F11 toggles fullscreen and Esc leaves it, read from egui's input in the pop-out pass so a focused widget does not swallow them differently from the buttons.
- **Grid.** A popped-out live disappears from the grid; the members panel still shows it watched and its checkbox still unwatches it.
- **Status.** Watch states (connecting, reconnecting, ended) draw centred in the pop-out as they do in a tile.

### 8.2 Settings and start screen (plan 5b)

- **Settings dialog.** An `egui::Window`, not collapsible, with: nickname; relay as three radio buttons and a URL field enabled for custom; fps ceiling as a drag value; audio output as a combo box with "System default" and each enumerated device by name; Save and Cancel; a red error line. When a room is open, a note reads "applies to the next room".
- **Start screen.** A Settings button top right. Under the ticket field, up to `RECENT_ROOMS_MAX` rows: "joined 3 min ago" style age and a Join button. Ticket text is not shown in full; the row shows the first and last few characters. A corrupt settings file shows its error above the form.
- **Status bar.** A Settings button after the master mute.

## 9. Error handling

### 9.1 Windows

- **Pop-out creation fails** (window or surface): status line message; the live stays in the grid.
- **Surface format unsupported** on a pop-out: same, with the format named.
- **Surface lost or outdated** on any window: that redraw is skipped, as today; the next `Resized` reconfigures.
- **Watch ends while popped out** (publisher left, unwatch, refusal): the pop-out closes on the next refresh; nothing is shown in the main window beyond today's behaviour.
- **Fullscreen request refused by the compositor**: winit reports nothing; the toggle label follows the requested state and the user can toggle again. No retry.

### 9.2 Settings and release

- **Settings file missing**: defaults, no message.
- **Settings file unreadable or invalid TOML**: defaults; the start screen shows the path and the parse error; no write until Save.
- **Save fails** (permissions, disk): the dialog shows the error and stays open; when saving after an open, the status line shows it.
- **Custom relay URL invalid**: Save is refused with the parse error.
- **Named audio device missing at room open**: the output start fails with a message naming the id; the room records it as today's `audio_output_error` and subscribes without audio.
- **Tag does not match the workspace version**: the release workflow fails in its first job and publishes nothing.

## 10. Testing

### 10.1 Plan 5a

- **Unit.** `PopOuts`: insert then `key_of` and `window_of` agree; a second insert of the same key is a no-op; `remove_window` and `remove_key` are inverses; `retain_watched` returns exactly the windows whose keys are absent and keeps the rest; `popped` matches the live set. `ui::state::visible_watches(snapshot, popped)`: returns the ordered watches minus the popped keys, and the grid draws from it; tested directly. `WindowCommand` emission from the overlay is exercised by the manual check, as all egui widget code is today.
- **Manual on Linux.** Watch two lives; pop one out; move it to the second monitor; F11, Esc, the overlay toggle; close it and see it return to the grid; unwatch a popped live from the members panel and see the window close; stop the publisher and see the window close. Check the preset selector and volume slider work in the pop-out.

### 10.2 Plan 5b

- **Unit.** `Settings`: defaults round trip through TOML; every field round trips; unknown keys are ignored; a missing file loads defaults; invalid TOML returns the error and does not touch the file; `save` leaves no temp file and replaces atomically (observed by reading back); `remember_room` moves a repeated ticket to the front with the new timestamp and caps the list; `validate` rejects fps 0 and a malformed relay URL and accepts a valid one. `Launch` from settings plus flags: each flag overrides its field, absent flags keep the file's value. `RelayChoice` to `RelaySetting`: default, disabled, and a valid custom URL map to their variants; the existing `bind_endpoint` test with relays disabled stays. `brp-audio`: `CpalOutput::new(Some("nonsense"))` fails with the not-found error without touching a device (parse failure path).
- **Workflow.** The version guard is a shell step; it is checked by pushing a mismatched tag once on a fork or by running the step locally with `GITHUB_REF_NAME` set. The first real release is `v0.1.0`.
- **Manual on Linux.** Open Settings from the start screen, set a nickname, custom relay URL, and a device; save; restart; confirm the file and the start screen; create a room and see it in recent rooms; join it from the row after a restart. Set a device, unplug it, open a room, read the status bar.
- **Deferred to Windows hardware.** Pop-outs and fullscreen on Windows, the WASAPI device list, the zip on a clean machine.

## 11. Constants added in this phase

| Constant | Value | Rationale |
|---|---|---|
| `RECENT_ROOMS_MAX` | 8 | Enough to cover a week of rooms for a small group without scrolling the start screen |
| `SETTINGS_FILE` | `settings.toml` | Beside `identity.key` in the platform config directory |
| `POPOUT_DEFAULT_SIZE` | 1280 × 720 | The main window's default size, already used in `window.rs` and promoted to a constant shared by both |

## 12. References

Verified on 2026-09-06 against the registry sources in the lock file:

- `winit` 0.30.13: `Fullscreen::Borderless(None)` uses the current monitor; `WindowId: From<u64>` exists for tests, though the bookkeeping module is generic anyway. Multiple windows share one `ActiveEventLoop`; `window_event` receives the `WindowId`.
- `wgpu` 30: a `Surface` is created per window from the shared `Instance`; `get_capabilities(&adapter).formats` lists what the pop-out may be configured with.
- `egui` 0.36.1: `egui::Window::new(..).open(&mut bool).collapsible(false).resizable(false)`; one `egui::Context` per OS window, as the current `EguiLayer` already does.
- `cpal` 0.18.2: `DeviceId` with `Display` and `FromStr` "for persisting"; `HostTrait::device_by_id`, `HostTrait::output_devices`, `DeviceTrait::description().name()`.
- `iroh` 1.1.0: `RelayMode::Custom(RelayMap)`; `iroh-relay` 1.1.0 implements `From<RelayUrl> for RelayMap`; `EndpointBuilder::relay_mode`.
- `toml` 1.1.5 and `serde` 1.0 are already in `Cargo.lock` as transitive dependencies.
- `softprops/action-gh-release@v2` attaches files listed under `files:` to the release for the pushed tag.
