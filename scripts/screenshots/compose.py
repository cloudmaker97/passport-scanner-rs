"""Frames the captured app screenshots into README images.

Run build.py first. Usage: python -I scripts/screenshots/compose.py [output folder]
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from build import OUT, ROOT, chrome_png  # noqa: E402

DEST = ROOT / "docs" / "repository" / "screenshots"

STYLE = """
* { box-sizing: border-box; }
html, body { margin: 0; width: %(w)dpx; height: %(h)dpx; overflow: hidden; }
body { font-family: "Segoe UI", "DejaVu Sans", sans-serif; color: #eeede5;
  background: radial-gradient(1200px 700px at 78%% 8%%, #2f4c55 0%%, transparent 60%%),
              radial-gradient(900px 600px at 0%% 100%%, #3a3346 0%%, transparent 60%%), #1b1d20; }
body.light { color: #1f2328;
  background: radial-gradient(1200px 700px at 80%% 0%%, #d6ecef 0%%, transparent 60%%),
              radial-gradient(900px 600px at 0%% 100%%, #ece6f5 0%%, transparent 60%%), #f4f3ee; }
.grid { position: absolute; inset: 0; opacity: .5;
  background-image: linear-gradient(rgba(255,255,255,.035) 1px, transparent 1px),
                    linear-gradient(90deg, rgba(255,255,255,.035) 1px, transparent 1px);
  background-size: 40px 40px; }
body.light .grid { background-image: linear-gradient(rgba(0,0,0,.035) 1px, transparent 1px),
                    linear-gradient(90deg, rgba(0,0,0,.035) 1px, transparent 1px); }
.window { position: absolute; border-radius: 14px; overflow: hidden;
  box-shadow: 0 40px 90px rgba(0,0,0,.55), 0 0 0 1px rgba(255,255,255,.08); background: #111; }
body.light .window { box-shadow: 0 40px 90px rgba(30,40,60,.25), 0 0 0 1px rgba(0,0,0,.08); }
.bar { height: 34px; display: flex; align-items: center; gap: 8px; padding: 0 14px;
  background: #2a2d31; font-size: 13px; color: #a9ada9; }
body.light .bar { background: #e6e5df; color: #555; }
.dot { width: 12px; height: 12px; border-radius: 50%%; }
.bar .title { margin-left: 10px; }
.window img { display: block; }
h1 { position: absolute; margin: 0; font-size: 56px; letter-spacing: -1.5px; line-height: 1.05; font-weight: 700; }
p.sub { position: absolute; margin: 0; font-size: 22px; line-height: 1.45; color: #bfc3bf; }
body.light p.sub { color: #4a4f55; }
.tags { position: absolute; display: flex; gap: 10px; flex-wrap: wrap; }
.tag { font-size: 15px; padding: 7px 14px; border-radius: 999px; border: 1px solid rgba(238,237,229,.22);
  background: rgba(255,255,255,.04); }
body.light .tag { border-color: rgba(0,0,0,.15); background: rgba(255,255,255,.6); }
.card { position: absolute; border-radius: 18px; overflow: hidden;
  box-shadow: 0 30px 70px rgba(0,0,0,.5), 0 0 0 1px rgba(255,255,255,.1); }
.card img { display: block; width: 100%%; }
.pill { position: absolute; display: flex; align-items: center; gap: 10px; padding: 12px 18px;
  border-radius: 12px; font-size: 18px; font-weight: 600; background: #16301f; color: #7ee2a0;
  border: 1px solid #2f6a44; box-shadow: 0 16px 40px rgba(0,0,0,.45); }
.label { position: absolute; font-size: 15px; letter-spacing: 2px; text-transform: uppercase; color: #a9ada9; }
"""


def window(src, x, y, width, title="Passport Scanner"):
    return (f'<div class="window" style="left:{x}px;top:{y}px;width:{width}px">'
            '<div class="bar"><span class="dot" style="background:#ff5f57"></span>'
            '<span class="dot" style="background:#febc2e"></span>'
            '<span class="dot" style="background:#28c840"></span>'
            f'<span class="title">{title}</span></div>'
            f'<img src="{src}" style="width:{width}px"></div>')


def html(w, h, body, light=False):
    return (f'<!doctype html><html><head><meta charset="utf-8"><style>{STYLE % dict(w=w, h=h)}'
            f'</style></head><body class="{"light" if light else ""}"><div class="grid"></div>'
            f'{body}</body></html>')


def uri(path):
    return path.resolve().as_uri()


def compose():
    app, cards = OUT / "app", OUT / "cards"
    DEST.mkdir(parents=True, exist_ok=True)
    check = ('<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" '
             'stroke-width="3" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12l5 5L20 6"/></svg>')
    shots = {
        "hero": (1800, 1000, False, (
            '<h1 style="left:90px;top:120px">Scan. Read.<br>Verify.</h1>'
            '<p class="sub" style="left:92px;top:285px;width:430px">Capture both sides of an ID card '
            'under white and infrared light, read the MRZ and check every digit — on Windows, '
            'macOS and Linux.</p>'
            '<div class="tags" style="left:92px;top:470px;width:440px">'
            '<span class="tag">ICAO 9303</span><span class="tag">TD1 · TD2 · TD3</span>'
            '<span class="tag">White + IR</span><span class="tag">PDF export</span>'
            '<span class="tag">Local web + MCP</span><span class="tag">Rust</span></div>'
            + window(uri(app / "front-dark-en.png"), 600, 120, 1120)
            + f'<div class="card" style="left:120px;top:680px;width:430px;transform:rotate(-5deg)">'
              f'<img src="{uri(cards / "L3KP8V1W6-back-white.png")}"></div>'
            + f'<div class="pill" style="left:92px;top:575px">{check} MRZ verified · 3/3 checks</div>'
        )),
        "light": (1800, 1260, True, (
            '<h1 style="left:90px;top:70px;font-size:46px">Every field, straight from the MRZ</h1>'
            '<p class="sub" style="left:92px;top:136px">Check digits verified, values normalised, '
            'ready to copy or export.</p>'
            + window(uri(app / "back-light-en.png"), 180, 220, 1440)
        )),
        "infrared": (1800, 900, False, (
            '<h1 style="left:90px;top:70px;font-size:46px">White light and infrared, every side</h1>'
            '<p class="sub" style="left:92px;top:136px">Security inks drop out under IR; '
            'photo, data and MRZ stay. Both images are kept with the scan.</p>'
            '<div class="label" style="left:110px;top:240px">White light</div>'
            f'<div class="card" style="left:110px;top:275px;width:760px"><img src="{uri(cards / "T7RX4M2N9-front-white.png")}"></div>'
            '<div class="label" style="left:930px;top:240px">Infrared</div>'
            f'<div class="card" style="left:930px;top:275px;width:760px"><img src="{uri(cards / "T7RX4M2N9-front-infrared.png")}"></div>'
        )),
    }
    for name, (w, h, light, body) in shots.items():
        path = DEST / f"{name}.png"
        chrome_png(html(w, h, body, light), OUT / f"compose-{name}.png", w, h)
        (OUT / f"compose-{name}.png").replace(path)
        print("wrote", path.relative_to(ROOT))
    for leftover in DEST.glob("*.html"):
        leftover.unlink()


if __name__ == "__main__":
    compose()
