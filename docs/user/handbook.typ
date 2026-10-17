// Page design of the user guide. It borrows the program window's look: the
// same neutral palette, Fira Sans, hairline rules, 2-pt corners, buttons drawn
// as chips, and a header and footer shaped like its top bar and status bar.

#let ink = rgb(21, 24, 27)
#let muted = rgb(83, 88, 94)
#let hair = rgb(220, 222, 225)
#let sunk = rgb(244, 245, 246)
#let good = rgb(31, 107, 54)
#let bad = rgb(168, 27, 16)

// The dark appearance, for the cover.
#let night = (
  ground: rgb(13, 15, 17),
  surface: rgb(19, 22, 25),
  sunk: rgb(8, 9, 11),
  ink: rgb(231, 233, 235),
  muted: rgb(157, 163, 169),
  line: rgb(38, 42, 46),
  good: rgb(95, 185, 124),
)

#let sans = "Fira Sans"
#let mono = "Fira Mono"

// Pandoc emits these for thematic breaks.
#let horizontalRule = line(length: 100%, stroke: 0.6pt + hair)
#let divider = horizontalRule

#let plain(it) = {
  if type(it) == str { it }
  else if it.has("text") { it.text }
  else if it.has("children") { it.children.map(plain).join("") }
  else if it.has("body") { plain(it.body) }
  else if it == [ ] { " " }
  else { "" }
}

// Bold text in the guide is always a label from the program window.
#let status-colors = (
  "MRZ verified": good,
  "Check digit mismatch": bad,
  "MRZ unverified": muted,
  "No MRZ": muted,
)
#let primary-buttons = ("Scan front", "Update available", "Install update", "Restart")
#let keys = ("Esc",)

#let chip(body, fill: white, stroke: hair, color: ink) = box(
  fill: fill,
  stroke: 0.6pt + stroke,
  radius: 2pt,
  inset: (x: 4.5pt, y: 0pt),
  outset: (y: 3pt),
  text(size: 0.88em, weight: "medium", fill: color, body),
)

#let ui-label(it) = {
  let label = plain(it.body)
  if label in status-colors {
    let color = status-colors.at(label)
    box(
      fill: color.transparentize(90%),
      radius: 2pt,
      inset: (x: 4.5pt),
      outset: (y: 3pt),
      text(size: 0.88em, weight: "medium", fill: color)[
        #box(baseline: -0.2em, circle(radius: 2.2pt, fill: color))#h(4pt)#it.body
      ],
    )
  } else if label in primary-buttons {
    chip(it.body, fill: ink, stroke: ink, color: white)
  } else if label in keys {
    box(
      stroke: (paint: hair, thickness: 0.6pt),
      radius: 2pt,
      inset: (x: 4pt),
      outset: (y: 2.5pt),
      fill: sunk,
      text(font: mono, size: 0.82em, fill: ink, it.body),
    )
  } else {
    chip(it.body)
  }
}

// --- Cover --------------------------------------------------------------

#let night-chip(body, primary: false) = box(
  fill: if primary { night.ink } else { night.surface },
  stroke: 0.6pt + if primary { night.ink } else { night.line },
  radius: 2pt,
  inset: (x: 6pt, y: 4.5pt),
  text(size: 7pt, weight: "medium", fill: if primary { night.ground } else { night.ink }, body),
)

#let night-label(body) = text(size: 6pt, weight: "semibold", tracking: 0.08em, fill: night.muted, upper(body))

// A specimen card in the preview pane: ICAO's sample data, never a real person.
#let specimen = {
  let mrz = (
    "IDD<<T220001293<<<<<<<<<<<<<<<",
    "6408125<2010315D<<<<<<<<<<<<<4",
    "MUSTERMANN<<ERIKA<<<<<<<<<<<<<",
  )
  box(
    width: 100%,
    height: 100%,
    fill: rgb(214, 218, 222),
    radius: 6pt,
    clip: true,
    inset: 9pt,
    {
      grid(
        columns: (auto, 1fr),
        column-gutter: 9pt,
        rect(width: 46pt, height: 58pt, radius: 2pt, fill: rgb(176, 182, 188)),
        stack(
          spacing: 5pt,
          ..(70%, 92%, 55%, 80%, 40%).map(w => rect(width: w, height: 4.5pt, radius: 1pt, fill: rgb(184, 190, 196))),
        ),
      )
      place(bottom + left, stack(
        spacing: 2.6pt,
        ..mrz.map(l => text(font: mono, size: 6.6pt, fill: rgb(40, 44, 48), tracking: 0.04em, l)),
      ))
    },
  )
}

#let window-mockup = {
  let field(name, value) = stack(
    spacing: 2.5pt,
    text(size: 5.5pt, fill: night.muted, name),
    text(size: 7pt, fill: night.ink, value),
  )
  let entry(title, sub, active: false) = block(
    width: 100%,
    fill: if active { night.surface } else { none },
    stroke: if active { 0.6pt + night.line } else { none },
    radius: 2pt,
    inset: 6pt,
    stack(spacing: 3pt, text(size: 7pt, fill: night.ink, title), text(size: 5.5pt, fill: night.muted, sub)),
  )
  block(
    width: 100%,
    fill: night.ground,
    stroke: 0.8pt + night.line,
    radius: 6pt,
    clip: true,
    {
      // Top bar
      block(width: 100%, inset: (x: 12pt, y: 9pt), grid(
        columns: (auto, auto, 1fr, auto),
        column-gutter: 5pt,
        align: horizon,
        text(size: 10pt, weight: "medium", fill: night.ink)[Passport Scanner],
        h(6pt),
        stack(dir: ltr, spacing: 4pt,
          night-chip[\+ New], night-chip(primary: true)[Scan front], night-chip[Scan back], night-chip[PDF]),
        night-chip[Settings],
      ))
      line(length: 100%, stroke: 0.6pt + night.line)
      // Panes
      grid(
        columns: (24%, 1fr, 30%),
        rows: 150pt,
        stroke: (x, _) => if x > 0 { (left: 0.6pt + night.line) },
        block(inset: 8pt, height: 100%, stack(spacing: 5pt,
          night-label[Documents],
          entry(active: true)[Document 2][Front · Back],
          entry[Document 1][Front · Back],
        )),
        block(inset: 12pt, height: 100%, {
          stack(dir: ltr, spacing: 3pt,
            night-chip(primary: true)[Front], night-chip[Back], h(6pt), night-chip(primary: true)[White], night-chip[IR])
          v(9pt)
          block(height: 98pt, width: 100%, specimen)
        }),
        block(inset: 9pt, height: 100%, {
          stack(spacing: 7pt,
            grid(columns: (1fr, auto), align: horizon, night-label[Document data],
              text(size: 5.5pt, weight: "medium", fill: night.good)[#box(baseline: -0.15em, circle(radius: 1.8pt, fill: night.good))#h(2.5pt)MRZ verified]),
            field[Surname][MUSTERMANN],
            field[Given names][ERIKA],
            field[Document number][T22000129],
            grid(columns: (1fr, 1fr), field[Date of birth][1964-08-12], field[Valid until][2020-10-31]),
          )
        }),
      )
      line(length: 100%, stroke: 0.6pt + night.line)
      // Status bar
      block(width: 100%, inset: (x: 12pt, y: 6pt), grid(
        columns: (1fr, auto),
        text(size: 6pt, fill: night.muted)[Scan complete],
        text(size: 6pt, fill: night.muted)[Document present],
      ))
    },
  )
}

// The icon is porcelain on transparent; on paper it sits in a dark tile.
#let app-tile(size) = box(baseline: 22%, width: size, height: size, radius: size * 0.22, fill: night.ground,
  align(center + horizon, image("icon.svg", width: 100%)))

#let cover(title, subtitle, author, version) = page(
  fill: night.ground,
  margin: (x: 22mm, top: 24mm, bottom: 20mm),
  header: none,
  footer: none,
  {
    set text(fill: night.ink)
    grid(
      columns: (1fr, auto),
      align: horizon,
      stack(dir: ltr, spacing: 10pt,
        box(baseline: 35%, image("icon.svg", width: 36pt)),
        text(size: 8pt, weight: "semibold", tracking: 0.14em, fill: night.muted, upper(subtitle))),
      if version != none {
        box(stroke: 0.6pt + night.line, radius: 2pt, inset: (x: 6pt, y: 4pt),
          text(size: 8pt, font: mono, fill: night.muted, version))
      },
    )
    v(30mm)
    text(size: 50pt, weight: "semibold", tracking: -0.02em, title)
    v(4mm)
    block(width: 75%, text(size: 13pt, weight: "light", fill: night.muted)[
      Scan ID cards and passports with a Plustek SecureScan, read the
      machine-readable zone and keep all four pictures in one PDF.
    ])
    v(1fr)
    window-mockup
    v(14mm)
    line(length: 100%, stroke: 0.6pt + night.line)
    v(4mm)
    grid(
      columns: (1fr, auto),
      text(size: 8pt, fill: night.muted, author),
      text(size: 8pt, fill: night.muted)[Free software · MIT License],
    )
  },
)

// --- Document -----------------------------------------------------------

#let chapter-starts-here() = query(heading.where(level: 1)).any(h => h.location().page() == here().page())

#let handbook(title: none, subtitle: none, author: none, version: none, lang: "en", doc) = {
  set document(title: title, author: plain(author))
  set text(font: sans, size: 10pt, fill: ink, lang: lang, number-type: "lining")
  set par(justify: false, leading: 0.72em, spacing: 1.15em)

  cover(title, subtitle, author, version)

  set page(
    paper: "a4",
    margin: (x: 24mm, top: 30mm, bottom: 26mm),
    header: context {
      if chapter-starts-here() { return }
      let before = query(heading.where(level: 1).before(here()))
      set text(size: 7.5pt, fill: muted)
      grid(
        columns: (auto, 1fr, auto),
        column-gutter: 8pt,
        align: horizon,
        [#app-tile(11pt)#h(5pt)#text(weight: "medium", fill: ink)[#title]],
        if before.len() > 0 { before.last().body },
        subtitle,
      )
      v(-3pt)
      line(length: 100%, stroke: 0.6pt + hair)
    },
    footer: context {
      line(length: 100%, stroke: 0.6pt + hair)
      v(-3pt)
      set text(size: 7.5pt, fill: muted)
      grid(
        columns: (1fr, auto),
        [#box(baseline: -0.15em, circle(radius: 2pt, fill: good))#h(5pt)#title #subtitle#if version != none [ · #version]],
        [#text(fill: ink, weight: "medium", counter(page).display()) / #counter(page).final().first()],
      )
    },
  )

  // Headings
  set heading(numbering: "1.1")
  show heading: set text(weight: "semibold", hyphenate: false)
  show heading.where(level: 1): it => {
    pagebreak(weak: true)
    if it.numbering != none {
      box(fill: ink, radius: 2pt, inset: (x: 6pt, y: 4pt),
        text(size: 8pt, weight: "medium", fill: white, tracking: 0.08em)[CHAPTER #counter(heading).display("1")])
      v(5mm)
    }
    block(text(size: 26pt, tracking: -0.015em, it.body))
    v(3mm)
    line(length: 100%, stroke: 0.6pt + hair)
    v(4mm)
  }
  show heading.where(level: 2): it => block(above: 2em, below: 1.1em, sticky: true, {
    set text(size: 13.5pt)
    if it.numbering != none {
      text(fill: muted, weight: "regular", counter(heading).display())
      h(8pt)
    }
    it.body
  })
  show heading.where(level: 3): it => block(above: 1.6em, below: 0.8em, sticky: true, text(size: 11pt, it.body))

  // Labels from the program window
  show strong: ui-label
  show emph: set text(style: "italic")

  // Links: chapter references carry their page.
  show link: it => {
    if type(it.dest) == label {
      it
      context {
        let target = query(it.dest)
        if target.len() > 0 {
          h(2pt)
          text(size: 0.82em, fill: muted)[→ p. #counter(page).at(target.first().location()).first()]
        }
      }
    } else if type(it.dest) == str {
      underline(stroke: 0.6pt + muted, offset: 2pt, it)
    } else {
      it
    }
  }
  show link: set text(fill: ink)

  // Lists: steps are numbered like buttons; bullets are small squares.
  set enum(
    numbering: n => box(
      width: 13pt, height: 13pt, radius: 2pt, fill: ink, baseline: 1pt,
      align(center + horizon, text(size: 7pt, weight: "semibold", fill: white, str(n))),
    ),
    body-indent: 8pt,
    spacing: 0.95em,
  )
  set list(
    marker: box(baseline: -0.28em, rect(width: 3.5pt, height: 3.5pt, fill: muted)),
    body-indent: 8pt,
    spacing: 0.85em,
  )

  // Code
  show raw: set text(font: mono, size: 0.9em)
  show raw.where(block: false): it => box(
    fill: sunk, radius: 2pt, inset: (x: 3pt), outset: (y: 2.5pt), it,
  )
  show raw.where(block: true): it => block(
    width: 100%, fill: sunk, stroke: 0.6pt + hair, radius: 3pt, inset: 12pt,
    text(size: if it.lang == "license" { 7.6pt } else { 8.4pt }, it),
  )

  // Tables are drawn like the program's panels.
  set table(
    stroke: (_, y) => if y > 1 { (top: 0.6pt + hair) },
    fill: (_, y) => if y == 0 { sunk },
    inset: (x: 9pt, y: 7pt),
    align: left,
  )
  set table.hline(stroke: 0.6pt + hair)
  show table.cell: set align(left)
  show table.cell.where(y: 0): set text(size: 7pt, weight: "semibold", fill: muted, tracking: 0.07em)
  show table.cell.where(y: 0): it => upper(it)
  show figure.where(kind: table): set block(breakable: true)
  show figure.where(kind: table): it => block(
    width: 100%, stroke: 0.6pt + hair, radius: 3pt, clip: true, above: 1.2em, below: 1.4em, it.body,
  )

  // Contents: one row per chapter, its sections listed beneath in one line.
  {
    heading(level: 1, numbering: none, outlined: false)[Contents]
    context {
      let chapters = query(heading.where(level: 1, outlined: true))
      for (i, chapter) in chapters.enumerate() {
        let sections = selector(heading.where(level: 2)).after(chapter.location())
        if i + 1 < chapters.len() { sections = sections.before(chapters.at(i + 1).location()) }
        let page = counter(page).at(chapter.location()).first()
        let number = if chapter.numbering != none { counter(heading).at(chapter.location()).first() }
        block(width: 100%, inset: (y: 8pt), stroke: (bottom: 0.6pt + hair), link(chapter.location(), grid(
          columns: (26pt, 1fr, auto),
          align: (left + top, left + top, right + top),
          if number != none {
            box(width: 17pt, height: 17pt, radius: 2pt, fill: sunk, stroke: 0.6pt + hair,
              align(center + horizon, text(size: 8pt, weight: "semibold", str(number))))
          },
          {
            text(size: 11pt, weight: "semibold", chapter.body)
            let names = query(sections).map(h => h.body)
            if names.len() > 0 {
              v(4pt)
              block(text(size: 8.5pt, fill: muted, names.join[#h(5pt)·#h(5pt)]))
            }
          },
          text(size: 11pt, weight: "medium", str(page)),
        )))
      }
    }
  }

  doc
}
