# Bundled fonts

Embedded with `include_bytes!` by `crates/app/src/ui/theme.rs`, so the app never fetches a font.

- **Chakra Petch** Regular, Medium, SemiBold: unmodified from
  [google/fonts](https://github.com/google/fonts/tree/main/ofl/chakrapetch). SIL OFL 1.1, see
  `ChakraPetch-OFL.txt`.
- **Lucide** (`lucide-static` 1.67.0): a subset of the icon font. ISC, see `Lucide-LICENSE.txt`.

To add an icon, take its codepoint from the package's `font/info.json`, add it to `icons` in
`theme.rs`, and regenerate the subset with every codepoint listed there:

```sh
uv run --no-project --with fonttools pyftsubset lucide.ttf --unicodes=U+e422,... \
  --output-file=Lucide-subset.ttf --no-hinting --desubroutinize --layout-features=''
```
