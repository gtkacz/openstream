# OpenStream

OpenStream is the user-facing name of this native peer-to-peer screen-sharing app.
Keep `brp` in crate names, Rust identifiers, executable names, storage paths, tickets,
URL schemes, wire protocols, and release machinery.

## Direction

A control room at night: exact, nocturnal, quick. Live video is the richest thing on screen, so
tiles sit on a black canvas and everything else stays near-black and neutral. The one hue is the
green of a live signal; amber and red mean warnings and errors. The primary action is the only
light surface on a screen. No glows, glass, or decorative gradients.

Every window is one ground (`#0D0E11`) with tone steps for depth, and lines only between the rail,
the dock, and the canvas. Spacing sits on a 4-point grid. Main controls are 36 points with 6-point
corners, cards have 10, dialogs 12, tiles 8. Text has primary, secondary, and muted roles, all at
4.5:1 or better where they are used. Long names truncate with a tooltip, never displacing actions.

## Type

[Chakra Petch](https://fonts.google.com/specimen/Chakra+Petch) everywhere: its squared cuts belong
to monitors and signal gear. Regular for text, Medium for controls and labels, SemiBold for titles,
embedded in the binary so nothing is fetched. Non-Latin nicknames fall back to egui's bundled faces.

A monospace UI would read as a terminal, so monospace is only for figures that change while they
are watched (upload, encoder counters, the stats readout) and for tickets. Chakra Petch's digits
differ in width (`1` is 358 units, `0` is 628), so those figures would jitter. They use egui's
bundled Hack, which adds no bytes. Static figures such as `2560 × 1440` stay in Chakra Petch.

Icons are a [Lucide](https://lucide.dev) subset, one stroke, in the colour of their text role. An
icon with no visible label names itself in a tooltip and to screen readers.

## App icon

A CRT drawn in scanlines: thirteen rows make a disc, the O, and the rows left unlit make the S, whose
curve is a 1:3 oscilloscope figure. The mark is one compound shape in one colour, so it holds as a
silhouette. The scanlines blur together at favicon size, so there the disc fills in and keeps only
the S. Files and how to regenerate them: [crates/app/assets/icon](crates/app/assets/icon/README.md).

## Layout

- **Start.** The form on the left, with Join taking the primary surface from Create as soon as a
  ticket is pasted. At 960 points and wider, the diagram beside it shows what a room is: one
  picture on everyone's screen, joined directly or through a relay.
- **Room.** The rail lists everyone, you first, with the streams they offer; your own streams carry
  their controls there, in the same shape as everyone else's. The dock holds what you do to the
  room: share, audio, upload, mute, Settings. Sharing no longer costs the video any height.
- **Tiles.** A name chip stays on every grid tile so screens can be told apart. Controls rise from
  the bottom edge on hover or Tab, hug their content, and lose their labels below 640 points of
  tile width. In a pop-out the chip leaves with the controls.
- **Compact, below 800 points.** The rail folds away. The dock opens a Room or Share sheet that
  slides up over the bottom of the video, which keeps playing above it.
- **Signature.** A small line between two nodes says how a person's video reaches you: straight for
  direct, a node in the middle for relayed, dashes while the path is unknown.

## Motion

Motion is feedback, and all of it can be interrupted.

- Press lands on pointer-down: a half-point inset on buttons, a stretched thumb on switches.
- Switches and the watched card retarget from their current value, so a second click reverses them.
- Tile controls rise from the bottom edge and sink back the same way in 160 ms with a cubic
  ease-out. The compact sheets slide up from the dock and back down into it.
- Dialogs rise 8 points over a dimmed room in 180 ms and close at once, so dismissing one never
  makes you wait. A click on the backdrop does not dismiss them, because they hold drafts; Esc and
  the close button do.
- A spinner runs only while something is in flight. Nothing else loops, and page content has no
  entrance. egui has no reduced-motion signal, so travel is kept to a few points everywhere.

## References

- Chakra Petch and Lucide files, licences, and how to regenerate the subset:
  [crates/app/assets/fonts](crates/app/assets/fonts/README.md).
- Taste skill: True-black flat, seed 1536711867, adapted to native egui. Its face here is a cool
  near-black ground, its signature element the connection path. Its pill buttons gave way to small
  radii that match the typeface.
- [Awesome DESIGN.md](https://github.com/voltagent/awesome-design-md), particularly Raycast (dark
  surface ladder, tight radii, the white primary) and Linear (near-black canvas, neutral chrome),
  taken as measures rather than copied. Vercel's system is mostly light and fits less well here.
- Apple's *Designing Fluid Interfaces* and Emil Kowalski's animation guidance, for interruptibility
  and for feedback on press. Their springs and gesture physics have nothing to act on in a
  pointer-driven panel layout, so they are not used.
- Unslop craft guidance by [Damir Sokolovsky](https://heydamir.design/unslop), CC BY 4.0.
