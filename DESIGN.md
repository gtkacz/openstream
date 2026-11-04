# OpenStream

OpenStream is the user-facing name of this native peer-to-peer screen-sharing app.
Keep `brp` in crate names, Rust identifiers, executable names, storage paths, tickets,
URL schemes, wire protocols, and release machinery.

## Direction

A quiet viewing room: calm, precise, immediate. Live video is the richest part of
the interface. Chrome uses black and graphite, with off-white primary actions.
Small green indicators identify live streams; amber and red mean warnings and errors.
No gradients, glows, decorative animations, or glass panels.

Use one proportional interface face, a 4-point spacing grid, 36-point main controls,
8-point control corners, and 16-point surface corners. Group by space and surface
tone. Text has primary, secondary, and muted roles. Keep full native keyboard focus
feedback. Long names wrap or truncate with a tooltip, never displace actions.

The start screen explains screen sharing and clearly separates creating a room from
joining one. The room keeps the video canvas dominant, members on the left, sharing
controls below, and quiet session information at the bottom. At narrow widths,
controls wrap and forms remain usable. Settings and pickers share the same material.

Motion is immediate feedback: short interruptible hover/press changes and a brief
overlay fade. No moving entrances, looping decoration, or layout animation.

## References

- Taste skill: True-black flat, seed 779756679; adapted to native egui and existing workflows.
  Keep egui's bundled proportional face in the app and the existing system font on
  the join handoff page; external Inter loading would add a network dependency to
  an otherwise self-contained interface. Keep the native room panel arrangement.
- [Awesome DESIGN.md](https://github.com/voltagent/awesome-design-md), particularly
  [Vercel](https://github.com/voltagent/awesome-design-md/blob/main/design-md/vercel/DESIGN.md):
  restrained typography and a consistent spacing/surface hierarchy, rather than its marketing effects.
- Unslop craft guidance by [Damir Sokolovsky](https://heydamir.design/unslop), CC BY 4.0.
