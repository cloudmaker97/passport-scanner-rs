"""Renders the demo cards, writes a demo session, and captures the desktop app.

Needs Python with Pillow, Google Chrome, and the desktop app built with
    cargo build --release -p passport-scanner-desktop --features render-smoke
Usage: python -I scripts/screenshots/build.py [output folder]
"""
import base64
import io
import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from cards import W, H, back, front, page, td1  # noqa: E402
from people import PEOPLE  # noqa: E402
from PIL import Image  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
OUT = pathlib.Path(ARGS[0]) if ARGS else ROOT / "target" / "screenshots"
CHROME = os.environ.get("CHROME", r"C:\Program Files\Google\Chrome\Application\chrome.exe")
APP = ROOT / "target" / "release" / ("passport-scanner-desktop" + (".exe" if os.name == "nt" else ""))
FONT = (ROOT / "crates" / "passport-scanner-ui" / "assets" / "FiraMono-Medium.ttf").as_uri()


def chrome_png(html, path, width, height):
    source = path.with_suffix(".html")
    source.write_text(html, encoding="utf-8")
    subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                    "--force-device-scale-factor=1", f"--window-size={width},{height}",
                    "--default-background-color=00000000",
                    f"--screenshot={path}", source.as_uri()], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def data_url(path):
    buffer = io.BytesIO()
    Image.open(path).convert("RGB").save(buffer, "JPEG", quality=90)
    return "data:image/jpeg;base64," + base64.b64encode(buffer.getvalue()).decode()


def iso(yymmdd):
    year = int(yymmdd[:2])
    year += 1900 if year > 40 else 2000
    return f"{year}-{yymmdd[2:4]}-{yymmdd[4:]}"


def scans():
    cards = OUT / "cards"
    cards.mkdir(parents=True, exist_ok=True)
    result = []
    for n, p in enumerate(PEOPLE, 1):
        mrz = td1(p["doc"], p["birth"], p["sex"], p["expiry"], p["mrz_surname"], p["given"])
        faces = {"front": front(p, mrz), "back": back(p, mrz)}
        for side, body in faces.items():
            images = []
            for source, infrared in (("white", False), ("infrared", True)):
                png = cards / f"{p['doc']}-{side}-{source}.png"
                chrome_png(page(body, FONT, infrared), png, W, H)
                images.append({"source": source, "url": data_url(png), "width": W, "height": H})
            fields = []
            if side == "back":
                field = lambda kind, value, check=None: {  # noqa: E731
                    "kind": kind, "value": value, "check": check, "span": None}
                fields = [
                    field("documentType", "I"), field("issuingState", "UTO"),
                    field("documentNumber", p["doc"], True),
                    field("dateOfBirth", iso(p["birth"]), True), field("sex", p["sex"]),
                    field("dateOfExpiry", iso(p["expiry"]), True), field("nationality", "UTO"),
                    field("surname", p["mrz_surname"]), field("givenName", p["given"]),
                ]
            result.append({
                "id": f"s{n}{side[0]}", "document": f"d{n}", "side": side, "at": p["at"],
                "format": "TD1" if side == "back" else None,
                "mrz": mrz if side == "back" else [],
                "mrzCheck": True if side == "back" else None,
                "fields": fields, "images": images,
            })
    # Newest first, back after front within a document.
    return list(reversed(result))


def session(all_scans, selected):
    return [
        {"event": "hello", "version": 1},
        {"event": "state",
         "device": {"model": "X-Mini 2000", "vendorId": 0x07B3, "productId": 0x2003,
                    "serial": None, "sources": ["white", "infrared"]},
         "scans": all_scans, "selected": selected},
        {"event": "settingsChanged", "settings": {
            "captureProfile": "general", "profileReady": "true", "buzzer": "on"}},
        {"event": "calibrationChanged", "source": "Factory", "range": None},
        {"event": "autoScanChanged", "enabled": True},
    ]


SHOTS = [
    # name, selected scan, theme, locale, view
    ("front-dark-en", "s3f", "dark", "en", ""),
    ("back-dark-en", "s3b", "dark", "en", ""),
    ("front-light-en", "s2f", "light", "en", ""),
    ("back-light-en", "s1b", "light", "en", ""),
    ("back-dark-de", "s2b", "dark", "de", ""),
    ("settings-dark-en", "s3b", "dark", "en", "settings"),
]


def capture(all_scans):
    shots = OUT / "app"
    shots.mkdir(parents=True, exist_ok=True)
    for name, selected, theme, locale, view in SHOTS:
        state = OUT / f"session-{name}.json"
        state.write_text(json.dumps(session(all_scans, selected)), encoding="utf-8")
        env = dict(os.environ,
                   PASSPORT_SCANNER_UI_SMOKE_SESSION=str(state),
                   PASSPORT_SCANNER_UI_SMOKE_SCREENSHOT=str(shots / f"{name}.png"),
                   PASSPORT_SCANNER_UI_SMOKE_THEME=theme,
                   PASSPORT_SCANNER_UI_SMOKE_LOCALE=locale,
                   PASSPORT_SCANNER_UI_SMOKE_VIEW=view,
                   PASSPORT_SCANNER_UI_SMOKE_CLOSE="1")
        subprocess.run([str(APP)], env=env, check=True, timeout=120)
        print("captured", name)


if __name__ == "__main__":
    all_scans = scans()
    if "--cards-only" not in sys.argv:
        capture(all_scans)
