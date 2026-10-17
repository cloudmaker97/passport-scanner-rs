`icon.ico` contains the Windows icon sizes of the repository-root `icon.png`.
Both are rendered from `icon-tile.svg`: the porcelain glyph of `icon.svg` on a
dark tile, so it stays visible on light Explorer and taskbar backgrounds. The
desktop window and the web favicon use the PNG; the executable embeds this ICO.
Its frames are plain bitmaps, which every Windows icon loader reads.

To regenerate from the repository root with Typst and Pillow installed:

```sh
printf '#set page(width: 100pt, height: 100pt, margin: 0pt, fill: none)\n#image("icon-tile.svg", width: 100%%)\n' > icon.typ
typst compile --ppi 368.64 icon.typ icon.png && rm icon.typ
python -c "from PIL import Image; Image.open('icon.png').save('crates/passport-scanner-desktop/assets/icon.ico', sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)], bitmap_format='bmp')"
```

See `../../../licenses/application-icon.md`.
