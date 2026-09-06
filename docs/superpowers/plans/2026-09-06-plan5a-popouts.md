# Plan 5a: Pop-outs and Fullscreen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A viewer moves any watched live into its own native window, on any monitor, makes it borderless fullscreen, and gets it back in the grid when the window closes, with the live decoded once whichever window shows it.

**Architecture:** The wgpu device, queue, and `TileRenderer` become shared across windows; each window owns its surface, configuration, and egui context in a new `WindowSurface`. `App` keeps the main window plus a map of pop-out windows keyed by `WindowId`, routed by a pure `PopOuts` bookkeeping module that maps windows to tile keys and says which windows to close when a watch ends. Panels emit `WindowCommand`s (pop out, fullscreen, return) beside the existing `RoomCommand`s; `App` applies them after the egui pass with the event loop it already holds. Popping out moves the live: the grid skips popped keys, so a tile is drawn by exactly one window per frame.

**Tech Stack:** Rust 2024, winit 0.30.13, wgpu 30, egui 0.36.1 with egui-winit and egui-wgpu 0.36, iroh 1.1 (`PublicKey` in `TileKey`).

**Spec:** `docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md`, sections 5.1 to 5.3, 7, 8.1, 9.1, 10.1, 11, refining `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md` section 5.6. Read both.

## Global Constraints

- **Start gate.** Phase 4 is in final review on `main` with uncommitted edits in the working tree. Before Task 1, run `git status --short`; if any file outside this plan's file list is modified or staged, stop and ask the user whether the phase 4 review has landed. Do not start until it has. If the user says to proceed while unrelated edits remain, every commit in this plan must use explicit pathspecs: `git add <files> && git commit -m "..." -- <files>`.
- Popping out moves a live; it is never shown in two windows at once. The grid skips popped keys; a pop-out shows exactly one key.
- `WindowCommand` and `RoomCommand` stay separate enums. Panels never hold a `Window`, a `Room`, or the event loop.
- `TileRenderer` stays single and lives on `App`, not on a window. `GpuContext` holds nothing per window.
- Every pop-out surface is configured with the main window's format (`GpuContext::format`); an unsupported format refuses the pop-out with a status line message. No pipeline per format.
- No wire changes. Nothing under `crates/room`, `crates/net`, `crates/pipeline`, `crates/audio`, `crates/proto` changes in this plan.
- Comments explain why. Doc comments state contracts on new public items. No task ids in code.
- One Conventional Commit per task, imperative subject, no co-author lines. The `Claude-Session:` trailer the harness requires is expected; no other trailers. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass before each commit. Before committing, run `git status --short`; if `.vscode/` files appear staged, run `git rm --cached -r .vscode` first (recurring environment quirk).
- Run tests with `cargo test -p brp` (the app crate is named `brp`, library `brp_app`). Nothing in this plan needs a display to test; the manual click-through in Task 5 is the user's.
- Verified library facts this plan relies on. winit 0.30.13: `ActiveEventLoop::create_window(WindowAttributes) -> Result<Window, OsError>`, `Window::default_attributes().with_title(..).with_inner_size(PhysicalSize<u32>)`, `Window::id() -> WindowId`, `Window::set_fullscreen(Option<Fullscreen>)`, `Fullscreen::Borderless(None)` targets the current monitor, `PhysicalSize::new` is `const fn`, `ApplicationHandler::window_event(&mut self, &ActiveEventLoop, WindowId, WindowEvent)` receives the id and the loop, so windows can be created from the redraw handler. wgpu 30: one `Surface` per window from the shared `Instance::create_surface(Arc<Window>)`; `Surface::get_capabilities(&Adapter).formats`; `Surface::get_default_config(&Adapter, w, h) -> Option<SurfaceConfiguration>`. egui 0.36.1: `Ui::input(|i| i.key_pressed(egui::Key::F11))`, `egui::Key::Escape`, `egui::Frame::NONE`, `egui::Popup::is_any_open(ctx)`; one `egui::Context` per OS window, which `EguiLayer` already provides.

## File Structure

```
crates/app/src/commands.rs          + WindowCommand
crates/app/src/popouts.rs           new: PopOuts<Id> bookkeeping (pure, tested)
crates/app/src/lib.rs               + pub mod popouts
crates/app/src/render/mod.rs        GpuContext becomes shared-only (instance, adapter, device, queue, format)
crates/app/src/render/surface.rs    new: WindowSurface (window, surface, config, egui layer)
crates/app/src/ui/state.rs          + visible_watches, live_title (pure, tested)
crates/app/src/ui/mod.rs            UiOutput.window_commands; draw takes the popped set; pub mod popout
crates/app/src/ui/tiles.rs          overlay becomes pub tile_overlay with a Placement; grid skips popped keys; pop out and fullscreen buttons
crates/app/src/ui/popout.rs         new: a pop-out window's egui pass (tested key rule)
crates/app/src/room_view.rs         + watched_keys
crates/app/src/window.rs            multi-window App: main + pop-outs, WindowCommand application, DEFAULT_WINDOW_SIZE
README.md                           pop-out and fullscreen paragraph; roadmap item 5 partial
docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md   + section 13 amendments
```

---

### Task 1: Window commands, pop-out bookkeeping, and the visible-watch helpers

**Files:**
- Modify: `crates/app/src/commands.rs`
- Create: `crates/app/src/popouts.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/ui/state.rs`
- Modify: `crates/app/src/ui/mod.rs` (the `UiOutput` struct only)

**Interfaces:**
- Consumes: `TileKey = (PublicKey, u32)` from `crate::render::tiles`; `RoomSnapshot`, `WatchView` from `brp_room`; `ordered_watches` in `ui/state.rs`.
- Produces: `WindowCommand::{PopOut(TileKey), PopOutFullscreen(TileKey), ToggleFullscreen(TileKey), ReturnToGrid(TileKey)}`; `PopOuts<Id>` with `new, insert(id, key) -> bool, key_of(id) -> Option<TileKey>, window_of(key) -> Option<Id>, is_popped(key) -> bool, remove_window(id) -> Option<TileKey>, remove_key(key) -> Option<Id>, retain_watched(&HashSet<TileKey>) -> Vec<Id>, popped() -> HashSet<TileKey>, is_empty() -> bool`; `ui::state::visible_watches(&RoomSnapshot, &HashSet<TileKey>) -> Vec<&WatchView>`; `ui::state::live_title(&RoomSnapshot, TileKey) -> Option<String>`; `UiOutput.window_commands: Vec<WindowCommand>`.

- [ ] **Step 1: Add `WindowCommand`**

Append to `crates/app/src/commands.rs`:

```rust
/// A command a panel wants applied to the windows, not the room: which live gets its own window
/// and whether that window is fullscreen. Queued and drained after the egui pass like
/// [`RoomCommand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    /// Moves the live out of the grid into a new window.
    PopOut(TileKey),
    /// Moves the live into a new window that starts borderless fullscreen.
    PopOutFullscreen(TileKey),
    /// Flips the live's pop-out between borderless fullscreen and windowed.
    ToggleFullscreen(TileKey),
    /// Closes the live's pop-out and puts it back in the grid.
    ReturnToGrid(TileKey),
}
```

In `crates/app/src/ui/mod.rs`, extend `UiOutput`:

```rust
/// Everything a completed egui pass produced: commands to apply to the room, commands to apply to
/// the windows, and where the video renderer should place each watched live's frame.
#[derive(Debug, Default)]
pub struct UiOutput {
    pub commands: Vec<RoomCommand>,
    pub window_commands: Vec<WindowCommand>,
    /// Where the video renderer draws each watched live, in egui points.
    pub tile_rects: Vec<(TileKey, egui::Rect)>,
}
```

and change the import line to `use crate::commands::{RoomCommand, WindowCommand};`. In `draw`, the struct literal at the end gains `window_commands: Vec::new(),` for now (Task 3 fills it).

- [ ] **Step 2: Write the failing bookkeeping tests**

Create `crates/app/src/popouts.rs`:

```rust
//! Which watched lives are shown in a window of their own. Pure bookkeeping between window ids
//! and tile keys, generic over the id so it is tested with integers instead of winit windows.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::render::tiles::TileKey;

/// The pop-out windows: one tile key per window, and each key in at most one window.
#[derive(Debug)]
pub struct PopOuts<Id> {
    by_window: HashMap<Id, TileKey>,
}

impl<Id: Copy + Eq + Hash> PopOuts<Id> {
    pub fn new() -> Self {
        Self {
            by_window: HashMap::new(),
        }
    }

    /// Records that window `id` shows `key`. Returns false, and changes nothing, when the key is
    /// already popped out: a live is never in two windows.
    pub fn insert(&mut self, id: Id, key: TileKey) -> bool {
        if self.is_popped(key) {
            return false;
        }
        self.by_window.insert(id, key);
        true
    }

    pub fn key_of(&self, id: Id) -> Option<TileKey> {
        self.by_window.get(&id).copied()
    }

    pub fn window_of(&self, key: TileKey) -> Option<Id> {
        self.by_window
            .iter()
            .find_map(|(id, k)| (*k == key).then_some(*id))
    }

    pub fn is_popped(&self, key: TileKey) -> bool {
        self.window_of(key).is_some()
    }

    pub fn remove_window(&mut self, id: Id) -> Option<TileKey> {
        self.by_window.remove(&id)
    }

    pub fn remove_key(&mut self, key: TileKey) -> Option<Id> {
        let id = self.window_of(key)?;
        self.by_window.remove(&id);
        Some(id)
    }

    /// Forgets every window whose live is not in `watched` and returns those ids so the caller
    /// closes the windows.
    pub fn retain_watched(&mut self, watched: &HashSet<TileKey>) -> Vec<Id> {
        let closed: Vec<Id> = self
            .by_window
            .iter()
            .filter(|(_, key)| !watched.contains(key))
            .map(|(id, _)| *id)
            .collect();
        for id in &closed {
            self.by_window.remove(id);
        }
        closed
    }

    /// The keys the grid must skip.
    pub fn popped(&self) -> HashSet<TileKey> {
        self.by_window.values().copied().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.by_window.is_empty()
    }
}

impl<Id: Copy + Eq + Hash> Default for PopOuts<Id> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn key(n: u8) -> TileKey {
        (SecretKey::from_bytes(&[n; 32]).public(), u32::from(n))
    }

    #[test]
    fn a_popped_key_is_found_from_both_sides() {
        let mut popouts = PopOuts::new();
        assert!(popouts.insert(1u32, key(1)));
        assert_eq!(popouts.key_of(1), Some(key(1)));
        assert_eq!(popouts.window_of(key(1)), Some(1));
        assert!(popouts.is_popped(key(1)));
        assert!(!popouts.is_popped(key(2)));
        assert_eq!(popouts.key_of(2), None);
    }

    #[test]
    fn a_key_is_never_in_two_windows() {
        let mut popouts = PopOuts::new();
        assert!(popouts.insert(1u32, key(1)));
        assert!(!popouts.insert(2, key(1)));
        assert_eq!(popouts.window_of(key(1)), Some(1));
        assert_eq!(popouts.key_of(2), None);
        assert_eq!(popouts.popped(), HashSet::from([key(1)]));
    }

    #[test]
    fn removing_by_window_and_by_key_are_inverses() {
        let mut popouts = PopOuts::new();
        popouts.insert(1u32, key(1));
        popouts.insert(2, key(2));
        assert_eq!(popouts.remove_window(1), Some(key(1)));
        assert_eq!(popouts.remove_window(1), None);
        assert_eq!(popouts.remove_key(key(2)), Some(2));
        assert_eq!(popouts.remove_key(key(2)), None);
        assert!(popouts.is_empty());
    }

    #[test]
    fn retain_watched_closes_exactly_the_windows_whose_live_ended() {
        let mut popouts = PopOuts::new();
        popouts.insert(1u32, key(1));
        popouts.insert(2, key(2));
        popouts.insert(3, key(3));
        let watched = HashSet::from([key(1), key(3), key(9)]);
        let mut closed = popouts.retain_watched(&watched);
        closed.sort_unstable();
        assert_eq!(closed, [2]);
        assert_eq!(popouts.popped(), HashSet::from([key(1), key(3)]));
        assert!(popouts.retain_watched(&watched).is_empty());
    }
}
```

Add `pub mod popouts;` to `crates/app/src/lib.rs`, alphabetically among the existing modules.

- [ ] **Step 3: Run the bookkeeping tests**

Run: `cargo test -p brp popouts`
Expected: 4 passed.

- [ ] **Step 4: Write the failing visible-watch and title tests**

In `crates/app/src/ui/state.rs`, after `ordered_watches`, add:

```rust
/// The watches the grid draws: [`ordered_watches`] minus the keys shown in a pop-out window.
pub fn visible_watches<'a>(
    snapshot: &'a RoomSnapshot,
    popped: &HashSet<TileKey>,
) -> Vec<&'a WatchView> {
    ordered_watches(snapshot)
        .into_iter()
        .filter(|w| !popped.contains(&(w.publisher, w.live_id)))
        .collect()
}

/// The live's title as its publisher advertises it, or `None` once the publisher is gone.
pub fn live_title(snapshot: &RoomSnapshot, key: TileKey) -> Option<String> {
    snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1))
        .map(|l| l.title.clone())
}
```

In the `tests` module of the same file, add (it already imports `SecretKey`, `PathKind`, `LiveInfo`, and the snapshot pieces; add `use brp_room::{WatchState, WatchView};` and `use brp_proto::SourceKind;` if not already imported there):

```rust
    fn watch(publisher: iroh::PublicKey, live_id: u32) -> WatchView {
        WatchView {
            publisher,
            live_id,
            preset_id: 1,
            state: WatchState::Live,
            frames_decoded: 0,
            keyframe_requests: 0,
            audio: false,
        }
    }

    #[test]
    fn visible_watches_skip_popped_keys_and_keep_the_order() {
        let a = SecretKey::from_bytes(&[1u8; 32]).public();
        let b = SecretKey::from_bytes(&[2u8; 32]).public();
        let mut snapshot = snapshot_with(Vec::new());
        snapshot.watches = vec![watch(b, 1), watch(a, 2), watch(a, 1)];
        let popped = HashSet::from([(a, 2)]);
        let keys: Vec<TileKey> = visible_watches(&snapshot, &popped)
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        // Key order follows the derived public key bytes, not the seed bytes, so sort the
        // expectation the same way `ordered_watches` does.
        let mut expected = vec![(a, 1), (b, 1)];
        expected.sort_by(|x, y| x.0.as_bytes().cmp(y.0.as_bytes()).then(x.1.cmp(&y.1)));
        assert_eq!(keys, expected);
        assert_eq!(visible_watches(&snapshot, &HashSet::new()).len(), 3);
    }

    #[test]
    fn live_title_follows_the_publisher_and_vanishes_with_it() {
        let mut publisher = member("bob");
        publisher.lives.push(LiveInfo {
            id: 4,
            title: "desk".into(),
            kind: SourceKind::Monitor,
            source_width: 64,
            source_height: 32,
            source_fps: 30,
            has_audio: false,
            presets: Vec::new(),
        });
        let id = publisher.id;
        let mut snapshot = snapshot_with(Vec::new());
        snapshot.members = vec![publisher];
        assert_eq!(live_title(&snapshot, (id, 4)), Some("desk".to_string()));
        assert_eq!(live_title(&snapshot, (id, 5)), None);
        snapshot.members.clear();
        assert_eq!(live_title(&snapshot, (id, 4)), None);
    }
```

- [ ] **Step 5: Run the state tests**

Run: `cargo test -p brp ui::state`
Expected: all pass, including the two new ones.

- [ ] **Step 6: Format, lint, commit**

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
git add crates/app/src/commands.rs crates/app/src/popouts.rs crates/app/src/lib.rs crates/app/src/ui/state.rs crates/app/src/ui/mod.rs
git commit -m "feat: add window commands and pop-out bookkeeping"
```

---

### Task 2: Split the GPU context into a shared half and a per-window surface

**Files:**
- Modify: `crates/app/src/render/mod.rs`
- Create: `crates/app/src/render/surface.rs`
- Modify: `crates/app/src/window.rs` (main window only; behaviour unchanged)

**Interfaces:**
- Consumes: `EguiLayer::new(&Window, &wgpu::Device, wgpu::TextureFormat)` and its `run/prepare/paint/cleanup/on_window_event` from `render/ui.rs` (unchanged).
- Produces: `GpuContext { instance, adapter, device, queue, format }` with `GpuContext::new(&ActiveEventLoop, Arc<Window>) -> Result<(GpuContext, WindowSurface), AppError>`; `WindowSurface { pub window: Arc<Window>, pub ui: EguiLayer, .. }` with `WindowSurface::new(&GpuContext, Arc<Window>) -> Result<Self, AppError>`, `resize(&mut self, &GpuContext, u32, u32)`, `size(&self) -> (u32, u32)`, `acquire(&self) -> Option<wgpu::SurfaceTexture>`.

- [ ] **Step 1: Rewrite `render/mod.rs`**

Replace the file's contents with:

```rust
//! wgpu device plus the tile and egui renderers. `GpuContext` is the half every window shares;
//! `surface::WindowSurface` is the half each window owns.
pub mod grid;
pub mod surface;
pub mod tiles;
pub mod ui;

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::error::AppError;
use surface::WindowSurface;

/// The GPU state shared by every window: one instance, adapter, device, and queue, and the
/// surface format every window is configured with so one tile pipeline serves them all.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
}

impl GpuContext {
    /// Picks the adapter against the main window's surface and returns that surface configured.
    /// The main window's surface is created here, not by [`WindowSurface::new`], because wgpu
    /// needs it to choose a compatible adapter and a window has at most one surface.
    pub fn new(
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
    ) -> Result<(Self, WindowSurface), AppError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| AppError::Window(format!("create_surface: {e}")))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| AppError::Window(format!("no suitable GPU adapter: {e}")))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("brp"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| AppError::Window(format!("request_device: {e}")))?;
        let format =
            egui_wgpu::preferred_framebuffer_format(&surface.get_capabilities(&adapter).formats)
                .map_err(|e| AppError::Window(format!("no usable surface format: {e}")))?;
        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
        };
        let main = WindowSurface::configure(&gpu, window, surface)?;
        Ok((gpu, main))
    }
}
```

- [ ] **Step 2: Create `render/surface.rs`**

```rust
//! The half of the GPU state each window owns: its surface, the surface configuration, and its
//! egui context. Every surface uses the shared format so the one tile pipeline draws into any.

use std::sync::Arc;

use winit::window::Window;

use super::GpuContext;
use super::ui::EguiLayer;
use crate::error::AppError;

pub struct WindowSurface {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub ui: EguiLayer,
}

impl WindowSurface {
    /// A surface for a window other than the main one, configured with the shared format.
    /// Refused when this surface cannot take that format, so the caller keeps the live in the
    /// grid and tells the user rather than building a second pipeline.
    pub fn new(gpu: &GpuContext, window: Arc<Window>) -> Result<Self, AppError> {
        let surface = gpu
            .instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| AppError::Window(format!("create_surface: {e}")))?;
        Self::configure(gpu, window, surface)
    }

    pub(super) fn configure(
        gpu: &GpuContext,
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
    ) -> Result<Self, AppError> {
        let formats = surface.get_capabilities(&gpu.adapter).formats;
        if !formats.contains(&gpu.format) {
            return Err(AppError::Window(format!(
                "surface does not support {:?} (offers {formats:?})",
                gpu.format
            )));
        }
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| AppError::Window("surface is not supported".into()))?;
        config.format = gpu.format;
        surface.configure(&gpu.device, &config);
        let ui = EguiLayer::new(&window, &gpu.device, gpu.format);
        Ok(Self {
            window,
            surface,
            config,
            ui,
        })
    }

    pub fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&gpu.device, &self.config);
        }
    }

    /// Current surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The texture to draw this frame into, or `None` when the surface is lost or outdated; the
    /// next `Resized` reconfigures it, so a skipped frame is the right response.
    pub fn acquire(&self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            _ => None,
        }
    }
}
```

- [ ] **Step 3: Adapt `window.rs` to the split, main window only**

In `crates/app/src/window.rs`:

Replace the imports `use crate::render::{GpuContext, ui::EguiLayer};` with

```rust
use crate::render::GpuContext;
use crate::render::surface::WindowSurface;
```

In `App`, replace the four fields

```rust
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    tiles: Option<TileRenderer>,
    ui: Option<EguiLayer>,
```

with

```rust
    gpu: Option<GpuContext>,
    main: Option<WindowSurface>,
    tiles: Option<TileRenderer>,
```

and in `App::new` replace `window: None, gpu: None, tiles: None, ui: None,` with `gpu: None, main: None, tiles: None,`.

Rewrite `redraw` so it uses the surface:

```rust
    fn redraw(&mut self) {
        if let Phase::Room(view) = &mut self.phase {
            view.refresh(&mut self.state, self.tiles.as_mut());
        }
        let (Some(gpu), Some(main), Some(tiles)) =
            (self.gpu.as_ref(), self.main.as_mut(), self.tiles.as_mut())
        else {
            return;
        };
        if let Phase::Room(view) = &self.phase {
            view.upload_frames(gpu, tiles);
        }
        let Some(surface) = main.acquire() else {
            return;
        };
        let target = surface.texture.create_view(&Default::default());
        let size = main.size();

        let mut output = UiOutput::default();
        let mut start_action = None;
        let mut ui_frame = main.ui.run(&main.window, [size.0, size.1], |root| match &self.phase {
            Phase::Start => start_action = start::draw(root, &mut self.start),
            Phase::Room(view) => {
                output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state);
            }
        });
        let pixels_per_point = ui_frame.screen.pixels_per_point;
        let placements: Vec<(TileKey, PixelRect)> = output
            .tile_rects
            .iter()
            .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
            .collect();
        tiles.update_fits(&gpu.queue, &placements);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = main
            .ui
            .prepare(&gpu.device, &gpu.queue, &mut encoder, &mut ui_frame);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tiles+ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            tiles.draw(&mut pass, &placements);
            main.ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        main.ui.cleanup(&mut ui_frame);
        main.window.pre_present_notify();
        gpu.queue.present(surface);
        if ui_frame.repaint_delay.is_zero() {
            main.window.request_redraw();
            self.next_repaint = None;
        } else {
            self.next_repaint = repaint_deadline(Instant::now(), ui_frame.repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
        if let Some(action) = start_action
            && let Some(intent) = self.start.submit(action)
        {
            self.open(intent);
        }
        if let Phase::Room(view) = &mut self.phase
            && had_commands
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if (start_action.is_some() || had_commands)
            && let Some(main) = &self.main
        {
            main.window.request_redraw();
        }
    }
```

Rewrite `resumed`:

```rust
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("brp")
                .with_inner_size(PhysicalSize::new(1280, 720)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let (gpu, main) = match GpuContext::new(event_loop, window) {
            Ok(pair) => pair,
            Err(error) => {
                let _: AppError = error;
                tracing::error!(%error, "could not initialise the GPU");
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.format));
        self.gpu = Some(gpu);
        self.main = Some(main);
    }
```

In `user_event`, replace the two `if let Some(window) = &self.window` blocks with `if let Some(main) = &self.main { main.window.set_title(..) }` and `if let Some(main) = &self.main { main.window.request_redraw(); }`.

Rewrite `window_event`:

```rust
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(main) = self.main.as_mut() else {
            return;
        };
        let response = main.ui.on_window_event(&main.window, &event);
        if response.repaint {
            main.window.request_redraw();
        }
        if response.consumed {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_ref() {
                    main.resize(gpu, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
```

In `about_to_wait`, replace `if let Some(window) = &self.window { window.request_redraw(); }` with `if let Some(main) = &self.main { main.window.request_redraw(); }`.

- [ ] **Step 4: Build, test, run once**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean, all tests pass (no new tests in this task; the split is verified by the build and by Step 5).

Run: `cargo run -p brp` and confirm the start screen appears and resizing the window still works, then close it. This is the only display check in the plan before Task 5, and it exists because this task touches every frame.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/app/src/render/mod.rs crates/app/src/render/surface.rs crates/app/src/window.rs
git commit -m "refactor: split the GPU context into a shared half and a per-window surface"
```

---

### Task 3: Tile overlay buttons, the grid skipping popped keys, and the pop-out pass

**Files:**
- Modify: `crates/app/src/ui/tiles.rs`
- Create: `crates/app/src/ui/popout.rs`
- Modify: `crates/app/src/ui/mod.rs`
- Modify: `crates/app/src/window.rs` (one call site)

**Interfaces:**
- Consumes: `WindowCommand`, `visible_watches`, `live_title` (Task 1); `preset_selector`, `offered_preset` from `ui/members.rs`; `volume_control` from `ui/mod.rs`.
- Produces: `ui::tiles::Placement::{Grid, PopOut { fullscreen: bool }}`; `ui::tiles::tile_overlay(ui, snapshot, state, commands, window_commands, watch, key, rect, placement)`; `ui::tiles::draw(ui, snapshot, state, popped, commands, window_commands) -> Vec<(TileKey, egui::Rect)>`; `ui::draw(root, snapshot, ticket, state, popped) -> UiOutput`; `ui::popout::draw(root, snapshot, state, key, fullscreen) -> UiOutput`; `ui::popout::fullscreen_toggle_requested(f11: bool, escape: bool, fullscreen: bool) -> bool`.

- [ ] **Step 1: Generalise the overlay in `ui/tiles.rs`**

Replace the file's contents with:

```rust
//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through. The
//! overlay is shared with pop-out windows, which place one live over their whole panel.

use std::collections::HashSet;

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::{offered_preset, preset_selector};
use super::state::{UiState, live_title, visible_watches};
use crate::commands::{RoomCommand, WindowCommand};
use crate::render::grid;
use crate::render::tiles::TileKey;

/// Where an overlay is drawn, which decides its window buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A cell of the main window's grid: offers pop out and fullscreen.
    Grid,
    /// A pop-out window: offers the fullscreen toggle and a return to the grid.
    PopOut { fullscreen: bool },
}

/// Draws the tile grid without the popped-out lives, returning where the video renderer should
/// place each drawn live's frame, in egui points.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
) -> Vec<(TileKey, egui::Rect)> {
    let mut placements = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let watches = visible_watches(snapshot, popped);
            if watches.is_empty() {
                let hint = if popped.is_empty() {
                    "Tick a live on the left to watch it"
                } else {
                    "Every watched live is in its own window"
                };
                ui.centered_and_justified(|ui| ui.weak(hint));
                return;
            }
            let rects = grid::layout(ui.max_rect(), watches.len());
            for (watch, rect) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                placements.push((key, rect));
                tile_overlay(
                    ui,
                    snapshot,
                    state,
                    commands,
                    window_commands,
                    watch,
                    key,
                    rect,
                    Placement::Grid,
                );
            }
        });
    placements
}

/// Status text, the stats readout, and the hover bar for one live drawn in `rect`.
#[allow(clippy::too_many_arguments)]
pub fn tile_overlay(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    window_commands: &mut Vec<WindowCommand>,
    watch: &WatchView,
    key: TileKey,
    rect: egui::Rect,
    placement: Placement,
) {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    let live = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live_title(snapshot, key).unwrap_or_else(|| "publisher left".to_string());
    let status = match watch.state {
        WatchState::Connecting => Some("connecting"),
        WatchState::Reconnecting => Some("reconnecting"),
        WatchState::Ended => Some("ended"),
        WatchState::Live => None,
    };
    if let Some(status) = status {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{title}: {status}"),
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }
    if state.stats_visible.contains(&key) {
        let preset = live
            .and_then(|l| l.presets.iter().find(|p| p.id == watch.preset_id))
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        ui.painter().text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "decoded {}  keyframe requests {}  preset {preset}",
                watch.frames_decoded, watch.keyframe_requests
            ),
            egui::FontId::monospace(13.0),
            egui::Color32::WHITE,
        );
    }
    // A preset dropdown extends past the tile; keeping overlays up while any popup is open stops
    // the dropdown from vanishing under the pointer.
    if !response.contains_pointer() && !egui::Popup::is_any_open(ui.ctx()) {
        return;
    }
    let bar = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(8.0, 8.0),
        egui::vec2(rect.width() - 16.0, 28.0),
    );
    ui.painter()
        .rect_filled(bar, 4.0, egui::Color32::from_black_alpha(160));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bar.shrink(4.0))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.strong(title.as_str());
            if let Some(live) = live {
                let Some(mut preset_id) = offered_preset(live, Some(watch.preset_id)) else {
                    return;
                };
                preset_selector(ui, ("tile-preset", key), &live.presets, &mut preset_id);
                if preset_id != watch.preset_id {
                    state.preset_choice.insert(key, preset_id);
                    commands.push(RoomCommand::Watch { key, preset_id });
                }
            }
            if watch.audio {
                let gain = snapshot
                    .members
                    .iter()
                    .find(|m| m.id == key.0)
                    .map(|m| m.gain)
                    .unwrap_or(1.0);
                if let Some(gain) = super::volume_control(ui, ("tile-volume", key), gain) {
                    commands.push(RoomCommand::SetVolume {
                        publisher: key.0,
                        gain,
                    });
                }
            }
            let mut stats = state.stats_visible.contains(&key);
            if ui.toggle_value(&mut stats, "stats").changed() {
                if stats {
                    state.stats_visible.insert(key);
                } else {
                    state.stats_visible.remove(&key);
                }
            }
            match placement {
                Placement::Grid => {
                    if ui.small_button("pop out").clicked() {
                        window_commands.push(WindowCommand::PopOut(key));
                    }
                    if ui.small_button("fullscreen").clicked() {
                        window_commands.push(WindowCommand::PopOutFullscreen(key));
                    }
                }
                Placement::PopOut { fullscreen } => {
                    let label = if fullscreen { "windowed" } else { "fullscreen" };
                    if ui.small_button(label).clicked() {
                        window_commands.push(WindowCommand::ToggleFullscreen(key));
                    }
                    if ui.small_button("back to grid").clicked() {
                        window_commands.push(WindowCommand::ReturnToGrid(key));
                    }
                }
            }
            if ui.small_button("close").clicked() {
                commands.push(RoomCommand::Unwatch(key));
            }
        },
    );
}
```

- [ ] **Step 2: Write the failing key-rule test and the pop-out pass**

Create `crates/app/src/ui/popout.rs`:

```rust
//! A pop-out window's egui pass: one live filling the window, the tile overlay on top, and the
//! keyboard rule for fullscreen. The panels of the main window are not drawn here.

use brp_room::RoomSnapshot;

use super::UiOutput;
use super::state::UiState;
use super::tiles::{Placement, tile_overlay};
use crate::commands::WindowCommand;
use crate::render::tiles::TileKey;

/// Draws the pop-out for `key`. The output's `tile_rects` holds the one placement, or nothing
/// when the watch is gone from the snapshot, in which case the window is about to be closed.
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    key: TileKey,
    fullscreen: bool,
) -> UiOutput {
    let mut output = UiOutput::default();
    let Some(watch) = snapshot
        .watches
        .iter()
        .find(|w| (w.publisher, w.live_id) == key)
    else {
        return output;
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let rect = ui.max_rect();
            output.tile_rects.push((key, rect));
            tile_overlay(
                ui,
                snapshot,
                state,
                &mut output.commands,
                &mut output.window_commands,
                watch,
                key,
                rect,
                Placement::PopOut { fullscreen },
            );
        });
    let (f11, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::F11),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if fullscreen_toggle_requested(f11, escape, fullscreen) {
        output.window_commands.push(WindowCommand::ToggleFullscreen(key));
    }
    output
}

/// F11 always toggles; Esc only leaves fullscreen, so it cannot enter it by accident.
pub fn fullscreen_toggle_requested(f11: bool, escape: bool, fullscreen: bool) -> bool {
    f11 || (escape && fullscreen)
}

#[cfg(test)]
mod tests {
    use super::fullscreen_toggle_requested;

    #[test]
    fn f11_toggles_both_ways_and_escape_only_leaves_fullscreen() {
        assert!(fullscreen_toggle_requested(true, false, false));
        assert!(fullscreen_toggle_requested(true, false, true));
        assert!(fullscreen_toggle_requested(false, true, true));
        assert!(!fullscreen_toggle_requested(false, true, false));
        assert!(!fullscreen_toggle_requested(false, false, true));
    }
}
```

- [ ] **Step 3: Thread the popped set and window commands through `ui/mod.rs`**

In `crates/app/src/ui/mod.rs`: add `pub mod popout;` to the module list (alphabetical: after `picker`), add `use std::collections::HashSet;`, and change `draw` to:

```rust
pub fn draw(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
    popped: &HashSet<TileKey>,
) -> UiOutput {
    let mut commands = Vec::new();
    let mut window_commands = Vec::new();
    status::draw(ui, snapshot, ticket, state, &mut commands);
    own_lives::draw(ui, snapshot, state, &mut commands);
    members::draw(ui, snapshot, state, &mut commands);
    let tile_rects = tiles::draw(
        ui,
        snapshot,
        state,
        popped,
        &mut commands,
        &mut window_commands,
    );
    picker::draw(ui.ctx(), state, &mut commands);
    UiOutput {
        commands,
        window_commands,
        tile_rects,
    }
}
```

Update the doc comment above it to mention that `popped` lists the lives shown in their own windows, which the grid skips.

In `crates/app/src/window.rs`, the one call `ui::draw(root, &view.snapshot, &view.ticket, &mut self.state)` becomes `ui::draw(root, &view.snapshot, &view.ticket, &mut self.state, &HashSet::new())` with `use std::collections::HashSet;` added. Task 4 replaces the empty set with the real one and applies `output.window_commands`; until then they are produced and dropped, which is the correct behaviour for a window that cannot yet open pop-outs.

- [ ] **Step 4: Test, lint, commit**

Run: `cargo test -p brp popout && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 1 new test passes; clippy clean (the `too_many_arguments` allow on `tile_overlay` is deliberate: the overlay takes the same inputs as before plus the window command sink and the placement, and bundling them into a struct for one call site adds nothing).

```bash
cargo fmt --all
git add crates/app/src/ui/tiles.rs crates/app/src/ui/popout.rs crates/app/src/ui/mod.rs crates/app/src/window.rs
git commit -m "feat: add pop-out and fullscreen controls to the tile overlay and a pop-out pass"
```

---

### Task 4: Multi-window App: pop-out windows, command application, lifecycle

**Files:**
- Modify: `crates/app/src/window.rs`
- Modify: `crates/app/src/room_view.rs`

**Interfaces:**
- Consumes: `PopOuts<WindowId>` (Task 1), `GpuContext`, `WindowSurface` (Task 2), `ui::draw(.., popped)`, `ui::popout::draw` (Task 3), `live_title` (Task 1).
- Produces: `window::DEFAULT_WINDOW_SIZE: PhysicalSize<u32>`; `RoomView::watched_keys(&self) -> HashSet<TileKey>`; the running behaviour of section 5.3 of the spec.

- [ ] **Step 1: Expose the watched set from `RoomView`**

In `crates/app/src/room_view.rs`, add after `new`:

```rust
    /// The keys of every watch in the current snapshot; pop-outs whose key is absent are closed.
    pub fn watched_keys(&self) -> HashSet<TileKey> {
        self.snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect()
    }
```

and in `refresh`, replace the inline `let live: HashSet<TileKey> = self.snapshot.watches.iter().map(..).collect();` with `let live = self.watched_keys();` so there is one definition.

- [ ] **Step 2: Rewrite `window.rs` for several windows**

Replace the whole file with the following. It keeps every existing behaviour of the main window and adds pop-outs.

```rust
//! The participant windows: a winit loop that shows the start screen until a room is open, then
//! draws the tile grid under the egui panels in the main window and one live per pop-out window.
//! Panel commands go to the room view; window commands are applied here.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use brp_room::Room;
use iroh::SecretKey;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Fullscreen, Window, WindowId},
};

use crate::commands::WindowCommand;
use crate::error::AppError;
use crate::launch::{self, Intent, Launch};
use crate::popouts::PopOuts;
use crate::render::GpuContext;
use crate::render::grid::{self, PixelRect};
use crate::render::surface::WindowSurface;
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::ui::UiFrame;
use crate::room_view::RoomView;
use crate::ui::start::{self, StartState};
use crate::ui::state::{UiState, live_title};
use crate::ui::{self, UiOutput, popout};

/// Initial inner size of the main window and of every pop-out, in physical pixels.
pub const DEFAULT_WINDOW_SIZE: PhysicalSize<u32> = PhysicalSize::new(1280, 720);

/// Wakes the winit event loop for a reason that does not arrive as a `WindowEvent`. Sent through
/// the `EventLoopProxy` from other threads (the room's background tasks, the open and share tasks).
pub enum AppEvent {
    /// The room's version counter moved; re-snapshot on the next redraw.
    RoomChanged,
    /// A watched live decoded a frame.
    NewFrame,
    /// Periodic wake so counters refresh while nothing is watched.
    Tick,
    /// The share task finished: the live started, or the error to show.
    ShareFinished(Result<(), String>),
    /// The open task finished: the room to show, or the error for the start screen.
    RoomOpened(Result<Arc<Room>, String>),
}

/// What the window shows: the start screen, or a room.
enum Phase {
    Start,
    // Boxed: `RoomView` is much larger than `Start`, and clippy's large_enum_variant lint
    // treats the size gap as a wasted-space signal for every `Phase` on the stack.
    Room(Box<RoomView>),
}

/// A live shown in a window of its own. `fullscreen` is the state we asked winit for; the label
/// follows it even if the compositor refused, so the user can ask again.
struct PopOutWindow {
    surface: WindowSurface,
    fullscreen: bool,
}

/// What must be torn down after the loop ends: share tasks still holding room handles, an open
/// that may still be producing a room, and the room itself.
pub struct Shutdown {
    pub room: Option<Arc<Room>>,
    pub tasks: Vec<JoinHandle<()>>,
    /// Awaited, never aborted: a room it produces after the window closed must still be left.
    pub pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
}

/// The winit `ApplicationHandler` for the participant windows: owns the phase, the window-local
/// UI state, the shared GPU state, the main window, and the pop-outs.
pub struct App {
    runtime: Handle,
    proxy: EventLoopProxy<AppEvent>,
    launch: Launch,
    /// Loaded once at startup; every room this window opens signs as this identity.
    secret: SecretKey,
    start: StartState,
    phase: Phase,
    state: UiState,
    pending_open: Option<JoinHandle<Result<Arc<Room>, String>>>,
    /// The earliest instant any window's egui asked for its next frame; `about_to_wait` sleeps
    /// until then instead of forever.
    next_repaint: Option<Instant>,
    gpu: Option<GpuContext>,
    main: Option<WindowSurface>,
    /// Shared by every window: a frame is uploaded once whichever window shows it.
    tiles: Option<TileRenderer>,
    popouts: PopOuts<WindowId>,
    popout_windows: HashMap<WindowId, PopOutWindow>,
}

impl App {
    /// An `intent` from the command line opens the room at once behind the connecting start
    /// screen; `None` waits for the user.
    pub fn new(
        runtime: Handle,
        proxy: EventLoopProxy<AppEvent>,
        launch: Launch,
        secret: SecretKey,
        nickname: String,
        intent: Option<Intent>,
    ) -> Self {
        let mut app = Self {
            runtime,
            proxy,
            launch,
            secret,
            start: StartState::new(nickname),
            phase: Phase::Start,
            state: UiState::new(),
            pending_open: None,
            next_repaint: None,
            gpu: None,
            main: None,
            tiles: None,
            popouts: PopOuts::new(),
            popout_windows: HashMap::new(),
        };
        if let Some(intent) = intent {
            app.start.connecting = true;
            app.open(intent);
        }
        app
    }

    /// Consumes the app once the loop has ended and hands back what still holds a room, in the
    /// order the caller must tear it down: share tasks, the pending open, then the room itself.
    /// Pop-out windows drop with the app; nothing in them outlives the loop.
    pub fn finish(self) -> Shutdown {
        let (room, tasks) = match self.phase {
            Phase::Room(view) => (Some(view.room), view.pending_share.into_iter().collect()),
            Phase::Start => (None, Vec::new()),
        };
        Shutdown {
            room,
            tasks,
            pending_open: self.pending_open,
        }
    }

    fn open(&mut self, intent: Intent) {
        let launch = self.launch.clone();
        let secret = self.secret.clone();
        let nickname = self.start.nickname.clone();
        let room_events = self.proxy.clone();
        let done = self.proxy.clone();
        self.pending_open = Some(self.runtime.spawn(async move {
            let outcome = launch::open_room(&launch, secret, intent, &nickname, room_events)
                .await
                .map_err(|error| error.to_string());
            // The window learns of the outcome through the event; the task output is for the
            // shutdown path, which must leave a room that opened after the window closed.
            let _ = done.send_event(AppEvent::RoomOpened(outcome.clone()));
            outcome
        }));
    }

    fn request_redraw_all(&self) {
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
        for popout in self.popout_windows.values() {
            popout.surface.window.request_redraw();
        }
    }

    fn is_main(&self, id: WindowId) -> bool {
        self.main.as_ref().is_some_and(|m| m.window.id() == id)
    }

    /// Keeps the earliest requested repaint across windows.
    fn note_repaint(&mut self, delay: Duration) {
        let deadline = repaint_deadline(Instant::now(), delay);
        self.next_repaint = match (self.next_repaint, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    /// Re-snapshots the room and closes pop-outs whose watch has ended. Runs at the start of every
    /// redraw, whichever window asked, so all windows draw from the same snapshot.
    fn refresh_room(&mut self) {
        let Phase::Room(view) = &mut self.phase else {
            return;
        };
        view.refresh(&mut self.state, self.tiles.as_mut());
        let watched = view.watched_keys();
        let closed = self.popouts.retain_watched(&watched);
        if closed.is_empty() {
            return;
        }
        for id in closed {
            self.popout_windows.remove(&id);
        }
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        self.refresh_room();
        if self.is_main(id) {
            self.redraw_main(event_loop);
        } else if self.popouts.key_of(id).is_some() {
            self.redraw_popout(event_loop, id);
        }
    }

    fn redraw_main(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(gpu), Some(main), Some(tiles)) =
            (self.gpu.as_ref(), self.main.as_mut(), self.tiles.as_mut())
        else {
            return;
        };
        if let Phase::Room(view) = &self.phase {
            view.upload_frames(gpu, tiles);
        }
        let popped = self.popouts.popped();
        let size = main.size();
        let mut output = UiOutput::default();
        let mut start_action = None;
        let mut ui_frame = main.ui.run(&main.window, [size.0, size.1], |root| match &self.phase {
            Phase::Start => start_action = start::draw(root, &mut self.start),
            Phase::Room(view) => {
                output = ui::draw(root, &view.snapshot, &view.ticket, &mut self.state, &popped);
            }
        });
        let placements = pixel_placements(&output, ui_frame.screen.pixels_per_point, size);
        present(gpu, tiles, main, &mut ui_frame, &placements);
        let repaint_delay = ui_frame.repaint_delay;
        if repaint_delay.is_zero() {
            main.window.request_redraw();
        } else {
            self.note_repaint(repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
        let had_window_commands = !output.window_commands.is_empty();
        if let Some(action) = start_action
            && let Some(intent) = self.start.submit(action)
        {
            self.open(intent);
        }
        if let Phase::Room(view) = &mut self.phase
            && had_commands
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if had_window_commands {
            self.apply_window_commands(event_loop, output.window_commands);
        }
        if (start_action.is_some() || had_commands || had_window_commands)
            && let Some(main) = &self.main
        {
            main.window.request_redraw();
        }
    }

    fn redraw_popout(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let Some(key) = self.popouts.key_of(id) else {
            return;
        };
        let (Some(gpu), Some(popout), Some(tiles), Phase::Room(view)) = (
            self.gpu.as_ref(),
            self.popout_windows.get_mut(&id),
            self.tiles.as_mut(),
            &self.phase,
        ) else {
            return;
        };
        view.upload_frames(gpu, tiles);
        let fullscreen = popout.fullscreen;
        let surface = &mut popout.surface;
        let size = surface.size();
        let mut output = UiOutput::default();
        let mut ui_frame = surface.ui.run(&surface.window, [size.0, size.1], |root| {
            output = popout::draw(root, &view.snapshot, &mut self.state, key, fullscreen);
        });
        let placements = pixel_placements(&output, ui_frame.screen.pixels_per_point, size);
        present(gpu, tiles, surface, &mut ui_frame, &placements);
        let repaint_delay = ui_frame.repaint_delay;
        if repaint_delay.is_zero() {
            surface.window.request_redraw();
        } else {
            self.note_repaint(repaint_delay);
        }

        let had_commands = !output.commands.is_empty();
        let had_window_commands = !output.window_commands.is_empty();
        if let Phase::Room(view) = &mut self.phase
            && had_commands
        {
            view.apply(output.commands, &self.runtime, &self.proxy, &mut self.state);
        }
        if had_window_commands {
            self.apply_window_commands(event_loop, output.window_commands);
        }
        if had_commands || had_window_commands {
            self.request_redraw_all();
        }
    }

    fn apply_window_commands(&mut self, event_loop: &ActiveEventLoop, commands: Vec<WindowCommand>) {
        for command in commands {
            match command {
                WindowCommand::PopOut(key) => self.open_popout(event_loop, key, false),
                WindowCommand::PopOutFullscreen(key) => self.open_popout(event_loop, key, true),
                WindowCommand::ToggleFullscreen(key) => {
                    if let Some(id) = self.popouts.window_of(key)
                        && let Some(popout) = self.popout_windows.get_mut(&id)
                    {
                        popout.fullscreen = !popout.fullscreen;
                        popout.surface.window.set_fullscreen(
                            popout
                                .fullscreen
                                .then_some(Fullscreen::Borderless(None)),
                        );
                    }
                }
                WindowCommand::ReturnToGrid(key) => {
                    if let Some(id) = self.popouts.remove_key(key) {
                        self.popout_windows.remove(&id);
                    }
                }
            }
        }
    }

    /// Opens a window for `key`. Failures leave the live in the grid and explain why in the
    /// status line; a key already popped out is left where it is.
    fn open_popout(&mut self, event_loop: &ActiveEventLoop, key: TileKey, fullscreen: bool) {
        if self.popouts.is_popped(key) {
            return;
        }
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let title = match &self.phase {
            Phase::Room(view) => live_title(&view.snapshot, key),
            Phase::Start => None,
        }
        .unwrap_or_else(|| "live".to_string());
        let attributes = Window::default_attributes()
            .with_title(format!("brp: {title}"))
            .with_inner_size(DEFAULT_WINDOW_SIZE);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.state.status = format!("pop-out failed: {error}");
                return;
            }
        };
        let surface = match WindowSurface::new(gpu, window) {
            Ok(surface) => surface,
            Err(error) => {
                self.state.status = format!("pop-out failed: {error}");
                return;
            }
        };
        if fullscreen {
            surface
                .window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let id = surface.window.id();
        self.popouts.insert(id, key);
        surface.window.request_redraw();
        self.popout_windows
            .insert(id, PopOutWindow { surface, fullscreen });
    }

    fn close_popout(&mut self, id: WindowId) {
        self.popouts.remove_window(id);
        self.popout_windows.remove(&id);
        if let Some(main) = &self.main {
            main.window.request_redraw();
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.main.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("brp")
                .with_inner_size(DEFAULT_WINDOW_SIZE),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let (gpu, main) = match GpuContext::new(event_loop, window) {
            Ok(pair) => pair,
            Err(error) => {
                let _: AppError = error;
                tracing::error!(%error, "could not initialise the GPU");
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.format));
        self.gpu = Some(gpu);
        self.main = Some(main);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::RoomOpened(Ok(room)) => {
                self.pending_open = None;
                self.state = UiState::new();
                if let Some(main) = &self.main {
                    main.window
                        .set_title(&format!("brp: {}", room.snapshot().nickname));
                }
                self.phase = Phase::Room(Box::new(RoomView::new(room)));
            }
            AppEvent::RoomOpened(Err(message)) => {
                self.pending_open = None;
                self.start.failed(message);
            }
            AppEvent::ShareFinished(outcome) => {
                if let Phase::Room(view) = &mut self.phase {
                    view.pending_share = None;
                }
                self.state.share_pending = false;
                if let Err(message) = outcome {
                    self.state.status = format!("share failed: {message}");
                }
            }
            AppEvent::RoomChanged | AppEvent::NewFrame | AppEvent::Tick => {}
        }
        self.request_redraw_all();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.is_main(id) {
            let Some(main) = self.main.as_mut() else {
                return;
            };
            let response = main.ui.on_window_event(&main.window, &event);
            if response.repaint {
                main.window.request_redraw();
            }
            if response.consumed {
                return;
            }
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = self.gpu.as_ref() {
                        main.resize(gpu, size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => self.redraw(event_loop, id),
                _ => {}
            }
            return;
        }
        let Some(popout) = self.popout_windows.get_mut(&id) else {
            return;
        };
        let response = popout
            .surface
            .ui
            .on_window_event(&popout.surface.window, &event);
        if response.repaint {
            popout.surface.window.request_redraw();
        }
        if response.consumed {
            return;
        }
        match event {
            // Closing a pop-out returns its live to the grid; the watch itself continues.
            WindowEvent::CloseRequested => self.close_popout(id),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_ref() {
                    popout.surface.resize(gpu, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop, id),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.next_repaint {
            Some(deadline) if deadline <= Instant::now() => {
                self.next_repaint = None;
                self.request_redraw_all();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

/// The egui-point rects of one pass as pixel viewports on a surface of `size`.
fn pixel_placements(
    output: &UiOutput,
    pixels_per_point: f32,
    size: (u32, u32),
) -> Vec<(TileKey, PixelRect)> {
    output
        .tile_rects
        .iter()
        .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
        .collect()
}

/// Records and submits one window's frame: the placed tiles, then egui on top. A lost surface
/// skips the frame; the next `Resized` reconfigures it.
fn present(
    gpu: &GpuContext,
    tiles: &TileRenderer,
    surface: &mut WindowSurface,
    ui_frame: &mut UiFrame,
    placements: &[(TileKey, PixelRect)],
) {
    let Some(texture) = surface.acquire() else {
        // The frame's texture deltas must still be applied and freed or they assert on drop.
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = surface
            .ui
            .prepare(&gpu.device, &gpu.queue, &mut encoder, ui_frame);
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        surface.ui.cleanup(ui_frame);
        return;
    };
    let target = texture.texture.create_view(&Default::default());
    tiles.update_fits(&gpu.queue, placements);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let buffers = surface
        .ui
        .prepare(&gpu.device, &gpu.queue, &mut encoder, ui_frame);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tiles+ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        tiles.draw(&mut pass, placements);
        surface.ui.paint(&mut pass, ui_frame);
    }
    gpu.queue
        .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
    surface.ui.cleanup(ui_frame);
    surface.window.pre_present_notify();
    gpu.queue.present(texture);
}

/// The instant egui wants the next frame, or `None` when it asked for nothing: egui reports
/// `Duration::MAX` in that case, which overflows an `Instant`.
fn repaint_deadline(now: Instant, delay: Duration) -> Option<Instant> {
    now.checked_add(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finite_delay_becomes_a_deadline_and_no_request_becomes_none() {
        let now = Instant::now();
        assert_eq!(
            repaint_deadline(now, Duration::from_millis(300)),
            Some(now + Duration::from_millis(300))
        );
        assert_eq!(repaint_deadline(now, Duration::ZERO), Some(now));
        assert_eq!(repaint_deadline(now, Duration::MAX), None);
    }
}
```

Notes for the implementer:

- Phase 2 acquired the surface texture before running egui; `present` acquires it after. That ordering is fine for wgpu and lets one function serve both windows. The lost-surface branch still runs `prepare` and `cleanup` because `TexturesDelta` panics on drop when entries remain (see the comment in `render/ui.rs`).
- In `redraw_popout`, the tuple pattern borrows `self.phase` immutably while `self.state` is borrowed mutably inside the closure; those are disjoint fields, which the borrow checker accepts because the closure captures field paths, not `self`. If the compiler disagrees, bind `let state = &mut self.state;` before the tuple and use `state` in the closure.
- `about_to_wait` runs after every batch of events. Because `next_repaint` is now the minimum across windows, one window's animation wakes them all; that is the same cost as the `NewFrame` fan-out and is accepted in spec section 7.

- [ ] **Step 3: Build, test, lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test -p brp`
Expected: clean; every existing test passes. There is no unit test for winit plumbing, by design; `PopOuts`, `visible_watches`, and the key rule carry the logic.

- [ ] **Step 4: Two-instance smoke on the dev machine**

Run two instances (`cargo run -p brp -- create --nickname a --no-relay` and `cargo run -p brp -- join <ticket> --nickname b --no-relay`, copying the ticket from the first status bar). Share a monitor from `a`, watch it from `b`, hover the tile and click `pop out`. Confirm: the tile leaves the grid, a window titled `brp: Monitor 1` shows the video, closing it returns the tile. Click `fullscreen`, press Esc, press F11 twice, click `back to grid`. Untick the live in the members panel while popped out and confirm the window closes. This is the implementer's own check; the user's full click-through is Task 5.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/app/src/window.rs crates/app/src/room_view.rs
git commit -m "feat: open watched lives in pop-out windows with borderless fullscreen"
```

---

### Task 5: Documentation, spec amendments, and the user's click-through

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md`

**Interfaces:** none.

- [ ] **Step 1: README**

In `README.md`, in the section that describes watching lives in the tile grid (search for "tile grid" under "How it works" or the usage section that mentions the hover overlay), add a paragraph:

```markdown
Hovering a tile shows `pop out` and `fullscreen`. Pop out moves the live into a
window of its own, which you can drag to another monitor; fullscreen does the
same and makes that window borderless fullscreen. In a pop-out, F11 toggles
fullscreen, Esc leaves it, and `back to grid` or closing the window puts the
live back in the grid. A live is decoded once wherever it is shown.
```

In the `## Roadmap` list, change item 5 to:

```markdown
5. **Window management and polish** — pop-outs and fullscreen done; settings
   UI and persistence and release packaging planned.
```

and after the "Phase 4 is designed in ..." sentence add:

```markdown
Phase 5 is designed in
[`docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md`](docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md);
pop-outs and fullscreen are implemented by
[`2026-09-06-plan5a-popouts.md`](docs/superpowers/plans/2026-09-06-plan5a-popouts.md).
```

In the status blockquote near the top, replace "Audio, pop-out windows, fullscreen, and persistent settings are the next phases" with "Persistent settings and release packaging are the next steps".

- [ ] **Step 2: Spec amendments**

Append to the spec:

```markdown
## 13. Amendments from the 5a implementation run

- **Main surface creation (5.1).** `GpuContext::new` takes the main window and returns the configured `WindowSurface` with it: wgpu picks the adapter against a surface, and a window has at most one surface, so the main surface cannot be created a second time by `WindowSurface::new`. Pop-outs use `WindowSurface::new`.
- **Fullscreen state (5.3).** The requested fullscreen state lives on the pop-out record (`PopOutWindow { surface, fullscreen }`) rather than a separate set of window ids; same meaning, one map fewer.
- **Constant name (11).** `POPOUT_DEFAULT_SIZE` is named `DEFAULT_WINDOW_SIZE` because the main window uses it too.
- **Repaint scheduling (5.3).** `next_repaint` is the earliest deadline across windows and wakes every window, matching the `NewFrame` fan-out in section 7.
```

If the implementation of Tasks 2 to 4 diverged from the spec anywhere else, add a bullet per divergence in the same style.

- [ ] **Step 3: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-06-phase5-windows-settings-release-design.md
git commit -m "docs: describe pop-outs and fullscreen and record the 5a amendments"
```

- [ ] **Step 4: Hand the click-through to the user**

Tell the user the plan is implemented and ask them to run the manual check from spec section 10.1 (it needs a display and two monitors, which subagents do not have):

1. Watch two lives; pop one out; move it to the second monitor.
2. F11, Esc, and the overlay toggle on the pop-out.
3. Close the pop-out and see the live return to the grid.
4. Unwatch a popped-out live from the members panel; the window closes.
5. Stop the publisher; the window closes.
6. Preset selector and volume slider work in the pop-out.

Report anything that fails as a bug to fix before plan 5b starts.
