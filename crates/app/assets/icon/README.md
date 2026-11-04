# App icon

- `openstream.svg`: the icon, and the source for everything else here. `handler.rs` installs it for
  Linux desktops.
- `openstream-mark.svg`: the same mark alone in `currentColor`, for single-colour use.
- `openstream-64.rgba`: raw pixels for winit's window icon, embedded by `icon.rs`.

`site/join/icon.svg` is the favicon: the same disc and S, without the scanlines, which blur together
at that size.

The colours are `GROUND` and `LIVE` from `theme.rs`. After changing `openstream.svg`, regenerate the
raw pixels:

```sh
rsvg-convert -w 64 -h 64 openstream.svg | magick png:- -depth 8 rgba:openstream-64.rgba
```
