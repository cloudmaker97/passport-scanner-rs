# Passport Scanner - consolidated documentation

Single reference merging the former `RUNNING`, `PROTOCOL-WIRE`, `PROTOCOL`,
`vendor-color-evidence`, `SHD-LAYOUT-AUDIT`, `AUTO-ROTATION`, `MRZ-DESKEW` and
`MRZ-PERFORMANCE` documents. Superseded interpretations are stated in corrected
form; the corrections themselves are listed in [§ 9](#9-corrected-earlier-claims).

Own runtime measurements: [evidence/](../evidence/). Vendor-derived data (decompiler
output, calibration-file audits) is not committed.

---

## 1. Running

```powershell
.\package.ps1                         # build everything into dist\
& '.\dist\Passport Scanner.exe'       # native Iced UI; other programs in tools\ and debug-tools\
```

**Requirements:** Rust ≥ 1.88; on Windows the MSVC linker (VS Build Tools, C++
workload) - build from PowerShell, Git Bash's GNU `link` shadows MSVC's.
Tesseract (Windows installer path auto-detected, else `TESSERACT=`). The scanner
uses in-box UVC + extension-unit drivers on Windows and Linux. Vendor
calibration files and Tesseract must exist on the target machine; `dist\` holds
all executables and the web UI. The web UI build needs, once:
`rustup target add wasm32-unknown-unknown` and
`cargo install wasm-bindgen-cli --version <wasm-bindgen version in Cargo.lock>`;
`package.ps1` checks the match and uses `wasm-opt` when installed.

### Programs

| Program | Purpose |
|---|---|
| `desktop` | native Iced UI, owns the scanner session |
| `web` | serves the browser client (`www\`) and proxies to the desktop; started/stopped by the desktop |
| `mcp` | stdio MCP bridge to the desktop's MCP server |
| `serve` | standalone browser UI, same engine (`cargo run -p passport-scanner-app --bin passport-scanner-serve`) |
| `scan` | scan once, write images, read document (`-- scans`; `--profile identity-bright`) |
| `read` | read a document from an existing image |
| `calibrate` | measure the platen, keep the result |
| `survey` | save every raw/processed frame per attempt: `survey 20 scans/trial [id1-reference] [profile=identity-bright]`; new output dir per trial; nonzero exit on failed capture/low white signal |
| `dump` | print EEPROM as parsed |
| `lamps` | measure which control key lights which lamp |
| `controls` / `expose` | list / sweep image controls (raw stack / real scan path) |
| `exposure` | read camera-control set (exposure, iris, focus) |
| `probe` | imaging-stage measurements on an image file |
| `time` | time repeated scans, cost of cutting waits |

All are named `passport-scanner-<name>.exe`. **Warning:** `controls`/`expose`
leave the swept control at its last value, and controls persist *in the device*
across processes - an extreme end value silently degrades all later captures.
`serve`/`scan` reset every control they depend on per capture, so one normal
scan fixes it. `PASSPORT_SCANNER_KEEP_RAW` makes `scan` also write
`white-raw.jpg`/`infrared-raw.jpg`.

### Desktop workflow

- **+ New**, **Scan front**, turn card, **Scan back**. Rescanning a side of
  a selected history item replaces that side. Auto-scan alternates front/back (side
  cannot be inferred); press **+ New** if only one face is wanted, or the next card
  becomes the back.
- Once status moves to processing/MRZ, both images are captured - card can be
  moved. Lift it clear long enough for the 200 ms sensor poll; a stationary card
  is not recaptured.
- Dark mode default. **Settings**: language (German/English), light theme,
  auto-scan, calibration, web, MCP. Language and theme
  persist; history/images are memory-only - export PDFs before closing.
- Language: saved choice, else the system language (`de*` → German, `en*` →
  English, anything else German). Desktop keeps it in `ui-settings.json` beside
  `settings.json`; the browser in `localStorage`, per browser.
- `PASSPORT_SCANNER_UI_MOCK=1` runs the desktop against a simulated scanner
  (ICAO 9303 TD1 specimen, drawn images, no PDF export).
- The scanner admits one holder: while desktop/`serve` runs, other tools cannot
  open it. Run only one desktop or `serve` at a time.
- `PASSPORT_SCANNER_RESTORE_SESSION=<json>`: maintenance-only restore of an
  explicit snapshot (`state` + optional `autoScanChanged` event). Not automatic
  persistence.

### Documents, recognition, PDF

- Export requires white front/back + IR front/back; PDF = one image per page in
  that order, original JPEGs embedded without recompression, no silent omission.
- MRZ search: upright → 180° → left → right per source; small-angle deskew on
  failure (§ 7). TD1/TD2/TD3 structure required; field and composite checks
  reported separately; "no MRZ", "failed" and "unverified" are distinct. Best
  verified reading wins, back wins ties. Images survive recognition failure.
- Recognized orientation also rotates preview and PDF (EXIF, § 7).
- PDF names: `YYYY-MM-DD--NNN--HH-MM-SS.pdf` (local time; `NNN` = one past the
  highest number of that day already in the target folder, from 001, never
  overwriting; `ui::pdf_name`). Desktop: optional permanent folder
  (**PDF location**, `pdfFolder` in `ui-settings.json`) saves without a
  dialog; otherwise the save dialog opens in Documents with the generated name.
  Browser: same pattern, day counter continued from the last name in
  `localStorage`.
- History right-click menu: **Save as …** (always a dialog, ignores the
  folder; complete documents only) and **Delete** (`Discard`, removes both sides).

### Document crop

The crop takes the largest bright row band (≥ 1/6 frame width unbroken bright
run), then columns within it. A card lying sideways is narrow on the sensor and,
under white light, its dark portrait can span almost the whole card width: those
rows failed the run test and the portrait was cut off (white only - IR renders
the portrait light; backs have no large dark area). The band is now grown through
contiguous rows that are bright at **both** card edges (window = 1/20 card width,
≥ 10 % bright, gap ≤ 1/100 height), then columns are recomputed. Detached lamp
reflections stay excluded. Real capture: white 1158×1118 → 1751×1119 (IR
1761×1118).

### Web interface & MCP

- **Web**: helper at <http://127.0.0.1:8733>; desktop session API reserves
  `127.0.0.1:8734`. The browser client is the desktop interface itself, compiled
  to WebAssembly (§ 2), so layout, texts and behaviour match. Only `/`,
  `/passport-scanner-webui.js`, `/passport-scanner-webui_bg.wasm` and
  `/favicon.png` are served, from `www\` beside the executable or
  `PASSPORT_SCANNER_WEBUI_DIR`; a missing build shows an explanatory page, and
  the favicon falls back to the embedded `icon.png`. `/?demo` runs the client against the simulated
  scanner. Reconnecting browsers get the same history; a client retries every 3 s
  while the service is unreachable. The Server section (web/MCP switches) exists
  only in the desktop. PDF button = same 4-page export, fetched first so a refusal
  shows its reason, then downloaded.
- **MCP**: read-only Streamable HTTP at <http://127.0.0.1:8735/mcp>, protocol
  `2025-11-25`; `initialize`, `ping`, `tools/list`, plus:

  | Tool | Args | Result |
  |---|---|---|
  | `list_documents` | `{}` | IDs, captured sides, MRZ availability |
  | `get_mrz` | `{"document_id":"d…"}` or `{}` (= newest) | MRZ lines, source side, fields, checks, composite |

  `rawLines` is the grid after conservative filler recovery, not raw Tesseract
  output. Names have no check digit. Cannot scan or change settings. Stdio bridge:
  `{"mcpServers":{"passport-scanner":{"command":"C:\\…\\dist\\tools\\passport-scanner-mcp.exe","args":[]}}}`
  (desktop must run with MCP on). POSTs after init need `MCP-Protocol-Version`
  and `Content-Type: application/json`; notifications → 202; no GET streaming.
  All listeners loopback-only, reject non-local Origin/Host, upload nothing.

### Capture profiles

| Profile | Sensor/lamp | Shading |
|---|---|---|
| `general` (default) | A1 white / A2 IR, General AWB, trims 3FFC/3FF8 | `Calibration_General.shd` |
| `identity` | A3 + ID AWB + 3FEC, bridge ID light | `Calibration_General_ID.shd` |
| `identity-bright` | as identity, white trim ×1.2 rounded once (43→52), non-compounding | same |

The desktop app and `serve` always capture with `general`; the interface offers
no choice. The other profiles remain for the diagnostic tools (`scan --profile`,
`survey profile=…`). Calibration is validated before activation; missing ID
calibration refuses the switch - General shading is never substituted. Changes
run between captures; while disconnected the request is pending and saved only
after validation. Own calibration (measure/delete) only under General. Missing
MRZ on an ID front is neither a failure nor an OCR success.

### Calibration

The lamp lights the platen unevenly (×3–5 across the frame); a per-pixel map
corrects every capture. Vendor measurement is used where installed. Own
measurement: blank white paper, **Measure now** or `calibrate`; non-blank captures
are refused with the reason. Written as `Calibration_PassportScanner_v2.shd`,
preferred over (never overwriting) the vendor file; **Use the vendor's** discards
it. Legacy `Calibration_PassportScanner.shd` is ignored (wrong row direction,
false prologue, different profile). Never pair ID shading with General
exposure/AWB/tables. Visual review still required - aspect/brightness metrics do
not certify colour or legibility.

### Timings & vendor config

Settling/lamp/control values come from the vendor's `Camera.ini` (model section
shown in settings, else `built-in defaults`). Editable for the current run:

| Setting | Source | Default |
|---|---|---|
| Wait after document placed | `FirstDelay` | 500 ms |
| Wait after lamp switch | `DelayTime-5M-White` | 400 ms |
| Beep on completion | - (ignores INI) | off |

Edit `Camera.ini` for permanent changes.

### Known limits

- Linux transport not written (`VendorTransport` seam exists; V4L2 capture works).
- Flat-field needs a matching profile map; no IR SHD on the reference unit - IR is
  grayscale with one contrast curve, softer and less even than white.
- MRZ only; full-page OCR, PDF417, NFC would sit behind `DocumentRecognizer`.

---

## 2. Wire contract (`crates/passport-scanner-protocol`)

The UI never calls its host; it exchanges commands/events, so the SDK surface
equals the UI surface. One interface crate serves both clients:

| Crate | Role | Target |
|---|---|---|
| `passport-scanner-ui` | Iced state/update/view, theme, i18n catalog, `Backend` trait, simulated `Mock` backend | native + wasm32 |
| `passport-scanner-desktop` | window; `HostBackend` runs the session in-process; `rfd` save dialog; file preferences | native |
| `passport-scanner-webui` | browser entry; `Http` backend (`EventSource("/events")`, `fetch POST /command`), `localStorage` preferences | wasm32 |

`Backend`: `connect`, `connected`, `send(Command)`, `poll() -> Inbound`
(event / unreadable / connection up-down), `export_pdf`, `copy_text`, optional
`services()` (web/MCP, native only). The interface polls every 100 ms, so its
reducer is identical on both targets. Interface text lives in a typed catalog
(`i18n::Strings`, `DE`/`EN`): a missing translation does not compile. Service
failures are labelled by `Failed.code` with the service's English detail kept;
unknown codes show the raw message. Browsers expose no system fonts to the
canvas, so Fira Mono is embedded for the MRZ.

- JSON tagged by one key: `{"command":"accept","id":"s1"}`,
  `{"event":"scanCompleted","scan":{…}}`. Enums `#[non_exhaustive]`; session
  opens with `{"event":"hello","version":1}`; unknown variants are reported, not
  mis-parsed.
- `Image` carries a `url` (small `data:` URL or host-served address), never bytes.
- `Field.span` = line/char range of provenance.
- On connect, `/events` (and the in-process subscription) first replays hello,
  state, settings, auto-scan, calibration and progress; a reconnect needs no
  extra request. Accepted HTTP connections are switched back to blocking mode:
  Windows hands them out non-blocking, and a multi-megabyte state event otherwise
  failed with `WSAEWOULDBLOCK` as soon as a browser read slower than it was written.

---

## 3. Device & transport

**Target:** Plustek X-Mini 2000, `USB\VID_07B3&PID_1A05`, serial `KJ12040G6002758`.
Evidence: Ghidra 12.1.4 / REA 6.5.0 decompilation of SecureScan V6.4.7.1
(`Camera.dll` `e99989a2…0d4eaa`, `etLib2.dll` `81f1394f…a23719`, `AVILib.dll`
`4c81fb7f…182231`), `Camera.ini`, and Frida traces of `SecureScan.exe` on
2026-10-16 (16 042 `DeviceIoControl`, 930 etLib2 calls). Static findings describe
shipped code, not necessarily what a given scan executes.

- PnP exposes only `MI_00` (Camera); **no HID** (HID imports in etLib2 unused -
  zero HID calls observed). `SupportSource=3` white+IR, no UV. MJPEG
  (`MJPG=1`, `SingleStream=1`), `Rotate180=1`, `StillPin=0` (app polls sensors;
  frames come from the running stream, the still pin is never used on this model).
- Bridge: code says eMPIA EM27595, EEPROM tag at `0x0000` says **`Aveo`**.
- Sensor: OmniVision OV5640-family register map, 5 MP (2592×1944).
- `Camera.dll` → `etLib2.dll` (13 entry points), device by **index** (0 = this
  unit; index 1 → `0x80004005`). Capture via DirectShow (`DXShow->`); Media
  Foundation only in the unrelated AVIMRZ `CameraApi.dll`.

### Extension Unit

```
KSPROPSETID {AAB8F252-8ED1-4972-8CED-96B17F04408B}, PropertyId 2, NodeId 3
Flags 0x10000001 GET|TOPOLOGY, 0x10000002 SET|TOPOLOGY
IOCTL_KS_PROPERTY 0x002F0003, input KSP_NODE (32 B), data buffer 35 B
```

Linux equivalent: `UVCIOC_CTRL_QUERY` on the XU with same GUID/selector. The
other UVC subtype uses a different GUID - don't substitute.

`etLib2_Custom_CMD_{Read,Write}(int dev, int cmd, int len, void *buf)` - dev 0,
len 32, returns 0 / `E_FAIL`. `Camera.dll` wraps them as hex-string helpers
`SetExtension` (`0x180032f80`) / `GetExtension` (`0x180032e50`, returns `buf[0..3]`
big-endian). The "selector" string is **not transmitted**; it chooses framing. On
the wire byte 0 is an opcode, buffers are zero-padded to 35 B:

| Selector | Write | Read |
|---|---|---|
| `F0` control | one SET of the payload (`<key> <value>`) | SET payload, wait 5 ms, GET |
| `00` image sensor | SET `[00,addr_lo,addr_hi]`, SET `[02,val_lo,val_hi]` | SET `[00,addr_lo,addr_hi]`, 5 ms, GET |
| `01` bridge regs | SET `[01,reg_lo,reg_hi]`, SET `[03,val_lo,val_hi]` | SET `[01,…]`, GET; reply bytes 1–2 = LE value |
| `EE` EEPROM | - | SET `[04,addr_lo,addr_hi]`, GET → `04` + run of bytes from addr |

**Every read is SET-then-GET**; a lone GET returns the previous exchange's stale
data. SET/GET each retry once after 50 ms. Example: `3501=002B` → `00 01 35 …`
then `02 2B 00 …`. Vendor reads EEPROM one byte per exchange; one exchange can
fetch a whole block. Chain: `Custom_CMD_Write` → `0x1800012d0` → `0x18000b350` →
`0x18000ab30` → `0x180010400`; bridge via `etLib2_RegWrite/RegRead_DevNum`
(`0x18000FC20` / `0x18000FB20`). Evidence: decompiled `etLib2.dll` (see § 3).

### Control keys (selector F0) - direction-dependent

Probe of `E0`–`FF`: only `F1`–`F7` implemented (`<key> 00 00 00`), rest
`ff ff ff ff`. `FB` (written by init) reads as unimplemented.

| Key | Write | Read |
|---|---|---|
| `F1` | General **white lamp** `01`/`00` | **document sensor A** (bit 0 of value byte) |
| `F2` | **IR lamp** `01`/`00` | **document sensor B** |
| `F3` | buzzer `01`/`00` (`BuzzerDelay=300` ms) | state |
| `F4`, `F5` | unknown, not needed | readable |
| `F6` | detection lamp, **inverted**: `01` off, `00` on - off during capture | readable |
| `F7` | init writes `01` | readable |

Document detection: `a = (Get(f0,f1)>>16)&1`, `b = (Get(f0,f2)>>16)&1` →
0 none / 1 A / 2 B / 3 both. Bridge **register 7 bit `0x40`** = ID light
(`etLib_LampUV`, `0x180033FA0`) - not a no-op; clearing F1/F2 without it gives a
black ID capture.

Lamp helpers: General white `0x180033b30` → (`F200` if from IR) `F101`; IR
`0x180033d80` → (`F100` if from white) `F201`; ID white → clear F1/F2, reg7
`|=0x40`; `LampCloseAll` `0x1800341C0` → `F100`, `F200`, reg7 `&=0xBF`. General
white also clears reg7 bit 6 whenever `IDLight=1` (handles inherited external ID
light). White/IR/ID are mutually exclusive; `ExposureOrder=1` = white first.

Legacy models without etLibEx: reg `0x83` bit 1 white, bit 2 IR, reg `0x80`
bit 6 cleared first, bit 3 buzzer; sensors via reg `0x84` bits 0/2. Irrelevant here.

### Capture sequence

Vendor General capture (`0x180014b80`): select lamp → optional sensor gamma/SDE
(disabled here) → EGW (6 General AWB pairs, 29-entry table, trim) → wait
`DelayTime-5M-*` (400 ms) → take next stream frame → afterwards `F600`,
LampCloseAll, buzzer ~300 ms. Observed ID trace (2026-10-16): register writes
finished at +300 / +1238 ms, capture armed at +700 / +1639 (400/401 ms later),
frame delivered ~120 ms after arming.

**This implementation deviates:** a fixed delay was unreliable (stale previous-lamp
frames even after discarding 2–4 frames; truncated JPEGs below 200 ms). Instead a
frame is accepted when its JPEG size differs from the pre-switch frame (~25 %
between lamps vs. 1–2 % frame-to-frame) **and** has stopped changing - no
decoding. Swept 400/100/50/0 ms: identical results, only duration changed;
internal delay kept at 50 ms so the lamp is physically up.

### Image controls

Stack exposes Brightness, Contrast, Hue, Saturation, Sharpness, Gamma,
WhiteBalance - no generic exposure/gain. `CmdEGW=1` selects **sensor** EGW
programming *instead of* the DirectShow `CameraProp` path (`0x1800108e0`); the two
are exclusive. `CameraProp-5M-*` first seven fields are auto(1)/manual(0) flags,
not enables. Stream startup resets the sensor, so tables must be written after
startup frames. The Rust path restores driver-reported native controls before
writing sensor tables (a reproducibility choice, not an observed vendor reset).
Exposure writes **are effective** with correct framing (exposure 8/16/43 differ
visibly). `SensorGamma`/`SensorSDE` first value is an enable flag → both disabled;
`EnableGamma`/`EnableColorProfile` default 0, so the bundled `.icm` is not active.

German ID cards: personal data is laser-engraved - low-contrast grey under white,
black under IR (carbon absorbs). The recogniser therefore prefers IR; white keeps
headings, photo and security features.

---

## 4. EEPROM

Live data across `0x0000–0x8100`; only the calibration area is needed (rest likely
UVC descriptors, bridge config, a token near `0x8000`).

### Calibration words `0x3FB8–0x3FFC`

| Address | Value (this unit) | Meaning |
|---|---|---|
| `0x3FB8`–`0x3FCC` | `3405 0025 … 3400 0004` | **ID** AWB (6 pairs, read by `etLib_ReadIDAWB` `0x180036030`) |
| `0x3FD0`–`0x3FE4` | `3405 0009 … 3400 0004` | **General** AWB (`etLib_ReadAWB` `0x180035d30`), written 3FE4→3FD0 before **both** white and IR tables |
| `0x3FE8` | `FFFFFFFF` | erased |
| `0x3FEC` | `3501 002B` | ID white exposure trim |
| `0x3FF0`/`0x3FF4` | `3F3A555A` `29524D84` | calibration stamp = `Stamp.ini` `Stamp=3f3a555a29524d84`; `Camera_VerifyCalibration` discards `.shd` of another unit |
| `0x3FF8` | `350B 0040` | IR gain trim |
| `0x3FFC` | `3501 0045` | General white exposure trim |

### Sensor tables `0x4000–0x4247`

`"CMOS"` + 8-byte header, then 4-byte big-endian `RRRRVVVV` entries; each table
ends with `Ax 00 00 00` + lead-in `74`. Parser `0x180028950`:

| Offset | Group | Entries | Use |
|---|---|---|---|
| `0x400D` | A0 | 52 | common init |
| `0x40E2` | A1 | 29 | **General white** (`350B=0014`) |
| `0x415B` | A2 | 29 | **IR** (`3402=0002, 3501=0080, 350B=0040`) |
| `0x41D4` | A3 | 29 | **ID white** (`5680=0003 … 350B=0015`) |

29-entry shape: 8× AE window `0x5680–87`, 18× lens shading (LENC) `0x5807–1D`,
`0x3402` AWB, `0x3501` exposure, `0x350B` gain. **`EEPROMShading=1`** = these
per-source LENC coefficients applied in-sensor; the `.shd` map is a second,
software correction on top. Always replay EEPROM rather than INI defaults
(INI `CmdWhiteEGW 3402→04, 350B→10, 3501→30`; `CmdIREGW 3402→02, 350B→40,
3501→80`; EEPROM gives e.g. exposure `0x2B` vs INI `0x15`).

| Capture | Table | AWB | Trim | Shading |
|---|---|---|---|---|
| General white | A1 | General | 3FFC | `Calibration_General.shd` |
| General IR | A2 | General | 3FF8 | General EEPROM shading, IR mode |
| ID white | A3 | ID | 3FEC | `Calibration_General_ID.shd` |

Vendor shading selector `0x18001ff10` picks the ID file only when `IDLight==1`
*and* ID light is active; `IDCalibrationSource=0` does not select General for ID.
Profile selection reads A3/ID AWB/3FEC atomically, never writes EEPROM, and a
failure keeps the previous profile. Evidence:
decompiled `Camera.dll` (see § 3).

---

## 5. Shading (`.shd`)

Location: `C:\Users\Public\Documents\Plustek\VID_07B3&PID_1A05\` -
`Calibration_General{,_ID}.{jpg,shd}` (JPEG 648×486 white reference; SHD
3 779 208 B), `Stamp.ini`. Vendor default calibration for X-Mini 2000 writes the
factory JPEGs from EEPROM (`0x180029140` / `0x1800292b0`) rather than shooting a
new sheet.

```c
struct ShadingFile {                // headerless little-endian float32
    float32 gain[486][648][3];      // from byte 0, BOTTOM-UP rows, interleaved BGR
    float32 geometry[18];           // TRAILER: crop rects + resize factors
};                                  // 648×486 = 2592×1944 / 4
```

General trailer starts `[0, 39, 2517, 1918]`, ends with two ~0.98 resize factors
+ same rect; ID starts `[831, 39, 2516, 1102]`. Gains: General 1–4.2105, ID
1–5.1064. Loader/saver converts rows and channels to top-down RGB and preserves
the trailer byte-identically; new own files use whole-frame geometry.

**Gain semantics:** `gain = TARGET / white_ref` (white_ref 8-bit integer; `240/gain`
integral in 100.00 % of 25 535 samples, implied ref 65–222). `AVILib!Apply`
picks TARGET by filename: white/general 240 (mode 0, 3 = ID), UV 240 (2), IR 215
(1). File source = selector 3, EEPROM source = 4 (`ApplyEEPROMXMini2000`, modes
6/7 white, 8/9 IR/UV). Actual apply (`IL_ShadingDataApplyEx` `0x18019e700`):
`out = min(255, round(raw × gain))`, **no black-point subtraction**,
nearest-block lookup (`x/4, y/4`, clamped), not bilinear; geometric correction
only for EEPROM modes 4–9.

### Audit (2026-10-17, retained files only)

Orientation check of `240/gain` vs vendor JPEG:

| File | Rows | MAE | Corr. |
|---|---|---:|---:|
| General | top-down ✗ | 10.901 | 0.884 |
| General | bottom-up ✓ | 5.674 | 0.956 |
| ID | top-down ✗ | 68.375 | −0.418 |
| ID | bottom-up ✓ | 9.029 | 0.917 |

Flips and RGB order fit worse. Both JPEGs carry a rectangular overlay, so they are
not valid calibration inputs. `reference_check` rejects: vendor General JPEG
(annotated edges), vendor ID JPEG (25 % dark), card raw (68 % dark), empty platen.
A printed card must never be used as calibration target.

The large top/bottom gradient was not a layout issue nor fixable by global
exposure (exposure 69 clipped the bottom while the top stayed dark). Restoring
the **complete factory General sensor profile** fixed it:

| Metric | Partial profile | Factory profile |
|---|---:|---:|
| Top paper RGB p90 | 82/113/107 | 189/197/203 |
| Bottom paper RGB p90 | 171/230/218 | 194/202/212 |
| Top–bottom intensity diff | 105.7 | 6.3 |
| Portrait pixels all ≥250 | 77.5 % | 0.0 % |

Result is still darker than an external reference photo (different card, not a
target); no evidence justifies per-channel stretch, arbitrary gamma or forcing
background to 255.

---

## 6. Open questions

- Runtime comparison of vendor output vs. Rust captures; static analysis cannot
  prove 20-scan reliability or colour match.
- Vendor's baseline ProcAmp values after other apps changed them (no reset seen in
  `Camera.dll`; driver behaviour unknown).
- Meaning of sensor read-back bytes needs hardware corroboration.
- Fresh white target needed to separate reflectance from residual exposure mismatch.

---

## 7. Recognition: orientation & deskew

### Automatic orientation

`Engine::orientation(&GrayImage) -> Result<Option<i16>>` → clockwise correction
0/90/180/−90; custom engines default to `None`. Verified MRZ orientation wins.
Otherwise on a ≤1400 px working copy: Tesseract OSD (psm 0), accepted only at
confidence ≥ 15; else sparse-text OCR (psm 11) at all four angles (2 workers),
requiring ≥ 4 confident words, ≥ 20 alphanumerics and absolute + relative margin
over the runner-up. Ambiguous/sparse → no rotation. Heuristic thresholds, not
calibrated probabilities. (OSD alone failed on the German ID front: conf 0.7–2.4,
and wrongly 180° at 1000 px; sparse text: 31 words upright vs 0–1.)

Applied to white and IR as JPEG EXIF orientation (no pixel recompression); Iced
and web preview honour it, PDF uses the matching placement matrix; sides are
independent; already-correct images aren't re-rotated.

Validation 2026-10-17: pilot front + 3 repeated captures × 4 rotations = all
correct (same document, not a diversity study); blank abstains; standalone
937–1116 ms. One real back: recognition, EXIF and PDF correct from all four
starts, PDFs verified with PyMuPDF (byte-exact JPEGs). Native tests use Iced's
EXIF loader with asymmetric fixtures. Suite 340 passed, Clippy/fmt clean.

Limits: English traineddata only; sparse/decorative documents may stay
unrotated; OCR subprocesses have no timeout; a crash can leave temp files. All
local (`CREATE_NO_WINDOW`), no uploads. Local test:
`PASSPORT_SCANNER_ORIENTATION_IMAGE=<abs path> cargo test --release -p passport-scanner-recognize local_front_all_four_rotations -- --ignored --nocapture`.

### Small-angle MRZ deskew

Only after the normal 4-orientation pass fails verification: estimate text-line
slope in a full-width strip (needs multiple rows and spread contrast; blanks and
platen edges insufficient), bounded ±5°. Two retries on an expanded rotation
canvas at native resolution - lower third, then a taller bottom band (2× upscale
and the old side inset hurt). OCR inputs only; stored JPEGs/PDF untouched; all
checks and cross-source conflict rejection still apply.

Validation 2026-10-17 (offline replays of one card,
[mrz-deskew-validation.json](../evidence/mrz-deskew-validation.json)): ±1° × 4 orientations
8/8; 21/22 tilted variants verified identically (incl. 6 held-out angles); the
held-out −2.5° stays unverified (angle estimated correctly, OCR fails) - the ±5°
bound is not a recognition guarantee. Front and blank negatives: no fields.
1.69–3.18 s per tilted replay (uncontrolled). Suite 348 passed. Reproduce:
`cargo run --release -p passport-scanner-recognize --example review_skew -- baseline-white.png tilted-white.png`
(each needs a `-infrared.png` sibling; real images stay in ignored `scans/`).

### MRZ reliability fixes (2026-10-17)

Failures on real German ID backs, and their fixes:

1. **Clipped edge.** The normal strip insets 1/40 per side; an off-centre card
   put the composite check digit inside the inset. The full-width retry existed
   but only ran with measurable skew. Now, when the first pass plainly shows a
   zone (`text::looks_like_zone`: ≥ 2 rows of mostly zone alphabet with `<<`),
   the full-width lower third is read before deskew. Card fronts without `<<`
   pay no extra OCR call.
2. **Filler misreads.** Tesseract reads `<` runs as K, E, C, S, G, Q, O, 0, 6, 8,
   X and drops or adds fillers. TD1 recovery (only for grids that do not verify
   as read): digit look-alikes repaired at digit positions only (Z→2, O/Q/D→0,
   S→5, G→6, B→8, I/L→1, each covered by a check digit); optional data kept
   verbatim up to the first `<<`, after which only fillers and their look-alikes
   may follow; name tail likewise after the padding run; rows from 18 chars.
   All individual and the composite check must verify afterwards. Document
   numbers and names are never edited.
3. **Cross-source conflicts.** Two verified readings differing only where one
   saw `<` and the other a look-alike are merged to fillers. Two readings that
   agree on every individually checked field and the name but differ in what only
   the composite digit covers (a checksum collision, e.g. `G` + wrong composite)
   resolve to the reading with fewer optional letters - the only error direction
   observed. Name conflicts stay ambiguous.
4. **German ID optional data** (TD1, `I…`, issuer `D`) is empty by
   specification, so look-alikes there are cleaned even though `K` weighs like a
   filler and the composite cannot tell.

Real captures, same card, release replay: 18/18 MRZ sides verified with identical
fields and no spurious optional data (previous build: 15/18; 10/12 on a 12-scan
series of one placement). 14 sides without MRZ (fronts, other card, blank
platen): no fields. Timings unchanged for fronts (~2.2–2.7 s), backs 1.7–2.9 s.

---

## 8. Recognition performance

White and IR OCR run concurrently (max two Tesseract processes, Tesseract
opt-in; custom engines stay sequential). Each source keeps its full
upright/180/left/right fallback; all results take part in conflict detection.
Release replays, 2026-10-17, recognition only ([mrz-performance.json](../evidence/mrz-performance.json),
[auto-rotation-performance.json](../evidence/auto-rotation-performance.json)):

| Sample | Sequential (ms) | Concurrent (ms) | + orientation fallback (ms) |
|---|---|---|---|
| Front, no MRZ | 2542 / 2529 / 2523 | 1390 / 1402 / 1417 (−44.6 %) | 2364 / 2341 / 2344 |
| Back, verified | 1786 / 1786 / 1789 | 1048 / 1046 / 1041 (−41.4 %) | 1044 / 1048 / 1052 (fallback not invoked) |

Single OCR calls 160–440 ms; decode/prep ~0.2 s per source; front uses 8 calls,
back 6. Temp PNGs use exclusive creation + auto cleanup (fixes a shared-file
race), flushed before OCR; worker panics → errors. Rejected: stdin PGM (front
~3.59 s), PGM files (slower), `OMP_THREAD_LIMIT=1` (no gain). Benchmark:
`cargo run --release -p passport-scanner-recognize --example benchmark_mrz -- <white> <infrared>`
(`MRZ_BENCH_SEQUENTIAL` for sequential; now includes orientation fallback).
Skipping OCR for known German ID fronts would have to stay opt-in - passport
fronts can carry an MRZ.

---

## 9. Corrected earlier claims

| Earlier claim | Correct |
|---|---|
| `F6` is the white lamp; no "white on" command exists | `F6` = detection lamp (inverted). General white = `F101`; the recorded trace was an **ID** capture lit via bridge reg 7 bit 6 |
| `etLib_LampUV` is a no-op | It sets bridge reg 7 `0x40` (ID light) |
| AWB `0x3FB8–CC` white, `0x3FD0–E4` IR | ID AWB vs. General AWB (General used for both white and IR) |
| Table 1 = ID variant, table 3 = white | A1 = General white, A2 = IR, A3 = ID white |
| Selector string is sent on the wire; one GET reads | Opcode in byte 0; every read is SET then GET |
| Exposure writes have no effect | They work with correct two-packet sensor framing |
| `.shd` has an 18-float prologue, top-down rows | Gains from byte 0, bottom-up BGR, 18-float **trailer** |
| `CameraProp` 0/1 = skip/override, applied with EGW | auto/manual flags; path exclusive with sensor EGW |
| Still pin triggers capture | Frames come from the MJPEG stream |
| Bridge is eMPIA | EEPROM tag says Aveo |
| Fixed 400 ms delay is sufficient | Content-change detection needed in this stack |
