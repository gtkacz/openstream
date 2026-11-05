# Bundled fonts

Embedded with `include_bytes!` by `crates/app/src/ui/theme.rs`, so the app never fetches a font.

- **IBM Plex Mono** Regular, Medium, SemiBold: unmodified from
  [IBM/plex](https://github.com/IBM/plex). SIL OFL 1.1 with the Reserved Font Name "Plex", see
  `IBMPlexMono-OFL.txt`. Keep these files unmodified: a subset would be a Modified Version and could
  not keep the name.
- **Lucide** (`lucide-static` 1.67.0): a subset of the icon font. ISC, see `Lucide-LICENSE.txt`.

To add an icon, take its codepoint from the package's `font/info.json`, add it to `icons` in
`theme.rs`, and regenerate the subset with every codepoint listed there:

```sh
uv run --no-project --with fonttools pyftsubset lucide.ttf --unicodes=U+e422,... \
  --output-file=Lucide-subset.ttf --no-hinting --desubroutinize --layout-features=''
```
