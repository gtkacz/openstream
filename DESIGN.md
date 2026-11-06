# OpenStream

OpenStream is the user-facing name of this native peer-to-peer screen-sharing app.
Keep `brp` in crate names, Rust identifiers, executable names, storage paths, tickets,
URL schemes, wire protocols, and release machinery.

## Direction

A terminal for a network: blue phosphor on a character grid, with the connections between people
drawn as a live circuit. It comes from the CRT, the oscilloscope, and retro-futurist control rooms,
and from the way VPN and blockchain software show links rather than hide them.

Four rules keep that from becoming costume:

- **The tube stops at the tile's edge.** Live video stays pixel exact. Nothing analog touches it, with
  one exception below.
- **Analog artefacts mean something went wrong.** A picture reduces to characters only when its
  signal is really gone.
- **Anything that looks like data is data.** Delay, paths, packets, and the traffic strip are all
  measured. The room map has no geography, because that would mean locating people by IP address,
  and it is a star around you, because your own links are the only ones you can know.
- **Things you touch stay on springs; things the network does tick.** Hover and press ease
  continuously. Packets and traffic step at about 12 frames a second.

Colour comes from the phosphor: blue, on a floor that is nearly black. Light runs from `#1E3FD8`
through `#38BDF8` to `#F6F8FF`, secondary text is `#A5D8FF`, and the surfaces rise from the floor
through the darkest blues of that same falloff. A palette holds a ground, three surface steps, a rule,
a wire, and three text roles, and everything in the interface reads through it. Choosing a phosphor is
disabled for now: the code stays, the Settings row is gone, and a saved choice is not applied.
Blue is the wiring and mint is the current: `#40FFAA`, the signal, marks only what is working right
now, so a quiet room is blue and a room gets greener the more is flowing. It has three steps, each with
a job: the signal itself for words, figures, and marks; `#27A095` for a wire carrying data; and
`#063837` behind a control that is on air. It marks live status, a rate above zero, packets, keyframes,
a person's glyph as a real heartbeat lands, the newest good event in the ledger, audio someone is
hearing, a confirmed copy, and the caret. A zero is never mint. In the light it gathers in soft patches
that wander, tinting the characters and the glow only where the light is strong, and the room map's
patches strengthen while something is flowing.
The one colour that does not change is the alarm, `#FF2D78`, because an alarm must read the same on every tube. It marks lost
signal and failures and nothing else.

Every window is one ground with tone steps for depth, under static scanlines and grain that sit below
the text so it keeps its edges. Corners are square. Spacing sits on a 4-point grid, and the type sits
on a character grid of 8 by 16 points. Main controls are 32 points tall and the start form's are 40.
Text has primary, secondary, and muted roles, all at 4.5:1 or better where they are used. Long names
truncate with a tooltip, never displacing actions.

Controls are bracketed labels, `[ Copy link ]`, in reverse video on hover and press. The one main
action on a screen is a solid block of `#1E3FD8` with `#F6F8FF` text. A bar that frames its controls already, like a tile's toolbar,
drops the brackets. Marks are characters too: `[x]` for a check and `(*)` for a radio. A field's caret
is a blinking block over the next character, in reverse video.

## Type

[IBM Plex Mono](https://github.com/IBM/plex) everywhere: Regular for text, Medium for controls and
labels, SemiBold for titles, embedded in the binary so nothing is fetched. At 14 points with the
tweak in `theme.rs`, one character is exactly 8 points wide, which lands on whole pixels at 1×, 1.25×,
1.5×, and 2×. Figures hold still because every character shares that width. Non-Latin nicknames fall
back to egui's bundled faces where Plex has no glyph.

Icons are a [Lucide](https://lucide.dev) subset, used only where a label will not fit, in the colour
of their text role. An icon with no visible label names itself in a tooltip and to screen readers.

Each person is known by a glyph: a 7 by 7 mirrored pattern drawn from their key, so it is theirs
across rooms and in everyone else's window, with no colour of its own.

## App icon

A CRT drawn in scanlines: thirteen rows make a disc, the O, and the rows left unlit make the S, whose
curve is a 1:3 oscilloscope figure. The mark is one compound shape in one colour, so it holds as a
silhouette. The scanlines blur together at favicon size, so there the disc fills in and keeps only
the S. Inside the app the mark takes the phosphor's colour. The desktop icon keeps its own colours.
Files and how to regenerate them: [crates/app/assets/icon](crates/app/assets/icon/README.md).

## Layout

- **Window.** The main window opens fullscreen, which hides its title bar, so the start screen and the
  dock each carry a button beside Settings that leaves it, and F11 does the same. Esc does not, because
  in this window it already closes dialogs and declines invites.
- **Start.** The brand and the form share one column of at most 1200 points, so they stay on one common
  edge on a wide window, while Settings and the fullscreen button sit at the window's top right. The
  light fills the window behind them as a field of characters that thins out softly around the brand,
  the form, and those buttons. At 960 points and wider the form keeps to the brand's edge with the
  field beside it; narrower, it centres.
- **Invite.** A link that names a room replaces the form with one question, in the same place: the
  name the room would see, Join as the main action with the keyboard on it, and Not now, which leads back
  to the full form. It asks rather than saying "You're invited" because a stranger's web page can
  send someone here too.
- **Rail.** The rail is a circuit with you at its root. Your streams branch off first, and everyone
  else in the room branches below, each with what they offer. A wire shows how someone reaches you:
  solid for direct, a node in the middle for relayed, dashes while the path is unknown, red dashes
  once the signal is lost. Packets travel along it only when something is really flowing, closer
  together the more there is, so a quiet link stays lit and still. A person's row gives their delay;
  a stream's row gives its rate, its quality, and a `[ ]` to watch it, and once watched a traffic
  strip of what really arrived, where a tall bright bar is a keyframe and a gap is a moment with no
  frames at all. Someone in voice branches a voice row first, with a level strip and a `[x]` to hear them.
  Your own stream unfolds its controls from its row, and a failure opens it by itself.
  Below all this a ledger keeps the room's last few events on a mission clock counted from when the
  room opened.
- **Dock.** It holds what you do to the room: share, audio, voice, upload, mute, Settings, fullscreen. Always one row,
  because a second would come out of the video. As the width falls it sheds words instead: the audio
  status first, then the tools' labels, the audio labels, the upload figure, and the main action's
  label last. The main action says what to do next: Share monitor carries it only while you are not sharing
  and someone else is, and the canvas's own Share button carries it while nobody is. The audio switch
  and its application chooser are one control on one surface.
- **Tiles.** A tile is the shape of its picture, known from the stream's preset before its first
  frame, so its name chip and controls sit on the video rather than around a letterbox. All tiles
  share one height, one 8-point gap apart. The column count is whichever shows the most picture in the
  area, so two wide streams stack in a normal window and sit side by side in a short wide one; the
  current count is kept while it stays within 10% of the best, so a window resized near a tie does not
  flip the layout. The block sits centred, and so does a last row that is not full. A name chip stays
  on every grid tile so screens can be told apart, with the path and delay beside the name. Controls
  rise from the bottom edge on hover or Tab, hug their content, and lose their labels below 430 points
  of tile width. In a pop-out the window is whatever shape it was dragged to, so the picture still
  letterboxes there, and the chip leaves with the controls.
- **Room map.** With others in the room and nothing on screen, the canvas is a map of the room: you at
  the centre and everyone else as far away as their round trip is long, on a logarithmic scale with
  rings at 10, 50, and 200 ms. Each person's label carries what they offer as one-click buttons. A
  ring flares when a heartbeat really arrives, and someone dims as their last one ages. Below 640 by
  440 points, or above 8 people, it could not be read, and the canvas lists the streams instead.
- **Empty canvas.** Alone in a room: the invite link and Share, on a black backing that keeps them
  legible over the light.
- **Dialogs.** Settings has two columns, labels on one edge and controls on another, stacking below
  400 points. Actions end on the content's right edge with the main one last.
- **Compact, below 800 points.** The rail folds away. The dock opens a Room or Share sheet that
  slides up over the bottom of the video, which keeps playing above it. A sheet is only as tall as
  what it holds, up to 60% of the window, past which it scrolls.
- **Start field.** The light gathers in slowly drifting folds across the whole start screen and goes
  fully quiet behind the brand, the form, and the window's buttons, easing back over 80 points, so
  text always sits on the ground and no fold leaves a hard edge. It is atmosphere rather than a
  control.

## Alignment

egui places a widget before it knows how big the widget is, and a group inside a row starts at the
row's top rather than its centre. Almost every misalignment this interface has had came from that, so
alignment is done by construction and held by tests in `crates/app/tests/ui_flow.rs`:

- A row is sized before its contents are placed, with `theme::split_row`. Controls in a row share one
  height, 32 points.
- Rows in the rail are measured and painted at computed positions. Centring something of unknown
  height, like the start form, uses the height measured last pass and asks for another pass when it
  changes, so no misplaced frame reaches the screen.
- A label's capitals and an icon beside it stay within a point of each other and of the middle of the
  button, at 1×, 1.25×, 1.5×, and 2×. The font's offset is calibrated on rendered pixels, because the
  metrics leave out rounding.
- Anything that must wrap avoids `ui.scope` and `add_enabled_ui`: a child scope cannot wrap in its
  parent's row. That rules out `theme::tactile` in a wrapped row, which is why the Share sheet stacks.
- Groups end on shared edges: the invite buttons split the rail's width equally, ending where the
  rows below end, and a row's figures all end on the rail's inner edge.

## Motion

Everything you touch is a critically damped spring (`crates/app/src/ui/motion.rs`). New input only
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
  lag whatever shape the tile keeps.
- **The tube.** A tile warms up where it stands: the picture opens from a bright line as its stream
  connects. A closed tile collapses back to that line and then to nothing, in the video shader, so
  nothing washes over a neighbour moving into its space.
- **Lost signal.** Four seconds of silence on the connection and the picture starts reducing to
  characters, cell by cell, losing its colour first; the picture's own brightness picks each
  character, and a white page inverts rather than becoming a block. It reads `No signal` in the alarm
  colour with how long it has been. At twelve seconds the connection is given up on and it says it is
  reconnecting, which leaves room for a Wi-Fi reconnect or a cellular handoff to come back on the same
  connection. A signal that comes back returns the picture at once, and someone gone for about 20
  seconds has left the room and the tile goes with them.
- **Surfaces.** Tile controls rise from the bottom edge and sink back the same way. Compact sheets
  grow from the dock, with the video tracking their edge. Dialogs rise 12 points over a deepening
  scrim and leave the way they came, without blocking the room while they go. A click on the backdrop
  does not dismiss them, because they hold drafts; Esc and the close button do.
- **Between states.** Screens dissolve into each other. Rows grow into the rail while what is below
  makes room, and quality details unfold and fold from wherever they have got to. The dock's notice
  line opens and shuts the same way, so the video gives up that height smoothly. The Connecting wash
  fades off the picture as video arrives, still saying what it last said. Stop sharing is always in
  the dock, so starting a stream shoves nothing sideways.
- **The connection line.** A path that resolves is watched doing it: the dashes close into a line and
  a relay's node grows out of it.
- **The light.** Behind the start screen and the room map, a soft field is drawn in characters from
  the same ramp a lost picture uses (`crates/app/src/render/backdrop.rs`). It resolves out of noise
  when it opens, its characters churn while it runs, and the pointer leaves a trail of scrambled cells
  behind it, with a ring of them running out from a click. On the start screen it fills the window
  in drifting folds, and the pointer also pushes the folds along its path. On the room map one sweep
  goes round as it opens. It stays low on the start screen, behind a form, and runs hotter on the map, where it is the
  picture. It never sits behind video.
- **The chrome's glow.** The interface is drawn again at a quarter of the size, blurred, and screened
  over the window, which gives text and lines a soft halo. Screened rather than added, because adding
  washes a colour out towards white.
- **At rest, nothing moves.** A settled spring asks for no frames, so an idle window sleeps. Pulses
  and spinners run only while something is in flight. The standing motion is the light at about 30
  frames a second and the circuit at about 12, only while the window has focus,
  and each settles to a stop without it. Level strips tick with the circuit. A full frame with the light takes about 1.3 ms on a desktop
  GPU and under 20 ms on a software renderer at 1080p.

The operating system's reduced-motion setting is read once per launch
(`crates/app/src/reduced_motion.rs`) and honoured everywhere. Things that would travel arrive in place
and fade instead, what is below a growing row jumps rather than slides, and the tube fades rather than
collapses. Drift, churn, the pointer's push, packets, the light's opening, and the caret's blink all
hold still. Standing motion waits for the answer, so
nobody who asked for less sees a burst of it first.

## References

- IBM Plex Mono and Lucide files, licences, and how to regenerate the subset:
  [crates/app/assets/fonts](crates/app/assets/fonts/README.md).
- The light takes its look from React Bits' Gradient ASCII, and the start screen's field from
  [Asciify's Field background](https://asciify.org/backgrounds?bg=field). Both are written from
  scratch for this renderer. The light needs to stay that way: React Bits is MIT with a Commons
  Clause, which forbids redistributing its components in ported form.
- Apple's *Designing Fluid Interfaces* and Emil Kowalski's animation guidance: springs,
  interruptibility, symmetric paths, feedback on press. Velocity handoff and momentum projection
  are not used yet, because nothing here is thrown; the [roadmap](docs/ROADMAP.md)'s collapsible rail
  would be the first.
- Unslop craft guidance by [Damir Sokolovsky](https://heydamir.design/unslop), CC BY 4.0.
