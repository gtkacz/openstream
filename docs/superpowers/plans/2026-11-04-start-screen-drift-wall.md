# Start Screen Drifting Wall Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the start screen's room diagram with a drifting perspective wall of still frames, painted natively in egui.

**Architecture:** A new `ui/wall/` module holds pure geometry (`geometry.rs`), embedded stills and their background decoding (`stills.rs`), and frame-by-frame state and painting (`mod.rs`). Each tile is a subdivided `egui::Mesh` whose vertices are projected on the CPU. A small `reduced_motion.rs` reads the system setting once per launch and tells the wall through an `AppEvent`.

**Tech Stack:** Rust, egui 0.36, `image` 0.25 (JPEG only), `ashpd` 0.13 (`settings`, Linux), `windows-sys` 0.61 (Windows).

**Spec:** `docs/superpowers/specs/2026-11-04-start-screen-drift-wall-design.md`. Its section 6 holds the approved values and the transform.

## Global Constraints

- Every animated value goes through `crates/app/src/ui/motion.rs`. Nothing overshoots.
- The only new or widened dependencies are `image` with `default-features = false, features = ["jpeg"]`, `ashpd`'s `settings` feature on Linux, and `windows-sys`'s `Win32_UI_WindowsAndMessaging`.
- Spec section 6's values become named constants, with those exact values.
- Comments explain why, only where the code cannot. Always use braces. Do not touch adjacent code.
- Commit on the current branch. Do not create a branch.
- **Never push.** Per `README.md`, any push to `main` whose subject does not start with `fix:` publishes a public release. Pushing is the user's decision.
- No `Co-Authored-By`, `Generated with`, session or model trailers on commits. Use the project's style: `feat(app): ...`, `docs: ...`.
- Run `/verify` right before each commit that changes code, not for commits of docs or tests alone.
- Wrap noisy commands as `repowise distill <cmd>`. Recover omitted output with `repowise expand <ref>`, not by re-running.
- When running the real app, set temporary `XDG_CONFIG_HOME` and `XDG_DATA_HOME`, because `register_scheme_handler` otherwise repoints the user's `brp://` links. Stop it by PID only, because the user may have a real instance open.

## Review Focus

Each line below has a test in the task named.

1. **Very small or extreme windows, such as 960×1 or 5000×1:** no panic, no non-finite vertices, and a bounded tile count. (Task 2)
2. **A failed decode:** that tile stays empty, other tiles load, and nothing panics. (Task 1)
3. **Returning after a long time away:** the first frame moves the wall by one short step, with no jump. (Task 3)
4. **The pointer leaving the window, or focus lost, during a hover:** lift and tilt settle, and the window stops asking for frames. (Task 3)
5. **A room opening while stills are still decoding, as with a join link:** release is clean, and a later draw starts fresh. (Task 3)

---

### Task 1: Stills and their notice

**Files:**
- Create: `crates/app/assets/stills/*.jpg`, `crates/app/assets/stills/README.md`
- Create: `crates/app/src/ui/wall/mod.rs` (only `pub mod stills;` for now), `crates/app/src/ui/wall/stills.rs`
- Modify: `crates/app/src/ui/mod.rs` (add `pub mod wall;`), `Cargo.toml`, `crates/app/Cargo.toml`, `ci/stage-linux.sh`, `ci/stage-windows.ps1`, `README.md` (`## License`)

**Interfaces:**
- Produces in `ui::wall::stills`:
  - `pub struct Still { pub name: &'static str, pub bytes: &'static [u8] }`
  - `pub static STILLS: &[Still]`
  - `pub const STRANDS: usize = 7`
  - `pub fn decode(bytes: &[u8]) -> Result<egui::ColorImage, image::ImageError>`
  - `pub struct Loader`, with `Loader::start() -> Self`, `Loader::start_from(stills: &'static [Still]) -> Self`, `poll(&mut self, ctx: &egui::Context) -> bool` (true while stills are outstanding) and `texture(&self, index: usize) -> Option<&egui::TextureHandle>`
  - `#[cfg(test)] Loader::loaded(textures: Vec<egui::TextureHandle>) -> Self` and `#[cfg(test)] Loader::finish(&mut self, ctx: &egui::Context)`, which blocks until every still has been received

- [ ] **Step 1: Commit the design and this plan**

```bash
git add docs/superpowers/specs/2026-11-04-start-screen-drift-wall-design.md docs/superpowers/plans/2026-11-04-start-screen-drift-wall.md
git commit -m "docs: design and plan the start screen's drifting wall"
```

- [ ] **Step 2: Assemble about 42 stills in `crates/app/assets/stills/`**

Copy the mockup's stills from `/tmp/brainstorm-2399704-1793805829/content/*.jpg`, except `anime-gits.jpg` and `film-bladerunner.jpg`. Fetch about 20 more with `/tmp/brainstorm-scripts/fetch-tiles.sh` as a model, across all four kinds.

If `/tmp` has been cleared, refetch using these sources:
- **Steam app ids:** 1245620, 1091500, 1145360, 413150, 730, 367520, 427520, 632470, 252950, 1086940
- **TMDB movies:** 76341, 157336, 438631, 129, 372058, 149
- **TMDB TV:** 30991, 1429
- **Web pages:** `github.com/gtkacz/openstream`, its `crates/app/src/ui/start.rs` page, `en.wikipedia.org/wiki/Screen_sharing`, `news.ycombinator.com`

Use names of the form `<kind>-<work>.jpg`, each 480 by 270 at quality 82.

Make a contact sheet with `magick montage` and look at it. Remove anything with nudity, implied nudity or gore, and keep text-heavy stills few.

Expected: `magick identify -format '%wx%h\n' crates/app/assets/stills/*.jpg | sort -u` prints only `480x270`.

- [ ] **Step 3: Write `crates/app/assets/stills/README.md`**

It opens with exactly this text, then has a table of `File | Work | Source | Retrieved` with one row per still:

```markdown
# Stills

Embedded by `crates/app/src/ui/wall/stills.rs` and shown on the start screen.

These images are frames from third-party games, films, anime and websites. They belong to their
respective owners, are used here as illustrative examples of screens being shared, and are not
covered by this repository's MIT licence. To ask for one to be removed, open an issue.

To remove a still, delete its file, its entry in `STILLS`, and its row below, then rebuild.
```

- [ ] **Step 4: Add the dependency, then write the failing tests in `stills.rs`**

Add `image = { version = "0.25", default-features = false, features = ["jpeg"] }` to the workspace and `image.workspace = true` to `crates/app`.

```rust
#[test] fn every_embedded_still_decodes_at_480_by_270() // for each in STILLS: decode(..).unwrap().size == [480, 270]
#[test] fn every_still_file_is_embedded_and_named_in_the_notice()
// read_dir(CARGO_MANIFEST_DIR/assets/stills): each *.jpg stem is in STILLS, the README contains the file name,
// and STILLS.len() is within 38..=46 and >= STRANDS
#[test] fn a_decoded_still_has_rounded_transparent_corners_and_an_opaque_body()
// image[(0, 0)].a() == 0; image[(240, 135)].a() == 255; image[(240, 0)].a() == 255
#[test] fn a_corrupt_still_is_an_error_rather_than_a_panic() // decode(b"not a jpeg").is_err()
#[test] fn a_failed_still_stays_empty_while_the_others_load()
// a static list of one good and one corrupt Still through Loader::start_from;
// after finish(), texture(good) is Some, texture(corrupt) is None, poll() == false
```

- [ ] **Step 5: Run them to confirm they fail**

Run: `repowise distill cargo test -p brp --lib wall::stills`
Expected: FAIL, because the items do not exist.

- [ ] **Step 6: Implement `stills.rs`**

- `STILLS` lists every file through a small `macro_rules!` around `include_bytes!`, ordered so the seven strands (`index % STRANDS`) each mix kinds.
- `decode` bakes the corners into alpha. The radius in pixels is `theme::RADIUS_CARD` times `width / TILE_WIDTH`, which is 28.8 at 480 wide. Each corner pixel's alpha is its antialiased coverage, `(radius + 0.5 - distance).clamp(0.0, 1.0)`. Build the image with `ColorImage::from_rgba_unmultiplied`.
- `TILE_WIDTH` belongs to Task 2. Define `const TILE_WIDTH: f32 = 200.0` privately here, and Task 2 replaces it with the import.
- `Loader::start_from` spawns one `std::thread`. It decodes in order and sends `(usize, Result<ColorImage, image::ImageError>)` over an `mpsc` channel, returning as soon as a `send` fails, so dropping the `Loader` stops decoding.
- `poll` drains with `try_recv`, uploads through `ctx.load_texture(name, image, egui::TextureOptions::LINEAR)`, and logs failures with `tracing::warn!`.
- The thread must not hold an `egui::Context`. That would keep the context alive through its own temporary memory and defeat the early stop.

- [ ] **Step 7: Run the tests**

Run: `repowise distill cargo test -p brp --lib wall::stills`
Expected: PASS.

- [ ] **Step 8: Ship the notice and point to it**

- `ci/stage-linux.sh`: `cp crates/app/assets/stills/README.md "$destination/STILLS-NOTICE.md"`
- `ci/stage-windows.ps1`: `Copy-Item crates\app\assets\stills\README.md (Join-Path $Destination "STILLS-NOTICE.md")`
- `README.md`, under `## License`, add: "The start screen's stills are third-party images and are not covered by this licence; see [`crates/app/assets/stills/README.md`](crates/app/assets/stills/README.md)."

Expected: `bash -n ci/stage-linux.sh` exits 0.

- [ ] **Step 9: Commit**

```bash
git add crates/app/assets/stills crates/app/src/ui/wall crates/app/src/ui/mod.rs Cargo.toml Cargo.lock crates/app/Cargo.toml ci/stage-linux.sh ci/stage-windows.ps1 README.md
git commit -m "feat(app): embed the start screen's stills and their notice"
```

---

### Task 2: Wall geometry

**Files:**
- Create: `crates/app/src/ui/wall/geometry.rs`
- Modify: `crates/app/src/ui/wall/mod.rs` (add `pub mod geometry;`), `crates/app/src/ui/wall/stills.rs` (import `TILE_WIDTH`)

**Interfaces:**
- Consumes: `stills::STRANDS`.
- Produces in `ui::wall::geometry`, all pure:
  - Constants from spec section 6: `TILE_WIDTH`, `TILE_HEIGHT`, `GAP`, `SCALE`, `PITCH_DEG`, `YAW_DEG`, `PERSPECTIVE`, `DEPTH`, `BASE_SPEED`, `SPEED_SPREAD`, `LIFT`, `REST_BRIGHTNESS`, `FADE_SPAN`, `TILT_PER_OFFSET_DEG` (4.8), plus `GRID` (6). The cell sizes are `COLUMN_WIDTH` (218.0) and `ROW_HEIGHT` (130.5).
  - `pub struct Pose { pub center: egui::Pos2, pub tilt_x_deg: f32, pub tilt_y_deg: f32 }`
  - `Pose::project(&self, x: f32, y: f32, z: f32) -> egui::Pos2`
  - `Pose::unproject(&self, screen: egui::Pos2) -> (f32, f32)`, onto `z = 0`
  - `pub struct Cell { pub column: i32, pub row: i64, pub x: f32, pub y: f32 }`, with `x` and `y` the cell's top-left in plane coordinates
  - `pub fn column_speed(column: i32) -> f32`, signed points per second, positive moving upward
  - `pub fn start_offset(column: i32, stills: usize) -> f64`
  - `pub fn still_index(column: i32, row: i64, stills: usize) -> usize`
  - `pub fn cell_at(pose: &Pose, screen: egui::Pos2, offset: &dyn Fn(i32) -> f64) -> (i32, i64)`
  - `pub fn visible_cells(pose: &Pose, area: egui::Rect, offset: &dyn Fn(i32) -> f64) -> Vec<Cell>`
  - `pub fn fade_at(area: egui::Rect, x: f32) -> f32`
  - `pub fn tile_mesh(pose: &Pose, cell: &Cell, lift_z: f32, brightness: f32, area: egui::Rect, texture: egui::TextureId) -> egui::Mesh`

**Decisions the implementer needs:**

- **Columns and rows are unbounded integers,** and `visible_cells` lists only those reaching `area`. This replaces a column count: unproject `area`'s four corners, whose bounds contain the whole rectangle because the projection maps it to a convex shape, then list every column and row they cover. Windows up to 4K therefore need no special handling.
- **Column `c`'s left edge is `(c as f32 - 0.5) * COLUMN_WIDTH`,** so column 0 is centred.
- **Row `r`'s top is `(r as f64 * ROW_HEIGHT as f64 - offset) as f32`.**
- **Offsets are `f64` and never wrap.** `still_index` repeats through `rem_euclid`, so nothing jumps at a repeat boundary. A wrapped offset would renumber the rows under a lifted tile.
- **Strands:** a column uses strand `s = column.rem_euclid(STRANDS)`, and `still_index` returns `s + STRANDS * row.rem_euclid(strand_len)`, with `strand_len` the number of stills whose `index % STRANDS == s`, at least 1. Repeat height is `strand_len * ROW_HEIGHT`.
- **Speed:** `BASE_SPEED * (1.0 + SPEED_SPREAD * p)`, with `p = ((c * 0.6180339887 + 0.35).rem_euclid(1.0)) * 2.0 - 1.0`. The sign is positive when `c.rem_euclid(2) == 0`.
- **Start offset:** `(c * 0.37).rem_euclid(1.0) * repeat height`.
- **Projection,** in CSS's transform order, with `a = PITCH_DEG + tilt_y_deg` and `b = YAW_DEG + tilt_x_deg`:

```text
z0 = z - DEPTH
x1 = x cos b + z0 sin b        z1 = -x sin b + z0 cos b
y2 = y cos a - z1 sin a        z2 = y sin a + z1 cos a
k  = PERSPECTIVE / (PERSPECTIVE - z2)
screen = center + (SCALE * x1 * k, SCALE * y2 * k)
```

- **`unproject`:** with `z = 0`, both screen equations are linear in `x` and `y`, so solve the 2×2 system directly.
- **`tile_mesh`:** the tile is inset by `GAP / 2` in its cell. Use a `GRID` by `GRID` grid of `Vertex { pos, uv, color }`, with `color = Color32::WHITE.gamma_multiply(brightness * fade_at(area, pos.x))`.
- **Hit-testing is on the resting cell:** `cell_at` unprojects and floors, with no quad test.

- [ ] **Step 1: Write the failing tests in `geometry.rs`**

```rust
#[test] fn projection_matches_the_mockups_transform() {
    // Values computed from the transform above, center at the origin, within 0.01.
    let rest = Pose { center: Pos2::ZERO, tilt_x_deg: 0.0, tilt_y_deg: 0.0 };
    // (0,0,0) -> (31.33, 34.64); (100,50,0) -> (140.01, 81.62); (0,0,64) -> (15.32, 16.94)
    let flat = Pose { center: Pos2::ZERO, tilt_x_deg: 14.0, tilt_y_deg: -16.0 };
    // (100,0,0) -> (107.27, 0.0)
}
#[test] fn unprojecting_a_projected_point_returns_it()
// every combination of tilt ±2.4 and plane points across ±1500: within 0.05
#[test] fn columns_alternate_direction_at_their_own_speeds()
// column_speed(0) ≈ 36.33; column_speed(1) ≈ -59.69; column_speed(-1) ≈ -50.77 (within 0.01)
#[test] fn every_visible_point_lands_on_a_listed_cell()
// areas 492x540, 832x780, 1116x1080, 2076x2160, posed on their centres, each tilt extreme, a 20x20 sample:
// visible_cells contains cell_at's (column, row); cells.len() < 1000
#[test] fn crossing_a_repeat_boundary_keeps_the_same_still_in_place()
// for offset o and o + repeat height, in both directions: same still_index at the same screen point
#[test] fn the_mesh_stays_within_a_quarter_point_of_true_perspective()
// tile centres inside a 2076x2160 wall, tilt extremes, lift 0 and LIFT:
// each grid cell's diagonal midpoint vs project(mid) differs by < 0.25
#[test] fn the_fade_reaches_full_at_26_percent_of_the_width()
// area left..left+1000: fade_at(left) == 0.0; fade_at(left+130) == 0.5; fade_at(left+260) == 1.0; fade_at(left+900) == 1.0
#[test] fn extreme_windows_give_finite_geometry_and_a_bounded_tile_count()
// areas 492x1, 2656x1, 1x1, 1x2160: every vertex finite, cells.len() < 1000, no panic
```

The measured mesh error for visible tiles at `GRID = 6` is below 0.09 points, so the 0.25 bound has headroom.

- [ ] **Step 2: Run them to confirm they fail**

Run: `repowise distill cargo test -p brp --lib wall::geometry`
Expected: FAIL, because the items do not exist.

- [ ] **Step 3: Implement `geometry.rs`, and replace `TILE_WIDTH` in `stills.rs` with the import**

- [ ] **Step 4: Run the tests**

Run: `repowise distill cargo test -p brp --lib wall`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/ui/wall
git commit -m "feat(app): compute the wall's perspective, cells and tile meshes"
```

---

### Task 3: Paint the wall on the start screen and remove the diagram

**Files:**
- Modify: `crates/app/src/ui/wall/mod.rs`, `crates/app/src/ui/start.rs`, `crates/app/src/ui/motion.rs`, `crates/app/src/render/ui.rs`, `crates/app/src/window.rs:739-744`, `crates/app/src/icon.rs`
- Test: `crates/app/src/ui/wall/mod.rs`, `crates/app/src/ui/start.rs`, `crates/app/tests/ui_flow.rs`

**Interfaces:**
- Consumes: Task 1's `Loader`, Task 2's `geometry`.
- Produces in `ui::wall`:
  - `pub fn draw(ui: &mut egui::Ui, area: egui::Rect)`
  - `pub fn allow_motion(ctx: &egui::Context, allowed: bool)`
  - `pub fn release(ctx: &egui::Context)`
  - `#[cfg(test)] pub(crate) fn hovered(ctx: &egui::Context) -> Option<(i32, i64)>`
  - `#[cfg(test)] pub(crate) fn has_state(ctx: &egui::Context) -> bool`
- Produces elsewhere:
  - `EguiLayer::ctx(&self) -> &egui::Context` in `render/ui.rs`
  - `start::wall_area(window: egui::Rect, column: egui::Rect) -> egui::Rect`, private
  - `motion::LONGEST_STEP` becomes `pub`

**Decisions the implementer needs:**

- **State** lives in egui temporary memory under one `Id`, as `Arc<Mutex<WallState>>`, because `Loader`'s receiver is not `Sync`. `start::draw` keeps its signature, so its test call sites are untouched. `WallState` holds the loader, `offsets: HashMap<i32, f64>`, `last_time: Option<f64>`, the hovered cell, and `lifting: Vec<(i32, i64)>`. The motion preference is a separate temporary `bool`.
- **Motion is off until `allow_motion(ctx, true)`,** because an unknown preference means still. `allow_motion` is the only thing that turns motion on, and Task 4 calls it.
- **Time step:** `(now - last_time).clamp(0.0, motion::LONGEST_STEP)`, from `ctx.input(|i| i.time)`. Do not use `stable_dt`, which returns `predicted_dt` after any non-immediate repaint.
- **Each frame, for each column reaching the area:** `speed = column_speed(c) * motion::appear(ctx, id, moving, motion::REFLOW)`, where `moving` is motion allowed, the window focused (`ctx.input(|i| i.focused)`), and the column not hovered. An offset starts at `start_offset`.
- **Tilt:** `motion::glide` on each axis, per spec section 6. The target is 0 with no hover position or with motion off.
- **Hover:** `ui.interact(area, id, egui::Sense::hover())`, then `cell_at` from `hover_pos()`.
- **Lift:** `motion::follow(.., motion::REFLOW)`, called only for the hovered cell and for those in `lifting`. Remove a cell from `lifting` once it reads 0 and is not hovered, because calling a spring for every tile would grow egui's temporary memory forever as rows change. Lift distance is `LIFT * t`, or 0 with motion off. Brightness is `lerp(REST_BRIGHTNESS..=1.0, t)`, times `motion::appear(ctx, id, true, motion::SURFACE)` per still. Do not use `motion::arrive`, which restarts after a gap and would fade a tile in again each time it re-enters the window.
- **Paint order:** tiles in cell order, then lifting tiles, then the hovered one. A tile with no texture yet paints nothing.
- **Painter:** `ui.painter().with_clip_rect(area)`, called before any other widget in the panel so the brand row and Settings sit on top.
- **Frames:**
  - While stills load, `request_repaint_after(50 ms + predicted_dt)`.
  - While motion is allowed and focused, `request_repaint_after(1/60 s + predicted_dt)`. `predicted_dt` comes from `ctx.input(|i| i.predicted_dt)`, because egui subtracts it from the delay. Without adding it, the delay becomes zero and `window.rs` redraws at once.
  - Otherwise, ask for nothing. Springs keep their own requests while they settle.
- **Start screen:**
  - `wall_area` runs from `column.left() + FORM_WIDTH + COLUMN_GAP` to the right of `window`, and from `window.top()` to `window.bottom()`.
  - Call `wall::draw` first inside the panel's `show` closure, using `ui.ctx().content_rect()`, when wide enough.
  - Rename `DIAGRAM_MIN_WINDOW_WIDTH` to `WALL_MIN_WINDOW_WIDTH`. Its value stays 960.
- **Release:** in `window.rs`, inside the existing `if let Some(main) = &self.main` at the `RoomOpened(Ok)` arm, call `wall::release(main.ui.ctx())`. The phase only moves from Start to Room, so there is no way back to handle.

- [ ] **Step 1: Write the failing tests in `wall/mod.rs`**

Use a helper that installs `Loader::loaded` with solid-colour textures, so no thread runs, and steps `ctx.run_ui` at 1/60 s. Past settling means 120 frames.

```rust
#[test] fn motion_stays_off_until_the_preference_is_known() // no allow_motion: offsets equal across 120 frames
#[test] fn steady_drift_asks_for_about_sixty_frames_a_second()
// allowed, focused, settled: 15 ms <= repaint_delay <= 18 ms; offsets change
#[test] fn losing_focus_settles_the_wall_and_asks_for_no_frames()
// then focused = false for 120 frames: repaint_delay == Duration::MAX; offsets equal across the last two frames
#[test] fn reduced_motion_holds_the_wall_still_and_only_brightens_on_hover()
// allow_motion(false), pointer over a tile, settled: offsets equal, tilt 0, vertex positions unchanged, hovered tile's colour brighter
#[test] fn hovering_a_tile_pauses_its_column_and_lifts_it_above_the_others()
// settled: hovered column's offset stops, a neighbour's does not; last mesh painted is the hovered tile's and differs from its resting position
#[test] fn a_long_gap_between_frames_moves_the_wall_by_one_short_step()
// time jumps 600 s: |offset change| <= motion::LONGEST_STEP * |column_speed|
#[test] fn the_pointer_leaving_the_window_lets_lift_and_tilt_settle()
// Event::PointerGone, settled: hovered == None, tilt 0, lifting empty; then focused = false asks for no frames
#[test] fn opening_a_room_mid_load_releases_cleanly_and_a_later_draw_starts_fresh()
// real Loader::start(): draw, release, draw: no panic, and the second draw has a new state
#[test] fn tile_meshes_are_clipped_to_the_area() // every Shape::Mesh's clip_rect == area
```

- [ ] **Step 2: Replace the tests in `start.rs` and `ui_flow.rs`**

In `start.rs`, delete `the_diagram_keeps_one_axis_and_two_clean_sides_at_every_window_size` and `a_diagram_with_no_room_collapses_instead_of_inverting`, then add:

```rust
#[test] fn the_wall_reaches_the_windows_edges_and_stops_short_of_the_form()
// windows 960x540, 1300x780, 2560x1440, 3840x2160: top, right, bottom equal the window's;
// left == column.left() + 444; at 1300x780, left == 468
#[test] fn below_960_points_the_start_screen_paints_no_wall() // 959 wide: no Shape::Mesh, and wall::has_state == false
#[test] fn a_tile_under_settings_is_not_hovered_and_settings_takes_the_click()
// 1440x900, pointer at the Settings button: wall::hovered == None; click returns StartAction::OpenSettings
```

In `crates/app/tests/ui_flow.rs`:
- Delete `the_start_diagram_puts_the_typed_name_on_the_screen_that_person_will_have`.
- Rename `the_start_form_is_centred_on_the_same_axis_as_the_diagram` to `the_start_form_is_centred_in_the_body`, and drop its `live` and `relayed` lines.
- Add `the_form_stays_clickable_beside_the_wall`: at 1280x800, clicking Create returns `StartAction::Create`.

- [ ] **Step 3: Run them to confirm they fail**

Run: `repowise distill cargo test -p brp --lib wall`
Expected: FAIL, because `draw`, `allow_motion`, `release` and `hovered` do not exist.

- [ ] **Step 4: Implement the wall and swap it in**

In `start.rs`, remove `room_diagram`, `diagram_screens`, `DiagramChip`, `paint_chip`, `shared_picture`, and the imports they leave unused (`brp_net::PathKind`, `members`, `PathShape`, `motion`). Make `icon::paint_mark_at` private. `dissolve_in` keeps its full path, so it still works.

- [ ] **Step 5: Run the app's tests, format and lint**

Run: `repowise distill cargo test -p brp && cargo fmt --all --check && repowise distill cargo clippy -p brp --all-targets -- -D warnings`
Expected: all pass, with no warnings about unused items.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src crates/app/tests/ui_flow.rs
git commit -m "feat(app): show a drifting wall of stills on the start screen"
```

---

### Task 4: Read the system's reduced-motion setting

**Files:**
- Create: `crates/app/src/reduced_motion.rs`
- Modify: `crates/app/src/lib.rs`, `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/window.rs` (`AppEvent`, `resumed`, `user_event`)

**Interfaces:**
- Consumes: `wall::allow_motion`, `EguiLayer::ctx`.
- Produces:
  - `pub async fn prefers_less_motion() -> bool`
  - `AppEvent::MotionPreference { less: bool }`

**Decisions the implementer needs:**

- **Linux:** `ashpd::desktop::settings::Settings::new().await`, then `read::<u32>("org.freedesktop.appearance", "reduced-motion")`, then `read::<bool>("org.gnome.desktop.interface", "enable-animations")`. Pure `fn decide(standard: Option<u32>, animations_enabled: Option<bool>) -> bool`: the first answer present decides, and no answer means false.
- **Patience:** wrap the read in `tokio::time::timeout` with `const PORTAL_PATIENCE: Duration = Duration::from_secs(2)`, through a generic private `async fn answer_within<T>(patience: Duration, read: impl Future<Output = T>) -> Option<T>`. A portal that activates slowly or never must not hold the wall still forever.
- **Windows:** `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut enabled as *mut _ as *mut c_void, 0)` with a `// SAFETY:` comment. Failure means false, and `enabled == 0` means true.
- **Elsewhere:** `#[cfg(not(any(target_os = "linux", windows)))]` returns false, matching `handler.rs`.
- **Wiring:** in `resumed`, after `self.main = Some(main)`, spawn the read on `self.runtime` and send the event through a clone of `self.proxy`. The arm in `user_event` calls `wall::allow_motion(main.ui.ctx(), !less)` and `self.request_redraw_all()`.
- **Read once per launch.** Add `// shortcut: read once per launch, subscribe to the portal's SettingChanged if live changes matter.`
- **Linux-only items,** including `decide`, `answer_within`, `PORTAL_PATIENCE` and their tests, carry `#[cfg(target_os = "linux")]`, so the Windows build stays free of dead-code warnings.

- [ ] **Step 1: Add the features and write the failing tests**

Add `Win32_UI_WindowsAndMessaging` to the workspace's `windows-sys`. In `crates/app/Cargo.toml`, under a Linux target section, add `ashpd = { workspace = true, features = ["settings"] }`.

```rust
#[test] fn the_first_answer_decides_and_silence_means_no_preference() {
    assert!(decide(Some(1), Some(true)));
    assert!(!decide(Some(0), Some(false)));
    assert!(decide(None, Some(false)));
    assert!(!decide(None, Some(true)));
    assert!(!decide(None, None));
}
#[tokio::test] async fn a_portal_that_never_answers_counts_as_no_answer() {
    assert_eq!(answer_within(Duration::ZERO, std::future::pending::<u32>()).await, None);
}
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `repowise distill cargo test -p brp --lib reduced_motion`
Expected: FAIL, because the module does not exist.

- [ ] **Step 3: Implement and wire it**

- [ ] **Step 4: Run the tests and lint**

Run: `repowise distill cargo test -p brp && repowise distill cargo clippy -p brp --all-targets -- -D warnings`
Expected: PASS. The Windows branch does not compile here, so CI is its first compile, and the hand-over must say so.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src Cargo.toml Cargo.lock crates/app/Cargo.toml
git commit -m "feat(app): keep the wall still when the system asks for less motion"
```

---

### Task 5: Judge it in the real app

This is a checkpoint with the user. Do not start Task 6 until they have accepted the wall.

**Files:** whatever the findings require, within `crates/app/src/ui/wall/` and `crates/app/src/reduced_motion.rs`.

- [ ] **Step 1: Launch it safely**

```bash
XDG_CONFIG_HOME=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d) cargo run -p brp --release
```

Keep the PID. Stop it only by that PID.

- [ ] **Step 2: Have the user judge it in motion**

Show them the running window and ask them to check: the look against version C, shimmer, seams, stutter, the hover and tilt, and stopping when unfocused. Also settle whether a lifted tile needs a shadow, as the mockup had; add one only if it is simple and noticeable.

Check at 1× and at a higher scale. If shimmer shows, resize stills on load to the tile's on-screen pixel size with `image::imageops::resize`, as the spec suggests.

If meshes visibly fail, stop and talk to the user before reaching for a wgpu pass.

- [ ] **Step 3: Measure the cost**

While drifting, record the process's CPU with `pidstat -p <pid> 1 10` and its GPU use with `nvidia-smi pmon -c 10`. Then repeat with another window focused.

Expected: CPU near zero when unfocused. Report the numbers. The spec sets no threshold, so a high number is the user's call.

- [ ] **Step 4: Check reduced motion on KDE with the user**

Ask the user to set System Settings → Animation speed to Instant, because this changes their desktop. Then read the portal:

```bash
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop --method org.freedesktop.portal.Settings.Read org.freedesktop.appearance reduced-motion
```

Relaunch and confirm the wall stays still and a hovered tile only brightens. Ask them to restore the setting.

If the portal's answer did not change, also read `org.kde.kdeglobals.KDE AnimationDurationFactor`, where 0 means off, as a third input to `decide`, with a test.

- [ ] **Step 5: Check a join-link launch**

Launch with a `brp://` link to an unreachable room and confirm the form responds while connecting and the wall is unaffected.

- [ ] **Step 6: Commit any fixes, then get the user's acceptance**

Use one commit per finding, with a `fix(app): ...` or `feat(app): ...` subject matching what changed.

---

### Task 6: Update the design documents

**Files:**
- Modify: `DESIGN.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Update `DESIGN.md` as spec section 10 lists**

Direction, Layout's Start entry, Signature, the Start diagram entry, Motion's new Start wall entry, "At rest, nothing moves", and the reduced-motion note.

Expected: `rg -n -i 'diagram' DESIGN.md` finds nothing.

- [ ] **Step 2: Update `docs/ROADMAP.md`'s Reduced motion entry**

Record that the lookup now exists in `crates/app/src/reduced_motion.rs`, that only the wall uses it, and that `org.freedesktop.appearance reduced-motion` is the portal key, plus the KDE result from Task 5. Applying it to `drive` remains open.

- [ ] **Step 3: Run everything**

Run: `repowise distill cargo test --workspace && cargo fmt --all --check && repowise distill cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. If something unrelated to this work fails, report it rather than fixing it here.

- [ ] **Step 4: Commit**

```bash
git add DESIGN.md docs/ROADMAP.md
git commit -m "docs: describe the start screen's wall and its motion rules"
```

- [ ] **Step 5: Hand over without pushing**

Report what ran and passed, the Task 5 measurements, and that the Windows lookup has not been compiled or run. Remind the user that pushing to `main` publishes a release containing the stills, unless the subject starts with `fix:`.
