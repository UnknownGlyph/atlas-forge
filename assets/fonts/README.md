# Bundled font

`NotoSansSC-Regular.ttf` is **Noto Sans SC**, Regular weight. It is the default
font of Atlas Forge's exports (PNG, PDF) and of its interface, so a map looks
the same on every computer.

- Source: `ofl/notosanssc/NotoSansSC[wght].ttf` in the official
  [google/fonts](https://github.com/google/fonts) repository.
- The file here is the variable font's `wght=400` instance (the variable
  font's default is Thin), made with fontTools:
  `fonttools varLib.instancer "NotoSansSC[wght].ttf" wght=400 --update-name-table -o NotoSansSC-Regular.ttf`.
  This is how Google Fonts makes its own static files. No glyphs or names
  were otherwise changed.
- License: SIL Open Font License 1.1, in `OFL.txt` beside it. The font is
  not covered by the project's MIT license.

The program looks for the font in `fonts/` next to the executable (as
packaged), then in `assets/fonts/` in a folder above it (a build in the
source tree). Without it, a system font with Chinese characters is used.
