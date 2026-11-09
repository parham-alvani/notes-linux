// `daftar-cli export`: a note on A4. The head has the person's logo and the
// logo of the folder the note is in (a company's), the title and a line under
// it; each page has a bar along its top edge and the page count at its foot.
// The colors are the reader's Daylight (web/src/reader.css). The body is
// `TypstNote`'s markup, which calls the functions below.

#import "/extras.typ": formulas, diagrams
#import "/mitex/specs/mod.typ": mitex-scope

#let ink = rgb("#2a2a28")
#let muted = rgb("#6a6a62")
#let rule = rgb("#d8d5cc")
#let panel = rgb("#efede6")
#let accent = rgb("#a85200")
#let gold = rgb("#8a6a00")
#let teal = rgb("#00707a")
#let blue = rgb("#0070a0")
#let green = rgb("#0a7a48")
#let red = rgb("#c01030")
#let purple = rgb("#8040a0")
#let bar = gradient.linear(teal, green, gold, accent, angle: 0deg)
#let mono = "DejaVu Sans Mono"

// A block whose first strong letter is Persian.
#let right-to-left(body) = {
  set text(dir: rtl, lang: "fa")
  body
}

// A wiki link or a link inside the vault: its text, in the link color.
#let internal(body) = text(fill: teal, body)

// `==text==`, and Obsidian's coloured marks (`==🔴 text==`), each in a pale
// wash of the reader's color for it; the plain mark is gold.
#let marked(color, body) = {
  let base = (
    red: red,
    orange: rgb("#c86400"),
    yellow: gold,
    green: green,
    blue: blue,
    purple: purple,
  ).at(color, default: gold)
  highlight(fill: base.lighten(78%), body)
}

// A note or a file embedded that is not a picture: its name.
#let embedded(body) = text(fill: muted, style: "italic")[↪ #body]

// A picture at its size, or the page's width when it is wider.
#let picture(path, width: auto) = layout(size => {
  let drawn = image(path, width: width)
  let wide = measure(drawn).width > size.width
  block(above: 0.8em, below: 0.8em, if wide { image(path, width: size.width) } else { drawn })
})

// The [n]th formula of the note: MiTeX's Typst math, or its TeX as code
// when MiTeX or Typst could not read it.
#let formula(n, block: false) = {
  let it = formulas.at(n)
  if it.code != none {
    math.equation(block: block, eval("$" + it.code + "$", scope: mitex-scope))
  } else {
    raw(block: block, lang: "latex", it.tex)
  }
}

// The [n]th Mermaid diagram, drawn, or its text as code when it did not parse.
#let diagram(n) = {
  let it = diagrams.at(n)
  if it.svg != none { align(center, picture(it.svg)) } else { raw(block: true, lang: "mermaid", it.source) }
}

// A task: its status in a box, done and cancelled ones greyed.
#let task(mark, body) = {
  let done = mark == "x" or mark == "X"
  let cancelled = mark == "-"
  let color = if done { green } else if cancelled { muted } else { accent }
  let sign = if done { text(fill: white, weight: "bold")[✓] } else if mark == " " { [] } else if cancelled { text(fill: muted)[–] } else { text(fill: accent, font: mono, mark) }
  box(
    width: 0.85em,
    height: 0.85em,
    baseline: 0.12em,
    radius: 2pt,
    stroke: 0.7pt + color,
    fill: if done { green } else { none },
    align(center + horizon, text(size: 0.65em, sign)),
  )
  h(0.4em)
  if done or cancelled { text(fill: muted, strike(body)) } else { body }
}

// A `[!figure]` callout: the picture centred, numbered, its caption under it.
#let numbered-figure(caption, body) = figure(body, caption: caption, kind: image, supplement: [Figure])

// An Obsidian callout, in its kind's color.
#let callout(kind, title, body) = {
  let color = if kind in ("tip", "hint", "important", "success", "check", "done") {
    green
  } else if kind in ("warning", "caution", "attention", "question", "help", "faq") {
    accent
  } else if kind in ("danger", "error", "bug", "failure", "fail", "missing") {
    red
  } else if kind == "example" {
    purple
  } else if kind in ("quote", "cite") {
    muted
  } else {
    blue
  }
  block(
    width: 100%,
    fill: color.lighten(92%),
    stroke: (left: 2.5pt + color, rest: 0.5pt + color.lighten(60%)),
    radius: 2pt,
    inset: (x: 10pt, y: 8pt),
    above: 0.9em,
    below: 0.9em,
  )[
    #text(weight: "bold", fill: color, title)
    #if body != [] [
      #v(0.15em)
      #body
    ]
  ]
}

#let note(title: "", subtitle: "", logo: none, folder-logo: none, footer: "", title-rtl: false, exported: none, contents: false, landscape: false, body) = {
  // The day of the export, with no time: the same note makes the same file all day.
  set document(title: title, date: exported)
  set page(
    paper: "a4",
    // `--landscape`: for a screen or a projector, where wide figures matter more than line length.
    flipped: landscape,
    margin: (top: 24mm, bottom: 20mm, x: 18mm),
    background: place(top, rect(width: 100%, height: 4pt, fill: bar)),
    header: context {
      if counter(page).get().first() > 1 {
        set text(size: 8pt, fill: muted)
        grid(columns: (1fr, auto), align: (start + horizon, end + horizon), title, subtitle)
        v(-0.6em)
        line(length: 100%, stroke: 0.5pt + rule)
      }
    },
    footer: context {
      line(length: 100%, stroke: 0.5pt + rule)
      v(-0.3em)
      set text(size: 8pt, fill: muted)
      grid(
        columns: (1fr, auto),
        align: (left + horizon, right + horizon),
        footer,
        {
          if exported != none [Exported #exported.display() #h(0.6em) ]
          text(fill: ink)[#counter(page).display() / #counter(page).final().first()]
        },
      )
    },
  )

  set text(font: ("Vazirmatn", mono), size: 10.5pt, fill: ink, lang: "en")
  // Justified, as TeX sets a page, with words broken at their syllables;
  // a table's narrow cells stay ragged.
  set par(justify: true, leading: 0.75em, spacing: 0.9em)
  set text(hyphenate: auto)
  show table: set par(justify: false)
  set list(marker: text(fill: accent)[•], indent: 0.4em, spacing: 0.65em)
  set enum(indent: 0.4em, spacing: 0.65em)
  set highlight(fill: gold.lighten(75%))
  show math.equation: set text(font: "New Computer Modern Math", dir: ltr)
  // Vazirmatn has no italic: emphasis is the reader's color for it.
  show emph: it => text(fill: green, it.body)
  show strong: it => text(weight: "bold", fill: accent, it.body)
  set footnote.entry(separator: line(length: 30%, stroke: 0.5pt + rule))
  show link: it => text(fill: teal, underline(offset: 2pt, stroke: 0.5pt + teal.lighten(40%), it))

  show heading: set text(weight: "bold")
  show heading.where(level: 1): it => block(above: 1.4em, below: 0.6em, breakable: false)[
    #grid(
      columns: (auto, 1fr),
      column-gutter: 7pt,
      align: horizon,
      box(width: 4pt, height: 13pt, radius: 2pt, fill: accent),
      text(size: 14pt, fill: ink, it.body),
    )
  ]
  show heading.where(level: 2): it => block(above: 1.2em, below: 0.5em, breakable: false, text(size: 12pt, fill: accent, it.body))
  show heading.where(level: 3): it => block(above: 1em, below: 0.45em, breakable: false, text(size: 11pt, fill: blue, it.body))
  show heading.where(level: 4): it => block(above: 0.9em, below: 0.4em, breakable: false, text(size: 10.5pt, fill: green, it.body))
  show heading.where(level: 5): it => block(above: 0.9em, below: 0.4em, breakable: false, text(size: 10.5pt, fill: purple, it.body))
  show heading.where(level: 6): it => block(above: 0.9em, below: 0.4em, breakable: false, text(size: 10.5pt, fill: muted, it.body))

  show raw.where(block: false): it => box(
    fill: panel,
    radius: 2pt,
    inset: (x: 3pt, y: 0pt),
    outset: (y: 2.5pt),
    text(font: mono, size: 0.86em, dir: ltr, fill: gold, it),
  )
  show raw.where(block: true): it => block(
    width: 100%,
    fill: panel,
    stroke: (left: 2.5pt + accent, rest: 0.5pt + rule),
    radius: 2pt,
    inset: (x: 9pt, y: 8pt),
    above: 0.8em,
    below: 0.9em,
    align(left, text(font: mono, size: 8.5pt, dir: ltr, lang: "en", it)),
  )

  show quote.where(block: true): it => block(
    stroke: (left: 2pt + accent),
    inset: (left: 10pt, y: 3pt),
    above: 0.8em,
    below: 0.9em,
    text(fill: muted, it.body),
  )

  // A figure keeps its picture and caption on one page; "Figure 3." is in the
  // accent, its words in the muted grey.
  show figure: set block(breakable: false, above: 1.1em, below: 1.2em)
  show figure.caption: it => block(width: 92%, {
    set par(justify: false)
    set text(size: 9pt, fill: muted)
    text(weight: "bold", fill: accent)[#it.supplement #context it.counter.display(it.numbering).]
    h(0.35em)
    it.body
  })

  set table(stroke: 0.5pt + rule, inset: (x: 7pt, y: 5pt), fill: (_, row) => if row == 0 { accent.lighten(88%) })
  show table.cell.where(y: 0): set text(weight: "bold", fill: accent)

  // The head of the first page.
  let mark(path) = if path == none { [] } else { box(height: 15mm, image(path, height: 15mm)) }
  block(above: 0pt, below: 1.3em, breakable: false)[
    #grid(
      columns: (auto, 1fr, auto),
      column-gutter: 12pt,
      align: (horizon, horizon, horizon),
      mark(logo),
      {
        set text(dir: if title-rtl { rtl } else { ltr })
        stack(
          spacing: 6pt,
          text(size: 18pt, weight: "bold", fill: ink, title),
          if subtitle != "" { text(size: 9pt, fill: muted, subtitle) },
        )
      },
      mark(folder-logo),
    )
    #v(0.4em)
    #line(length: 100%, stroke: 1pt + bar)
  ]

  // `--contents`: the headings down to the third, each a link to its page,
  // then the note on a page of its own.
  if contents {
    show outline.entry.where(level: 1): it => {
      v(0.55em, weak: true)
      text(weight: "bold", fill: accent, it)
    }
    show outline.entry: set text(size: 10pt)
    outline(title: text(size: 13pt, fill: ink)[Contents], depth: 3, indent: 1.2em)
    pagebreak(weak: true)
  }

  body
}
