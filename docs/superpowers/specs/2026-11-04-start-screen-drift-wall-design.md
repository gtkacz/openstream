# Start screen: drifting wall of shared screens

Status: design awaiting approval, 2026-11-04. Replaces the room diagram on the start screen. The running reference is a throwaway browser mockup, version C, based on React Bits' DriftWall. No React or DriftWall code ships; only the idea does.

## 1. Intent

The right side of the start screen currently shows a painted diagram of one sharer and two viewers, using the logo as the shared picture. It explains the product accurately but looks like a diagram. It should instead show what the product is for: a tilted wall of frames from the things people share, such as games, anime, films and websites, slowly drifting past.

The wall is atmosphere, not a control. Someone opening the app should see that this is about screens, then use the form.

## 2. Goals

- At 960 points wide and above, a drifting wall replaces the diagram. It runs to the window's top, right and bottom edges and dissolves toward the form.
- Tiles are static frames of recognisable games, anime, films and websites.
- Columns drift in alternating directions at different speeds, in perspective. The wall tilts slightly toward the pointer. Hovering a tile lifts and brightens it and pauses its column.
- The wall drifts only while the window has focus. Without focus it settles to a stop and the window draws nothing.
- With the operating system's reduced-motion setting on, the wall stays still.
- `DESIGN.md` describes the new screen and the amended motion rules.

## 3. Non-goals

- Clickable tiles. Clicking a tile does nothing, and the cursor does not suggest otherwise.
- Live or personalised content, such as recent rooms' pictures. Stills are fixed at build time.
- Fetching images at runtime. Stills are embedded, so the screen works offline, as the fonts do.
- A setting to turn the wall off, or to tune it.
- The rest of the roadmap's reduced-motion item. Other springs keep their current travel.
- Reacting to the reduced-motion setting changing while the app runs. It is read once per launch.
- A wall below 960 points wide. The form centres alone, as it does today.

## 4. Anti-goals

The work has failed, even if it looks right, if any of these is true:

- The form is slower to appear or to respond because of the wall.
- Someone who has asked their system to reduce motion sees the wall drift.
- The window keeps drawing while unfocused, or the app keeps the stills in memory during a call.
- A tile can receive a hover or a click through the Settings button, a dialog, or the form.
- The screen shows something awkward to have open at work, such as nudity or gore.

## 5. Decisions already made

These came from the user:

- **Version C.** Edge to edge, not contained in the old panel, and not the calmer version.
- **Commercial stills, knowingly.** The repo is public under MIT and the app ships as one binary, so the stills are committed and embedded in every release. MIT cannot license them. The user was told the realistic risk is a takedown notice against the repo and its releases, and accepted it instead of using openly licensed material.
- **Stops when unfocused.** `DESIGN.md` is updated to match.

These are the designer's calls, shown to the user without objection:

- **The diagram goes completely.** The typed name on a screen and the Direct and Relayed explanation leave this screen. The connection line stays in the rail and on tiles.
- **About 42 stills.** At the mockup's 32 KB average, that adds roughly 1.3 MB to a 58 MB binary.
- **A notice for the stills.** It states they belong to their owners and are not covered by MIT, and lists each one with its source so any can be dropped from future builds.
- **Reduced motion stills the wall** and leaves opacity changes in place, matching the roadmap's "gentler rather than none".
- **Stills load only for the start screen** and are released when a room opens.

## 6. Behaviour

### Layout

Today's column and form are unchanged. The wall's rectangle is:

- **Left:** the column's left edge plus 444 points, the form's 380 and the existing 64-point gap.
- **Top, right, bottom:** the window's content edges, past the panel's 24-point margin.

The wall is painted beneath the brand row, so the Settings button sits on top of it. Below `DIAGRAM_MIN_WINDOW_WIDTH` (960 points, to be renamed for the wall) nothing is painted.

### Values

The user approved these by choosing version C. Keep them as named constants. The tile height differs from the reference on purpose, because shared screens are 16:9.

| Value | Setting |
|---|---|
| Tile | 200 by 112.5 points, 16:9, 12-point corners, as `theme::RADIUS_CARD` |
| Gap between tiles and between columns | 18 points |
| Scale applied to the whole wall | 1.18 |
| Pitch and yaw | 16° and −14° |
| Perspective distance | 1200 points |
| Wall's distance back from the viewer | 120 points |
| Base drift speed | 42 points per second |
| Column speed spread | each column's speed is `1 + 0.45 × p`, where `p = ((c × 0.6180339887 + 0.35) mod 1) × 2 − 1` for column index `c` |
| Column direction | alternating, even columns upward |
| Starting offset per column | `(c × 0.37) mod 1` of the column's repeat height |
| Pointer tilt | at most ±2.4° per axis: the pointer's offset across the wall's rectangle, from −0.5 to 0.5, times 4.8° |
| Hover lift | 64 points toward the viewer |
| Resting brightness | 55% opacity over `theme::GROUND`, 100% when hovered |
| Fade toward the form | opacity 0 at the wall's left edge to 100% at 26% of its width |

The mockup always used seven columns. The real count comes from the wall's width, so that tiles fill the rectangle at any window size up to 4K with the wall at its largest pointer tilt.

The mockup lives only in `/tmp`, so its geometry is recorded here. The plane of columns is centred on the wall's rectangle, with the perspective's vanishing point at the rectangle's centre. In CSS's order, its transform is:

`translate(-50%, -50%) scale(1.18) rotateX(pitch + tiltY) rotateY(yaw + tiltX) translateZ(-120px)`

- `tiltX` is the pointer's horizontal offset times 4.8°, and `tiltY` is its vertical offset times −4.8°.
- A column's offset grows by its signed speed times the frame's time, and the column is drawn shifted up by its offset, so a positive speed moves it upward.
- A hovered tile moves along the plane's own Z axis, before the plane's transform applies.

### Motion

Everything that eases follows `DESIGN.md`: critically damped springs through `crates/app/src/ui/motion.rs`, with no overshoot. The mockup's CSS curves are not carried over.

- **Column speed** is the column's base speed times a 0-to-1 spring. Its target is 1, or 0 if the window is unfocused, the column is hovered, or reduced motion applies.
- **Lift and brightness** ease with the same spring per tile, at `motion::REFLOW`.
- **Pointer tilt** eases toward the pointer and back to rest when it leaves the wall.
- A column's stills repeat at its repeat height, so a tile never jumps.

### Focus and frames

Focus is `ctx.input(|i| i.focused)`. Once every spring has settled and no column is moving, the wall asks for no frames. This must hold: the window's `repaint_delay` goes back to idle.

Steady drift asks for frames with `request_repaint_after` at about 60 per second, not with an uncapped `request_repaint`. `crates/app/src/render/surface.rs` uses `get_default_config` and never sets a present mode. wgpu may therefore pick a mode that isn't vsynced, and uncapped repaints could then spin the GPU.

Two details affect this:

- **egui shortens the delay.** It subtracts `predicted_dt`, 1/60 of a second by default, from any requested delay, and neither `egui-winit` nor this app overrides it. A request for 1/60 of a second becomes zero and `window.rs` redraws at once, so the request must add `predicted_dt` to the intended frame time.
- **Springs stay uncapped.** `motion::drive` calls `request_repaint()` while a spring moves. The wall's springs may keep that, like every other spring in the app, because they settle within a fraction of a second. The cap applies once they have settled.

### Hover

Pointer position comes from an `ui.interact` over the wall's rectangle with `Sense::hover`, so anything above it, such as Settings or a dialog, takes the pointer first.

The hovered tile is the one whose resting cell contains the pointer once projected. A cell is the tile plus its half of the gap on each side, as in the mockup. Cells touch, so moving across a column stays on a tile and the column stays paused, instead of restarting at each gap. Cells do not move with the lift, so a tile cannot flicker at its edge by lifting away from the pointer.

The hovered tile is painted last, so it sits above its neighbours.

### Reduced motion

Read the setting once per launch, off the UI thread:

- **Linux:** the desktop portal's `org.freedesktop.appearance reduced-motion`, then `org.gnome.desktop.interface enable-animations`. No answer means no preference. On the development machine, which runs KDE, the portal returned both keys (`reduced-motion` 0, `enable-animations` true). Whether KDE's own animation setting changes them has not been tested. If it does not, add `org.kde.kdeglobals.KDE AnimationDurationFactor`, where 0 means off.
- **Windows:** `SystemParametersInfoW` with `SPI_GETCLIENTAREAANIMATION`.

Until the answer arrives the wall stays still, so nobody who asked for less motion sees a burst of it.

With the setting on, there is no drift and no pointer tilt. A hovered tile brightens but does not lift.

## 7. Suggested approach

This is advice, and the builder may change it if the first real run shows a better path.

**Painting.** egui has no 3D transforms, but `egui::Mesh` takes arbitrary textured triangles with per-vertex colour. Project each tile's corners on the CPU and subdivide each tile into a small grid, about 6 by 6. Two triangles alone would show a visible seam, because egui's texture mapping is not perspective-correct. Per-vertex colour supplies both the fade and the dimming. Bake the rounded corners into each texture's alpha at load, so they tilt with the tile.

This keeps the wall inside egui's layering, so Settings and dialogs need no special handling and `crates/app/tests/ui_flow.rs` can drive it. The alternative is a dedicated wgpu pass like the video tiles' renderer. It would be perspective-correct, but it adds a pipeline and has to be composited under egui, so reach for it only if meshes visibly fail.

**Sharpness.** egui-wgpu is not known to generate mipmaps, so stills drawn much smaller than their pixel size may shimmer as they drift. Stills are 480 by 270, about 2:1 at 1× scale. If shimmer shows in the first run, resize stills on load to the tile's on-screen pixel size.

**Module.** Put the wall in a new `crates/app/src/ui/wall/` module, with projection, column layout and hit-testing as pure functions so they can be tested without a window.

**State.** Keeping wall state in egui's temporary memory, as springs do, leaves `start::draw`'s signature and its many test call sites alone. `StartState` derives `Eq`, so it cannot hold floats. Release the state and textures on the transition to `Phase::Room` in `crates/app/src/window.rs`.

**Decoding.** Decode on a background thread, so the first frame never waits. Each still fades in as it arrives. Tests that draw the start screen at 960 points or wider will also trigger decoding, so keep that from slowing the suite noticeably.

**Dependencies.**

- `image` is already in the tree through `arboard`, with `bmp`, `png` and `tiff`. Add it directly to `crates/app` with `default-features = false` and `jpeg`. PNG would avoid the feature but makes the stills several times larger.
- `ashpd` is already a workspace dependency with `screencast`. Add it to `crates/app` for Linux only and enable its `settings` feature.
- `windows-sys` needs `Win32_UI_WindowsAndMessaging` added for the Windows lookup.

## 8. Stills

Stills live in `crates/app/assets/stills/` and are embedded with `include_bytes!`, alongside a `README.md`.

- **Format:** JPEG, 480 by 270, quality about 82.
- **Mix:** about 42 across games, anime, films and websites, spread so no column clusters one kind.
- **Exclusions:** no nudity, implied nudity or gore. The mockup's Ghost in the Shell and Blade Runner 2049 stills are out for this reason.
- **Text-heavy stills:** kept few, since they are the most likely to shimmer and the hardest to read at this size.
- **Starting point:** the mockup's 22 other stills, from Steam store screenshots, TMDB backdrops without language text, and headless Chrome captures. They are in `/tmp/brainstorm-2399704-1793805829/content/`, which may not survive a reboot, so the first build step should copy what it keeps. The fetch script is `/tmp/brainstorm-scripts/fetch-tiles.sh`.

The `README.md` states that the stills belong to their respective owners, are not covered by the repository's MIT licence, and are used as illustrative examples. It lists each file's work, source URL and retrieval date, so a still can be dropped from future builds by deleting its file and its list entry, then rebuilding. The main `README.md` gets a sentence pointing to this exception.

Deleting a still does not remove it from anything already published. Once pushed, it stays in git history, in forks and clones, and inside the binaries of past releases. Removing it fully could mean rewriting history and pulling or rebuilding old release archives, and forks and clones cannot be recalled. Where the stills live is therefore a decision to settle before the first push.

The notice ships with releases, because the archives carry the MIT `LICENSE` beside a binary that embeds the stills. `ci/stage-linux.sh` and `ci/stage-windows.ps1` both copy it, under a name such as `STILLS-NOTICE.md`. The Windows zip carries no `README.md`, so the main README's sentence does not cover it.

## 9. Code removed

From `crates/app/src/ui/start.rs`: `room_diagram`, `diagram_screens`, `DiagramChip`, `paint_chip`, `shared_picture`, their unit tests, and the `PathKind`, `members` and `motion` imports they leave unused.

From `crates/app/tests/ui_flow.rs`: `the_start_diagram_puts_the_typed_name_on_the_screen_that_person_will_have` goes. `the_start_form_is_centred_on_the_same_axis_as_the_diagram` becomes a test that the form is still vertically centred.

`members::paint_path` and `PathShape` stay, because the rail and tiles use them. `icon::paint_mark_at` loses its only caller outside `icon.rs`, so it can become private.

## 10. Documentation changes

**`DESIGN.md`:**

- **Direction:** after "No glows, glass, or decorative gradients", name the wall's fade toward the form as the one exception.
- **Layout, Start:** the brand, form and Settings button share the column, and the wall runs from the form's gap to the window's edges at 960 points and wider.
- **Signature:** remove "the start diagram" from the line's users.
- **Start diagram:** replace this entry with one describing the wall.
- **Motion:** add a Start wall entry. Amend "At rest, nothing moves" to say the wall is the one standing motion, drifting only while the window has focus, at about 60 frames a second.
- **Reduced motion note:** "travel is kept to a few points everywhere" gains the wall as its exception, because the wall reads the operating system's setting.

**`docs/ROADMAP.md`:** the Reduced motion entry records that the lookup now exists and only the wall uses it, and that `org.freedesktop.appearance reduced-motion` is the portal key found. Applying it to `drive` remains open.

## 11. Testing

Automated, without a display or network:

- The wall's rectangle reaches the window's top, right and bottom edges at sizes from 960 by 540 to 3840 by 2160, and never overlaps the form.
- Projected tiles leave no gap in the rectangle at any of those sizes, at the largest pointer tilt in each direction.
- Crossing a column's repeat height in either direction moves a tile only by the drift, and the same still stays at that position.
- With the 6-by-6 grid, or whatever replaces it, the difference between the mesh's texture mapping and true perspective stays under one pixel.
- Hit-testing finds the cell under the pointer, and the pointer never lands between cells when crossing a column.
- Hovering over Settings lifts no tile, and Settings still takes a click at that position.
- Below 960 points, the start screen paints no wall.
- Once focus is lost and springs settle, the frame output asks for no repaint. With focus back and springs settled, steady drift asks for a non-zero delay near the 60-per-second cap, after egui's `predicted_dt` subtraction.
- With reduced motion on, column offsets and pointer tilt do not change across frames, and hover changes brightness but not lift.
- Every embedded still decodes at 480 by 270, and every file in the stills folder is listed in its `README.md`.

By hand, in the real app, because a test cannot judge these:

- **The look.** Earlier work on this app's visual identity showed stills mislead, so the user judges this in motion. Do this first, before polishing, because the mesh approach is unproven.
- **Motion quality.** Look for shimmer, seams and stutter at 1× and at a higher scale factor.
- **Cost.** Report CPU and GPU use while drifting, and confirm no frames while unfocused.
- **KDE reduced motion.** Turn KDE's animations off and check the wall stays still after a relaunch. Restore the setting afterwards.
- **Join link launch.** Start from a `brp://` link and confirm the form stays responsive while connecting.

Use temporary `XDG_CONFIG_HOME` and `XDG_DATA_HOME` when running the app, because `register_scheme_handler` otherwise repoints the user's `brp://` links at the test binary. Do not kill by name, since a real instance may be open.

Windows runtime checks stay deferred, as in earlier phases. The Windows lookup is compiled and unit-tested only, and the hand-over must say so.

## 12. Left to the builder

- The grid density, subject to the texture-mapping test.
- The column count formula and how stills are assigned to columns, provided a column's stills are distinct and neighbours don't obviously match.
- Whether a lifted tile needs a shadow. The mockup had a soft one. It is worth keeping only if it can be done simply and is noticeable in the real app.
- Where the reduced-motion lookup lives, beside `crates/app/src/visibility.rs` being the obvious place, and how its answer reaches the wall.
- Which additional stills to use, within section 8.
