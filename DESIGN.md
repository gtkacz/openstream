# OpenStream

OpenStream is the user-facing name of this native peer-to-peer screen-sharing app.
Keep `brp` in crate names, Rust identifiers, executable names, storage paths, tickets,
URL schemes, wire protocols, and release machinery.

## Direction

A control room under deep water: exact, calm, quick. Live video is the richest thing on screen, so
tiles sit on a black canvas and everything else stays in dark sea tones. The one hue is sky blue
(`#5CC8FF`): it marks a live signal, the primary action, and whatever is switched on or focused.
Amber and red mean warnings and errors. No glows, glass, or decorative gradients, with one exception:
the start wall dissolves toward the form, because a hard edge beside the form reads as a mistake.

Every window is one ground (`#0A1017`) with tone steps for depth, and lines only between the rail,
the dock, and the canvas. Spacing sits on a 4-point grid. Main controls are 36 points tall, with
8-point corners; cards and tiles have 12, dialogs 16, chips 6. An icon button is square. Text has
primary, secondary, and muted roles, all at 4.5:1 or better where they are used. Long names truncate
with a tooltip, never displacing actions.

## Type

[IBM Plex Sans](https://github.com/IBM/plex) everywhere: engineered without being themed, with an
unambiguous `&`, `(`, `1`, `l`, and `I`. Regular for text, Medium for controls and labels, SemiBold
for titles, embedded in the binary so nothing is fetched. Non-Latin nicknames fall back to egui's
bundled faces where Plex has no glyph.

Plex was chosen over the more usual Inter because egui cannot switch on OpenType features, so
tabular figures have to be a font's default. Every Plex digit is 600 units wide, which lets a figure
that changes while it is watched (upload, encoder counters, the stats readout) hold still in the
interface's own face. A test holds this.

A monospace UI would read as a terminal, and would not align anything: alignment here is about
where boxes sit, not how wide letters are, and labels would run about a quarter wider, which pushes
the rail and dock out of their widths. The bundled Hack appears only for room tickets, where the
characters are the data.

Icons are a [Lucide](https://lucide.dev) subset, one stroke, in the colour of their text role. An
icon with no visible label names itself in a tooltip and to screen readers.

## App icon

A CRT drawn in scanlines: thirteen rows make a disc, the O, and the rows left unlit make the S, whose
curve is a 1:3 oscilloscope figure. The mark is one compound shape in one colour, so it holds as a
silhouette. The scanlines blur together at favicon size, so there the disc fills in and keeps only
the S. Files and how to regenerate them: [crates/app/assets/icon](crates/app/assets/icon/README.md).

## Layout

- **Start.** The brand, the form, and the Settings button share one column of at most 1200 points, so
  they stay on two common edges on a wide window. At 960 points and wider, a wall of still frames
  runs from the form's gap to the window's top, right, and bottom edges, passing under Settings.
- **Invite.** A link that names a room replaces the form with one question, in the same place: the
  name the room would see, Join as the accent with the keyboard on it, and Not now, which leads back
  to the full form. It asks rather than saying "You're invited" because a stranger's web page can
  send someone here too.
- **Room.** The rail lists everyone, you first, with the streams they offer; your own streams carry
  their controls there, in the same shape as everyone else's. The dock holds what you do to the
  room: share, audio, upload, mute, Settings. Sharing no longer costs the video any height.
- **Dock.** Always one row, because a second would come out of the video. As the width falls it
  sheds words instead: the audio status first, then the tools' labels, the audio labels, the upload
  figure, and the main action's label last. Accent says what to do next: Share monitor carries it
  only while you are not sharing and someone else is, and the canvas's own Share button carries it
  while nobody is. The audio switch and its application chooser are one control on one surface.
- **Tiles.** A tile is the shape of its picture, known from the stream's preset before its first
  frame, so its corners, name chip, and controls sit on the video rather than around a letterbox. All
  tiles share one height, one 8-point gap apart. The column count is whichever shows the most
  picture in the area, so two wide streams stack in a normal window and sit side by side in a short
  wide one; the current count is kept while it stays within 10% of the best, so a window resized near
  a tie does not flip the layout. The block sits centred, and so does a last row that is not full. A
  name chip stays on every grid tile so screens can be told apart. A tile with no picture yet carries
  a wash, so it reads as a tile rather than words adrift on the canvas. Controls rise from the bottom
  edge on hover or Tab, hug their content, and lose their labels below 640 points of tile width. In a
  pop-out the window is whatever shape it was dragged to, so the picture still letterboxes there, and
  the chip leaves with the controls.
- **Empty canvas.** It holds whatever gets something onto it, at the place the video will appear.
  Alone in a room: the invite link and Share. With others and no streams: Share. With streams on
  offer: one target per stream, which starts it with a click.
- **Dialogs.** Settings has two columns, labels on one edge and controls on another, stacking below
  400 points. Actions end on the content's right edge with the main one last.
- **Compact, below 800 points.** The rail folds away. The dock opens a Room or Share sheet that
  slides up over the bottom of the video, which keeps playing above it. A sheet is only as tall as
  what it holds, up to 60% of the window, past which it scrolls.
- **Signature.** A small line between two nodes says how a person's video reaches you: straight for
  direct, a node in the middle for relayed, dashes while the path is unknown. The rail and a tile
  waiting for its video draw it with one function.
- **Start wall.** Frames of the things people share, tilted in perspective, say what the product is
  for. It is atmosphere rather than a control, so clicking a tile does nothing. The stills are embedded
  and are not covered by the MIT licence: see [crates/app/assets/stills](crates/app/assets/stills/README.md).

## Alignment

egui places a widget before it knows how big the widget is, and a group inside a row starts at the
row's top rather than its centre. Almost every misalignment this interface has had came from that, so
alignment is done by construction and held by tests in `crates/app/tests/ui_flow.rs`:

- A row is sized before its contents are placed, with `theme::split_row`. Controls in a row share one
  height, 36 points.
- Two lines beside something, such as a name and its detail beside an avatar, are measured and painted
  at computed positions. Centring something of unknown height, like the start form, uses the height
  measured last pass and asks for another pass when it changes, so no misplaced frame reaches the
  screen.
- A label's capitals and an icon beside it stay within a point of each other and of the middle of the
  button, at 1×, 1.25×, 1.5×, and 2×. The font's offset is calibrated on rendered pixels, because the
  metrics leave out rounding.
- Anything that must wrap avoids `ui.scope` and `add_enabled_ui`: a child scope cannot wrap in its
  parent's row. That rules out `theme::tactile` in a wrapped row, which is why the Share sheet stacks.
- Groups end on shared edges: the invite buttons split the rail's width equally, ending where the
  cards below end, and everything in a card starts on its one left edge.

## Motion

Everything animated is a critically damped spring (`crates/app/src/ui/motion.rs`). New input only
moves a spring's target, so motion continues from the value on screen at its current velocity, and
anything can be reversed mid-flight without a jump. Nothing overshoots, because nothing here is
thrown. No input is ever locked out for an animation to finish.

- **Controls.** Every button, dropdown, and drag value goes through one wrapper, `theme::tactile`:
  hover eases in about 120 ms, a press lands within a frame or two of pointer-down and lets go over
  about 220 ms, and selection and focus ease too. Fields, radios, checks, and switches ease the same
  way, a segmented control's thumb travels between its choices, and a chevron turns over.
- **Tiles.** When the grid changes, tiles and their video flow to their new places in about 360 ms,
  with X and Y independent. A tile is its target plus an offset that decays to nothing, and only a
  change of place adds to the offset, so resizing the window or dragging the rail stays 1:1 with no
  lag whatever shape the tile keeps. A tile asked for from a card grows out of that card. A closed
  tile darkens to the canvas and shrinks where it stood, in the video shader rather than under a wash,
  which would also darken a neighbour moving into its space.
- **Surfaces.** Tile controls rise from the bottom edge and sink back the same way. Compact sheets
  grow from the dock, with the video tracking their edge. Dialogs rise 12 points over a deepening
  scrim and leave the way they came, without blocking the room while they go. A click on the backdrop
  does not dismiss them, because they hold drafts; Esc and the close button do.
- **Between states.** Screens dissolve into each other. Stream cards grow into the rail while what is
  below makes room, and quality details unfold and fold from wherever they have got to. The dock's
  notice line opens and shuts the same way, so the video gives up that height smoothly. The Connecting
  wash fades off the picture as video arrives, still saying what it last said. Join and Create cross-fade their surfaces. Stop sharing is always in the dock, so
  starting a stream shoves nothing sideways.
- **The connection line.** A path that resolves is watched doing it: the dashes close into a line and
  a relay's node grows out of it. A tile waiting for video shows the same line with a pulse crossing
  it from the publisher, blue while connecting and amber while reconnecting.
- **The start wall.** Columns drift at their own speeds in alternating directions. Hovering a tile
  lifts and brightens it and pauses its column, and the wall leans slightly toward the pointer. Each
  still fades in as it finishes loading, and all of them are released when a room opens.
- **At rest, nothing moves.** A settled spring asks for no frames, so an idle window sleeps. Pulses
  and spinners run only while something is in flight, capped at 20 frames a second. The one standing
  motion is the start wall, which drifts at about 60 frames a second, only while the window has
  focus, and settles to a stop without it.

egui has no reduced-motion signal, so travel is kept to a few points everywhere except the start wall,
which reads the operating system's setting itself and stays still when asked. Applying that setting
to every other spring is on the [roadmap](docs/ROADMAP.md).

## References

- IBM Plex Sans and Lucide files, licences, and how to regenerate the subset:
  [crates/app/assets/fonts](crates/app/assets/fonts/README.md).
- Taste skill: True-black flat, seed 25039755, adapted to native egui. Its signature element here is
  the connection path. Its pill buttons gave way to small radii, its neutral ground and accent-free
  buttons gave way to the requested oceanic palette, and its Inter gave way to Plex for the digits.
- [Awesome DESIGN.md](https://github.com/voltagent/awesome-design-md), particularly Raycast (dark
  surface ladder, tight radii, the white primary) and Linear (near-black canvas, neutral chrome),
  taken as measures rather than copied. Vercel's system is mostly light and fits less well here.
- Apple's *Designing Fluid Interfaces* and Emil Kowalski's animation guidance: springs,
  interruptibility, symmetric paths, feedback on press. Velocity handoff and momentum projection
  are not used yet, because nothing here is thrown; the [roadmap](docs/ROADMAP.md)'s collapsible rail
  would be the first.
- Unslop craft guidance by [Damir Sokolovsky](https://heydamir.design/unslop), CC BY 4.0.
