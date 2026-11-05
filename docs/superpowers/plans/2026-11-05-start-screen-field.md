# Start Screen Field Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the start screen's drifting wall of stills and its horizon band with one window-filling field of characters. The field goes softly quiet around the brand, the form and the window buttons, and the pointer pushes it.

**Architecture:** The existing character light (`crates/app/src/render/backdrop.rs` + `backdrop.wgsl`) gains a `Field` kind. It replaces `Horizon` and draws domain-warped folds. It quiets around three regions that the start screen measures each frame and passes in points. A pure `Push` buffer of recent pointer samples feeds a displacement into the folds. The wall module, its stills, the `image` dependency and the release notice are then deleted, and the docs follow.

**Tech Stack:** Rust, egui 0.3x, wgpu, WGSL. Package `brp` (lib `brp_app`) in `crates/app`.

**Spec:** `docs/superpowers/specs/2026-11-05-start-screen-field-design.md`

## Global Constraints

- Field look, unchanged from the horizon's: gain 0.74, ink 0.85, halo 0.22, wash 0.24, ground `theme::ground()`, dim `theme::wire()`, phosphor `theme::accent()`, hot `theme::text()`, signal 1.0.
- Quiet: three regions (brand row content, the form's whole rectangle, the Settings + fullscreen buttons). `quiet` is 1 inside a region and falls to 0 at **80 points** outside it with a smoothstep. The strongest region wins, and the strength is **1**.
- Folds: `p = at × 1.5`; `q`, `r`, `v` and the noise exactly as in spec §6 "The folds". `amount = (0.06 + 0.94 × smoothstep(0.44, 0.8, v) + 0.22 × wake) × gain × (1 − quiet)`; `order = 1 − 0.8 × clamp(amount, 0, 1)`.
- Push: a sample every **0.008** window heights of travel, at most **24**, each living **1.2 s** with linear life. Velocity is measured from the last recorded sample and smoothed `0.6 × previous + 0.4 × new`. `near = exp(−|at − sample|² / 0.0045)`, `d = 0.012 × Σ velocity × life × near`, softened `d / (1 + |d| / 0.06)`. Light `+0.22 × wake`, scramble `0.35 × wake`, with `wake = max(life × near × min(|velocity|, 1.5) / 1.5)`.
- `backdrop::FRAME` (33 ms) stays the start screen's frame rate. Do not raise it without the user's say-so.
- Every spec number is a named constant with a one-line doc comment saying why. Comments explain why, never what. No new dependencies.
- Commit only once the user has said to commit. Never push: a push to `main` publishes a release. Create no branch unless asked.
- When running the app, use temporary `XDG_CONFIG_HOME` and `XDG_DATA_HOME`, and never kill by process name.
- Gate for every task: `cargo fmt --check`, `cargo clippy -p brp --all-targets -- -D warnings`, `cargo test -p brp`.

## Review Focus

- **Scale factor 1.25 or 2 (a HiDPI monitor):** regions arrive in points and must quiet the same on-screen area. Pinned by the conversion test in Task 1.
- **An error line or update notice growing the form:** the form region must grow with it, so new text never sits on a bright fold. Pinned by Task 1's region test, which sets `state.error`.
- **A narrow window (640 × 480), where the form is centred:** the regions follow the form. Pinned in Task 1.
- **The pointer leaving the window and coming back far away:** this must not register as a huge flick that tears the field. Pinned in Task 2.
- **Reduced motion or an unfocused window:** the field draws the identical frame even while the pointer moves and clicks. Pinned by the GPU test in Task 2.

Also check in review that the light's opening still keys on the kind's discriminant, not its payload. The regions change every frame, and a payload-keyed opening would restart forever.

---

### Task 1: The field and its quiet regions

**Files:**
- Modify: `crates/app/src/render/backdrop.rs`: `BackdropKind` (line 34), `look` (117), `Scene` (75), `prepare` (316–420); new tests module
- Modify: `crates/app/src/render/backdrop.wgsl`: `Scene`, remove `horizon` (80–86), add the field; the `fs_cells` branch (110–114)
- Modify: `crates/app/src/ui/start.rs`: move `backdrop::request` (162–169) to the end of `draw`, with the measured regions
- Test: `crates/app/tests/tile_render.rs`: `draw_the_horizon` (378) and `the_horizon_lights_…` (441)

**Interfaces:**
- Produces: `pub enum BackdropKind { Field { quiet: [egui::Rect; 3] }, Scope { center: Pos2, radius: f32 } }`. The quiet regions are in points, in the order brand, form, window buttons. A region that is not `is_positive()` quiets nothing.
- Produces (private, `backdrop.rs`): `fn quiet_in_heights(quiet: [Rect; 3], pixels_per_point: f32, window_height: f32) -> [[f32; 4]; 3]`, giving `[min.x, min.y, max.x, max.y]` in window heights, with a non-positive region mapped to a point `FAR_AWAY = 1.0e4` heights away. Also `const QUIET_REACH_POINTS: f32 = 80.0;` and `const QUIET_STRENGTH: f32 = 1.0;`.
- Produces: in `Scene` (Rust and WGSL), `pad` becomes `quiet_reach: f32, quiet_strength: f32` (same slot, so alignment is unchanged), and `quiet: [[f32; 4]; 3]` / `array<vec4<f32>, 3>` is appended after `rings`. `Scope` sends `quiet_strength = 0`.

- [ ] **Step 1: Write the failing unit test** in a new `#[cfg(test)] mod tests` at the end of `backdrop.rs`:

```rust
#[test]
fn quiet_regions_turn_points_into_window_heights_and_ignore_empty_ones() {
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let regions = [
        Rect::from_min_max(pos2(24.0, 36.0), pos2(170.0, 68.0)),
        Rect::NOTHING,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 100.0)),
    ];
    let heights = quiet_in_heights(regions, 2.0, 1000.0);
    assert!(close(heights[0], [0.048, 0.072, 0.34, 0.136]), "{:?}", heights[0]);
    assert!(heights[1].iter().all(|edge| *edge >= 1.0e3), "{:?}", heights[1]);
    assert!(close(heights[2], [0.0, 0.0, 0.1, 0.2]), "{:?}", heights[2]);
}
```

- [ ] **Step 2: Write the failing start-screen test** in `start.rs`'s tests module. Add a helper `fn text_rect(output: &egui::FullOutput, needle: &str) -> egui::Rect`, which takes the first `Shape::Text` whose galley text contains `needle` and returns `Rect::from_min_size(text.pos, text.galley.size())`. Then:

```rust
#[test]
fn the_field_stays_quiet_around_the_brand_the_form_and_the_window_buttons() {
    for (width, height) in [(640.0, 480.0), (960.0, 540.0), (1280.0, 800.0), (1920.0, 1080.0), (3840.0, 2160.0)] {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut state = StartState::new("Alex".into());
        state.error = "Couldn't reach the room".into();
        // The form is centred from the height measured on the previous pass.
        let output = (0..3).map(|_| run(&ctx, &mut state, sized(width, height)).0).last().unwrap();
        let Some(BackdropRequest { kind: BackdropKind::Field { quiet: [brand, form, buttons] }, .. }) =
            backdrop::take_request(&ctx)
        else {
            panic!("the start screen asked for no field at {width}");
        };
        assert!(brand.expand(1.0).contains_rect(text_rect(&output, "OpenStream")), "{width}");
        for line in ["Share your screen", "Your name", "Create room", "Have an invite?", "Join room", "Couldn't reach the room"] {
            assert!(form.expand(1.0).contains_rect(text_rect(&output, line)), "{line} at {width}");
        }
        assert!(buttons.max.x >= width - 25.0 && buttons.min.y <= 37.0 && buttons.width() >= 64.0, "{buttons:?}");
        if width >= 960.0 {
            assert!(brand.max.x <= width / 2.0 && form.max.x <= width / 2.0, "{form:?} at {width}");
        } else {
            assert!((form.center().x - width / 2.0).abs() <= 1.0, "{form:?} at {width}");
        }
    }
}

#[test]
fn an_invite_keeps_the_field_quiet_around_its_question() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut state = invited(&valid_ticket());
    let output = (0..3).map(|_| run(&ctx, &mut state, sized(1440.0, 900.0)).0).last().unwrap();
    let Some(BackdropRequest { kind: BackdropKind::Field { quiet: [_, form, _] }, .. }) =
        backdrop::take_request(&ctx)
    else {
        panic!("no field");
    };
    assert!(form.expand(1.0).contains_rect(text_rect(&output, "Join")));
}
```

- [ ] **Step 3: Write the failing GPU tests** in `tile_render.rs`. Rename `draw_the_horizon` to `fn draw_the_field(device, queue, size, quiet: [egui::Rect; 3]) -> Vec<u8>`, requesting `BackdropKind::Field { quiet }`, and add `const OPEN: [egui::Rect; 3] = [egui::Rect::NOTHING; 3];`. Point `the_signal_gathers_…` at `draw_the_field(…, OPEN)`. Replace `the_horizon_lights_the_foot_of_the_window_and_leaves_the_top_dark` with:

```rust
#[test]
fn the_field_lights_folds_across_the_window_rather_than_a_band() {
    // gpu() guard as in the neighbouring tests
    let size = (640, 360);
    let pixels = draw_the_field(&device, &queue, size, OPEN);
    let (lit, _) = phosphor_channels();
    let lit_in = |rows: std::ops::Range<u32>| pixels.chunks(4).skip((rows.start * size.0) as usize)
        .take(((rows.end - rows.start) * size.0) as usize).filter(|pixel| pixel[lit] > 96).count();
    let (top, bottom) = (lit_in(0..180), lit_in(180..360));
    assert!(top > 0 && bottom > 0, "top {top}, bottom {bottom}");
    assert!((top + bottom) * 10 < (size.0 * size.1) as usize * 6, "folds, not a flood");
}

#[test]
fn a_quiet_region_leaves_plain_ground_whatever_the_field_does() {
    // gpu() guard
    let size = (640, 360);
    let open = draw_the_field(&device, &queue, size, OPEN);
    let (lit, _) = phosphor_channels();
    assert!(open.chunks(4).any(|pixel| pixel[lit] > 96), "nothing to quiet");
    let everywhere = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(640.0, 360.0));
    let quiet = draw_the_field(&device, &queue, size, [everywhere, egui::Rect::NOTHING, egui::Rect::NOTHING]);
    let ground = brp_app::ui::theme::ground().to_array();
    let worst = quiet.chunks(4).map(|pixel| (0..3).map(|c| pixel[c].abs_diff(ground[c])).max().unwrap()).max().unwrap();
    assert!(worst <= 4, "{worst} levels off the ground");
}
```

If `the_field_lights_folds_…` fails only because the folds at clock 0 miss one half of a 640 × 360 window, render it at 1280 × 720 rather than lowering the bar. If `the_signal_gathers_…` fails, retune only its three thresholds, keeping the three properties it names, and add a one-line comment saying they were tuned for the field.

- [ ] **Step 4: Run them to verify they fail**

Run: `cargo test -p brp --lib -- render::backdrop ui::start && cargo test -p brp --test tile_render`
Expected: compile errors for `quiet_in_heights`, `BackdropKind::Field` and `draw_the_field`.

- [ ] **Step 5: Implement the field.**
  - In `backdrop.rs`: replace `Horizon` with `Field { quiet }` and give it a doc comment. `look()` gives `Field` the horizon's values. Add `quiet_in_heights` (`edge × pixels_per_point / window_height`). In `prepare`, the kind match yields the quiet uniforms. `Field` sends `quiet_reach = QUIET_REACH_POINTS × pixels_per_point / window.1`, `quiet_strength = QUIET_STRENGTH`, and its converted regions. `Scope` sends `quiet_strength = 0.0` and `[[FAR_AWAY; 4]; 3]`. Keep `kind_value` 0.0 for `Field`.
  - In `backdrop.wgsl`: mirror the `Scene` change and delete `horizon`. Add the field's functions with this body, which the spec fixes and the tests do not determine:

```wgsl
fn lattice(at: vec2<f32>) -> f32 {
    return chance(vec2<u32>(vec2<i32>(floor(at)) + vec2<i32>(4096)), 7u);
}

fn noise(at: vec2<f32>) -> f32 {
    let i = floor(at);
    let f = fract(at);
    let u = f * f * (3.0 - 2.0 * f);
    let low = mix(lattice(i), lattice(i + vec2<f32>(1.0, 0.0)), u.x);
    let high = mix(lattice(i + vec2<f32>(0.0, 1.0)), lattice(i + vec2<f32>(1.0, 1.0)), u.x);
    return mix(low, high, u.y);
}

fn fbm(start: vec2<f32>) -> f32 {
    var at = start;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var n = 0; n < 4; n++) {
        sum += amplitude * noise(at);
        at = at * 2.03 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return sum / 0.9375;
}

fn folds(at: vec2<f32>) -> f32 {
    let t = scene.drift_clock;
    let p = at * 1.5;
    let q = vec2<f32>(fbm(p + vec2<f32>(0.0, t * 0.07)), fbm(p + vec2<f32>(5.2, 1.3) + vec2<f32>(t * 0.06, 0.0)));
    let r = vec2<f32>(
        fbm(p + 2.0 * q + vec2<f32>(1.7, 9.2) - vec2<f32>(t * 0.04)),
        fbm(p + 2.0 * q + vec2<f32>(8.3, 2.8) + vec2<f32>(t * 0.035)),
    );
    return fbm(p + 2.2 * r);
}

fn quiet(at: vec2<f32>) -> f32 {
    var most = 0.0;
    for (var n = 0; n < 3; n++) {
        let region = scene.quiet[n];
        let outside = max(max(region.xy - at, at - region.zw), vec2<f32>(0.0));
        most = max(most, 1.0 - smoothstep(0.0, scene.quiet_reach, length(outside)));
    }
    return most * scene.quiet_strength;
}
```

    `fn field(at: vec2<f32>) -> Light` returns the Global Constraints' `amount` with `wake = 0` and its `order`. `fs_cells` calls it where it called `horizon`.
  - In `start.rs`: declare `let mut quiet = [egui::Rect::NOTHING; 3];` before the panel. Inside it, record `ui.min_rect()` at the end of the brand closure into `quiet[0]` and at the end of the buttons closure into `quiet[2]`, and the `ui.vertical(…)` response rect around `form(…)` into `quiet[1]`. Call `backdrop::request` with `BackdropKind::Field { quiet }` after the panel, instead of before it. The repaint request stays where it is.

- [ ] **Step 6: Run them to verify they pass**

Run: `cargo test -p brp --lib -- render::backdrop ui::start && cargo test -p brp --test tile_render`
Expected: PASS. A machine with no GPU adapter prints "no GPU adapter; skipping" for the GPU tests, and that does not count as a pass: run them where an adapter exists.

- [ ] **Step 7: Look at it.** Run `cargo run -p brp --example ui_preview -- /tmp/field start 1600 1000 60`, convert the PPM to PNG, and check that the field fills the window behind the wall with soft quiet around the brand and form.

- [ ] **Step 8: Commit** (only once the user has said to commit)

```bash
git add crates/app/src/render/backdrop.rs crates/app/src/render/backdrop.wgsl crates/app/src/ui/start.rs crates/app/tests/tile_render.rs
git commit -m "feat(app): draw the start screen's light as a field that goes quiet around the form"
```

---

### Task 2: The pointer push

**Files:**
- Modify: `crates/app/src/render/backdrop.rs`: new `Push`, `Backdrop.push`, `step_pointer`, the not-moving branch of `prepare`, `Scene`
- Modify: `crates/app/src/render/backdrop.wgsl`: `Scene`, the push in `fs_cells` and `field`
- Test: `backdrop.rs` tests module and `crates/app/tests/tile_render.rs`

**Interfaces:**
- Consumes: `fn field(at)` and `Scene` from Task 1.
- Produces (private, `backdrop.rs`): `#[derive(Default)] struct Push { samples: Vec<PushSample>, anchor: Option<(Pos2, f32)>, velocity: Vec2 }` and `struct PushSample { at: Pos2, velocity: Vec2, age: f32 }`. Positions are in window heights, newest first. Its methods are `fn step(&mut self, dt: f32, here: Option<Pos2>)`, `fn clear(&mut self)` and `fn uniforms(&self) -> ([[f32; 4]; MAX_PUSH], [[f32; 4]; MAX_PUSH / 4])`. The first array holds `[x, y, vx, vy]` per sample; the second holds the lives `1 − age / PUSH_SECONDS`, four to a `vec4`, in sample order. Constants: `PUSH_SPACING = 0.008`, `MAX_PUSH = 24`, `PUSH_SECONDS = 1.2`, `PUSH_SMOOTHING = 0.6`.
- Produces: `Scene` appends `push: [[f32; 4]; 24]` and `push_life: [[f32; 4]; 6]`, mirrored in WGSL as `array<vec4<f32>, 24>` and `array<vec4<f32>, 6>`.

- [ ] **Step 1: Write the failing unit tests** in `backdrop.rs`'s tests module:

```rust
fn walk(push: &mut Push, steps: usize, dt: f32, stride: f32) {
    for n in 0..=steps {
        push.step(dt, Some(pos2(n as f32 * stride, 0.5)));
    }
}

#[test]
fn push_records_a_sample_every_0_008_window_heights_of_travel() {
    let mut push = Push::default();
    // Steps of 0.005 record on every second step, well clear of 0.008's float edge.
    walk(&mut push, 10, 0.01, 0.005);
    assert_eq!(push.samples.len(), 5);
}

#[test]
fn push_keeps_the_newest_24_samples_and_lets_them_age_out() {
    let mut push = Push::default();
    walk(&mut push, 40, 0.01, 0.01);
    assert_eq!(push.samples.len(), 24);
    assert!((push.samples[0].at.x - 0.40).abs() < 1e-5);
    for _ in 0..13 {
        push.step(0.1, None);
    }
    assert!(push.samples.is_empty());
}

#[test]
fn push_velocity_is_window_heights_per_second_from_the_last_sample() {
    let mut push = Push::default();
    walk(&mut push, 50, 0.01, 0.01);
    assert!((push.velocity.x - 1.0).abs() < 1e-3 && push.velocity.y.abs() < 1e-6, "{:?}", push.velocity);
}

#[test]
fn a_pointer_that_leaves_and_comes_back_far_away_does_not_throw_the_field() {
    let mut push = Push::default();
    push.step(0.01, Some(pos2(0.1, 0.1)));
    push.step(0.01, None);
    push.step(0.01, Some(pos2(0.9, 0.9)));
    assert!(push.samples.is_empty() && push.velocity == Vec2::ZERO);
}

#[test]
fn push_lives_pack_four_to_a_vec4_in_sample_order() {
    let mut push = Push::default();
    walk(&mut push, 2, 0.01, 0.01);
    push.step(0.6, None);
    let (samples, lives) = push.uniforms();
    assert!((lives[0][0] - 0.5).abs() < 1e-5 && (lives[0][1] - (1.0 - 0.61 / 1.2)).abs() < 1e-5);
    assert_eq!(lives[1], [0.0; 4]);
    assert!((samples[0][0] - 0.02).abs() < 1e-6);
}
```

- [ ] **Step 2: Write the failing GPU test** in `tile_render.rs`. Add a helper `fn draw_with(device, queue, backdrop: &mut Backdrop, size, pointer: Pointer, moving: bool) -> Vec<u8>` that factors out `draw_the_field`'s body, and make `draw_the_field` call it. Then:

```rust
#[test]
fn a_held_field_draws_the_same_frame_while_the_pointer_moves_and_clicks() {
    // gpu() guard
    let size = (640, 360);
    let mut backdrop = Backdrop::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    backdrop.skip_opening();
    let before = draw_with(&device, &queue, &mut backdrop, size, Default::default(), false);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let pointer = Pointer { at: Some(egui::pos2(320.0, 180.0)), down: true };
    let after = draw_with(&device, &queue, &mut backdrop, size, pointer, false);
    assert!(before == after, "a held field moved");
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p brp --lib render::backdrop && cargo test -p brp --test tile_render a_held_field`
Expected: compile errors for `Push`, `draw_with` and `Pointer` (import it from `brp_app::render::backdrop`).

- [ ] **Step 4: Implement `Push::step`** with this algorithm, which the tests do not fully determine:

```text
age every sample by dt; drop those with age >= PUSH_SECONDS
if here is None: anchor = None; return
if anchor is None: anchor = (here, 0); velocity = ZERO; return
since = anchor.since + dt
velocity = velocity * PUSH_SMOOTHING + (here - anchor.at) / max(since, 1/240) * (1 - PUSH_SMOOTHING)
if anchor.at.distance(here) >= PUSH_SPACING: insert {here, velocity, 0} at the front, truncate to MAX_PUSH, anchor = (here, 0)
else: anchor = (anchor.at, since)
```

  `clear` empties all three fields. In `Backdrop`, add `push: Push`. Step it in `step_pointer` with the same `here` the stirs use, or `None` when `pointer.at` is `None`. Clear it in `prepare`'s not-moving branch beside `stirs` and `rings`, and write `push.uniforms()` into `Scene`.

- [ ] **Step 5: Implement the push in WGSL.** Add `struct Wake { shift: vec2<f32>, strength: f32 }` and `fn push_at(at: vec2<f32>) -> Wake`. It loops over the 24 samples, reading the life as `scene.push_life[n / 4][n % 4]` and skipping lives ≤ 0, and computes `d` and `wake` exactly as in the Global Constraints. Name the constants `PUSH_NEAR = 0.0045`, `PUSH_SCALE = 0.012`, `PUSH_SOFTEN = 0.06`, `PUSH_LIGHT = 0.22`, `PUSH_STIR = 0.35` and `PUSH_FAST = 1.5`. `field` takes the `Wake`, samples `folds(at - wake.shift)` and adds `PUSH_LIGHT * wake.strength` inside the bracket before gain and quiet. `fs_cells` computes the wake only for the field kind, and seeds `stirred` with `max(1.0 - resolved, PUSH_STIR * wake.strength)`.

- [ ] **Step 6: Run them to verify they pass**

Run: `cargo test -p brp --lib render::backdrop && cargo test -p brp --test tile_render`
Expected: PASS on a machine with a GPU adapter.

- [ ] **Step 7: Commit** (only once the user has said to commit)

```bash
git add crates/app/src/render/backdrop.rs crates/app/src/render/backdrop.wgsl crates/app/tests/tile_render.rs
git commit -m "feat(app): let the pointer push the start screen's field"
```

---

### Task 3: Remove the wall and its stills, and update the docs

**Files:**
- Delete: `crates/app/src/ui/wall/` (`mod.rs`, `geometry.rs`, `reveal.rs`, `stills.rs`), `crates/app/assets/stills/` (120 JPEGs + `README.md`)
- Modify: `crates/app/src/ui/mod.rs:18`, `crates/app/src/window.rs:50,963`, `crates/app/src/ui/start.rs` (line 5 import, 136–144 constants, 181–185 wall draw, 205–209 inset, 225–231 `wall_area`, tests 438–537)
- Modify: `Cargo.toml:48` and `crates/app/Cargo.toml:41` (drop `image`), `ci/stage-linux.sh:13`, `ci/stage-windows.ps1:15`, `README.md:457–459` (the stills sentence and the blank line before it)
- Modify: `crates/app/tests/ui_flow.rs:1250` (rename only), `crates/app/examples/ui_preview.rs:7` (doc line)
- Modify: `DESIGN.md` (lines 37, 80–82, 125–127, the Motion entries at 184–200, 207–208, 215), and `docs/superpowers/specs/2026-11-04-start-screen-drift-wall-design.md` (its status line)

**Interfaces:**
- Consumes: `BackdropKind::Field` and `backdrop::FRAME` from Task 1.
- Produces: in `start.rs`, rename `WALL_MIN_WINDOW_WIDTH` to `FORM_BESIDE_FIELD_MIN_WIDTH` (960.0), with the doc comment "Wide enough to keep the form on the brand's edge with the field beside it, rather than centred". Rename the local `with_wall` to `form_on_the_left`. `COLUMN_GAP` and `wall_area` go.

- [ ] **Step 1: Write the replacement tests** in `start.rs`. Delete the four wall tests. Add a `fn frame(n: u32, focused: bool) -> egui::RawInput` helper based on `sized(1440.0, 900.0)` with `time: Some(f64::from(n) / 60.0)` and `focused`. Rewrite the Settings half of `a_tile_under_settings_is_not_hovered_and_settings_takes_the_click` as `settings_takes_a_click_on_the_start_screen`, keeping its pointer, press and assertion at `pos2(1400.0, 52.0)`. Then add:

```rust
#[test]
fn the_start_screen_asks_for_frames_only_while_the_field_can_move() {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    crate::ui::motion::set_less_motion(&ctx, false);
    let mut state = StartState::new("Alex".into());
    let delay = |output: &egui::FullOutput| output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    let focused = (0..=120).map(|n| run(&ctx, &mut state, frame(n, true)).0).last().unwrap();
    assert!(delay(&focused) > Duration::ZERO && delay(&focused) <= backdrop::FRAME, "{:?}", delay(&focused));
    let unfocused = (121..=240).map(|n| run(&ctx, &mut state, frame(n, false)).0).last().unwrap();
    assert_eq!(delay(&unfocused), Duration::MAX);
}
```

If the unfocused delay is not `Duration::MAX`, find what still asks for frames, such as a caret blink. Fix it only if it's the start screen's own request. Otherwise report it rather than loosening the test.

Add `use std::time::Duration;` to the tests module.

- [ ] **Step 2: Run the new tests before removing the wall**

Run: `cargo test -p brp --lib -- ui::start`
Expected: both may already PASS, because the wall's faster requests also stay under `FRAME` and also stop when the window is unfocused. They pin that behaviour across the removal rather than drive new code. If either fails, note which assertion failed before continuing.

- [ ] **Step 3: Delete and edit.** `git rm -r crates/app/src/ui/wall crates/app/assets/stills`. Remove `pub mod wall;`, the `wall` import and `wall::release` call in `window.rs`, and the `wall::draw` call, `wall_area` and the wall's constants in `start.rs`. Remove the `image` lines from both `Cargo.toml` files, after `rg -n "image::" crates` shows no other user. Remove the `STILLS-NOTICE` copy lines from both stage scripts and the stills sentence from `README.md`. Rename `the_form_stays_clickable_beside_the_wall` to `the_form_stays_clickable_over_the_field`. Change `ui_preview.rs`'s doc line to "`PREVIEW_POINTER=x,y` holds the pointer there, to see the light's scramble around it." Update `DESIGN.md` exactly as spec §9 lists. Add `Status: superseded on 2026-11-05 by [the start screen field](2026-11-05-start-screen-field-design.md).` as the first line under the old wall spec's title.

- [ ] **Step 4: Run the whole gate**

Run: `cargo fmt --check && cargo clippy -p brp --all-targets -- -D warnings && cargo test -p brp && rg -n "ui::wall|wall::|assets/stills|STILLS-NOTICE" crates ci README.md DESIGN.md`
Expected: every check passes, and the final `rg` prints nothing.

- [ ] **Step 5: Commit** (only once the user has said to commit)

```bash
git add -A crates ci Cargo.toml Cargo.lock README.md DESIGN.md docs/superpowers/specs/2026-11-04-start-screen-drift-wall-design.md
git commit -m "feat(app): replace the start screen's wall of stills with the field"
```

---

### Task 4: Check it in the real app

No code unless a check fails. Each check below reports what was seen. Show the user before changing any value.

- [ ] **Step 1:** Run `cargo build --release -p brp`. Then run the binary with `XDG_CONFIG_HOME=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)`, focus the window and watch for at least a minute. Pass: the folds drift, no hard edge appears around the logo, form or buttons as a fold passes under them, and all text stays legible. Check at 1× and at a scale factor of 1.5 or 2.
- [ ] **Step 2:** Sweep the pointer across the field slowly, then quickly. Pass: the field visibly bends along the path and settles within about 1.2 s. If it looks choppy at about 30 frames a second, record it for the user and do not change `FRAME`.
- [ ] **Step 3:** Measure the cost with a temporary, uncommitted frame timer. Record the frame time on the desktop GPU, and on Mesa's lavapipe software Vulkan driver at 1920 × 1080, where a full frame must stay under 20 ms. Select lavapipe with `VK_DRIVER_FILES` pointing at its `lvp_icd.*.json` under `/usr/share/vulkan/icd.d/`.
- [ ] **Step 4:** Unfocus the window. Pass: no frames are drawn. Then open an invite link (`brp://…`) and narrow the window below 960 points. Pass: the quiet follows the form in both.
- [ ] **Step 5:** Report the results to the user, with screenshots from the desktop's screenshot tool, saying which checks were seen and which were not.
