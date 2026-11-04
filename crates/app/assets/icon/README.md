# App icon

- `openstream.svg`: the icon, and the source for everything else here. `handler.rs` installs it for
  Linux desktops.
- `openstream-mark.svg`: the same mark alone in `currentColor`, for single-colour use.
- `openstream-64.rgba`: raw pixels for winit's window icon, embedded by `icon.rs`.
- `openstream.ico`: the Windows `.exe` icon, linked by `build.rs` through `brp.rc`. Its 16 and 24
  pixel sizes use the favicon's drawing.

`site/join/icon.svg` is the favicon: the same disc and S, without the scanlines, which blur together
at that size. `icon.rs` paints the mark in the app from its own copy of the bars, and a test checks
that copy against `openstream.svg`.

The icon keeps its own colours, `#5CC8FF` on `#0A1017`, so it does not change with the UI theme.
After changing `openstream.svg`, regenerate from this directory:

```sh
rsvg-convert -w 64 -h 64 openstream.svg | magick png:- -depth 8 rgba:openstream-64.rgba
for size in 16 24; do rsvg-convert -w $size -h $size -o $size.png ../../../../site/join/icon.svg; done
for size in 32 48 64 256; do rsvg-convert -w $size -h $size -o $size.png openstream.svg; done
magick 16.png 24.png 32.png 48.png 64.png 256.png openstream.ico
rm 16.png 24.png 32.png 48.png 64.png 256.png
```
