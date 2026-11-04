# Roadmap

Work that is planned but not started. [DESIGN.md](../DESIGN.md) describes what the interface does
today. Sizes are rough: small is a change to one area, large crosses crates.

## Reduced motion

Honour the operating system's setting. This is the one accessibility gap in the motion system, and
it was deferred rather than rejected.

- **Why:** `DESIGN.md` keeps travel to a few points everywhere because egui exposes no signal. That
  limits the problem without solving it for someone who needs motion gone.
- **How:** read the setting directly: `SPI_GETCLIENTAREAANIMATION` on Windows, and on Linux
  `org.gnome.desktop.interface enable-animations`, plus the desktop portal's setting if it offers one.
  Check what the portal exposes before relying on it.
- **Where:** every spring goes through `drive` in `crates/app/src/ui/motion.rs`, so there is one place
  to apply it. `flowing_rect` would return its target and `rise` would return nothing, while opacity
  changes stay, because reduced motion means gentler rather than none.
- **Size:** small, plus the platform lookups.
- **Done when:** with the setting on, a tile and a dialog arrive by fading with no travel, and a test
  holds it through a stubbed signal. Check on Windows and on a Linux desktop.

## Spotlight

Double-click a tile to give it the main area, with the other tiles in a strip beside it.

- **Why:** every tile gets an equal share today. The only way to make one stream large is to pop it
  out into another window.
- **How:** tiles only sense hover now, so they need a click sense that leaves the toolbar's controls
  working. A second layout in `crates/app/src/render/grid.rs` gives the spotlight and strip rects, and
  a new place for each tile makes `flowing_rect` animate the change with no further work.
- **Decide first:** whether the strip goes beside or below, which is probably the same "most picture"
  choice the grid already makes, and whether the choice lasts when streams come and go.
- **Size:** small to medium.

## Collapsible rail

Let the room rail fold away, and flick it shut from its edge.

- **Why:** one 16:9 stream in a 1440×900 window grows from 1136×639 to 1424×801, 57% more picture.
- **How:** the rail uses egui's built-in resize handle (`crates/app/src/ui/members.rs`), which has no
  way to carry a throw. Replacing it with our own drag handle gives 1:1 tracking, rubber-banding at
  the width limits, and momentum projection with velocity handoff on release, which would be the
  first thrown thing in the interface. Bounce is justified there and nowhere else.
- **Keep:** a click target and a key, because a drag alone is not accessible. The compact layout's
  sheets stay as they are.
- **Size:** medium.

## Audio level meter

Show that sound is actually flowing, next to the audio control and on whoever is audible.

- **Why:** "Waiting for a listener" and "1 listener" say who is connected, not whether capture hears
  anything. A silent or wrong application selection looks exactly like a working one.
- **How:** there is no level signal today. Measure a peak at capture and at decode in `crates/audio`,
  and carry it through `crates/pipeline` and `crates/room` into the snapshot.
- **Constraint:** the window must still sleep at rest. Throttle to 20 frames a second like the other
  in-flight motion, and ask for no frames during silence.
- **Size:** large, because it crosses crates, though each step is small.

## Source picker thumbnails

Show what each monitor or window looks like before sharing it.

- **Why:** the picker lists names and resolutions only, and picking the wrong window is how a screen
  share leaks something.
- **How:** capture one frame per source when the picker opens, off the UI thread, and fill the
  thumbnails in as they arrive. As far as we can tell this is Windows only, because Linux goes through
  the portal's own chooser. Confirm that first.
- **Depends on:** the Windows runtime checks that are still outstanding, since this adds to that path.
- **Size:** medium.
