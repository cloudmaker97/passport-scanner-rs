# MRZ recognition safeguards

TD1 uses three 30-character rows: document information, dates/nationality,
then names. The parser now validates those roles before extracting fields.
The normative layout is ICAO Doc 9303 Part 5, sections 4.2.2 and 4.2.4:
https://www.icao.int/sites/default/files/publications/DocSeries/9303_p5_cons_en.pdf

The previous reader selected the first same-length OCR rows, immediately
returned the first infrared parse, and scanned only the lower third in the
original orientation. Consequently unrelated rows could populate name fields,
while an upside-down back could yield no zone at all.

The current reader tries upright, 180 degrees, left 90, then right 90.
`read_frame_oriented` returns the selected clockwise angle with the diagnostic
reading; its strip is mapped back to original image coordinates. Input images
are never rotated or otherwise changed by recognition. The OCR input preserves
grayscale strokes at 2x enlargement instead of thresholding them globally.

If that first reading fails verification, the reader estimates small text-line
slopes within five degrees. Only supported multi-line geometry enables deskew.
It retries a full-width lower-third strip, then a taller strip with room for
sloping rows, at native resolution. This avoids both clipping edge characters
and doubling interpolation artifacts. There are at most two additional OCR
calls for each supported orientation. Straight verified readings still finish
after the first call. All transformations affect only temporary OCR images;
display/PDF quarter-turn orientation remains independent.

`recognize_oriented` retains the display correction across both light sources.
Verified MRZ orientation takes priority; conflicting source angles abstain.
For MRZ-less fronts, a conservative local whole-image text comparison can supply
an orientation without supplying any document fields. Existing EXIF orientation
is applied before recognition, making correction idempotent. The application
stores the resulting rotation in JPEG EXIF metadata for native/web previews and
PDF export, without recompressing pixels. See [orientation validation](../../docs/implementation/PASSPORT-SCANNER.md#7-recognition-orientation--deskew).

Only structurally valid candidates compete. A uniquely supported local TD1
row permutation can be reordered. Missing or ambiguous rows are rejected.
All required individual and composite checks must pass before the recognizer
publishes document fields. Equally supported but conflicting source readings
are rejected rather than resolved by the infrared-first ordering. The pure
parser still exposes checksum failures for diagnostics; the composite result
is attached to the joined `MrzLine` field.

Recovery adjusts only trailing optional-field filler counts and isolated K/X
noise in padding after the explicit name terminator. It never reconstructs
missing name, number or date characters. Observed optional-data letters remain
unchanged: K and filler contribute the same value modulo ten, so a checksum
cannot prove that K is a misread filler. Names have no checksum; passing checks
does not certify their spelling. Extended document-number layouts are rejected
until their relocated check digit is supported, rather than returning a
silently truncated document number.

Validation on 2026-10-17:

- 45 unit tests passed across `passport-scanner-mrz` and `passport-scanner-recognize`,
  including ICAO fictitious TD1/TD2/TD3 specimens, malformed/permuted/missing
  rows, conflicting names, checksum failures, legitimate single-letter names,
  optional-data checksum collisions, and actual synthetic-pixel rotations.
- Offline replay of one retained real ID back succeeded at all four rotations,
  with required correction angles 180, -90, 0 and 90. Every parsed grid passed
  individual and composite checks. This is an orientation replay, not four
  independent physical scans or an OCR accuracy certification.
- A later front capture produced no zone for either illumination source.
- No real personal identifiers or real scan images were added to test fixtures.

Run the privacy-conscious offline diagnostic with:

```text
cargo run -p passport-scanner-recognize --example review_mrz -- path/to/scan.jpg
```

It prints angles, line lengths and check results, never recognized field values.
Optional `MRZ_REVIEW_SHAPES=1` prints only letter/digit classes for OCR debugging.
