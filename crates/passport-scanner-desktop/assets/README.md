`icon.ico` contains Windows icon sizes derived from the supplied repository-root
`icon.png`, preserving its design and transparency. The desktop window embeds
the original PNG; the executable embeds this ICO.

To regenerate from the repository root with Pillow installed:

```python
from PIL import Image
Image.open("icon.png").save(
    "crates/passport-scanner-desktop/assets/icon.ico",
    sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)],
)
```

See `../../../licenses/application-icon.md` for the supplied upstream reference,
the packaged license, and the limits of the recorded asset provenance.
