# The look of a note: reader, editor and cursors

How a note is drawn when it is read and when it is edited, in the three apps. The look was decided on 2026-11-22 ("naz, refined"): the palettes stay, and the decoration that carried no meaning goes. This spec is the one text the apps are built against; a colour or a size that changes here changes in each app.

## What the look keeps and what it drops

A colour says what a thing is: a heading's level, bold text, code, a link, a callout's kind, a highlight's colour, a task's status, a date that is late. Nothing else is coloured, and nothing is a gradient.

| Part | Rule |
| --- | --- |
| Title | The heading 1 colour, weight 750, 1.9 em. No gradient. |
| Headings | One weight (650; 600 for levels 5 and 6). The level is the size and the colour: 1.6, 1.35, 1.12, 1, 1 and 0.875 em. No rule under a heading and no bar beside it. |
| Bold | The strong colour, weight 650. |
| Italic | Slanted, in the text's colour. |
| Highlight | A tint under the text (the highlight colour at 32%), and the text keeps its own colours. The six colours of `==🔴…==` tint the same way. |
| Link | The link colour with a thin underline of the same colour. |
| Tag | A pill with a 6 px corner, the tag's colour at 14%, no border. |
| Inline code | The code colour on the code background, 5 px corner, no border. |
| Code block | The code background, 12 px corner. The language is a word in its colour, with no pill and no bar. |
| Callout | The kind's colour at 10% with a hairline of the same colour at 26%, 12 px corner, the kind's icon. No bar. |
| Quote | A 3 px bar in the accent at the side the lines start. Nothing behind the text. |
| List | One marker colour at every level. |
| Table | A hairline box with a 12 px corner; the header row on the panel colour. |
| Rule (`---`) | One hairline. |
| Reading progress | A 2 px line in the accent. |

## Palettes

There are three: Daylight (light), naz (dark) and naz, soft (dark). The values of Daylight and naz are those of `web/src/reader.css`, `Naz.kt`, `NSPalette` and `editor_style.rs`, as before.

### naz, soft

naz, soft is naz with the colour strength capped, for long sessions. It is made from naz by a rule, so it is never a second table to keep by hand:

1. The neutrals are a table: background `#323232` → `#2f2f2d`, panel `#3a3a35` → `#383835`, code background `#282826` → `#272725`, text `#f5f5f0` → `#e2e2dd`, hairline `#4a4a4a` → `#474744`.
2. Every other colour goes to OKLCH. Its chroma is capped at 0.095 (0.14 for the reds `#ff5070` and `#ff7088`, which turn to pink below that), its lightness at 0.90, and its hue stays.

`SoftPalette.soften` in `notes-mac/Sources/DaftarCore` is the rule in code.

### The setting

`appearance.dark_palette` is `"naz"` (the default) or `"naz-soft"`. It is a shared setting (`vault-config.md`): a vault may set it. The reader page takes it as `palette` (`"naz"` or `"soft"`) in `daftar.show` and sets `data-palette` on the root.

## Text

The default text size is 16 (`appearance.text_size`) and the default measure is 35 em (`appearance.text_width`), about 70 letters in a line. The line height is 1.65, and 2 for a right-to-left block. A config file that sets a size or a width keeps it.

The reader and the editor have the same size, line height and measure, so a line stays where it is when the mode changes.

## Cursors

| Cursor | Rule |
| --- | --- |
| Reading block | The accent at 7% behind the block, with a 2.5 px bar in the accent at 65% where the lines start: at the right for a right-to-left block. It slides to the next block. |
| Reading line | A band in the accent at `appearance.reading_line` percent on the line being read, with a 5 px corner and no bar. |
| Caret | Two points wide. It moves between places in 70 ms, stays solid while keys come, and fades slowly after 650 ms with no key. |
| Current line | A neutral tint (the text colour at 4.5%), weaker than the reading block. |
| Neovim, normal mode | A block on one letter. It does not blink. Insert mode has the caret above. The mode is shown in the status bar: NORMAL, INSERT, VISUAL. |

Reduce Motion stops the slide, the glide and the fade.

## Live preview

The marks of one element (`**`, `==`, a backtick, `[[ ]]`) are shown only while the caret is inside that element. A heading's `#` marks are shown on the caret's line. A task's box is drawn as soon as `- [ ] ` is complete, and a click on it changes the status. A callout, a table and a code block are drawn in the editor as the reader draws them.

## The format bar

A selection inside one paragraph shows a bar over its start (under its end when there is no room above): bold, italic, highlight, code, strike and a link to a note. A button runs the same command as its key, so it is one undo step. The bar never takes the keys from the text, and `editor.format_bar = false` turns it off. It is not shown in Neovim mode or with many cursors.

## Tests every app has

- naz, soft: the accent `#ffb040` becomes `#e9b87d`, heading 2 `#ffff88` becomes `#e3e499`, the green `#20ffa0` becomes `#a3ecbf`, the red `#ff5070` becomes `#e1717e`, the grey `#a8a8a0` stays.
- With no setting, the text size is 16 and the measure is 35 em.
- A heading in the editor is 1.6 times the text size at level 1.

## Where it stands

| Part | macOS | Linux | Android |
| --- | --- | --- | --- |
| Reader in the new look | done | done | done (its own Compose reader: headings, callouts, highlights, italic and bold) |
| `appearance.dark_palette` | done | done | done (Settings, Reading, Soft dark colours) |
| New default size and measure | done | done (the editor has no measure yet) | not done: the phone has its own text scale and line width |
| Editor: sizes, weights, italic, line height | done | done (branch) | not started |
| Editor: highlight, callout, quote, list markers and current line in the new look | done | done (a quote and a callout are a tint behind each line, since the view draws no panel) | not started |
| Marks of one element at a time | done | done (branch) | not started |
| Format bar | done (not in Neovim mode) | done (not in Neovim mode) | not started |
| Caret in the accent, with a glide | done (Neovim mode's cursor slides already) | not started (GtkTextView draws its own caret) | not started |
| Editor: task box and date chips, a table as a grid | not started | not started | not started |
| Reader: section bar, table header and sort, footnote popover | not started | not started | not started |
| One palette file with generators | not started | not started | not started |
