"""Draws the fictitious Utopia identity cards used for the README screenshots.

Utopia (UTO) is the imaginary state ICAO 9303 uses for its specimens. Every
holder, number and portrait here is invented; the portraits are vector
drawings, never photographs.
"""
import math
import random

W, H = 1200, 757  # ID-1, 85.6 x 54 mm


def check_digit(text):
    def value(c):
        if c.isdigit():
            return int(c)
        if c.isalpha():
            return ord(c) - ord("A") + 10
        return 0

    return str(sum(value(c) * w for c, w in zip(text, [7, 3, 1] * 40)) % 10)


def td1(doc, birth, sex, expiry, surname, given):
    """Three TD1 lines with correct check digits; dates are YYMMDD."""
    line1 = f"I<UTO{doc}{check_digit(doc)}".ljust(30, "<")
    upper = f"{birth}{check_digit(birth)}{sex}{expiry}{check_digit(expiry)}UTO".ljust(29, "<")
    composite = line1[5:30] + upper[0:7] + upper[8:15] + upper[18:29]
    line2 = upper + check_digit(composite)
    name = f"{surname}<<{given.replace(' ', '<')}"
    line3 = name.ljust(30, "<")[:30]
    return [line1, line2, line3]


# ---------------------------------------------------------------- patterns

def rosette(cx, cy, r, amp, lobes, copies, color, width, opacity):
    paths = []
    for k in range(copies):
        phase = k * 2 * math.pi / (copies * lobes)
        pts = []
        for i in range(721):
            t = i / 720 * 2 * math.pi
            rr = r + amp * math.sin(lobes * t + phase * lobes)
            pts.append(f"{cx + rr * math.cos(t):.1f},{cy + rr * math.sin(t):.1f}")
        paths.append(f'<polyline points="{" ".join(pts)}"/>')
    return (f'<g fill="none" stroke="{color}" stroke-width="{width}" '
            f'opacity="{opacity}">{"".join(paths)}</g>')


def waves(y0, y1, step, amp, length, color, width, opacity, phase=0.0):
    paths = []
    y = y0
    n = 0
    while y <= y1:
        pts = []
        for x in range(-10, W + 11, 8):
            yy = y + amp * math.sin(x / length + n * 0.18 + phase) \
                + amp * 0.4 * math.sin(x / (length * 0.37) + n * 0.07)
            pts.append(f"{x},{yy:.1f}")
        paths.append(f'<polyline points="{" ".join(pts)}"/>')
        y += step
        n += 1
    return (f'<g fill="none" stroke="{color}" stroke-width="{width}" '
            f'opacity="{opacity}">{"".join(paths)}</g>')


def microtext(y, text, color, size=5.5, opacity=0.5):
    line = (text + " ") * 60
    return (f'<text x="0" y="{y}" font-family="Arial" font-size="{size}" fill="{color}" '
            f'opacity="{opacity}" letter-spacing="0.6">{line}</text>')


def emblem(cx, cy, s, color, opacity=1.0):
    """A stylised crest: a five-pointed star in a ring, framed by laurel."""
    star = []
    for i in range(10):
        r = s * (0.24 if i % 2 == 0 else 0.10)
        a = -math.pi / 2 + i * math.pi / 5
        star.append(f"{cx + r * math.cos(a):.1f},{cy + r * math.sin(a):.1f}")
    leaves = []
    radius = s * 0.40
    for i in range(9):
        theta = math.radians(112 + i * 17)
        lx = cx + radius * math.cos(theta)
        ly = cy + radius * math.sin(theta)
        rot = math.degrees(theta) + 90 + 28
        for x, r in ((lx, rot), (2 * cx - lx, 180 - rot)):
            leaves.append(f'<ellipse cx="{x:.1f}" cy="{ly:.1f}" rx="{s * 0.035:.1f}" '
                          f'ry="{s * 0.085:.1f}" transform="rotate({r:.1f} {x:.1f} {ly:.1f})"/>')
    ring = (f'<circle cx="{cx}" cy="{cy}" r="{s * 0.30:.1f}" fill="none" '
            f'stroke="{color}" stroke-width="{s * 0.025:.1f}"/>')
    return (f'<g fill="{color}" opacity="{opacity}">{ring}'
            f'<polygon points="{" ".join(star)}"/>{"".join(leaves)}</g>')


# ---------------------------------------------------------------- portrait

def portrait(p, uid, ghost=False):
    """A flat vector head-and-shoulders drawing in a 300 x 380 box."""
    skin, shade, hair, shirt = p["skin"], p["shade"], p["hair"], p["shirt"]
    style = p["style"]
    parts = [f'''
<defs>
  <linearGradient id="bg{uid}" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#e9eef2"/><stop offset="1" stop-color="#c9d3da"/>
  </linearGradient>
  <radialGradient id="face{uid}" cx="0.45" cy="0.4" r="0.7">
    <stop offset="0" stop-color="{skin}"/><stop offset="1" stop-color="{shade}"/>
  </radialGradient>
  <linearGradient id="hair{uid}" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{p["hair_hi"]}"/><stop offset="1" stop-color="{hair}"/>
  </linearGradient>
  <linearGradient id="shirt{uid}" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="{p["shirt_hi"]}"/><stop offset="1" stop-color="{shirt}"/>
  </linearGradient>
</defs>
<rect width="300" height="380" fill="url(#bg{uid})"/>''']
    if style == "long":
        parts.append(f'<path d="M58 380 C40 300 50 210 64 146 C80 64 220 64 236 146 '
                     f'C250 210 260 300 242 380Z" fill="url(#hair{uid})"/>')
    if style == "bun":
        parts.append(f'<circle cx="150" cy="58" r="44" fill="url(#hair{uid})"/>')
    # shoulders and neck
    parts.append(f'<path d="M20 380 C26 318 70 292 118 282 L182 282 C230 292 274 318 280 380Z" '
                 f'fill="url(#shirt{uid})"/>')
    parts.append(f'<path d="M124 236 L124 290 C138 304 162 304 176 290 L176 236Z" fill="{shade}"/>')
    parts.append(f'<path d="M124 236 L176 236 L176 262 C160 270 140 270 124 262Z" '
                 f'fill="#000" opacity=".10"/>')
    if p.get("collar"):
        parts.append(f'<path d="M118 282 L150 330 L182 282 L170 280 L150 306 L130 280Z" '
                     f'fill="{p["collar"]}"/>')
    else:
        parts.append(f'<path d="M120 284 C134 306 166 306 180 284" fill="none" '
                     f'stroke="{p["shirt_hi"]}" stroke-width="6" stroke-linecap="round"/>')
    # ears and head
    parts.append(f'<ellipse cx="86" cy="170" rx="12" ry="20" fill="{shade}"/>'
                 f'<ellipse cx="214" cy="170" rx="12" ry="20" fill="{shade}"/>')
    parts.append(f'<path d="M150 72 C196 72 218 108 216 160 C214 214 190 252 150 256 '
                 f'C110 252 86 214 84 160 C82 108 104 72 150 72Z" fill="url(#face{uid})"/>')
    if p.get("beard"):
        parts.append(f'<path d="M92 180 C96 230 120 258 150 258 C180 258 204 230 208 180 '
                     f'C200 214 186 226 150 226 C114 226 100 214 92 180Z" fill="{hair}" opacity="0.85"/>')
    # hair on top
    if style == "long":
        parts.append(f'<path d="M82 160 C76 96 110 62 150 62 C196 62 226 96 220 160 '
                     f'C214 128 200 106 176 96 C150 104 120 120 100 120 C92 132 86 144 82 160Z" '
                     f'fill="url(#hair{uid})"/>')
    elif style == "short":
        parts.append(f'<path d="M84 150 C76 90 112 58 154 60 C200 62 226 96 216 150 '
                     f'C212 122 200 106 186 100 C160 108 120 104 100 112 C92 124 88 136 84 150Z" '
                     f'fill="url(#hair{uid})"/>')
    elif style == "bun":
        parts.append(f'<path d="M84 156 C78 98 110 70 150 70 C192 70 222 98 216 156 '
                     f'C206 118 182 98 150 98 C118 98 94 118 84 156Z" fill="url(#hair{uid})"/>')
    # brows, eyes, nose, mouth
    parts.append(f'<path d="M106 140 Q124 130 140 138 M160 138 Q176 130 194 140" fill="none" '
                 f'stroke="{p["brow"]}" stroke-width="5" stroke-linecap="round"/>')
    for ex in (124, 176):
        parts.append(f'<ellipse cx="{ex}" cy="160" rx="13" ry="7.5" fill="#fbfbf8"/>'
                     f'<circle cx="{ex}" cy="160" r="6" fill="{p["eye"]}"/>'
                     f'<circle cx="{ex}" cy="160" r="2.6" fill="#141414"/>'
                     f'<circle cx="{ex + 2}" cy="158" r="1.3" fill="#ffffff"/>'
                     f'<path d="M{ex - 14} 158 Q{ex} 148 {ex + 14} 158" fill="none" '
                     f'stroke="#3a2a24" stroke-width="2.2"/>')
    if p.get("glasses"):
        parts.append('<g fill="none" stroke="#2b2b2e" stroke-width="3.5">'
                     '<rect x="102" y="144" width="44" height="32" rx="10"/>'
                     '<rect x="154" y="144" width="44" height="32" rx="10"/>'
                     '<path d="M146 156 Q150 152 154 156 M102 154 L88 150 M198 154 L212 150"/></g>')
    parts.append(f'<path d="M154 168 C156 182 160 192 160 198" fill="none" stroke="{p["line"]}" '
                 f'stroke-width="2.4" stroke-linecap="round" opacity=".45"/>'
                 f'<path d="M140 200 Q150 208 160 200" fill="none" stroke="{p["line"]}" '
                 f'stroke-width="3" stroke-linecap="round" opacity=".8"/>'
                 f'<ellipse cx="112" cy="198" rx="16" ry="9" fill="#e57f7f" opacity=".16"/>'
                 f'<ellipse cx="188" cy="198" rx="16" ry="9" fill="#e57f7f" opacity=".16"/>')
    parts.append(f'<path d="M128 222 Q150 238 172 222 Q150 228 128 222Z" fill="{p["lip"]}"/>'
                 f'<path d="M128 222 Q150 232 172 222" fill="none" stroke="{p["line"]}" '
                 f'stroke-width="2" stroke-linecap="round"/>')
    if style == "long":
        parts.append(f'<path d="M84 140 C74 200 86 260 64 330 C80 326 92 318 98 306 '
                     f'C96 250 94 190 104 132Z" fill="url(#hair{uid})"/>'
                     f'<path d="M216 140 C226 200 214 260 236 330 C220 326 208 318 202 306 '
                     f'C204 250 206 190 196 132Z" fill="url(#hair{uid})"/>'
)
    filt = ' style="filter:grayscale(1) contrast(.9)"' if ghost else ""
    return f'<svg viewBox="0 0 300 380" width="300" height="380"{filt}>{"".join(parts)}</svg>'


# ---------------------------------------------------------------- card faces

INK = "#1a1a1a"
LABEL = "#2b2b2b"
SANS = "Arial, Helvetica, sans-serif"


def label(x, y, lines, size=15):
    """A multilingual caption, one line per language group, bold as on the card."""
    if isinstance(lines, str):
        lines = [lines]
    spans = "".join(f'<tspan x="{x}" dy="{0 if i == 0 else size + 1}">{t}</tspan>'
                    for i, t in enumerate(lines))
    return (f'<text class="lbl" x="{x}" y="{y}" font-family="{SANS}" font-size="{size}" '
            f'font-weight="700" fill="{LABEL}">{spans}</text>')


def value(x, y, text, size=40, spacing=2.5):
    return (f'<text x="{x}" y="{y}" font-family="FiraMono" font-size="{size}" fill="{INK}" '
            f'letter-spacing="{spacing}">{text}</text>')


def background(seed):
    random.seed(seed)
    return "".join([
        '<defs><linearGradient id="paper" x1="0" y1="0" x2="1" y2="1">'
        '<stop offset="0" stop-color="#f3efe0"/><stop offset=".45" stop-color="#eef0e2"/>'
        '<stop offset=".8" stop-color="#e5edd8"/><stop offset="1" stop-color="#f1ecd9"/></linearGradient>'
        '<linearGradient id="holo" x1="0" y1="0" x2="1" y2="1">'
        '<stop offset="0" stop-color="#7fd3e8"/><stop offset=".35" stop-color="#d7a6f0"/>'
        '<stop offset=".65" stop-color="#f3e08a"/><stop offset="1" stop-color="#8be0b8"/>'
        '</linearGradient></defs>',
        f'<rect width="{W}" height="{H}" fill="url(#paper)"/>',
        '<g class="vis">',
        waves(0, H, 6, 10, 70, "#b9c79a", 0.7, 0.55),
        waves(0, H, 9, 26, 160, "#d8bf94", 0.6, 0.40, 1.3),
        rosette(1010, 300, 250, 22, 30, 6, "#a7b98a", 0.8, 0.45),
        rosette(1010, 300, 180, 16, 22, 5, "#cfae84", 0.7, 0.40),
        rosette(160, 640, 160, 14, 20, 4, "#a7b98a", 0.7, 0.35),
        emblem(860, 360, 360, "#9aa98a", 0.16),
        microtext(H - 6, "REPUBLIK UTOPIA · REPUBLIC OF UTOPIA · RÉPUBLIQUE D'UTOPIE", "#6b7a55"),
        '</g>',
    ])


def front(person, mrz):
    p = person
    body = [background(p["doc"])]
    body.append(f'<g class="vis" font-family="{SANS}" fill="#1b1b1b">'
                '<text x="54" y="70" font-size="40" font-weight="800" letter-spacing=".5">REPUBLIK UTOPIA</text>'
                '<text x="56" y="94" font-size="16" font-weight="700">REPUBLIC OF UTOPIA / RÉPUBLIQUE D&#39;UTOPIE</text>'
                '<text x="54" y="128" font-size="27" font-weight="800">AUSWEIS</text>'
                '<text x="56" y="150" font-size="16" font-weight="700">IDENTITY CARD / CARTE D&#39;IDENTITÉ</text></g>')
    body.append(f'<text x="{W - 50}" y="80" text-anchor="end" font-family="FiraMono" font-size="54" '
                f'fill="{INK}" letter-spacing="5">{p["doc"]}</text>')
    body.append(f'<g transform="translate(56 180)">{portrait(p, "f" + p["doc"])}</g>'
                .replace('width="300" height="380"', 'width="340" height="430"'))
    x = 440
    body.append(label(x, 140, "Name/Surname/Nom"))
    body.append(value(x, 186, p["surname"]))
    if p.get("birth_name"):
        body.append(value(x, 240, "GEB. " + p["birth_name"]))
    body.append(label(x, 280, "Vornamen/Given names/Prénoms"))
    body.append(value(x, 326, p["given"]))
    body.append(label(x, 368, ["Geburtstag/Date of birth/", "Date de naissance"]))
    body.append(value(x, 430, p["birth_print"]))
    body.append(label(x + 330, 368, ["Staatsangehörigkeit/Nationality/", "Nationalité"]))
    body.append(value(x + 330, 430, p["nationality"]))
    body.append(label(x, 472, "Geburtsort/Place of birth/Lieu de naissance"))
    body.append(value(x, 518, p["place"]))
    body.append(label(x, 560, ["Gültig bis/Date of expiry/", "Date d'expiration"]))
    body.append(value(x, 622, p["expiry_print"]))
    body.append(value(W - 236, 606, p["can"], 46, 4))
    body.append(label(x, 652, ["Unterschrift der Inhaberin/des Inhabers -",
                               "Signature of bearer - Signature de la titulaire/du titulaire"], 13))
    body.append(f'<text x="{x + 60}" y="732" font-family="Segoe Script, Brush Script MT, cursive" '
                f'font-size="50" fill="#141414" transform="rotate(-6 {x + 60} 720)">{p["signature"]}</text>')
    body.append(f'<g class="vis"><circle cx="{W - 140}" cy="240" r="62" fill="url(#holo)" opacity=".35"/>'
                + emblem(W - 140, 240, 120, "#ffffff", 0.6) + '</g>')
    return body


def back(person, mrz):
    p = person
    body = [background(p["doc"] + "b")]
    x = 54
    body.append(label(x, 60, "Augenfarbe/Colour of eyes/Couleur des yeux"))
    body.append(value(x + 10, 104, p["eyes"], 36))
    body.append(label(x, 136, "Größe/Height/Taille"))
    body.append(value(x + 10, 180, p["height"], 36))
    body.append(label(x, 212, "Datum/Date/Date"))
    body.append(value(x + 10, 256, p["issue_print"], 36))
    body.append(label(x, 300, "Behörde/Authority/Autorité"))
    body.append(value(x + 10, 344, p["authority"], 36))
    body.append(label(620, 60, "Anschrift/Address/Adresse"))
    body.append(value(630, 104, p["city"], 36))
    body.append(value(630, 156, p["street"], 36))
    body.append(label(620, 388, ["[a] Doktorgrad/Doctoral degree/Grade de docteur",
                                 "[b] Ordens- oder Künstlername/Religious name,",
                                 "stage or pen name/Nom de religion ou nom d'artiste"], 12))
    body.append(f'<g class="vis" opacity=".45" transform="translate(490 190) scale(.42)">'
                f'{portrait(p, "b" + p["doc"], ghost=True)}</g>'
                f'<text class="vis" x="500" y="270" font-family="FiraMono" font-size="30" fill="#555" '
                f'opacity=".5">{p["expiry_print"][:6]}{p["expiry_print"][8:]}</text>')
    body.append(f'<g class="vis"><circle cx="{W - 110}" cy="420" r="36" fill="none" stroke="#3d8f6a" '
                f'stroke-width="7" opacity=".8"/></g>')
    body.append(f'<rect x="0" y="468" width="{W}" height="14" fill="#9a9a96" opacity=".55"/>')
    # MRZ band
    body.append(f'<rect x="0" y="482" width="{W}" height="{H - 482}" fill="#f8f8f4"/>')
    for i, line in enumerate(mrz):
        body.append(f'<text x="60" y="{560 + i * 74}" font-family="OCRB, FiraMono" font-size="56" '
                    f'fill="#111" textLength="{W - 120}" lengthAdjust="spacing">{line.replace("<", "&lt;")}</text>')
    return body


PAGE = """<!doctype html><html><head><meta charset="utf-8"><style>
@font-face {{ font-family: FiraMono; src: url("{font}"); }}
html, body {{ margin: 0; background: #1d1f22; }}
.card {{ width: {w}px; height: {h}px; border-radius: 42px; overflow: hidden; }}
body.ir .vis, body.ir .lbl {{ display: none; }}
body.ir svg.face {{ filter: grayscale(1) contrast(1.1) brightness(.92); }}
body.ir svg.face rect.paper-ir {{ display: block; }}
</style></head><body class="{mode}"><div class="card"><svg class="face" width="{w}" height="{h}"
viewBox="0 0 {w} {h}" xmlns="http://www.w3.org/2000/svg">{body}</svg></div></body></html>"""


def page(body, font, infrared):
    content = "".join(body)
    if infrared:
        content = content.replace('fill="url(#paper)"', 'fill="#d6d6d3"')
    return PAGE.format(font=font, w=W, h=H, body=content, mode="ir" if infrared else "")
