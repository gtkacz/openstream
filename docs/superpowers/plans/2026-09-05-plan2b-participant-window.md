# Plan 2b: Participant Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `brp create` and `brp join` open one native window in which a participant sees the room's members, watches any of their lives in a tile grid, and shares, tunes, and stops their own lives; the headless `watch` command is removed.

**Architecture:** The window is a thin renderer over `brp_room::Room`. A winit `App` holds an `Arc<Room>`, re-snapshots when the room's version counter moves, and pulls decoded frames from one `WatchHandle` slot per tile. egui panels are pure functions of the snapshot plus a small window-local state; they emit `RoomCommand`s that the app applies after the egui pass, so the synchronous room calls run on the UI thread and the one async call, `start_live` with its portal dialog, runs on tokio and reports back as a user event. The phase 1 single-quad NV12 renderer becomes a tile renderer with one shared pipeline and per-tile planes drawn into per-tile viewports under egui. The publisher gains frame pacing so a preset's advertised frame rate is real.

**Tech Stack:** Rust 2024, winit 0.30, wgpu 30, egui, egui-wgpu, egui-winit 0.36, tokio, the phase 1 and plan 2a crates.

**Spec:** `docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md` (sections 3, 5.2, 7, 8, 10, 11; frame-rate amendment of 2026-09-05), refining `docs/superpowers/specs/2026-09-04-p2p-screen-sharing-design.md` (section 5.6 and the main window bullets of section 7). Read both.

## Global Constraints

- Linux only. No Windows or macOS code. Audio, pop-outs, fullscreen, and settings persistence stay out of scope.
- Preset switching is unsubscribe plus subscribe: the window calls `Room::watch` again with the new preset id; the watcher replaces the entry. `SwitchPreset` is never sent.
- Constants live in `crates/proto/src/constants.rs`. This plan adds none; it uses `SOURCE_PRESET_ID`, `MIN_BITRATE_KBPS`, `MAX_BITRATE_KBPS`, `MAX_LIVES_PER_PARTICIPANT`, `STATS_LOG_INTERVAL`, `RELAY_ONLINE_TIMEOUT`. Grid columns are the ceiling of the square root of the tile count.
- Frames between threads go through `LatestSlot`. The window reads a tile's frame with `try_take` on redraw and never blocks.
- Nothing on the media path panics. The app crate keeps `AppError`; room failures surface in the window's status line, never as a crash.
- Comments explain why. Doc comments state contracts on public items. No task ids in code.
- One Conventional Commit per task, imperative subject, no co-author lines. The `Claude-Session:` trailer the harness requires on every commit of this session is expected; no other trailers. `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass before each commit.
- Tests are deterministic and hardware-free: pure geometry, preset arithmetic, pacing, and rate math. egui panels are not unit tested; they are exercised by the manual two-machine check in Task 8. No GPU, portal, or relay in the test suite.
- The app crate becomes a library plus a binary (Task 2) so each module lands lint-clean before the window wires it. Dead-code warnings would otherwise fail clippy for every module added ahead of Task 7.
- Verified library facts this plan relies on (egui 0.36.1, wgpu 30.0.1, winit 0.30.13, iroh-base 1.1): `egui::Context::copy_text(String)`, `Context::pixels_per_point()`, `Popup::is_any_open(&Context)`, `Frame::NONE`, `CentralPanel::default().frame(Frame)`, `SidePanel::left(id).resizable(bool)`, `TopBottomPanel::bottom(id)`, `ComboBox::from_id_salt(impl Hash + Debug).selected_text(..).show_ui(ui, |ui| ..)`, `DragValue::new(&mut T).range(a..=b).speed(f64).suffix(..)`, `Response::{changed, clicked, contains_pointer, dragged, has_focus}`, `Ui::{allocate_rect, max_rect, scope_builder, centered_and_justified, add_enabled, toggle_value, small_button, colored_label, weak, strong, monospace, painter, ctx}`, `UiBuilder::new().max_rect(Rect).layout(Layout)`, `Layout::left_to_right(Align)`, `Painter::{text, rect_filled}`, `Rect::{from_min_size, shrink, center, left_top, left_bottom, width, height}`, `Color32::{WHITE, LIGHT_RED, from_black_alpha}`, `FontId::{proportional, monospace}`, `Align2::{CENTER_CENTER, LEFT_BOTTOM}`, `ViewportOutput::repaint_delay`; `wgpu::RenderPass::set_viewport(x, y, w, h, min_depth, max_depth)`; `iroh::PublicKey: Hash + Ord + Debug` with `as_bytes()` and `fmt_short()`.

## File Structure

```
crates/pipeline/src/pacer.rs              Pacer: admit or skip captured frames at a preset rate
crates/pipeline/src/publisher.rs          + Option<Pacer> in Publisher::start and the encode loop
crates/pipeline/src/lib.rs                + pub use pacer::Pacer
crates/pipeline/tests/publisher.rs        call sites pass None
crates/room/src/registry.rs               builds a Pacer when preset.fps < source fps

crates/app/Cargo.toml                     + [lib] brp_app
crates/app/src/lib.rs                     module list of the library
crates/app/src/main.rs                    binary: parse CLI, dispatch
crates/app/src/cli.rs                     Publish, Create, Join; WindowArgs
crates/app/src/error.rs                   AppError (EmptyTicket removed)
crates/app/src/participant.rs             create/join: room, event loop, shutdown (replaces watch.rs)
crates/app/src/window.rs                  App: winit handler, snapshot refresh, tiles, command application
crates/app/src/commands.rs                RoomCommand
crates/app/src/presets.rs                 pure preset-list edits for the bottom panel
crates/app/src/render/mod.rs              GpuContext (unchanged) + module list
crates/app/src/render/grid.rs             dimensions, layout, to_pixels
crates/app/src/render/tiles.rs            TileRenderer, TileKey, fit_scale (replaces video.rs)
crates/app/src/render/ui.rs               EguiLayer (+ repaint_delay on UiFrame)
crates/app/src/ui/mod.rs                  draw(): panel order, UiOutput
crates/app/src/ui/state.rs                UiState, BitrateMeter, ordering helpers
crates/app/src/ui/members.rs              left panel + preset_selector
crates/app/src/ui/own_lives.rs            bottom panel
crates/app/src/ui/status.rs               status bar
crates/app/src/ui/tiles.rs                central panel: tile rects and hover overlays
README.md                                 usage for create/join, status, roadmap
```

---

### Task 1: Pace encoders to the preset frame rate

**Files:**
- Create: `crates/pipeline/src/pacer.rs`
- Modify: `crates/pipeline/src/lib.rs`, `crates/pipeline/src/publisher.rs`, `crates/pipeline/tests/publisher.rs`, `crates/room/src/registry.rs`

**Interfaces:**
- Consumes: `Publisher::start(live_id, preset_id, slot, converter, encoder)` from plan 2a; `LiveRegistry::subscribe` where `source: SourceInfo` and `state.preset: Preset` are already in scope.
- Produces: `brp_pipeline::Pacer` with `Pacer::new(fps: u32) -> Pacer` and `Pacer::admit(&mut self, capture_ts_us: u64) -> bool`; `Publisher::start(live_id: u32, preset_id: u32, slot: Arc<LatestSlot<Arc<CaptureFrame>>>, converter: Box<dyn FrameConverter>, encoder: Box<dyn VideoEncoder>, pacer: Option<Pacer>) -> Publisher`.

- [ ] **Step 1: Write the failing tests**

Create `crates/pipeline/src/pacer.rs`:

```rust
//! Holds an encoder to a preset frame rate below the capture rate.

/// Admits or skips captured frames so an encoder runs at `fps` while the capture runs faster.
/// A quarter-interval tolerance absorbs compositor jitter without letting the next capture through.
#[derive(Debug, Clone)]
pub struct Pacer {
    interval_us: u64,
    next_due_us: Option<u64>,
}

impl Pacer {
    pub fn new(fps: u32) -> Self {
        Self {
            interval_us: 1_000_000 / u64::from(fps.max(1)),
            next_due_us: None,
        }
    }

    /// True when the frame should be encoded. `capture_ts_us` is the capture clock in microseconds.
    pub fn admit(&mut self, capture_ts_us: u64) -> bool {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::Pacer;

    #[test]
    fn halves_a_sixty_hertz_capture_to_thirty() {
        let mut pacer = Pacer::new(30);
        let admitted: Vec<bool> = (0..6).map(|i| pacer.admit(i * 16_667)).collect();
        assert_eq!(admitted, [true, false, true, false, true, false]);
    }

    #[test]
    fn early_jitter_within_a_quarter_interval_is_admitted() {
        let mut pacer = Pacer::new(30);
        assert!(pacer.admit(0));
        assert!(!pacer.admit(16_667));
        assert!(pacer.admit(33_333 - 5_000), "5 ms early is inside the tolerance");
    }

    #[test]
    fn a_stall_admits_the_next_frame_and_paces_from_it() {
        let mut pacer = Pacer::new(30);
        assert!(pacer.admit(0));
        assert!(pacer.admit(500_000));
        assert!(!pacer.admit(516_667));
        assert!(pacer.admit(533_333));
    }

    #[test]
    fn first_frame_is_always_admitted() {
        assert!(Pacer::new(1).admit(123));
    }
}
```

Add to `crates/pipeline/src/lib.rs`:

```rust
pub mod pacer;
pub use pacer::Pacer;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp-pipeline pacer`
Expected: the four tests panic at `todo!()`.

- [ ] **Step 3: Implement `admit`**

```rust
    pub fn admit(&mut self, capture_ts_us: u64) -> bool {
        let tolerance = self.interval_us / 4;
        if let Some(due) = self.next_due_us
            && capture_ts_us + tolerance < due
        {
            return false;
        }
        self.next_due_us = Some(capture_ts_us + self.interval_us);
        true
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p brp-pipeline pacer`
Expected: 4 passed.

- [ ] **Step 5: Thread the pacer through the publisher**

In `crates/pipeline/src/publisher.rs`:

Add `use crate::pacer::Pacer;` to the imports.

Change the signature and the spawn:

```rust
    pub fn start(
        live_id: u32,
        preset_id: u32,
        slot: Arc<LatestSlot<Arc<CaptureFrame>>>,
        converter: Box<dyn FrameConverter>,
        encoder: Box<dyn VideoEncoder>,
        pacer: Option<Pacer>,
    ) -> Self {
```

```rust
            .spawn(move || encode_loop(worker, converter, encoder, pacer))
```

Change `encode_loop`:

```rust
fn encode_loop(
    inner: Arc<Inner>,
    mut converter: Box<dyn FrameConverter>,
    mut encoder: Box<dyn VideoEncoder>,
    mut pacer: Option<Pacer>,
) {
    let mut last: Option<Arc<CaptureFrame>> = None;
    while !inner.stop.load(Ordering::Relaxed) {
        let frame = match inner.slot.take_timeout(IDLE_KEYFRAME_RETRY) {
            SlotWait::Value(frame) => {
                // Skipped frames never become `last`: an idle keyframe retry re-encodes an admitted one.
                if pacer
                    .as_mut()
                    .is_some_and(|pacer| !pacer.admit(frame.capture_ts_us))
                {
                    continue;
                }
                last = Some(frame);
                last.as_deref()
            }
            SlotWait::Timeout if inner.keyframe.pending() => last.as_deref(),
            SlotWait::Timeout => continue,
            SlotWait::Closed => break,
        };
```

The rest of the loop is unchanged.

- [ ] **Step 6: Update the call sites**

`crates/pipeline/tests/publisher.rs`: both `Publisher::start(...)` calls gain a trailing `None,` argument after the encoder.

`crates/room/src/registry.rs`: add `use brp_pipeline::{LatestSlot, Pacer, Publisher};` (replacing the existing `use brp_pipeline::{LatestSlot, Publisher};`) and, inside `LiveRegistry::subscribe`, replace the `Publisher::start` call:

```rust
                Ok(parts) => {
                    let slot = fan.attach();
                    // Presets at the source rate pass every frame; only slower presets are paced.
                    let pacer = (state.preset.fps < source.fps).then(|| Pacer::new(state.preset.fps));
                    let publisher = Publisher::start(
                        live_id,
                        preset_id,
                        slot.clone(),
                        parts.converter,
                        parts.encoder,
                        pacer,
                    );
```

- [ ] **Step 7: Run the workspace tests and lints**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green. The `let ... && ...` chain requires edition 2024, which the workspace uses.

- [ ] **Step 8: Commit**

```bash
git add crates/pipeline crates/room/src/registry.rs
git commit -m "feat: pace encoders to the preset frame rate"
```

---

### Task 2: Library split and tile grid geometry

**Files:**
- Modify: `crates/app/Cargo.toml`, `crates/app/src/main.rs`, `crates/app/src/render/mod.rs`
- Create: `crates/app/src/lib.rs`, `crates/app/src/render/grid.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: library crate `brp_app` exposing today's modules; `brp_app::render::grid::{dimensions(count: usize) -> (usize, usize), layout(area: egui::Rect, count: usize) -> Vec<egui::Rect>, to_pixels(rect: egui::Rect, pixels_per_point: f32, surface: (u32, u32)) -> PixelRect}`; `PixelRect { x: f32, y: f32, width: f32, height: f32 }`.

- [ ] **Step 1: Split the crate into a library and a binary**

Add to `crates/app/Cargo.toml` after the `[[bin]]` table:

```toml
[lib]
name = "brp_app"
path = "src/lib.rs"
```

Create `crates/app/src/lib.rs`:

```rust
//! brp: peer-to-peer screen sharing. Everything the `brp` binary wires together.
pub mod cli;
pub mod error;
pub mod identity;
pub mod publish;
pub mod render;
pub mod watch;
pub mod window;
```

Replace the module declarations at the top of `crates/app/src/main.rs` so the file starts:

```rust
//! brp: peer-to-peer screen sharing.
use brp_app::cli::{Cli, Command};
use brp_app::{publish, watch};
use clap::Parser;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;
```

and delete the `mod cli; mod error; mod identity; mod publish; mod render; mod watch; mod window;` lines and the `use crate::cli::{Cli, Command};` line. The body of `main` is unchanged.

Run: `cargo build -p brp && cargo clippy -p brp --all-targets -- -D warnings`
Expected: builds. If clippy reports a private type in a public interface, make that type `pub`; nothing else changes.

- [ ] **Step 2: Write the failing grid tests**

Create `crates/app/src/render/grid.rs`:

```rust
//! Tile grid geometry. Columns are the ceiling of the square root of the tile count, which yields
//! the master spec's one to nine layouts.

use egui::{Rect, Vec2};

/// Columns and rows for `count` tiles: 1x1, 2x1, 2x2, 2x2, 3x2, 3x2, 3x3, 3x3, 3x3 for one to nine.
pub fn dimensions(count: usize) -> (usize, usize) {
    todo!()
}

/// One rect per tile in row-major order, tiling `area` exactly.
pub fn layout(area: Rect, count: usize) -> Vec<Rect> {
    todo!()
}

/// A viewport in physical pixels, clamped to the surface so wgpu never sees an out-of-range viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn to_pixels(rect: Rect, pixels_per_point: f32, surface: (u32, u32)) -> PixelRect {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn dimensions_follow_the_ceiling_square_root_rule() {
        let dims: Vec<(usize, usize)> = (0..=9).map(dimensions).collect();
        assert_eq!(
            dims,
            [
                (0, 0),
                (1, 1),
                (2, 1),
                (2, 2),
                (2, 2),
                (3, 2),
                (3, 2),
                (3, 3),
                (3, 3),
                (3, 3)
            ]
        );
    }

    #[test]
    fn three_tiles_fill_a_two_by_two_grid_row_major() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 100.0));
        let rects = layout(area, 3);
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::from_min_size(pos2(100.0, 50.0), vec2(100.0, 50.0)));
        assert_eq!(rects[1], Rect::from_min_size(pos2(200.0, 50.0), vec2(100.0, 50.0)));
        assert_eq!(rects[2], Rect::from_min_size(pos2(100.0, 100.0), vec2(100.0, 50.0)));
    }

    #[test]
    fn zero_tiles_yield_no_rects() {
        assert!(layout(Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0)), 0).is_empty());
    }

    #[test]
    fn pixels_scale_by_the_point_ratio_and_clamp_to_the_surface() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 100.0));
        let px = to_pixels(rect, 2.0, (400, 400));
        assert_eq!(
            px,
            PixelRect {
                x: 20.0,
                y: 40.0,
                width: 380.0,
                height: 200.0
            }
        );
    }
}
```

Add `pub mod grid;` to `crates/app/src/render/mod.rs` next to `pub mod ui;`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp grid`
Expected: panics at `todo!()`.

- [ ] **Step 4: Implement**

```rust
pub fn dimensions(count: usize) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }
    let cols = (count as f64).sqrt().ceil() as usize;
    (cols, count.div_ceil(cols))
}

pub fn layout(area: Rect, count: usize) -> Vec<Rect> {
    let (cols, rows) = dimensions(count);
    if cols == 0 {
        return Vec::new();
    }
    let cell = Vec2::new(area.width() / cols as f32, area.height() / rows as f32);
    (0..count)
        .map(|index| {
            let (col, row) = (index % cols, index / cols);
            let offset = Vec2::new(col as f32 * cell.x, row as f32 * cell.y);
            Rect::from_min_size(area.min + offset, cell)
        })
        .collect()
}

pub fn to_pixels(rect: Rect, pixels_per_point: f32, surface: (u32, u32)) -> PixelRect {
    let (max_x, max_y) = (surface.0 as f32, surface.1 as f32);
    let x = (rect.min.x * pixels_per_point).clamp(0.0, max_x);
    let y = (rect.min.y * pixels_per_point).clamp(0.0, max_y);
    let right = (rect.max.x * pixels_per_point).clamp(x, max_x);
    let bottom = (rect.max.y * pixels_per_point).clamp(y, max_y);
    PixelRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
}
```

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test -p brp grid && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 4 passed, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app
git commit -m "refactor: split the app into a library and add tile grid geometry"
```

---

### Task 3: Tile renderer

**Files:**
- Create: `crates/app/src/render/tiles.rs`
- Modify: `crates/app/src/render/mod.rs`, `crates/app/src/render/video.rs`

**Interfaces:**
- Consumes: `render::grid::PixelRect`; `brp_codec::RawFrame { width, height, y_stride, uv_stride, y, uv }` with `chroma_rows()`; the existing `nv12.wgsl` shader whose bind group is Y texture, UV texture, sampler, and a `Fit` uniform at bindings 0 to 3.
- Produces: `pub type TileKey = (iroh::PublicKey, u32)`; `TileRenderer::new(device: &wgpu::Device, format: wgpu::TextureFormat) -> TileRenderer`; `TileRenderer::upload(&mut self, device, queue, key: TileKey, frame: &RawFrame)`; `TileRenderer::retain(&mut self, keep: impl Fn(&TileKey) -> bool)`; `TileRenderer::update_fits(&self, queue: &wgpu::Queue, placements: &[(TileKey, PixelRect)])`; `TileRenderer::draw(&self, pass: &mut wgpu::RenderPass<'static>, placements: &[(TileKey, PixelRect)])`; `fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2]`.

- [ ] **Step 1: Move `fit_scale` and its tests into the new module**

Create `crates/app/src/render/tiles.rs`:

```rust
//! Draws every watched live as a letterboxed NV12 quad in its own viewport. One pipeline and
//! sampler are shared; each tile owns its planes, its fit uniform, and its bind group.

use std::collections::HashMap;

use brp_codec::RawFrame;
use bytemuck::{Pod, Zeroable};
use iroh::PublicKey;
use wgpu::util::DeviceExt;

use super::grid::PixelRect;

const SHADER: &str = include_str!("nv12.wgsl");

/// A watched live: publisher and live id. Tiles, watch handles, and per-tile UI choices share it.
pub type TileKey = (PublicKey, u32);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Fit {
    scale: [f32; 2],
    _pad: [f32; 2],
}

struct Tile {
    width: u32,
    height: u32,
    y: wgpu::Texture,
    uv: wgpu::Texture,
    fit: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

pub struct TileRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tiles: HashMap<TileKey, Tile>,
}

/// Clip-space scale that letterboxes or pillarboxes `video` inside `viewport`.
pub fn fit_scale(video: (u32, u32), viewport: (u32, u32)) -> [f32; 2] {
    let va = video.0 as f32 / video.1.max(1) as f32;
    let wa = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
    if va > wa {
        [1., wa / va]
    } else {
        [va / wa, 1.]
    }
}

#[cfg(test)]
mod tests {
    use super::fit_scale;
    #[test]
    fn wide_video_in_tall_window_letterboxes() {
        assert_eq!(fit_scale((1920, 1080), (1000, 1000)), [1., 0.5625]);
    }
    #[test]
    fn tall_video_in_wide_window_pillarboxes() {
        let [x, y] = fit_scale((1080, 1920), (1920, 1080));
        assert!((x - 0.3164).abs() < 0.001 && y == 1.0);
    }
    #[test]
    fn matching_aspect_fills_window() {
        assert_eq!(fit_scale((1280, 720), (1920, 1080)), [1., 1.]);
    }
}
```

In `crates/app/src/render/video.rs`: delete the `pub fn fit_scale` function and the whole `#[cfg(test)] mod tests` block, and add `use super::tiles::fit_scale;` to the imports. `VideoRenderer` keeps compiling until Task 7 deletes it.

Add `pub mod tiles;` to `crates/app/src/render/mod.rs`.

Run: `cargo test -p brp fit_scale`
Expected: 3 passed (now from `tiles`).

- [ ] **Step 2: Implement the renderer**

Append to `crates/app/src/render/tiles.rs` above the tests module:

```rust
impl TileRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12-layout"),
            entries: &[
                tex(0),
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nv12-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            tiles: HashMap::new(),
        }
    }

    /// Uploads a decoded frame, reallocating the tile's planes when the size changes.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: TileKey,
        frame: &RawFrame,
    ) {
        let needs_alloc = self
            .tiles
            .get(&key)
            .is_none_or(|t| (t.width, t.height) != (frame.width, frame.height));
        if needs_alloc {
            let tile = self.allocate(device, frame.width, frame.height);
            self.tiles.insert(key, tile);
        }
        let Some(tile) = self.tiles.get(&key) else {
            return;
        };
        let copy = |texture| wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        };
        queue.write_texture(
            copy(&tile.y),
            &frame.y,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.y_stride as u32),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            copy(&tile.uv),
            &frame.uv,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.uv_stride as u32),
                rows_per_image: Some(frame.chroma_rows() as u32),
            },
            wgpu::Extent3d {
                width: frame.width / 2,
                height: frame.chroma_rows() as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Drops planes for watches that no longer exist.
    pub fn retain(&mut self, keep: impl Fn(&TileKey) -> bool) {
        self.tiles.retain(|key, _| keep(key));
    }

    /// Writes each placed tile's letterbox scale. Call before the render pass is recorded.
    pub fn update_fits(&self, queue: &wgpu::Queue, placements: &[(TileKey, PixelRect)]) {
        for (key, rect) in placements {
            if let Some(tile) = self.tiles.get(key) {
                let fit = Fit {
                    scale: fit_scale(
                        (tile.width, tile.height),
                        (rect.width as u32, rect.height as u32),
                    ),
                    _pad: [0.; 2],
                };
                queue.write_buffer(&tile.fit, 0, bytemuck::bytes_of(&fit));
            }
        }
    }

    /// Draws every placed tile that has received a frame. Tiles without a frame stay black.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>, placements: &[(TileKey, PixelRect)]) {
        pass.set_pipeline(&self.pipeline);
        for (key, rect) in placements {
            let Some(tile) = self.tiles.get(key) else {
                continue;
            };
            if rect.width < 1.0 || rect.height < 1.0 {
                continue;
            }
            pass.set_viewport(rect.x, rect.y, rect.width, rect.height, 0.0, 1.0);
            pass.set_bind_group(0, &tile.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }

    fn allocate(&self, device: &wgpu::Device, width: u32, height: u32) -> Tile {
        let make = |label, w, h, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let y = make("nv12-y", width, height, wgpu::TextureFormat::R8Unorm);
        let uv = make(
            "nv12-uv",
            width / 2,
            height.div_ceil(2),
            wgpu::TextureFormat::Rg8Unorm,
        );
        let fit = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("nv12-fit"),
            contents: bytemuck::bytes_of(&Fit {
                scale: [1.; 2],
                _pad: [0.; 2],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let y_view = y.create_view(&Default::default());
        let uv_view = uv.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: fit.as_entire_binding(),
                },
            ],
        });
        Tile {
            width,
            height,
            y,
            uv,
            fit,
            bind_group,
        }
    }
}
```

- [ ] **Step 3: Build, test, lint**

Run: `cargo test -p brp && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green. The renderer has no GPU-free test; its correctness is checked in Task 8.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render
git commit -m "feat: render watched lives as tiles with per-tile viewports"
```

---

### Task 4: Preset edits for the bottom panel

**Files:**
- Create: `crates/app/src/presets.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `brp_proto::{LiveInfo, Preset, Codec, template_presets}`, `constants::{SOURCE_PRESET_ID, MIN_BITRATE_KBPS, MAX_BITRATE_KBPS}`.
- Produces: `presets::templates_for(info: &LiveInfo) -> Vec<Preset>`, `presets::toggle_template(info: &LiveInfo, template_id: u32) -> Vec<Preset>`, `presets::with_bitrate(info: &LiveInfo, preset_id: u32, kbps: u32) -> Vec<Preset>`, `presets::with_fps(info: &LiveInfo, fps: u32) -> Vec<Preset>`, `presets::with_codec(info: &LiveInfo, codec: Codec) -> Vec<Preset>`. Every function returns the full list to hand to `Room::set_presets`.

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/presets.rs`:

```rust
//! Pure edits to one live's preset list. The bottom panel calls these and hands the result to
//! `Room::set_presets`, which validates and restarts the affected encoders.

use brp_proto::constants::{MAX_BITRATE_KBPS, MIN_BITRATE_KBPS, SOURCE_PRESET_ID};
use brp_proto::{Codec, LiveInfo, Preset, template_presets};

/// The derived presets the templates offer for this live at its current frame rate and codec,
/// Source excluded. Empty when the live has no Source preset, which the registry never produces.
pub fn templates_for(info: &LiveInfo) -> Vec<Preset> {
    todo!()
}

/// Adds the template when absent, removes it when present. Source is never removed.
pub fn toggle_template(info: &LiveInfo, template_id: u32) -> Vec<Preset> {
    todo!()
}

pub fn with_bitrate(info: &LiveInfo, preset_id: u32, kbps: u32) -> Vec<Preset> {
    todo!()
}

/// Sets every preset's frame rate, clamped to one through the source rate.
pub fn with_fps(info: &LiveInfo, fps: u32) -> Vec<Preset> {
    todo!()
}

pub fn with_codec(info: &LiveInfo, codec: Codec) -> Vec<Preset> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_proto::SourceKind;

    fn live() -> LiveInfo {
        let mut presets = template_presets(2560, 1440, 60, Codec::Hevc);
        // Source plus 1080p only, so 720p and 480p are available templates.
        presets.retain(|p| p.id <= 2);
        LiveInfo {
            id: 1,
            title: "desk".into(),
            kind: SourceKind::Monitor,
            source_width: 2560,
            source_height: 1440,
            source_fps: 60,
            has_audio: false,
            presets,
        }
    }

    #[test]
    fn templates_exclude_source_and_follow_the_current_rate_and_codec() {
        let mut info = live();
        for preset in &mut info.presets {
            preset.fps = 30;
            preset.codec = Codec::Av1;
        }
        let templates = templates_for(&info);
        let ids: Vec<u32> = templates.iter().map(|p| p.id).collect();
        assert_eq!(ids, [2, 3, 4]);
        assert!(templates.iter().all(|p| p.fps == 30 && p.codec == Codec::Av1));
    }

    #[test]
    fn toggling_adds_a_missing_template_in_id_order_and_removes_a_present_one() {
        let info = live();
        let added = toggle_template(&info, 4);
        assert_eq!(added.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!((added[2].width, added[2].height), (852, 480));
        let removed = toggle_template(&info, 2);
        assert_eq!(removed.iter().map(|p| p.id).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn source_cannot_be_toggled_off_and_unknown_ids_are_ignored() {
        let info = live();
        assert_eq!(toggle_template(&info, SOURCE_PRESET_ID), info.presets);
        assert_eq!(toggle_template(&info, 99), info.presets);
    }

    #[test]
    fn bitrate_applies_to_one_preset_and_clamps_to_the_allowed_range() {
        let info = live();
        let presets = with_bitrate(&info, 2, 999_999);
        assert_eq!(presets[1].bitrate_kbps, MAX_BITRATE_KBPS);
        assert_eq!(presets[0].bitrate_kbps, info.presets[0].bitrate_kbps);
        assert_eq!(with_bitrate(&info, 1, 10)[0].bitrate_kbps, MIN_BITRATE_KBPS);
    }

    #[test]
    fn frame_rate_applies_to_every_preset_within_the_source_rate() {
        let info = live();
        assert!(with_fps(&info, 30).iter().all(|p| p.fps == 30));
        assert!(with_fps(&info, 144).iter().all(|p| p.fps == 60));
        assert!(with_fps(&info, 0).iter().all(|p| p.fps == 1));
    }

    #[test]
    fn codec_applies_to_every_preset() {
        assert!(
            with_codec(&live(), Codec::H264)
                .iter()
                .all(|p| p.codec == Codec::H264)
        );
    }
}
```

Add `pub mod presets;` to `crates/app/src/lib.rs` (keep the list alphabetical).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p brp presets`
Expected: panics at `todo!()`.

- [ ] **Step 3: Implement**

```rust
pub fn templates_for(info: &LiveInfo) -> Vec<Preset> {
    let Some(source) = info.presets.iter().find(|p| p.id == SOURCE_PRESET_ID) else {
        return Vec::new();
    };
    template_presets(
        info.source_width,
        info.source_height,
        source.fps,
        source.codec,
    )
    .into_iter()
    .filter(|p| p.id != SOURCE_PRESET_ID)
    .collect()
}

pub fn toggle_template(info: &LiveInfo, template_id: u32) -> Vec<Preset> {
    let mut presets = info.presets.clone();
    if let Some(position) = presets.iter().position(|p| p.id == template_id) {
        if template_id != SOURCE_PRESET_ID {
            presets.remove(position);
        }
    } else if let Some(template) = templates_for(info)
        .into_iter()
        .find(|p| p.id == template_id)
    {
        presets.push(template);
        presets.sort_by_key(|p| p.id);
    }
    presets
}

pub fn with_bitrate(info: &LiveInfo, preset_id: u32, kbps: u32) -> Vec<Preset> {
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            if preset.id == preset_id {
                preset.bitrate_kbps = kbps.clamp(MIN_BITRATE_KBPS, MAX_BITRATE_KBPS);
            }
            preset
        })
        .collect()
}

pub fn with_fps(info: &LiveInfo, fps: u32) -> Vec<Preset> {
    let fps = fps.clamp(1, info.source_fps.max(1));
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            preset.fps = fps;
            preset
        })
        .collect()
}

pub fn with_codec(info: &LiveInfo, codec: Codec) -> Vec<Preset> {
    info.presets
        .iter()
        .cloned()
        .map(|mut preset| {
            preset.codec = codec;
            preset
        })
        .collect()
}
```

- [ ] **Step 4: Run the tests and lints**

Run: `cargo test -p brp presets && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 6 passed, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/presets.rs crates/app/src/lib.rs
git commit -m "feat: add pure preset edits for the own-lives panel"
```

---

### Task 5: Room commands and window-local UI state

**Files:**
- Create: `crates/app/src/commands.rs`, `crates/app/src/ui/mod.rs`, `crates/app/src/ui/state.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `render::tiles::TileKey`; `brp_room::{RoomSnapshot, MemberView, WatchView}`; `brp_proto::{Preset, SourceKind}`; `constants::STATS_LOG_INTERVAL`.
- Produces: `commands::RoomCommand` enum; `ui::state::UiState` with `new()`, `next_title(&mut self, kind: SourceKind) -> String`, and public fields `status: String`, `share_pending: bool`, `preset_choice: HashMap<TileKey, u32>`, `stats_visible: HashSet<TileKey>`, `bitrate_edits: HashMap<(u32, u32), u32>`, `fps_edits: HashMap<u32, u32>`, `upload_kbps: u64`; `ui::state::BitrateMeter::default()` with `update(&mut self, total_bytes: u64, now: Instant) -> u64`; `ui::state::ordered_members(&RoomSnapshot) -> Vec<&MemberView>`; `ui::state::ordered_watches(&RoomSnapshot) -> Vec<&WatchView>`; `ui::state::total_encoded_bytes(&RoomSnapshot) -> u64`.

- [ ] **Step 1: Write the command enum**

Create `crates/app/src/commands.rs`:

```rust
//! What the panels ask the room to do. Panels only emit these; the window applies them after the
//! egui pass, so widget code never holds the room.

use brp_proto::{Preset, SourceKind};

use crate::render::tiles::TileKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomCommand {
    /// Starts a watch, or switches its preset when the key is already watched.
    Watch { key: TileKey, preset_id: u32 },
    Unwatch(TileKey),
    /// Opens the portal picker for a new live of this kind.
    Share(SourceKind),
    StopLive(u32),
    SetPresets { live_id: u32, presets: Vec<Preset> },
}
```

- [ ] **Step 2: Write the failing state tests**

Create `crates/app/src/ui/mod.rs`:

```rust
//! The participant window's egui chrome. Panels read the room snapshot and window-local state and
//! emit room commands; they never touch the room.

pub mod state;
```

Create `crates/app/src/ui/state.rs`:

```rust
//! State the snapshot cannot carry, and the ordering and rate helpers every panel shares.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use brp_proto::SourceKind;
use brp_proto::constants::STATS_LOG_INTERVAL;
use brp_room::{MemberView, RoomSnapshot, WatchView};

use crate::render::tiles::TileKey;

#[derive(Debug, Default)]
pub struct UiState {
    /// Last error or notice, shown in the status bar.
    pub status: String,
    /// True while the portal picker is open for a new live.
    pub share_pending: bool,
    monitor_shares: u32,
    window_shares: u32,
    /// Preset picked for a remote live before it is watched.
    pub preset_choice: HashMap<TileKey, u32>,
    pub stats_visible: HashSet<TileKey>,
    /// Bitrate being edited per (live id, preset id), committed when the widget is released.
    pub bitrate_edits: HashMap<(u32, u32), u32>,
    /// Frame rate being edited per live id, committed when the widget is released.
    pub fps_edits: HashMap<u32, u32>,
    pub upload_kbps: u64,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// New lives are titled by kind and ordinal; the ordinal never repeats within a session.
    pub fn next_title(&mut self, kind: SourceKind) -> String {
        todo!()
    }
}

/// Aggregate encode rate from the cumulative byte counters in successive snapshots.
#[derive(Debug, Default)]
pub struct BitrateMeter {
    last_bytes: u64,
    last_at: Option<Instant>,
    kbps: u64,
}

impl BitrateMeter {
    /// Recomputes once per stats interval; in between it returns the previous rate.
    pub fn update(&mut self, total_bytes: u64, now: Instant) -> u64 {
        todo!()
    }
}

pub fn total_encoded_bytes(snapshot: &RoomSnapshot) -> u64 {
    snapshot
        .own_lives
        .iter()
        .flat_map(|live| live.presets.iter())
        .filter_map(|preset| preset.encoder.as_ref())
        .map(|encoder| encoder.bytes_encoded)
        .sum()
}

/// Members by nickname then id, so the panel does not reorder between snapshots.
pub fn ordered_members(snapshot: &RoomSnapshot) -> Vec<&MemberView> {
    todo!()
}

/// Watches by publisher then live id, so tiles keep their grid position.
pub fn ordered_watches(snapshot: &RoomSnapshot) -> Vec<&WatchView> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use brp_net::PathKind;
    use iroh::SecretKey;
    use std::time::Duration;

    #[test]
    fn titles_count_per_kind_and_never_repeat() {
        let mut state = UiState::new();
        assert_eq!(state.next_title(SourceKind::Monitor), "Monitor 1");
        assert_eq!(state.next_title(SourceKind::Window), "Window 1");
        assert_eq!(state.next_title(SourceKind::Monitor), "Monitor 2");
    }

    #[test]
    fn meter_reports_kilobits_per_second_once_per_interval() {
        let start = Instant::now();
        let mut meter = BitrateMeter::default();
        assert_eq!(meter.update(0, start), 0);
        assert_eq!(meter.update(1_000, start + Duration::from_millis(100)), 0);
        let bytes = 250_000 * STATS_LOG_INTERVAL.as_secs();
        assert_eq!(meter.update(bytes, start + STATS_LOG_INTERVAL), 2_000);
        assert_eq!(
            meter.update(bytes, start + STATS_LOG_INTERVAL + Duration::from_millis(1)),
            2_000
        );
    }

    fn member(nickname: &str) -> MemberView {
        MemberView {
            id: SecretKey::generate().public(),
            nickname: nickname.into(),
            lives: Vec::new(),
            seen_ago_ms: 0,
            path: PathKind::Unknown,
        }
    }

    #[test]
    fn members_sort_by_nickname() {
        let me = SecretKey::generate().public();
        let snapshot = RoomSnapshot {
            me,
            nickname: "me".into(),
            version: 1,
            members: vec![member("zed"), member("amy"), member("kim")],
            own_lives: Vec::new(),
            watches: Vec::new(),
        };
        let names: Vec<&str> = ordered_members(&snapshot)
            .iter()
            .map(|m| m.nickname.as_str())
            .collect();
        assert_eq!(names, ["amy", "kim", "zed"]);
    }
}
```

Add to `crates/app/src/lib.rs`: `pub mod commands;` and `pub mod ui;` (alphabetical). Add `iroh.workspace = true` is already in `crates/app/Cargo.toml`; confirm `brp-net.workspace = true` is present too (it is).

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p brp ui::state`
Expected: panics at `todo!()`.

- [ ] **Step 4: Implement**

```rust
    pub fn next_title(&mut self, kind: SourceKind) -> String {
        let counter = match kind {
            SourceKind::Monitor => &mut self.monitor_shares,
            SourceKind::Window => &mut self.window_shares,
        };
        *counter += 1;
        let name = match kind {
            SourceKind::Monitor => "Monitor",
            SourceKind::Window => "Window",
        };
        format!("{name} {counter}")
    }
```

```rust
    pub fn update(&mut self, total_bytes: u64, now: Instant) -> u64 {
        let Some(last_at) = self.last_at else {
            self.last_at = Some(now);
            self.last_bytes = total_bytes;
            return self.kbps;
        };
        let elapsed = now.duration_since(last_at);
        if elapsed < STATS_LOG_INTERVAL {
            return self.kbps;
        }
        let bits = total_bytes.saturating_sub(self.last_bytes) * 8;
        self.kbps = bits / 1000 / elapsed.as_secs().max(1);
        self.last_bytes = total_bytes;
        self.last_at = Some(now);
        self.kbps
    }
```

```rust
pub fn ordered_members(snapshot: &RoomSnapshot) -> Vec<&MemberView> {
    let mut members: Vec<&MemberView> = snapshot.members.iter().collect();
    members.sort_by(|a, b| {
        a.nickname
            .cmp(&b.nickname)
            .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    members
}

pub fn ordered_watches(snapshot: &RoomSnapshot) -> Vec<&WatchView> {
    let mut watches: Vec<&WatchView> = snapshot.watches.iter().collect();
    watches.sort_by(|a, b| {
        a.publisher
            .as_bytes()
            .cmp(b.publisher.as_bytes())
            .then(a.live_id.cmp(&b.live_id))
    });
    watches
}
```

- [ ] **Step 5: Run the tests and lints**

Run: `cargo test -p brp ui::state && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 3 passed, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/commands.rs crates/app/src/ui crates/app/src/lib.rs
git commit -m "feat: add room commands and window-local UI state"
```

---

### Task 6: The panels

**Files:**
- Create: `crates/app/src/ui/members.rs`, `crates/app/src/ui/own_lives.rs`, `crates/app/src/ui/status.rs`, `crates/app/src/ui/tiles.rs`
- Modify: `crates/app/src/ui/mod.rs`

**Interfaces:**
- Consumes: Task 4 `presets::*`; Task 5 `RoomCommand`, `UiState`, `ordered_members`, `ordered_watches`; Task 2 `grid::layout`; Task 3 `TileKey`; `brp_room::{RoomSnapshot, OwnLiveView, WatchView, WatchState}`; `brp_net::PathKind`; `brp_proto::{Codec, Preset, SourceKind}` and `constants::{SOURCE_PRESET_ID, MIN_BITRATE_KBPS, MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT}`.
- Produces: `ui::draw(ctx: &egui::Context, snapshot: &RoomSnapshot, ticket: &str, state: &mut UiState) -> UiOutput`; `ui::UiOutput { commands: Vec<RoomCommand>, tile_rects: Vec<(TileKey, egui::Rect)> }` deriving `Default`; `ui::members::preset_selector(ui: &mut egui::Ui, id_salt: impl Hash + Debug, presets: &[Preset], selected: &mut u32)`.

- [ ] **Step 1: Status bar**

Create `crates/app/src/ui/status.rs`:

```rust
//! Bottom status bar: ticket, member count, upload rate, identity, last notice.

use brp_room::RoomSnapshot;

use super::state::UiState;

pub fn draw(ctx: &egui::Context, snapshot: &RoomSnapshot, ticket: &str, state: &UiState) {
    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Copy ticket").clicked() {
                ui.ctx().copy_text(ticket.to_string());
            }
            ui.separator();
            let members = snapshot.members.len();
            let plural = if members == 1 { "" } else { "s" };
            ui.label(format!("{members} member{plural}"));
            ui.separator();
            ui.label(format!("up {} kbps", state.upload_kbps));
            ui.separator();
            ui.label(format!("{} ({})", snapshot.nickname, snapshot.me.fmt_short()));
            if !state.status.is_empty() {
                ui.separator();
                ui.weak(state.status.as_str());
            }
        });
    });
}
```

- [ ] **Step 2: Members panel**

Create `crates/app/src/ui/members.rs`:

```rust
//! Left panel: every member with a path badge, and each of their lives with a watch checkbox and
//! a preset selector.

use std::fmt::Debug;
use std::hash::Hash;

use brp_net::PathKind;
use brp_proto::Preset;
use brp_proto::constants::SOURCE_PRESET_ID;
use brp_room::RoomSnapshot;

use super::state::{UiState, ordered_members};
use crate::commands::RoomCommand;

pub fn draw(
    ctx: &egui::Context,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::SidePanel::left("members")
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Room");
            if snapshot.members.is_empty() {
                ui.weak("nobody else yet");
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for member in ordered_members(snapshot) {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.strong(member.nickname.as_str());
                        ui.weak(member.id.fmt_short().to_string());
                        ui.weak(path_badge(member.path));
                    });
                    for live in &member.lives {
                        let key = (member.id, live.id);
                        let watch = snapshot
                            .watches
                            .iter()
                            .find(|w| w.publisher == key.0 && w.live_id == key.1);
                        let mut watched = watch.is_some();
                        let mut preset_id = watch
                            .map(|w| w.preset_id)
                            .or_else(|| state.preset_choice.get(&key).copied())
                            .unwrap_or(SOURCE_PRESET_ID);
                        // A remembered choice the publisher has since removed falls back to Source.
                        if !live.presets.iter().any(|p| p.id == preset_id) {
                            preset_id = SOURCE_PRESET_ID;
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            let label = format!(
                                "{} {}x{}",
                                live.title, live.source_width, live.source_height
                            );
                            if ui.checkbox(&mut watched, label).changed() {
                                commands.push(if watched {
                                    RoomCommand::Watch { key, preset_id }
                                } else {
                                    RoomCommand::Unwatch(key)
                                });
                            }
                            let before = preset_id;
                            preset_selector(ui, ("member-preset", key), &live.presets, &mut preset_id);
                            if preset_id != before {
                                state.preset_choice.insert(key, preset_id);
                                if watched {
                                    commands.push(RoomCommand::Watch { key, preset_id });
                                }
                            }
                        });
                    }
                }
            });
        });
}

pub fn path_badge(path: PathKind) -> &'static str {
    match path {
        PathKind::Direct => "direct",
        PathKind::Relayed => "relayed",
        PathKind::Unknown => "path unknown",
    }
}

/// Shared by the members panel and the tile overlay.
pub fn preset_selector(
    ui: &mut egui::Ui,
    id_salt: impl Hash + Debug,
    presets: &[Preset],
    selected: &mut u32,
) {
    let text = presets
        .iter()
        .find(|p| p.id == *selected)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for preset in presets {
                ui.selectable_value(
                    selected,
                    preset.id,
                    format!("{} {}x{}", preset.name, preset.width, preset.height),
                );
            }
        });
}
```

- [ ] **Step 3: Own lives panel**

Create `crates/app/src/ui/own_lives.rs`:

```rust
//! Bottom panel: this participant's lives with per-preset encoder state and the controls that edit
//! presets, plus the share buttons.

use brp_proto::constants::{MAX_BITRATE_KBPS, MAX_LIVES_PER_PARTICIPANT, MIN_BITRATE_KBPS};
use brp_proto::{Codec, SourceKind};
use brp_room::{OwnLiveView, RoomSnapshot};

use super::state::UiState;
use crate::commands::RoomCommand;
use crate::presets;

pub fn draw(
    ctx: &egui::Context,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    egui::TopBottomPanel::bottom("own-lives")
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("My lives");
                let can_share = !state.share_pending
                    && snapshot.own_lives.len() < MAX_LIVES_PER_PARTICIPANT;
                if ui
                    .add_enabled(can_share, egui::Button::new("Share monitor"))
                    .clicked()
                {
                    commands.push(RoomCommand::Share(SourceKind::Monitor));
                }
                if ui
                    .add_enabled(can_share, egui::Button::new("Share window"))
                    .clicked()
                {
                    commands.push(RoomCommand::Share(SourceKind::Window));
                }
                if state.share_pending {
                    ui.weak("waiting for the picker");
                }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for live in &snapshot.own_lives {
                    live_rows(ui, live, state, commands);
                }
            });
        });
}

fn live_rows(
    ui: &mut egui::Ui,
    live: &OwnLiveView,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) {
    let info = &live.info;
    let current_fps = info.presets.first().map(|p| p.fps).unwrap_or(info.source_fps);
    let current_codec = info.presets.first().map(|p| p.codec).unwrap_or(Codec::H264);
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong(info.title.as_str());
        ui.weak(format!(
            "{}x{} @ {} fps",
            info.source_width, info.source_height, info.source_fps
        ));

        let fps = state.fps_edits.entry(info.id).or_insert(current_fps);
        let response = ui.add(
            egui::DragValue::new(fps)
                .range(1..=info.source_fps.max(1))
                .suffix(" fps"),
        );
        // Commit on release rather than on every change: each commit restarts the encoders.
        if !response.dragged() && !response.has_focus() {
            let value = *fps;
            state.fps_edits.remove(&info.id);
            if value != current_fps {
                commands.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::with_fps(info, value),
                });
            }
        }

        let mut codec = current_codec;
        egui::ComboBox::from_id_salt(("codec", info.id))
            .selected_text(codec_name(codec))
            .show_ui(ui, |ui| {
                for candidate in [Codec::Hevc, Codec::H264, Codec::Av1] {
                    ui.selectable_value(&mut codec, candidate, codec_name(candidate));
                }
            });
        if codec != current_codec {
            commands.push(RoomCommand::SetPresets {
                live_id: info.id,
                presets: presets::with_codec(info, codec),
            });
        }

        if ui.button("Stop").clicked() {
            commands.push(RoomCommand::StopLive(info.id));
        }
    });

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label("Templates:");
        for template in presets::templates_for(info) {
            let mut enabled = info.presets.iter().any(|p| p.id == template.id);
            if ui.checkbox(&mut enabled, template.name.as_str()).changed() {
                commands.push(RoomCommand::SetPresets {
                    live_id: info.id,
                    presets: presets::toggle_template(info, template.id),
                });
            }
        }
    });

    for view in &live.presets {
        let preset = &view.preset;
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.monospace(format!(
                "{:<7} {}x{}",
                preset.name, preset.width, preset.height
            ));
            let key = (info.id, preset.id);
            let kbps = state.bitrate_edits.entry(key).or_insert(preset.bitrate_kbps);
            let response = ui.add(
                egui::DragValue::new(kbps)
                    .range(MIN_BITRATE_KBPS..=MAX_BITRATE_KBPS)
                    .speed(100.0)
                    .suffix(" kbps"),
            );
            if !response.dragged() && !response.has_focus() {
                let value = *kbps;
                state.bitrate_edits.remove(&key);
                if value != preset.bitrate_kbps {
                    commands.push(RoomCommand::SetPresets {
                        live_id: info.id,
                        presets: presets::with_bitrate(info, preset.id, value),
                    });
                }
            }
            match (&view.encoder, &view.last_error) {
                (Some(encoder), _) => {
                    let plural = if encoder.subscribers == 1 { "" } else { "s" };
                    ui.label(format!(
                        "{} · {} viewer{plural} · {} frames",
                        encoder.name, encoder.subscribers, encoder.frames_encoded
                    ))
                }
                (None, Some(error)) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("failed: {error}"))
                }
                (None, None) => ui.weak("idle"),
            };
        });
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}
```

- [ ] **Step 4: Tile grid and hover overlays**

Create `crates/app/src/ui/tiles.rs`:

```rust
//! Central panel: reserves one rect per watched live for the video renderer and draws the hover
//! overlay and status text on top. The panel frame is transparent so the tiles show through.

use brp_room::{RoomSnapshot, WatchState, WatchView};

use super::members::preset_selector;
use super::state::{UiState, ordered_watches};
use crate::commands::RoomCommand;
use crate::render::grid;
use crate::render::tiles::TileKey;

pub fn draw(
    ctx: &egui::Context,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
) -> Vec<(TileKey, egui::Rect)> {
    let mut placements = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let watches = ordered_watches(snapshot);
            if watches.is_empty() {
                ui.centered_and_justified(|ui| ui.weak("Tick a live on the left to watch it"));
                return;
            }
            let rects = grid::layout(ui.max_rect(), watches.len());
            for (watch, rect) in watches.iter().zip(rects) {
                let key = (watch.publisher, watch.live_id);
                placements.push((key, rect));
                overlay(ui, snapshot, state, commands, watch, key, rect);
            }
        });
    placements
}

fn overlay(
    ui: &mut egui::Ui,
    snapshot: &RoomSnapshot,
    state: &mut UiState,
    commands: &mut Vec<RoomCommand>,
    watch: &WatchView,
    key: TileKey,
    rect: egui::Rect,
) {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    let live = snapshot
        .members
        .iter()
        .find(|m| m.id == key.0)
        .and_then(|m| m.lives.iter().find(|l| l.id == key.1));
    let title = live
        .map(|l| l.title.clone())
        .unwrap_or_else(|| "publisher left".to_string());
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
                let mut preset_id = watch.preset_id;
                preset_selector(ui, ("tile-preset", key), &live.presets, &mut preset_id);
                if preset_id != watch.preset_id {
                    state.preset_choice.insert(key, preset_id);
                    commands.push(RoomCommand::Watch { key, preset_id });
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
            if ui.small_button("close").clicked() {
                commands.push(RoomCommand::Unwatch(key));
            }
        },
    );
}
```

- [ ] **Step 5: Compose the panels**

Replace `crates/app/src/ui/mod.rs` with:

```rust
//! The participant window's egui chrome. Panels read the room snapshot and window-local state and
//! emit room commands; they never touch the room.

pub mod members;
pub mod own_lives;
pub mod state;
pub mod status;
pub mod tiles;

use brp_room::RoomSnapshot;

use crate::commands::RoomCommand;
use crate::render::tiles::TileKey;
use state::UiState;

#[derive(Debug, Default)]
pub struct UiOutput {
    pub commands: Vec<RoomCommand>,
    /// Where the video renderer draws each watched live, in egui points.
    pub tile_rects: Vec<(TileKey, egui::Rect)>,
}

/// Panels are declared outermost first; the central panel takes what remains.
pub fn draw(
    ctx: &egui::Context,
    snapshot: &RoomSnapshot,
    ticket: &str,
    state: &mut UiState,
) -> UiOutput {
    let mut commands = Vec::new();
    status::draw(ctx, snapshot, ticket, state);
    own_lives::draw(ctx, snapshot, state, &mut commands);
    members::draw(ctx, snapshot, state, &mut commands);
    let tile_rects = tiles::draw(ctx, snapshot, state, &mut commands);
    UiOutput {
        commands,
        tile_rects,
    }
}
```

- [ ] **Step 6: Build and lint**

Run: `cargo test -p brp && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green. If clippy flags `too_many_arguments` on `overlay`, group `watch`, `key`, and `rect` into a small `struct Placed<'a> { watch: &'a WatchView, key: TileKey, rect: egui::Rect }` built in the loop and passed by value.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/ui
git commit -m "feat: add the participant window panels"
```

---

### Task 7: The window, `create` and `join`, removal of `watch`

**Files:**
- Create: `crates/app/src/participant.rs`
- Modify: `crates/app/src/window.rs` (rewrite), `crates/app/src/render/ui.rs`, `crates/app/src/render/mod.rs`, `crates/app/src/cli.rs`, `crates/app/src/main.rs`, `crates/app/src/lib.rs`, `crates/app/src/error.rs`
- Delete: `crates/app/src/watch.rs`, `crates/app/src/render/video.rs`

**Interfaces:**
- Consumes: Task 2 `grid::to_pixels`; Task 3 `TileRenderer`, `TileKey`; Task 5 `RoomCommand`, `UiState`, `BitrateMeter`, `total_encoded_bytes`; Task 6 `ui::draw`, `UiOutput`; `brp_room::{Room, RoomConfig, RoomTimings, RoomSnapshot, WatchHandle}`; `identity::load_or_create`; `GpuContext`, `EguiLayer`.
- Produces: `cli::Command::{Publish(PublishArgs), Create(CreateArgs), Join(JoinArgs)}`, `cli::WindowArgs { nickname: Option<String>, fps: u32, no_relay: bool }`; `participant::run(runtime: &Runtime, ticket: Option<String>, args: WindowArgs) -> Result<(), AppError>`; `window::App::new(runtime: tokio::runtime::Handle, room: Arc<Room>, proxy: EventLoopProxy<AppEvent>) -> App`; `window::App::take_pending_share(&mut self) -> Option<JoinHandle<()>>`; `window::AppEvent::{RoomChanged, NewFrame, Tick, ShareFinished(Result<(), String>)}`; `render::ui::UiFrame.repaint_delay: Duration`.

- [ ] **Step 1: Surface egui's repaint request**

In `crates/app/src/render/ui.rs`, add `pub repaint_delay: std::time::Duration,` to `UiFrame` with the doc comment `/// Zero when egui wants another frame at once, for animations and open popups.` and, in `run`, before constructing `UiFrame`:

```rust
        let repaint_delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .unwrap_or(std::time::Duration::MAX);
```

and include `repaint_delay,` in the returned struct.

- [ ] **Step 2: CLI**

Replace the `Command`, `WatchArgs` parts of `crates/app/src/cli.rs`:

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Share one live headlessly and print the ticket.
    Publish(PublishArgs),
    /// Open a new room in the participant window.
    Create(CreateArgs),
    /// Join a room with a ticket in the participant window.
    Join(JoinArgs),
}
```

Delete `WatchArgs` and add:

```rust
#[derive(Args, Debug)]
pub struct WindowArgs {
    /// Shown to other participants. Defaults to the short peer id.
    #[arg(long)]
    pub nickname: Option<String>,
    /// Capture ceiling for lives shared from the window; each live's presets can go lower.
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    #[arg(long)]
    pub no_relay: bool,
}
#[derive(Args, Debug)]
pub struct CreateArgs {
    #[command(flatten)]
    pub window: WindowArgs,
}
#[derive(Args, Debug)]
pub struct JoinArgs {
    pub ticket: String,
    #[command(flatten)]
    pub window: WindowArgs,
}
```

`PublishArgs`, `CodecArg`, and `SourceArg` are unchanged.

- [ ] **Step 3: Participant entry point**

Create `crates/app/src/participant.rs`:

```rust
//! `brp create` and `brp join`: a room participant with the window. Owns the room's lifetime
//! around the winit loop and tears it down when the window closes.

use std::str::FromStr;
use std::sync::Arc;

use brp_capture::PortalCapture;
use brp_net::RelaySetting;
use brp_proto::RoomTicket;
use brp_proto::constants::{RELAY_ONLINE_TIMEOUT, STATS_LOG_INTERVAL};
use brp_room::codecs::FfmpegCodecs;
use brp_room::{Room, RoomConfig, RoomTimings};
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

use crate::cli::WindowArgs;
use crate::error::AppError;
use crate::identity;
use crate::window::{App, AppEvent};

pub fn run(runtime: &Runtime, ticket: Option<String>, args: WindowArgs) -> Result<(), AppError> {
    let ticket = ticket
        .as_deref()
        .map(RoomTicket::from_str)
        .transpose()?;
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let change_proxy = proxy.clone();
    let frame_proxy = proxy.clone();

    let room = runtime.block_on(async {
        let secret = identity::load_or_create()?;
        let nickname = args
            .nickname
            .clone()
            .unwrap_or_else(|| secret.public().fmt_short().to_string());
        let config = RoomConfig {
            secret,
            relay,
            nickname,
            target_fps: args.fps,
            capture: Arc::new(PortalCapture),
            encoders: Arc::new(FfmpegCodecs::default()),
            decoders: Arc::new(FfmpegCodecs::default()),
            on_change: Arc::new(move || {
                let _ = change_proxy.send_event(AppEvent::RoomChanged);
            }),
            on_frame: Arc::new(move || {
                let _ = frame_proxy.send_event(AppEvent::NewFrame);
            }),
            timings: RoomTimings::default(),
        };
        let room = match ticket {
            Some(ticket) => Room::join(config, ticket).await?,
            None => Room::create(config).await?,
        };
        if relay == RelaySetting::Default && !room.online(RELAY_ONLINE_TIMEOUT).await {
            tracing::warn!(
                "relay registration timed out; the ticket may only work on the local network"
            );
        }
        Ok::<_, AppError>(Arc::new(room))
    })?;
    println!("Ticket:\n{}\n", room.ticket());

    // Encoder byte counters and last-seen ages move without any frame arriving, so a slow tick
    // keeps the status bar honest when nothing is watched.
    let ticker = runtime.spawn({
        let proxy = proxy.clone();
        async move {
            let mut tick = tokio::time::interval(STATS_LOG_INTERVAL);
            loop {
                tick.tick().await;
                if proxy.send_event(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    let mut app = App::new(runtime.handle().clone(), room.clone(), proxy);
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));

    ticker.abort();
    let pending_share = app.take_pending_share();
    drop(app);
    // Abort only requests cancellation; wait for both tasks so their Arc<Room> clones are gone
    // before the room is unwrapped (a cancelled JoinError is expected).
    let _ = runtime.block_on(ticker);
    if let Some(task) = pending_share {
        task.abort();
        let _ = runtime.block_on(task);
    }
    match Arc::try_unwrap(room) {
        Ok(room) => runtime.block_on(room.leave()),
        Err(_) => tracing::warn!("room still referenced at exit; skipping the orderly leave"),
    }
    outcome
}
```

- [ ] **Step 4: The window**

Replace `crates/app/src/window.rs` entirely:

```rust
//! The participant window: a winit loop that draws the tile grid under the egui panels and turns
//! panel commands into room calls.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use brp_proto::SourceKind;
use brp_room::{Room, RoomSnapshot, WatchHandle};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy},
    window::{Window, WindowId},
};

use crate::commands::RoomCommand;
use crate::error::AppError;
use crate::render::grid::{self, PixelRect};
use crate::render::tiles::{TileKey, TileRenderer};
use crate::render::{GpuContext, ui::EguiLayer};
use crate::ui::{self, UiOutput};
use crate::ui::state::{BitrateMeter, UiState, total_encoded_bytes};

pub enum AppEvent {
    /// The room's version counter moved; re-snapshot on the next redraw.
    RoomChanged,
    /// A watched live decoded a frame.
    NewFrame,
    /// Periodic wake so counters refresh while nothing is watched.
    Tick,
    /// The portal picker closed: the live started, or the error to show.
    ShareFinished(Result<(), String>),
}

pub struct App {
    runtime: Handle,
    room: Arc<Room>,
    proxy: EventLoopProxy<AppEvent>,
    snapshot: RoomSnapshot,
    ticket: String,
    state: UiState,
    meter: BitrateMeter,
    handles: HashMap<TileKey, WatchHandle>,
    pending_share: Option<JoinHandle<()>>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    tiles: Option<TileRenderer>,
    ui: Option<EguiLayer>,
}

impl App {
    pub fn new(runtime: Handle, room: Arc<Room>, proxy: EventLoopProxy<AppEvent>) -> Self {
        let snapshot = room.snapshot();
        let ticket = room.ticket().to_string();
        Self {
            runtime,
            room,
            proxy,
            snapshot,
            ticket,
            state: UiState::new(),
            meter: BitrateMeter::default(),
            handles: HashMap::new(),
            pending_share: None,
            window: None,
            gpu: None,
            tiles: None,
            ui: None,
        }
    }

    /// A share still waiting on the portal holds an `Arc<Room>`; the caller aborts it before leaving.
    pub fn take_pending_share(&mut self) -> Option<JoinHandle<()>> {
        self.pending_share.take()
    }

    fn refresh(&mut self) {
        if self.room.version() != self.snapshot.version {
            self.snapshot = self.room.snapshot();
        }
        // The relay address can arrive after the first snapshot without bumping the version.
        self.ticket = self.room.ticket().to_string();
        let live: HashSet<TileKey> = self
            .snapshot
            .watches
            .iter()
            .map(|w| (w.publisher, w.live_id))
            .collect();
        self.handles.retain(|key, _| live.contains(key));
        if let Some(tiles) = self.tiles.as_mut() {
            tiles.retain(|key| live.contains(key));
        }
        self.state.upload_kbps = self
            .meter
            .update(total_encoded_bytes(&self.snapshot), Instant::now());
    }

    fn redraw(&mut self) {
        self.refresh();
        let (Some(window), Some(gpu), Some(tiles), Some(ui)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.tiles.as_mut(),
            self.ui.as_mut(),
        ) else {
            return;
        };
        for (key, handle) in &self.handles {
            if let Some(frame) = handle.slot.try_take() {
                tiles.upload(&gpu.device, &gpu.queue, *key, &frame);
            }
        }
        let surface = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let target = surface.texture.create_view(&Default::default());
        let size = (gpu.config.width, gpu.config.height);

        let mut output = UiOutput::default();
        let ui_frame = ui.run(window, [size.0, size.1], |ctx| {
            output = ui::draw(ctx, &self.snapshot, &self.ticket, &mut self.state);
        });
        let pixels_per_point = ui_frame.screen.pixels_per_point;
        let placements: Vec<(TileKey, PixelRect)> = output
            .tile_rects
            .iter()
            .map(|(key, rect)| (*key, grid::to_pixels(*rect, pixels_per_point, size)))
            .collect();
        tiles.update_fits(&gpu.queue, &placements);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let buffers = ui.prepare(&gpu.device, &gpu.queue, &mut encoder, &ui_frame);
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
            ui.paint(&mut pass, &ui_frame);
        }
        gpu.queue
            .submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
        ui.cleanup(&ui_frame);
        window.pre_present_notify();
        gpu.queue.present(surface);
        if ui_frame.repaint_delay.is_zero() {
            window.request_redraw();
        }

        self.apply(output.commands);
    }

    fn apply(&mut self, commands: Vec<RoomCommand>) {
        if commands.is_empty() {
            return;
        }
        // `Room::watch` spawns its task with `tokio::spawn`, which needs a runtime on this thread.
        // The handle is cloned so the guard does not borrow `self` while `share` needs it mutably.
        let runtime = self.runtime.clone();
        let _guard = runtime.enter();
        for command in commands {
            let result = match command {
                RoomCommand::Watch { key, preset_id } => {
                    self.room.watch(key.0, key.1, preset_id).map(|handle| {
                        self.handles.insert(key, handle);
                    })
                }
                RoomCommand::Unwatch(key) => self.room.unwatch(key.0, key.1).map(|()| {
                    self.handles.remove(&key);
                }),
                RoomCommand::StopLive(live_id) => self.room.stop_live(live_id),
                RoomCommand::SetPresets { live_id, presets } => {
                    self.room.set_presets(live_id, presets)
                }
                RoomCommand::Share(kind) => {
                    self.share(kind);
                    Ok(())
                }
            };
            if let Err(error) = result {
                self.state.status = error.to_string();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn share(&mut self, kind: SourceKind) {
        if self.pending_share.is_some() {
            return;
        }
        let title = self.state.next_title(kind);
        self.state.share_pending = true;
        self.state.status.clear();
        let room = self.room.clone();
        let proxy = self.proxy.clone();
        self.pending_share = Some(self.runtime.spawn(async move {
            let outcome = room
                .start_live(kind, title)
                .await
                .map(|_live_id| ())
                .map_err(|error| error.to_string());
            let _ = proxy.send_event(AppEvent::ShareFinished(outcome));
        }));
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = format!("brp: {}", self.snapshot.nickname);
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title(&title)
                .with_inner_size(PhysicalSize::new(1280, 720)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                tracing::error!(%error, "could not create window");
                event_loop.exit();
                return;
            }
        };
        let gpu = match GpuContext::new(event_loop, &window) {
            Ok(gpu) => gpu,
            Err(error) => {
                let _: AppError = error;
                event_loop.exit();
                return;
            }
        };
        self.tiles = Some(TileRenderer::new(&gpu.device, gpu.config.format));
        self.ui = Some(EguiLayer::new(&window, &gpu.device, gpu.config.format));
        self.gpu = Some(gpu);
        self.window = Some(window);
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::ShareFinished(outcome) => {
                self.state.share_pending = false;
                self.pending_share = None;
                if let Err(message) = outcome {
                    self.state.status = format!("share failed: {message}");
                }
            }
            AppEvent::RoomChanged | AppEvent::NewFrame | AppEvent::Tick => {}
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(ui) = self.ui.as_mut() {
            let response = ui.on_window_event(&window, &event);
            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}
```

If the borrow checker rejects the closure in `redraw` capturing `self.snapshot`, `self.ticket`, and `self.state` while `ui`, `gpu`, and `tiles` are borrowed, destructure once at the top of `redraw` instead: `let Self { window, gpu, tiles, ui, snapshot, ticket, state, .. } = self;` and use those bindings; then call `self.apply(...)` only after the last use of the bindings.

- [ ] **Step 5: Wire the binary, drop `watch` and the old renderer**

`crates/app/src/lib.rs` becomes:

```rust
//! brp: peer-to-peer screen sharing. Everything the `brp` binary wires together.
pub mod cli;
pub mod commands;
pub mod error;
pub mod identity;
pub mod participant;
pub mod presets;
pub mod publish;
pub mod render;
pub mod ui;
pub mod window;
```

`crates/app/src/main.rs`: replace `use brp_app::{publish, watch};` with `use brp_app::{participant, publish};` and the dispatch with:

```rust
    let result = match cli.command {
        Command::Publish(args) => runtime.block_on(publish::run(args)),
        Command::Create(args) => participant::run(&runtime, None, args.window),
        Command::Join(args) => participant::run(&runtime, Some(args.ticket), args.window),
    };
```

`crates/app/src/render/mod.rs`: module list becomes `pub mod grid; pub mod tiles; pub mod ui;`.

`crates/app/src/error.rs`: remove the `EmptyTicket` variant; nothing else refers to it once `watch.rs` is gone.

```bash
git rm crates/app/src/watch.rs crates/app/src/render/video.rs
```

- [ ] **Step 6: Build, test, lint**

Run: `cargo test --workspace && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green. Then a smoke run without a peer:

```bash
cargo run -p brp -- create --no-relay
```

Expected: the window opens with an empty members list, "nobody else yet", the share buttons, and a status bar with a working Copy ticket button. Click Share monitor: the portal picker opens; cancelling it shows `share failed: ...` in the status bar and no live appears. Accepting it adds a live row with Source and template checkboxes, a frame-rate control, a codec selector, and `idle` on every preset. Close the window: the process exits without a panic and the log shows no router shutdown warning.

- [ ] **Step 7: Commit**

```bash
git add crates/app
git commit -m "feat: add the participant window with create and join, remove watch"
```

---

### Task 8: README and the two-machine check

**Files:**
- Modify: `README.md`

**Interfaces:** none.

- [ ] **Step 1: Update the README**

Replace the status block (the `> **Status:**` paragraph) with:

```markdown
> **Status:** experimental, Linux only. A participant creates or joins a room,
> shares several monitors or windows with presets, and watches other members'
> lives in a tile grid. Audio, pop-out windows, and Windows support are planned
> phases rather than current features.
```

Replace the `## Usage` code block with:

````markdown
```
cargo build --release

# Open a room in the participant window and print its ticket
./target/release/brp create [--nickname N] [--fps 60] [--no-relay]

# Join that room in the participant window
./target/release/brp join <ticket> [--nickname N] [--fps 60] [--no-relay]

# Share one live headlessly, creating a room or joining one
./target/release/brp publish --nickname alice [--ticket <ticket>] [--fps 60] [--bitrate-kbps N] [--codec hevc|h264|av1] [--source monitor|window] [--no-relay]
```

In the window, tick a live in the left panel to watch it and pick its preset; hover a
tile for its preset selector and stats. The bottom panel lists your own lives with a
frame-rate control, a codec selector, template checkboxes, and a bitrate per preset.
`--fps` is the capture ceiling for lives shared from the window.
````

Keep the paragraph about tickets, membership gating, encoders, and `--no-relay` that follows.

In `## Development`, add `cargo test -p brp                   # grid, preset, pacing, and rate helpers` under the existing `cargo test -p brp-room` line.

In `## Roadmap`, change item 1 to `1. **Linux vertical slice** — done.` and item 2 to `2. **Rooms and multiple lives** — current phase.`, and add after the phase 1 plan link:

```markdown
The slice 2 design is in
[`docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md`](docs/superpowers/specs/2026-09-04-slice2-rooms-and-multi-live-design.md),
implemented by plans
[`2026-09-04-plan2a-room-layer.md`](docs/superpowers/plans/2026-09-04-plan2a-room-layer.md) and
[`2026-09-05-plan2b-participant-window.md`](docs/superpowers/plans/2026-09-05-plan2b-participant-window.md).
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: describe the participant window in the README"
```

- [ ] **Step 3: Two-machine check (spec section 10, manual)**

On machine A: `brp create --nickname alice`. Copy the ticket. On machine B: `brp join <ticket> --nickname bob`.

1. Both windows list the other member within one heartbeat, with a path badge that reads `direct` or `relayed` once a watch is running.
2. Alice shares a monitor and a window; Bob shares a monitor. Each publisher sees its lives in the bottom panel; each viewer sees them under the member in the left panel.
3. Bob ticks all three lives. The grid shows a 2x2 layout with three tiles; video appears in each; the encoder rows on the publishers show the encoder name and one viewer each.
4. Bob switches one tile to 720p from the hover overlay. The tile keeps its last frame, then shows the new preset; the publisher's 720p row starts an encoder and the Source row goes idle after the grace.
5. Alice sets the monitor live's frame rate to 30. Bob's tile stats show decoded frames advancing at roughly half the rate; Alice's encoder rows restarted.
6. Alice untick a template that Bob watches. Bob's tile falls back to Source without user action.
7. Alice closes her window. Bob's tiles for Alice's lives show `reconnecting`, then `ended`, and the member disappears within the expiry; the checkboxes clear.
8. Bob copies his own ticket. On machine C: `brp join <bob's ticket>`. C sees Bob and can watch Bob's live.

Record the outcome of each step in the commit message body of a final `docs:` commit if any README wording needs correcting; otherwise no commit is needed.

## Plan self-review notes

Spec coverage: left panel with members, path badge, watch checkbox, preset selector (Task 6 members); tile grid with the square-root rule, letterboxing, hover overlay with title, preset selector, stats toggle, status text over the last frame for reconnecting and ended (Tasks 2, 3, 6 tiles); bottom panel with encoder state, template checkboxes, bitrate, frame rate, codec, stop, share buttons, titles by kind and ordinal (Tasks 4, 5, 6 own_lives); status bar with copy ticket, member count, aggregate encode bitrate, nickname (Tasks 5, 6 status); one render pass with tiles before egui, redraw on frame, version bump, and input (Task 7); `create` and `join` with `--nickname`, `--fps`, `--no-relay`; `watch` removed; `publish` untouched (Task 7); error handling of section 8 that lands in the window: portal denied as a status line, encoder failure marked on the preset row, member expiry clearing the checkbox through the snapshot, refused connections shown as reconnecting (Tasks 6, 7); frame pacing amendment (Task 1); README (Task 8); manual two-machine check (Task 8).

Deliberate scope notes: window-local edits to bitrate and frame rate commit when the widget is released, because each commit restarts encoders. The hover overlay stays visible while any egui popup is open, which is simpler than tracking the one combo box's popup id and only affects tiles while a dropdown is showing. The lib-plus-bin split of the app crate exists so Tasks 2 to 6 pass clippy without dead-code allowances. Members and watches are sorted in the UI layer because the room's snapshot iterates hash maps.
