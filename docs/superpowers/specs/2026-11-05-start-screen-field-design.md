# Start screen: a field of characters instead of the wall

Status: approved in conversation, 2026-11-05. Supersedes
[the drifting wall](2026-11-04-start-screen-drift-wall-design.md). The running reference is a throwaway
browser mockup that ports `backdrop.wgsl` to WebGL; it lives only in the gitignored
`.taste-skill/.superpowers/brainstorm/1367420-1793904833/content/field-quiet.html` (option 2, "Soft
halo"), so every value it used is recorded here.

## 1. Intent

The start screen's wall of film, anime, game and website stills makes OpenStream feel like a generic
media-consumption platform. It is out of place next to the terminal-and-phosphor interface the rest of
the app now has. The user's concern is how the screen feels, not what it claims.

The wall and the band of light along the foot of the window are replaced by one field of characters
that fills the window: large folds of light that drift and change shape, drawn by the light the app
already has (`crates/app/src/render/backdrop.rs` and `backdrop.wgsl`). The visual reference is
[Asciify's Field background](https://asciify.org/backgrounds?bg=field). Around the brand, the form and
the top-right buttons the characters thin out and dim gradually, so text stays legible without the
field stopping at a hard edge.

## 2. Goals

- The start screen, including its invite variant, shows the field behind the brand, the form, and the
  Settings and fullscreen buttons, at every window size.
- The wall, its stills, its notice and its code are gone from the app and from future release archives.
- The field moves only while the window has focus, and holds still for someone who asked their system
  for less motion, exactly as the light does today.
- The pointer pushes the field along its path, in addition to today's scramble trail and click rings.
- `DESIGN.md` describes the new screen.

## 3. Non-goals

- The other two directions the user saw and did not choose: a field written in the machine's own key,
  and the room map shown before anyone has joined.
- Any change to the room map's light (`BackdropKind::Scope`), the palette, the type, or the form.
- Clickable or interactive content in the field beyond the pointer push, scramble and rings.
- A setting to turn the field off or tune it.
- Removing the stills from git history or from past releases. The user settled this when the wall
  shipped in v1.4.0: deleting them only affects future builds.

## 4. Anti-goals

The work has failed, even if it looks right, if any of these is true:

- The form is slower to appear or to respond.
- A bright fold leaves a hard-edged hole around the logo, a field or a button. This is what the user
  rejected in the first mockup.
- Any text role on the start screen falls below the 4.5:1 that `DESIGN.md` requires, measured against
  the brightest the field can be behind it.
- The window keeps drawing while unfocused, or someone who asked for less motion sees the field drift.
- A frame costs more than `DESIGN.md` allows: about 1.3 ms on a desktop GPU and under 20 ms on a
  software renderer at 1080p.

## 5. Decisions already made

These came from the user:

- **Field.** Chosen over "Your key" and "Before anyone arrives" from moving mockups. The user was told
  that a field on its own says nothing about people connecting directly, and that it resembles a stock
  web background; they answered that feel was the point.
- **Soft halo.** Each region the field must stay quiet around gets a wide, soft falloff. This was
  chosen over a tight cut-out box per element, rejected as a "dead zone" that "looks too weird". It was
  also chosen over a column fade that would leave the whole left side quiet. The user chose it after
  the designer recommended the column fade, warning that halos merge into a form-shaped patch when a
  fold passes under the form.
- **The rest of this document's sections 1, 2 and 6**, apart from the post-review calls below, played
  back in chat and approved with "Go ahead".

These are the designer's calls. They were shown to the user as guesses and were not objected to:

- The pointer push stays.
- The light keeps stepping at about 30 frames a second rather than the wall's 60, unless the push
  looks choppy at that rate. If it does, ask the user before raising the rate.
- The old wall spec gets a one-line "Superseded by" note at its top.

These are the designer's calls made after a builder's review, and were not shown to the user before
this document:

- **Full quiet inside each region.** The mockup used a strength of 0.8, so a bright fold still showed
  faintly behind text. By hand arithmetic, not measured, that drops muted text to about 2.9:1, the
  error line to about 2.4:1 and the brand name to about 4.0:1. The strength is therefore 1, which
  keeps the 80-point soft falloff outside each region and so leaves no hard edge. The centre of each
  halo is plain ground rather than faintly lit.
- **Narrow windows keep the 80-point falloff.** At 640 by 480 the halos leave the field at full
  strength only in strips about 50 points wide at each side. Scaling the falloff with the window was
  rejected as a new rule nobody asked for: the main window opens fullscreen, and the field is
  atmosphere.

## 6. Behaviour

### What replaces what

- `BackdropKind::Horizon`, whose only caller is the start screen, becomes a field kind. Its look stays
  the horizon's look in `look()`: gain 0.74, ink 0.85, halo 0.22, wash 0.24, the theme's ground, `wire`
  as dim, `accent` as phosphor, `text` as hot, and the signal at full strength. Grain, churn, glyph
  ramp, cell size, halo pass, signal patches and the opening that resolves out of noise all stay as
  they are.
- The `horizon` light function in `backdrop.wgsl` is replaced by the folds below.
- The wall is no longer drawn, loaded or released.

### The folds

All positions are in window heights (`at` in `fs_cells`), and `t` is `drift_clock`, which already
advances at half a unit a second.

- **Noise.** Smooth value noise on an integer lattice, using the shader's existing `chance` hash with
  salt 7, so a cell keeps its numbers at any window size. Lattice points are offset by 4096 before
  hashing, so negative coordinates hash too. Interpolation is smoothstep (`f² × (3 − 2f)`). `fbm`
  sums 4 octaves; each one doubles frequency by 2.03, offsets by (1.7, 9.2), and halves amplitude, and
  the sum is divided by 0.9375.
- **Domain warp.** With `p = at × 1.5`:
  - `q = (fbm(p + (0, 0.07t)), fbm(p + (5.2, 1.3) + (0.06t, 0)))`
  - `r = (fbm(p + 2q + (1.7, 9.2) − 0.04t), fbm(p + 2q + (8.3, 2.8) + 0.035t))`
  - `v = fbm(p + 2.2r)`
- **Light.** `amount = (0.06 + 0.94 × smoothstep(0.44, 0.8, v) + 0.22 × wake) × 0.74 × (1 − quiet)`,
  where `wake` comes from the pointer push below and is 0 without it. The 0.06 floor leaves sparse
  faint dots in the dark between folds.
- **Opening order.** Bright places resolve first: `order = 1 − 0.8 × amount`, using the amount after
  the quiet, so the ground around the interface resolves last.

### The quiet

There are three quiet regions, wherever the layout puts them: the brand row's content (mark and
name), the form's whole rectangle, and the Settings and fullscreen buttons. `quiet` is 1 inside a
region and falls to 0 at 80 points outside it, with a smoothstep falloff measured from the region's
edge. Where regions overlap, the strongest wins. Inside a region the light is 0, so text always sits
on the ground colour. Section 5 explains why this is stronger than the mockup.

The mockup used one region per form row, each the form's full 380-point width. Those rows are at most
24 points apart, so under an 80-point falloff they behave as the single form rectangle used here.

The start screen knows these rectangles during its egui pass, so it passes them in points with its
backdrop request. They follow the layout, so a narrow window's centred form, the invite form, an error
line or the update notice all stay covered as long as they sit inside the form's rectangle.

### The pointer

Today's scramble trail and click rings stay unchanged. The push is new:

- A sample is recorded every 0.008 window heights of pointer travel, holding the position and a
  velocity in window heights per second. The velocity is measured from the last recorded sample, not
  the last pointer event, and smoothed as `0.6 × previous + 0.4 × new` on every frame the pointer is
  in the window. The mockup smoothed on every pointer event instead. That is equivalent in effect,
  because only travel records a sample.
  Keep at most 24 samples, each living 1.2 seconds with its strength falling linearly to zero.
- Each cell samples the folds at `at − d`, where `d = 0.012 × Σ velocity × life × near`, with
  `near = exp(−|at − sample|² / 0.0045)`. Soften it as `d / (1 + |d| / 0.06)` so a fast flick cannot
  tear the field.
- The cell gains `0.22 × wake` light, and scrambles with strength `0.35 × wake`, where `wake` is the
  largest `life × near × min(|velocity|, 1.5) / 1.5` over the samples.
- That light is added before the gain and the quiet are applied, so the quiet dims it too and the
  pointer cannot light up the ground under text.
- Samples, like stirs, are cleared whenever the light is not moving.

### Motion, focus and reduced motion

These are unchanged from the light today. It moves only while the window has focus and
`motion::ambient_allowed` is true. Otherwise it holds its current frame, skips the opening, and drops
stirs, rings and push samples. The start screen asks for frames at `backdrop::FRAME` only while
focused. The wall's own 60-a-second request goes away with it.

## 7. Suggested approach

This is advice, and the builder may change it.

- Give the field kind its quiet regions in points, as `Scope` carries its centre. Convert them to window
  heights in `prepare`, as `Scope`'s centre and reach already are.
- Push samples live in `Backdrop` beside `stirs` and `rings`, and are stepped in `step_pointer`. Uniform
  arrays of `vec4` keep the WGSL layout simple. One `vec4` per sample can carry position and velocity.
  WGSL uniform arrays need a 16-byte stride, so the lives cannot be a bare `f32` array as in the
  mockup: pack four to a `vec4`.
- Keep the quiet geometry and push stepping as pure functions, so they can be tested without a GPU.
- `start.rs` keeps the 960-point threshold for placing the form on the left rather than centring it.
  Rename the constants named for the wall.

## 8. Code and files removed

- `crates/app/src/ui/wall/` (`mod.rs`, `geometry.rs`, `reveal.rs`, `stills.rs`), `pub mod wall;` in
  `crates/app/src/ui/mod.rs`, and `use crate::ui::wall;` and `wall::release` in
  `crates/app/src/window.rs`.
- In `crates/app/src/ui/start.rs`: the `wall::draw` call, `wall_area`, `COLUMN_GAP` if nothing else
  uses it, and the wall tests (`the_wall_reaches_the_windows_edges_and_stops_short_of_the_form`,
  `the_wall_paints_past_the_panels_margin_to_the_windows_edges`,
  `below_960_points_the_start_screen_paints_no_wall`,
  `a_tile_under_settings_is_not_hovered_and_settings_takes_the_click`) and their helpers.
- `crates/app/assets/stills/`: 120 JPEGs and their `README.md`, about 4.2 MB of the binary.
- The `STILLS-NOTICE.md` copy in `ci/stage-linux.sh` (line 13) and `ci/stage-windows.ps1` (line 15).
- The licence-exception sentence in `README.md` (lines 458 and 459).
- The `image` dependency in `crates/app/Cargo.toml` and the workspace `Cargo.toml`, if nothing else
  uses it. Only the wall does today.
- In `crates/app/tests/ui_flow.rs`, `the_form_stays_clickable_beside_the_wall` is still a valid check.
  Keep it under a name that doesn't mention the wall.
- In `crates/app/tests/tile_render.rs`, the GPU tests that use `BackdropKind::Horizon`.
  `the_horizon_lights_the_foot_of_the_window_and_leaves_the_top_dark` no longer holds, so replace it
  with a test of the field. The signal-patch test's thresholds may need retuning for the field.
- In `crates/app/examples/ui_preview.rs`, reword the doc line about the wall's reveal. Keep
  `PREVIEW_POINTER`. A held pointer has no velocity, so it cannot show the push.

## 9. Documentation changes

**`DESIGN.md`:**

- Line 37: drop "One still in six on the wall takes it too."
- Layout, **Start** (lines 80 to 82): the wall and its horizon become a field of characters filling the
  window, thinning out softly around the brand, the form and the buttons.
- Layout, **Start wall** (lines 125 to 127): replace it with a **Start field** entry. The field is
  atmosphere rather than a control.
- Motion, **The light**: on the start screen it fills the window in slowly drifting folds that thin
  out around the interface, and the pointer pushes it as well as scrambling it.
- Motion, **The start wall**: remove.
- Motion, **At rest, nothing moves**: drop "the wall at about 60".
- Reduced motion paragraph: drop "and a still shows its picture whole while the pointer is on it".
- References: "The light and the wall's reveal take their look from React Bits' Gradient ASCII and
  Dither Veil" becomes "The light takes its look from React Bits' Gradient ASCII, and the start
  screen's field from Asciify's Field background (asciify.org)". The shader is our own WGSL. Asciify
  is MIT, but none of its code was read or ported.

**The old wall spec:** add a "Superseded by" line under its title, pointing here.

## 10. Testing

Automated, without a display:

- **Quiet regions:** at 640 by 480, 960 by 540, 1280 by 800, 1920 by 1080 and 3840 by 2160, and on
  the invite screen, the regions cover the brand, the whole form and the top-right buttons. When the
  form sits on the left, the window's right half has no quiet outside the buttons' halo, allowing for
  the form's halo crossing the middle by a few points at 960 wide. Regions are passed in points and
  converted at any scale factor.
- **Contrast:** with the brightest possible fold behind them, every text role inside a quiet region
  still meets 4.5:1. `tile_render.rs` already renders the light on a GPU. Forcing the brightest case
  may need a test-only hook.
- **Push:** samples are spaced by travel, capped at 24, age out at 1.2 seconds, and are dropped when
  the light is not moving.
- **Frames:** with focus lost and springs settled, the start screen asks for no repaint. With focus
  and motion allowed, it asks at `backdrop::FRAME`.
- **Reduced motion:** the drift clock, churn and push do not advance.
- **The form still takes a click** over the field.
- `cargo test`, `cargo clippy -- -D warnings` and `cargo fmt --check` pass, and no file under
  `crates/app/assets/stills/` remains.

By hand, in the real app, first, because stills have misled this project before:

- **The look in motion,** with a bright fold under the logo and under the form (no hard edges, text
  legible), at 1× and a higher scale factor.
- **The push at about 30 frames a second.** If it looks choppy, show the user before changing the rate.
- **Cost:** frame time on the desktop GPU, and on llvmpipe at 1080p against the 20 ms limit.
- **Idle:** no frames while unfocused.
- **The invite screen** and a window narrower than 960 points.

Use temporary `XDG_CONFIG_HOME` and `XDG_DATA_HOME` when running the app, and never kill by name.
Windows and macOS runtime checks stay deferred as before. KDE's reduced-motion behaviour is still
untested and stays out of scope.

## 11. Left to the builder

- How the quiet regions and push samples are packed into the uniform buffer.
- Whether the push's displacement needs a little curl to read as fluid, provided it stays within the
  softened limit.
- Names of the new kind, constants and tests.
- Exact wording of the `DESIGN.md` entries, within section 9.
