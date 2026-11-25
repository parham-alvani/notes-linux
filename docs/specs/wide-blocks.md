# Wide blocks: tables, code and pictures wider than the prose

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Nothing is written to the vault: this is how a note is laid out.

Prose reads best at about 70 to 85 letters a line, so the reader and the editor keep it to the measure, `appearance.text_width` (`look.md`, part 1). A table, a code block or a figure often needs more: today a wide table scrolls sideways in its own box and a wide figure is scaled down to the measure, while the window has room beside the text. A wide block may now grow past the measure, into that room; the prose stays as it is.

## 1. The setting

| Key | Default | Range | Meaning |
| --- | --- | --- | --- |
| `appearance.block_width` | 60 | 0 to 400 | The widest a wide block may be, in em of the text size, as `text_width`. 0 keeps every block inside the measure, as before. |

A vault may set it (`vault-config.md`), as `text_width`. With `text_width = 0` (no measure) there is nothing to grow past, and this spec changes nothing. A `block_width` smaller than `text_width` acts as 0.

## 2. Which blocks are wide

- A table (with its delimiter row).
- A fenced code block, and a Mermaid diagram.
- Display math (`$$ … $$`).
- A picture that stands alone in its paragraph: `![[x.svg]]` or `![](x.png)` with nothing else on its lines, with or without a size (`|600`). A picture inside a sentence stays in the measure.
- A `[!figure]` callout: its panel grows to hold its picture, and its caption stays in the measure.

Nothing else grows: paragraphs, headings, lists, quotes, other callouts, task lists and embeds of notes keep the measure. A wide block inside a list item, a quote or another callout keeps the measure too.

## 3. Its width and its place

The numbers are in the reader's pixels (CSS px on Linux and macOS, dp on Android).

- `M` is the measure: `text_width` em at the text size (560 at size 16 and width 35).
- `R` is the room the page keeps at each side of the pane: the reader's side padding (2.5 em, or 16 px under 720 px of pane), as for the measure.
- `L`, the wide limit, is the smaller of `block_width` em at the text size and the pane's width less `2 R`. `L` is never less than `M`.
- `N` is the block's own width: a table's columns as drawn, a code block's longest line (no wrap), a diagram's or a picture's natural width, or the width its size asks for (`|600`).

Then:

| Case | Width | Place |
| --- | --- | --- |
| `N ≤ M` | as before | as before: at the measure's start edge (a picture as before) |
| `M < N ≤ L` | `N` | centred in the pane, which is the measure's centre, so it grows the same on both sides |
| `N > L` | `L` | centred in the pane; then as before: a table and a code block scroll sideways in their box, a picture and a diagram are scaled down to `L` |

A right-to-left block is placed the same way: centred is centred. The width is worked out when the note is laid out and when the pane, the text size or a setting changes, never on a key.

The core of each app has the rule as one function, `WideBlock.place` (Swift), `WideBlock::place` (Rust), `WideBlock.place` (Kotlin): given `N`, `M`, the pane's width, `R` and `block_width` in px, it returns the width and the start offset from the pane's left edge, or nothing when the block keeps the measure.

## 4. Where it applies

| Place | Rule |
| --- | --- |
| The reader (macOS and Linux share it, `web/src`) | Every wide block of section 2. |
| The macOS editor | A picture under its line. A table's grid and a code block's panel keep the measure for now: TextKit puts a line wider than its text container at the container's edge, so they would need the container `L` wide and every other paragraph held to the measure by indents, a rewrite of the editor's layout. |
| The Linux editor | A table's grid: `Metrics::fits` holds against `L`, and a wide grid's rows get the margins of its place. `R` is the editor's least room, 32 px. A code block's panel and a picture keep the measure for now: GtkTextView wraps a code block at the measure and draws a picture inside it. |
| Android, the reader | Only where the screen is wider than the measure plus `2 R` (a tablet, a phone on its side); on a phone held upright `L = M` and nothing changes. |
| Android, the editor | Not in this spec. |
| PDF export, Quick Look | Not in this spec: a page has no room beside the text. |

## 5. Tests every app must have

With the text size 16, `text_width = 35` (`M` = 560), `R` = 40 and `block_width = 60` (960 px):

1. `aBlockThatFitsKeepsTheMeasure`: `N` = 500 in a pane of 1400 gives nothing (as before).
2. `aWideBlockGrowsToItsOwnWidth`: `N` = 800 in a pane of 1400 gives width 800 at offset 300.
3. `theLimitIsTheSettingOrThePane`: `N` = 1200 in a pane of 1400 gives 960 at offset 220; in a pane of 900 it gives 820 (900 less 80) at offset 40.
4. `aNarrowPaneKeepsTheMeasure`: in a pane of 600, `L` would be 520, less than `M`, so `N` = 800 gives the measure's own width and place: nothing.
5. `blockWidthZeroIsOff`: `block_width = 0` gives nothing for any `N`; so does `text_width = 0`.
6. `whichBlocksAreWide` (reader or core): a table, a fenced block, `$$` math and `![[x.svg]]` alone on its line are wide; `see ![[x.svg]] here` and a table inside a `>` quote are not.
