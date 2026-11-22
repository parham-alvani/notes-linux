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

## Part 2: what it adds, and the rules every part keeps (part 2)

Part 2 draws more of the note in the editor (the task box, date chips, a table as a grid, callouts and code blocks, a slash bar, focus mode), adds a link preview to the reader and the editor, and gives the reader a section bar, a header that stays, a sort, a footnote popover and a soft change. It also brings Linux's caret and measure and the phone's editor up to part 1.

Four teams build it: macOS, Linux, Android, and the reader page (`web/src`, changed in `notes-mac/web` and copied to `notes-linux/web`, so the Mac and Linux get the reader parts from one change).

**A drawn part is a way to see the text, never the text.** These rules hold for every part below:

| Rule | What it means |
| --- | --- |
| The file | The buffer and the file hold only the Markdown. A box, a chip, a grid line, an icon or a panel is never a character of the text. Copy, cut, find, undo and save work on the raw text. |
| Inside | The caret is in a range from before its first character to after its last, both ends counted. A selection that touches the range counts too. This is part 1's rule for marks. |
| Raw | While the caret is inside, the drawn part goes and the raw characters show in the editor's ordinary colours. When the caret leaves, the part is drawn again with no delay. |
| Find | A match inside a drawn part shows that part raw while it is the current match. |
| Off | `editor.live_preview = false` turns off every drawn part of the editor (sections 1 to 4). The colours of part 1 stay. |
| Not Markdown | Code blocks, inline code, the front matter, `$$` math and `%%` comments hold no drawn part: a `- [ ]` or a `📅` there is plain text. |
| Cost | A key press styles its paragraph (or its table, callout or code block) again and nothing else. No part walks the whole note on a key. A part is drawn only where the note is styled, which is where it is seen. |
| Neovim mode | The drawn parts of sections 1 to 4 and focus mode work in Neovim mode. A click that changes the text is sent to Neovim as a change of those lines, never written to the view. The slash bar is not there. |
| Many cursors | With more than one cursor the drawn parts stay, each raw where a cursor is inside. The slash bar is not shown. |
| A read-only vault | Everything is drawn. A click that would change the text does nothing. |
| Units | 1 em is the text size (`appearance.text_size`, 16 by default). A px is a point on the Mac and a dp on the phone. A percent after a colour is its alpha. |
| Motion | Every fade and slide named below is off under Reduce Motion (macOS Reduce Motion, GTK's `gtk-enable-animations = false`, Android's animator scale 0, the page's `prefers-reduced-motion`): the change is made at once. |

When a toolkit cannot draw a part as written, the app builds the nearest thing and the "Where it stands" table says what it is.

### Settings of part 2

Each follows `vault-config.md`: a shared key may be set by a vault and is written to the vault's shared section when changed in the app; it gets its line in each app's template, in the allow-list and in `vault-config.md`'s tables in the change that adds it.

| Key | Default | Value | Kind | What it does |
| --- | --- | --- | --- | --- |
| `editor.live_preview` | `true` | boolean | shared, exists | Now also the switch of sections 1 to 4. The phone reads it from part 2 on. |
| `editor.format_bar` | `true` | boolean | shared, exists | The phone reads it from part 2 on. |
| `editor.slash_menu` | `true` | boolean | shared, new | The slash bar (section 5). `false`: a `/` is only a `/`. |
| `editor.focus` | `false` | boolean | shared; the Mac has it today as its own key, and it becomes shared | Focus mode (section 6). A value a Mac already wrote to `[macos.editor]` is still read first, by the platform table's rule. |
| `appearance.link_preview` | `true` | boolean | shared, new | The link preview (section 7), in the reader and the editor. The footnote popover is not under it. |
| `appearance.section_bar` | `true` | boolean | shared, new | The reader's section bar (section 8). |
| `appearance.text_width` | 35 | 0 to 400 | shared, exists | Now the editor's measure on Linux too (section 9). 0 is no limit. |

The table sort, the header that stays, the footnote popover, the soft change, the caret and the date chips have no setting. None of the new keys names a program or a path, so none is refused from a vault.

## Editor: the task box (part 2)

**When a line is a task.** A line is a task when it is, in this order: any indent, any number of `>` quote marks each with its space, a list marker (`-`, `*`, `+`, or a number with `.` or `)`), one space, `[`, exactly one character, `]`, one space. This is `TaskLine`'s rule. The box is drawn as soon as that last space is typed, so `- [ ]` with nothing after it is still raw.

**What the box covers.** With a `-`, `*` or `+` marker, the box takes the place of the marker, its space and the brackets (`- [ ]`, five characters). With a number, the number stays as the list marker and the box takes the place of the brackets alone (`[x]`). The space after `]` stays a space.

| Part | Rule |
| --- | --- |
| Size | 1.1 em square, its bottom 0.18 em under the baseline. The five raw characters take the width of the box and no more. |
| Drawing | The status's icon from the reader's table (`tasks.css`), filled with the status's colour. The icon is a mask, so it has one colour. |
| Pointer | A pointing hand over the box. The box grows to 1.12 times its size while the pointer is on it (not under Reduce Motion). |
| Raw | The caret inside `- [ ]` (offsets 0 to 5 of `- [ ] Buy`) shows the raw marks. At offset 6, the start of the text, the box is drawn. |

**The drawing of each status.** The symbol is looked up in the reader's table first; a symbol that is not there is drawn by its type from the vault's statuses (`task-reasons-and-statuses.md`, section 2). The reader takes the same rule for a symbol its table lacks, so the two agree.

| Status | Icon (Lucide) | Colour | The line's text |
| --- | --- | --- | --- |
| ` ` Todo, and any TODO type | `square` | list marker | as written |
| `x`, `X` Done, and any DONE type | `square-check-big` | done | the done colour, struck through with a line in the faint colour |
| `/` In Progress, and any IN_PROGRESS type (`r` In Review) | `square-dot` | accent | as written |
| `-` Cancelled, and any CANCELLED type | `square-x` | done | the done colour, struck through |
| `>`, `<`, `!`, `?`, `*`, `"`, `i`, `b`, `l`, `I`, `S`, `f`, `k`, `w`, `u`, `p`, `d`, `c` | the reader's icon for the symbol | the reader's colour for the symbol | as written |
| NON_TASK type, or a symbol the vault does not have | `square-dashed` | list marker | as written |

The strike never covers the date chips or the `[reason:: …]` field, as in the reader.

**A click or a tap on the box** changes the status as the app's task toggle does (Ctrl+Return on Linux, the same command on the Mac; `EditorCommands.toggleTask`, which is `TaskLine.toggled`): an open task becomes `x` with `✅ today`, a repeating one leaves its next occurrence on the line above, a closed one becomes ` ` and loses its ✅ or ❌ date, and a reason goes with the status it was given with. The rules are section 3 of `task-reasons-and-statuses.md`; the click adds none.

| Detail | Rule |
| --- | --- |
| Undo | One click is one undo step. |
| The caret | It stays where it was. When lines were added above (a repeating task), it stays on the same text. The click does not move the caret into the box, so the box does not turn raw. |
| Focus | The click does not take or give the keyboard focus. On the phone a tap does not open the keyboard. |
| Other buttons | A right click (a long press on the phone) opens the status menu the app already has for a task, when it has one. A click with Shift, Ctrl, Alt or ⌘ is an ordinary click in the text. |
| Touch | The target on the phone is at least 40 dp square around the box. |
| The file | The editor's text changes, and the save is the editor's ordinary save. The click never writes the file by itself. |

Settings: `editor.live_preview`.

## Editor: date chips (part 2)

**Which text is a chip.** On a task line (the rule above), each of `➕`, `⏳`, `🛫`, `📅`, `✅`, `❌` followed by one space and a real day written `YYYY-MM-DD` is a chip, from the emoji to the last digit. `🔁` is a chip from the emoji to the last character of its rule: the rule ends before the next of these emoji, before a priority emoji, before a `^block` id, or at the line's end, with the spaces at its end left out. A date that is not a real day (`📅 2026-02-30`) is not a chip and stays as written; the audit names it. A date on a line that is not a task is not a chip.

**How a chip is drawn.** Nothing is hidden: a chip is the emoji and the date as written, with a pill behind them.

| Part | Rule |
| --- | --- |
| Text | 0.92 em, in the chip's colour. |
| Pill | The chip's colour at 14%, corner 1 em (a full pill), 0.5 em of room at each side, no border. Its height is the text's line, not the row's. |
| Between chips | 0.3 em. |
| Direction | A chip reads left to right in a right-to-left line too: `2026-10-30`, never `30-10-2026`. |
| On a closed task | The chips are drawn at 75% and are not struck through. |
| Raw | The caret inside the chip shows the text at the full size with no pill and the ordinary colour. |
| A click | An ordinary click in the text: the caret goes there, so the chip turns raw. |

| Chip              | Colour                          |
| ----------------- | ------------------------------- |
| `📅` due          | orange                          |
| `📅` due and late | the red, weight 600             |
| `⏳` scheduled    | blue                            |
| `🛫` start        | purple                          |
| `➕` created      | muted text                      |
| `✅` done         | spring green                    |
| `❌` cancelled    | the red, at the ordinary weight |
| `🔁` repeat       | pink                            |

**Late.** A due date is late when its day is before today and the task is open (its status type is TODO or IN_PROGRESS). A task due today is not late. Only `📅` is ever late. "Today" is the computer's day; the styled lines on screen are styled again when the day changes, and nothing is checked on a key press.

The priority emoji are not chips in part 2.

Settings: `editor.live_preview`.

## Editor: a table as a grid (part 2)

**Which lines are a grid.** A header row, then a delimiter row (`| --- | :-: |`, `MarkdownTable.isDelimiter`), then any number of rows, each a line that starts with `|` after its indent (`MarkdownTable.isRow`). The lines must start at the line's start or after spaces only: a table inside a quote, a callout or a list item is not a grid in part 2 and is drawn as before. A header with no delimiter row yet is not a grid. A table of more than 200 rows is not a grid.

**How it is drawn.** As the reader's table.

| Part | Rule |
| --- | --- |
| Box | One hairline (the table hairline colour, 1 px) around the table, 12 px corner. The box is as wide as its columns, never wider than the measure. |
| Header row | On the panel colour, weight 600, the text colour. |
| Lines | A 1 px hairline under each row but the last and between columns. No stripes. |
| Cell | 0.35 em of room above and below, 0.7 em at each side. The text is in the text font, with part 1's inline styles and marks rule. |
| Pipes | Not drawn: a pipe is the hairline between two columns. An escaped `\|` is drawn as a plain pipe. |
| Padding spaces | The spaces the formatter puts in a cell are not drawn. A column is as wide as its widest cell as drawn. |
| Delimiter row | Not drawn as text: it is the hairline under the header. Its colons set each column's side: `:--` start, `:-:` centre, `--:` end. |
| Wide table | A table wider than the measure scrolls sideways in its own box; the text around it does not move. |
| Direction | Columns go in the order the reader gives the same table. A cell's text takes its own direction. |

**When the table shows raw.** The caret in a cell does not show the pipes: a cell is edited in place. The whole table shows raw, in the code font, while a selection reaches outside one cell, and while the caret is on the delimiter row (reached with the arrow keys). With `editor.live_preview = false` it is always raw. Raw, the keys below still work, as they do today.

**Keys, with the caret in a table** (grid or raw):

| Key | What it does | The text |
| --- | --- | --- |
| Tab | To the next cell, the caret at the start of its text; from a row's last cell to the first cell of the next row; the delimiter row is passed over. In the table's last cell, a new empty row is added at the end and the caret goes to its first cell. | The table is formatted (`MarkdownTable.tab`, as today). |
| Shift+Tab | To the cell before; from a row's first cell to the last cell of the row above. In the first header cell it stays. | Formatted. |
| Enter | To the same column of the row below. On the header row, to the first row under the delimiter row. On the last row, a new empty row is added at the end and the caret goes to its same column. | Formatted. This replaces today's Enter, which put a new row under the caret's row. |
| Enter on a last row whose cells are all empty | The table ends: that row is removed and the caret is on an empty line under the table. | As today. |
| Shift+Enter | The same as Enter. A cell never holds a line break. | As Enter. |
| A pipe typed in a cell | The pipe is typed as it is. The row has one more cell, and the next Tab or Enter gives every row that cell. | The formatter fills short rows. |
| Arrows, Home, End | As in any text, by drawn line. Left and Right at a cell's edge go to the next cell's text, over the pipe and the padding. | No change. |

Each of Tab, Shift+Tab and Enter is one undo step, with its formatting. Formatting is the existing table formatter (`MarkdownTable.format`: each column as wide as its widest cell, at least three, short rows filled), so the file is a normal Markdown table after every move. Format on save (`daftar-cli fmt`) has the last word on the file, as for any text. A key in a cell styles that table again and no other line.

A click in a cell puts the caret at the nearest place in that cell's text; a click on an empty cell puts it between the cell's two padding spaces.

Settings: `editor.live_preview`.

## Editor: callouts and code blocks as the reader draws them (part 2)

### Callout

A callout is a run of `>` lines whose first is `> [!kind]`, with `+` or `-` and a title after it or not.

| Part | Rule |
| --- | --- |
| Panel | The kind's colour at 10%, with a 1 px hairline of the kind's colour at 26% around the whole callout, 12 px corner. One panel for the run, as wide as the measure. No bar. |
| Room | 0.6 em above the title and under the last line, 1 em at each side. |
| Kind | The colour and the icon are the reader's for the kind (`reader.css`, `data-callout`). A kind it does not know is drawn as `note`. |
| Title line | The kind's icon, 1.1 em square in the kind's colour, 0.4 em before the title. The title is in the kind's colour, weight 650. With no title, the kind's name is drawn with its first letter in capital (`> [!tip]` reads "Tip"); that name is drawn only, never written. |
| Body | The text colour, with every part 1 and part 2 style inside it (a task box in a callout is drawn and can be clicked). |
| Marks | `> ` at the start of each line and `[!kind]` on the first are drawn at no width. They show on the caret's line only, as a heading's `#` does. |
| `+` and `-` | Drawn as nothing. The body is always shown in the editor: a callout does not fold there. |
| A quote | A `>` run with no `[!kind]` stays part 1's quote: the 3 px bar, nothing behind. |

A new line in a callout continues it as today (`> ` is written by Enter). An empty `>` line stays in the panel.

### Code block

A code block is from its opening fence (three or more backticks or `~`) to its closing fence.

| Part | Rule |
| --- | --- |
| Panel | The code background, 12 px corner, as wide as the measure, one panel from the opening fence to the closing fence. No border. |
| Room | 1.1 em at the start side of the text, 0.6 em at the end side. |
| Opening fence | The fence characters are drawn at no width. The language word is drawn at the line's start in the language colour: the code font, 0.72 em, weight 600, in small letters. With no language the line is empty. No pill, no bar. |
| Closing fence | Drawn at no width, and its line is 0.4 em high. |
| Body | The code font at 0.85 em, the code text colour, highlighted as today. It reads left to right always. |
| Raw | The caret on a fence line shows that line as written (` ```rust `) at the full height. The body never changes with the caret. |
| What it is not | No copy button. A `mermaid`, `tasks` or `math` block is drawn as a code block: nothing is rendered in the editor. |

An unclosed fence runs to the end of the note, and so does its panel.

Settings: `editor.live_preview`.

## Editor: the slash bar (part 2)

The Mac and Linux have a slash menu today (`EditorCommands.slashCommands`, `opensSlashMenu`, `apply`). Part 2 makes it one thing in both: a small list that is filtered as the person types and never takes the keys.

**When it opens.** A typed `/` opens it when the `/` is the first character of its line or comes after a space or a tab (`opensSlashMenu`), so a path or a web address never opens it. It does not open in a code block, inline code, the front matter, math or a comment, in Neovim mode, with many cursors, in a read-only vault, or with `editor.slash_menu = false`. A pasted `/` does not open it.

**The list.**

| Part | Rule |
| --- | --- |
| Place | Under the `/`, 4 px away, its start edge on the `/`. Over it when there is no room below. It stays inside the editor by 8 px. |
| Size | 280 px wide. At most 8 rows show; the rest scroll. A row is 32 px high. |
| Look | The background colour, a 1 px hairline, 12 px corner, a soft shadow. The chosen row has the accent at 14% behind it, 6 px corner. No border in the accent. |
| A row | The item's icon, 16 px, in its colour; its title in the text colour; its group at the end side in the faint colour, 0.82 em. |
| Focus | The list never has the keyboard focus. Every key goes to the text first. |

**The items**, in this order. The id is what a test names; the text is what is written, with `▮` for where the caret goes.

| Id | Title | Group | Text |
| --- | --- | --- | --- |
| `h1`, `h2`, `h3` | Heading 1, Heading 2, Heading 3 | Text | `# ▮`, `## ▮`, `### ▮` |
| `task` | Task | Lists | `- [ ] ▮` |
| `task-today` | Task due today | Lists | `- [ ] ▮ 📅 <today>` |
| `bullet`, `number` | Bulleted list, Numbered list | Lists | `- ▮`, `1. ▮` |
| `quote` | Quote | Blocks | `> ▮` |
| `callout` | Callout | Blocks | `> [!note] ▮`, then a line `> ` |
| `note`, `info`, `tip`, `important`, `warning`, `danger`, `example`, `question`, `success`, `summary`, `todo` | Note callout, Info callout, … (the kind, then "callout") | Blocks | `> [!<kind>] ▮`, then a line `> ` |
| `code` | Code block | Blocks | a line of three backticks, the line `▮`, a line of three backticks |
| `mermaid`, `math`, `tasks` | Mermaid diagram, Math block, Tasks query | Blocks | as today |
| `table` | Table | Blocks | `\| ▮Column \| Column \|`, the delimiter row, one empty row, formatted |
| `divider` | Rule | Blocks | `---` and a new line, the caret under it |
| `link` | Link to a note | Links | `[[▮]]`, and the `[[` list of notes opens |
| `embed` | Embed a note or a file | Links | `![[▮]]` |
| `date` | Today's date | Insert | `<today>` as `YYYY-MM-DD`, the caret after it |
| `time` | The time now | Insert | `HH:MM` |
| `red`, `orange`, `yellow`, `green`, `blue`, `purple` | Red highlight, … | Highlight | `==🔴▮==`, … (yellow is `==▮==`, the default colour) |

The callout kinds put the caret in the title, so a title can be typed at once; today's four leave it in the body, and that changes. The `yellow` item writes no `🟡`, which draws as the default. The Group column is the row's faint word and, with nothing typed, the header over each group.

**Filtering.** What is typed after the `/` up to the caret is the query. The list shows the items whose title or id matches it (`Fuzzy.matches`, the palette's rule), best first, and in the list's order when two match the same. With an empty query every item shows, under its group's header; with a query there are no headers. When nothing matches, the list hides and still listens: one Backspace can bring it back. The first row is chosen whenever the list changes.

| Key | What it does |
| --- | --- |
| Letters, digits, `-` | Go into the text, and filter. |
| Down, Up (and Ctrl+N, Ctrl+P) | Choose the next or the row before. They stop at the ends. |
| Enter, Tab | Insert the chosen row. With the list hidden, the key is an ordinary key. |
| Escape | Closes the list. The `/` and what was typed stay in the text. The `/` does not open the list again until a new `/` is typed. |
| Space | Goes into the text and closes the list. |
| Backspace | Deletes a character. When it deletes the `/`, the list closes. |
| Left, Right, or a click in the text | Closes the list when the caret leaves the place between the `/` and the end of the query. |
| A click on a row | Inserts that row. The focus stays in the text. |

**Inserting** replaces the `/` and the query with the item's text, as one undo step (`EditorCommands.apply`). A block item (every one of group Text, Lists and Blocks) typed after other text on the line starts a line of its own. Nothing else of the line changes.

Settings: `editor.slash_menu`.

## Editor: focus mode (part 2)

Focus mode dims every paragraph but the caret's. It changes how the text is drawn and nothing else: no text, no style kept in the note, no undo step.

| Part | Rule |
| --- | --- |
| The setting | `editor.focus`, `false` by default, shared. |
| The command | "Turn On Focus Mode" and "Turn Off Focus Mode" in the command palette, group View. It changes the setting, so it goes where a changed setting goes. It has no key by default. |
| The paragraph | The run of lines around the caret with no blank line in it (`FocusParagraph.range`). On a blank line, that line alone. So a list with no blank line, a table, a callout and a quote are each one paragraph. |
| A selection | Every paragraph the selection touches stays bright. With many cursors, each cursor's paragraph. |
| Dimmed text | Drawn at 45%: each colour keeps its hue and gets 45% of its alpha. Where a toolkit can only set one colour, the text colour at 45% for all dimmed text. |
| Dimmed drawings | A tint, a panel, a box, a chip, a grid and a picture of a dimmed paragraph are at 45% of what they were. |
| The current line | Part 1's tint stays. |
| The change | When the caret goes to another paragraph, the old one dims and the new one brightens over 120 ms. |
| Where | The editor only, Neovim mode too. The reader is never dimmed. |
| Cost | Nothing happens on a key that leaves the caret in its paragraph, apart from that paragraph growing. A move changes two ranges. The dim is laid only where the note is styled. |

Settings: `editor.focus`.

## Link preview (part 2)

Resting the pointer on a link to a note shows a small card with the note's title and its first lines. The reader has such a card today (after 450 ms); part 2 sets one rule for the reader and the editor.

**When.** The pointer rests on a `[[link]]` (with an alias, a heading or a block id too) for 350 ms without moving off it. In the editor the link may be drawn or raw. It is not shown for an embed (`![[…]]`), for a link to a file that is no note, for a web link, inside another card, while a mouse button is down, or with `appearance.link_preview = false`. In the editor it is not shown when a key was pressed since the pointer last moved, so typing under a still pointer never opens it. A long press on the phone is out of scope.

**What it shows.**

| Part | Rule |
| --- | --- |
| Head | The note's title, weight 700, in the accent. After it, `› Heading` in the heading 3 colour when the link names a heading. At the end side, the note's folder as `A › B` in the faint colour, 0.82 em. The head is on the panel colour with a hairline under it and stays while the body scrolls. |
| Body | The note's first lines (`LinkPreview.excerpt`, below), at 0.9 em. The reader draws them as it draws a note; the editor draws them with the editor's styles, marks hidden. |
| A link to a heading | The first lines of that heading's section, without the heading itself. |
| A link to a block | That block alone. |
| No such note | The head is the link's name in the faint colour, and the body says "No note yet." in italic. |
| An empty note | The body says "The note is empty." in italic. |

**The first lines** are one pure rule, the same in every app: the note's text without its front matter; without a first heading whose text is the note's title; from the first line that is not blank; up to 12 lines or 600 characters, whichever comes first, cut at a line's end when one is in reach, with `…` when text was left out. A code block that is cut is closed. An embed in those lines is shown as its link, not followed.

**The card.**

| Part | Rule |
| --- | --- |
| Size | 360 px wide, or the view's width less 16 px when that is less. At most 240 px high; a longer body scrolls with the wheel. |
| Place | Under the link, 6 px away, its start edge on the link's. Over the link when there is no room below. Inside the view by 8 px. |
| Look | The background colour, a 1 px hairline in the table hairline colour, 12 px corner, a shadow of `0 14px 40px` black at 35%. No ring and no border in the accent. |
| Focus | The card never takes the keyboard focus, and no key goes to it. The pointer may go onto it, and the wheel scrolls it. |
| It opens | With a fade of 140 ms. |
| It closes | 250 ms after the pointer has left both the link and the card; at once on any key, on a scroll of the note, on a click outside it, when the note changes, and when the window loses the focus. |
| A click | On the card's head opens the note, as a click on the link does. A click in the body does nothing. Links in the body are not live. |

Settings: `appearance.link_preview`.

## Reader: section bar, table header and sort, footnote popover, soft change (part 2)

These are the reader's: the page (`web/src`) for the Mac and Linux, and the Compose reader on the phone.

### Section bar

| Part | Rule |
| --- | --- |
| What it shows | The heading path of the place being read: the last heading above the line 90 px under the top of the view, with its parents before it. A parent is the nearest earlier heading of a lower level, so `# A`, `### C` gives "A › C". |
| When | Only when there is such a heading and it has left the view. At the top of a note, and in a note with no heading, there is no bar. A heading folded away is not counted. |
| Place | At the top of the reading view, over the text, as wide as the view. It stays while the note scrolls. The reading progress line is at its bottom edge. |
| Size | 28 px high. The text is 0.8 em, on one line, at the measure's start edge. |
| Look | The background colour at 92% with a 1 px hairline under it. No shadow. The last heading is in the text colour, weight 600; its parents are in the muted colour; `›` between them is in the faint colour with 0.4 em at each side. |
| Long path | One heading shows at most 24 em and ends in `…`. When the path is wider than the bar, parents are left out from the start and `…` is put there. The last heading always shows. |
| Direction | Each heading takes its own direction. The path reads from the side the first heading reads from. |
| A click | On a heading goes to that heading: it comes to the top of the view, under the bar, with the smooth scroll the reader has, and the reading cursor goes to it. The bar takes no keyboard focus and has no key. |
| It comes and goes | With a fade of 120 ms. |
| Not shown | In a preview card, an embed, print and export, and with `appearance.section_bar = false`. |

### A table header that stays

While any body row of a table is in the view and its header row would be above the view, the header row is drawn at the top of the view (under the section bar when it shows), on the panel colour, not see-through, with the hairline under it. It leaves with the table's last row. A table wider than the view still scrolls sideways, and its header moves sideways with it. This is for a note's tables; the properties table has no header, and a base keeps its own.

### Sorting by a column

| Part | Rule |
| --- | --- |
| A click on a header cell | Sorts the body rows by that column: the first click from small to large, the second from large to small, the third back to the file's order. A click on another column starts at the first. |
| The mark | `↑` (small to large) or `↓` after the header's text, in the accent, 0.8 em. No mark in the file's order. The pointer is a hand over a header cell. |
| A cell's value | Its text as drawn (a link's shown text, no marks), with the spaces at its ends left out. |
| A column of numbers | When every cell that is not empty is a number (digits with an optional sign, one `.`, `,` between thousands, and a `%` at the end), the rows are ordered by the number. |
| Any other column | By the text, with capital and small letters as the same. A date written `YYYY-MM-DD` is in order this way. |
| Empty cells | Last, in both directions. |
| Ties | Keep the file's order. |
| A view only | The file is not changed, and nothing is written. A task box in a sorted row still changes its own line. |
| How long it lasts | While the note is shown, and through a redraw of the same note when the table (counted from the note's start) has the same header texts. It is gone when another note is shown. |
| Not for | The properties table, a base (it has its own sort), and a table inside a preview card. |

### Footnote popover

A footnote reference (`[^1]`) shows its footnote where the reference is, so there is no jump to the end of the note.

| Part | Rule |
| --- | --- |
| It opens | When the pointer rests on the reference for 350 ms, and at once on a click or a tap on it. A click no longer scrolls to the footnote. |
| What it shows | The link preview's card: the head says "Footnote" and the reference's label (`Footnote 1`), the body is the footnote's text as the reader draws it, without its back arrow. Links in it work. |
| It closes | As the preview's card does when it was opened by resting; on Escape, on a click outside it, or on a second click on the reference when it was opened by a click. |
| Focus | It never takes the keyboard focus. |
| On the phone | A tap opens it as a small sheet over the text; a tap outside closes it. |
| No such footnote | Nothing opens, and the click does nothing. |

The list of footnotes at the end of the note stays. `appearance.link_preview = false` does not turn the popover off.

### A soft change

| Change | Rule |
| --- | --- |
| Between two notes | The note that leaves fades out as the new one fades in, 120 ms, with no movement. The new note is drawn at its place first; the fade starts only then. |
| Between reading and editing | The same 120 ms crossfade, at the same line, as part 1 keeps it. The view that comes is drawn first; when it is not ready after 400 ms the change is made at once. |
| The keys | They go to the view that comes from the start of the change. A key during the fade is never lost. |
| No fade | When the same note is drawn again (a change on disk, a tick, a sort, a setting), and under Reduce Motion. |

**Reduce Motion in part 2.** These are all off, and each change is made at once: the box's growth under the pointer, the dim's 120 ms, the card's 140 ms, the section bar's 120 ms, the smooth scroll to a heading (it jumps), and the 120 ms crossfade.

Settings: `appearance.section_bar`, `appearance.link_preview`.

## Linux: the caret and the editor's measure (part 2)

### The caret in the accent

GtkTextView draws its own caret, so the editor hides that one and draws the caret itself, over the text. The Caret row of the Cursors table is the rule; these are its numbers on Linux.

| Part | Rule |
| --- | --- |
| Colour | The accent of the palette in use (`#ffb040` in naz, `#a85200` in Daylight, naz, soft's own). |
| Size | 2 px wide, as high as its line's text. One caret: no second caret for the other direction. |
| The glide | It moves between two places in 70 ms. A move of more than the view's height does not glide. |
| While keys come | Solid. |
| With no key for 650 ms | It fades between 100% and 25% and back, once in 1200 ms, for as long as the view has the focus. GTK's `gtk-cursor-blink = false` keeps it solid. |
| With no focus | Not drawn. |
| With a selection | Drawn at the selection's moving end. |
| Neovim mode | Unchanged: normal mode's block and its slide stay as they are. Insert mode has this caret. |
| Reduce Motion | No glide and no fade: a solid caret. |
| Cost | The caret is drawn from the place the view gives for the cursor. A key press reads no more of the note for it. |

No setting.

### The measure

The editor's line is as wide as the reader's.

| Part | Rule |
| --- | --- |
| Width | `appearance.text_width` em of the text size: 35 by default, so 560 px at size 16. |
| Place | In the middle of the pane: the room at each side is half of what is left. |
| A narrow pane | The room at each side is never less than 32 px; the line is then the pane's width less 64 px. |
| `text_width = 0` | No limit: the line is the pane's width less 64 px. |
| What follows it | A picture, a table's grid, a callout's panel and a code block's panel are laid inside the measure. The margin's marks (problems, line numbers) stay at the pane's edge. |
| When it is worked out | When the pane's width, the text size or the setting changes. Never on a key. |
| Neovim mode | The same. |

Settings: `appearance.text_width`.

## Android: the editor (part 2)

The phone has no editor of a note's text today: it writes only its named edits (a tick, a status, a date, a property). Part 2 gives it one, small, so the rest of this section has a place to be.

**The editor itself.**

| Part | Rule |
| --- | --- |
| Opening | "Edit" in a note's menu opens the note's text at the place being read. Not in a read-only vault, and not for a week note in `Calendar/`. |
| Saving | On Done and on Back, when the text changed. The save is one edit through the phone's ordinary vault write, so it is one commit, as a tick is. |
| The file changed meanwhile | The edit is "this text, when the file is still the text the editor opened". When it is not, nothing is written, the editor stays open, and it says so with two choices: Reload (the file's text; what was typed is dropped after a confirm) and Copy My Text. |
| Leaving | Back to reading at the editor's line, with the soft change. |

**Part 1 on the phone.** The values are part 1's; a size is times the phone's own text size, and the phone's line width stays the phone's.

| Part | Rule |
| --- | --- |
| Sizes | Headings at 1.6, 1.35, 1.12, 1, 1 and 0.875 times the text size, in their level's colour. |
| Weights | Headings 650 (600 for levels 5 and 6), bold 650 in the strong colour. |
| Italic | Slanted, in the text's colour. |
| Line height | 1.65, and 2 for a right-to-left line. |
| Highlight | The highlight colour at 32% under the text; the six colours of `==🔴…==` the same way. |
| Callout | The kind's colour at 10% behind the callout's lines with the 26% hairline and 12 dp corner, the icon and the title as in the callout section above. When the text field cannot draw one panel, a tint behind each line, as Linux did in part 1. |
| Quote | A 3 dp bar in the accent at the side the lines start. |
| List markers | One marker colour at every level. |
| Current line | The text colour at 4.5% behind the caret's line. |
| Marks | The marks of one element show only while the caret is inside it; a heading's `#` on the caret's line (`editor.live_preview`). |
| Format bar | With a selection inside one paragraph: bold, italic, highlight, code, strike and a link to a note. On the phone the bar sits over the keyboard, as wide as the screen, since the system's own bar (cut, copy, paste) is over the selection. A button is 48 dp. A button is one undo step and keeps the selection. `editor.format_bar = false` turns it off. |
| Caret | The accent, 2 dp wide. No glide in part 2. |

**The task box and the date chips** are the two sections above, with a tap for a click. A tap on the box changes the status and does not open the keyboard or move the caret; its target is at least 40 dp. A long press on the box opens the status sheet the phone has for a task.

**Not on the phone in part 2:** a table as a grid, the code block's panel, the slash bar, focus mode and the link preview. The reader's parts (the section bar, the header that stays, the sort, the footnote popover, the soft change) are on the phone.

Settings: the phone reads `editor.live_preview` and `editor.format_bar` (shared) and shows them in its Settings.

## Tests every app has

- naz, soft: the accent `#ffb040` becomes `#e9b87d`, heading 2 `#ffff88` becomes `#e3e499`, the green `#20ffa0` becomes `#a3ecbf`, the red `#ff5070` becomes `#e1717e`, the grey `#a8a8a0` stays.
- With no setting, the text size is 16 and the measure is 35 em.
- A heading in the editor is 1.6 times the text size at level 1.

## Tests every app has (part 2)

Ported by name (snake case in Rust, camelCase in Swift and Kotlin). Each is a pure rule over text, with today given as 2026-11-22, so none needs a window. An app tests only the parts it has.

1. `aTaskBoxIsDrawnWhenTheMarkIsComplete`: `- [ ] ` has a box, and `- [ ]` with no space after has none. `  * [x] a`, `1. [/] a`, `> - [ ] a` and `- [r] a` have one. `- [] a`, `- [xx] a`, `-[ ] a` and `[ ] a` have none. `- [ ] a` in a code block has none.
2. `theBoxCoversTheMarkAndTheBrackets`: in `- [ ] Buy` the box is characters 0 to 5 (`- [ ]`); in `1. [x] a` it is 3 to 6 (`[x]`). A caret at 0, 3 or 5 of `- [ ] Buy` shows the raw marks; a caret at 6 shows the box.
3. `aClickOnTheBoxIsTheTaskToggle`: `- [ ] Call ➕ 2026-11-20` becomes `- [x] Call ➕ 2026-11-20 ✅ 2026-11-22`, and a second click gives the first line back. `- [/] a` becomes `- [x] a ✅ 2026-11-22`. `- [-] a ❌ 2026-11-01` becomes `- [ ] a`. `- [/] a [reason:: waiting]` loses its reason. The result is always what `TaskLine.toggled` gives, and the caret's offset in the text of the line is the same after.
4. `eachStatusHasItsDrawing`: ` ` is `square` in the list marker colour, `x` and `X` `square-check-big` in the done colour, `/` `square-dot` in the accent, `-` `square-x` in the done colour, `>` `square-arrow-right` in blue. With the vault's `r` "In Review" (IN_PROGRESS), `r` is `square-dot` in the accent. `~`, which the vault does not have, is `square-dashed` in the list marker colour.
5. `datesOnATaskAreChips`: in `- [ ] Pay 📅 2026-11-20 ⏳ 2026-11-18 🔁 every week ➕ 2026-11-01` there are four chips: due (`📅 2026-11-20`), scheduled, repeat (`🔁 every week`, with no space at its end) and created. `- [ ] a 📅 2026-02-30` has none. `Meeting 📅 2026-11-20`, which is no task, has none.
6. `aLateDueDateIsRed`: `- [ ] a 📅 2026-11-21` is late; `- [ ] a 📅 2026-11-22` is not; `- [x] a 📅 2026-11-21 ✅ 2026-11-22` and `- [-] a 📅 2026-11-21` are not; `- [/] a 📅 2026-11-21` is. `- [ ] a ⏳ 2026-11-01` has a blue chip, never a red one.
7. `aChipIsRawWithTheCaretInIt`: in `- [ ] a 📅 2026-11-20`, a caret just before `📅`, in the date, or just after the last `0` shows the chip raw; a caret in `a` shows it drawn.
8. `aTableIsAGridOnlyWithItsDelimiterRow`: `| A | B |` over `| --- | --- |` over `| 1 | 2 |` is a grid; the header alone is not; the same three lines each after `> ` are not; a table of 201 rows is not.
9. `enterGoesToTheRowBelow`: in the grid of case 8 with a second row `| 3 | 4 |`, Enter in `2` puts the caret at the start of `4`, and the text is the formatted table. Enter in `4` gives `| A   | B   |`, `| --- | --- |`, `| 1   | 2   |`, `| 3   | 4   |`, `|     |     |` with the caret in the second cell of the last row. Enter in `B` goes to `2`.
10. `enterOnAnEmptyLastRowEndsTheTable`: after case 9, Enter again removes the empty row and leaves the caret on an empty line under `| 3   | 4   |`.
11. `tabMovesBetweenCellsAndAddsARow`: Tab in `1` goes to `2`, in `2` to `3`, in `B` to `1` (the delimiter row is passed over); Tab in the last cell adds an empty row and goes to its first cell; Shift+Tab in `3` goes to `2`, and in `A` it stays. These are `MarkdownTable.tab`'s cases, kept.
12. `aCalloutHasItsKindsLook`: `> [!warning] Mind` over `> the gap` is one panel of two lines, kind `warning`, title "Mind". `> [!tip]` has the title "Tip". `> [!nonsense] x` is drawn as `note`. `> plain` is a quote, not a callout. `> [!note]- x` has the same panel as `> [!note] x`.
13. `aCodeBlockHasItsLanguageWord`: three backticks and `Rust`, a line, three backticks: one panel of three lines with the word `rust`. With no language there is no word. With no closing fence the panel runs to the note's end.
14. `theSlashBarOpensOnlyAtAWordsStart`: a `/` typed at the start of a line and after `a ` opens it; in `a/`, `https:/` and `10/` it does not; in a code block, inline code and the front matter it does not; in Neovim mode and with `editor.slash_menu = false` it does not.
15. `theSlashBarFiltersByTitleAndId`: `/tab` has Table first; `/h2` has Heading 2 first; `/rule` and `/divider` both have Rule; `/call` has Callout before the kinds; `/zzz` has no row, and `/zzz` is still in the text. With nothing typed, the first row is Heading 1.
16. `enterInsertsAndEscapeLeavesTheSlash`: with the line `/task`, Enter gives `- [ ] ` with the caret at its end. With `/warn`, Enter gives `> [!warning] ` and a line `> ` under it, the caret at the end of the first line. With `/rule`, `---` and a new line. With `/tab` and Escape, the line is still `/tab`. With `text /code`, the code block starts on a line of its own.
17. `focusKeepsTheCaretsParagraph`: in `a`, blank, `b`, `c`, blank, `d` with the caret in `c`, the bright range is `b` and `c`; with the caret on a blank line it is that line; with a selection from `a` to `b` it is `a`, the blank line and `b` with `c`. `editor.focus` is `false` with no setting.
18. `thePreviewIsTheFirstLines`: a note named Plan with a front matter, `# Plan`, a blank line and 20 lines `l1` to `l20` gives `l1` to `l12` and `…`. A link `[[Plan#Next]]` gives the lines under `## Next` up to the next heading of that level or higher, without `## Next`. One line of 900 letters gives 600 and `…`. A note that is only a front matter is empty. A name with no note says "No note yet.".
19. `theSectionBarIsTheHeadingPath`: with `# A`, `## B`, `### C`, `## D`: a place under C gives A › B › C; under D, A › D; before A, no bar. With `# A` then `### C`, a place under C gives A › C. A folded-away heading is not in a path.
20. `aColumnSortsByNumbersOrByText`: the cells `10`, `9`, `100` go 9, 10, 100, and on the second click 100, 10, 9. `1,200`, `85%`, `-3` go -3, 85%, 1,200. `b`, `A`, `c` go A, b, c. `10`, `x`, `9` is a text column: 10, 9, x. With an empty cell among `2`, `1`, the empty one is last in both directions. Two rows with the same value keep the file's order. The third click gives the file's order, and the note's text is the same before and after.
21. `aFootnoteShowsWhereItsReferenceIs`: with `a[^1]` and `[^1]: The note.`, the popover's head is "Footnote 1" and its body "The note."; a reference with no footnote opens nothing.
22. `theEditorsLineIsTheMeasure` (Linux): at size 16 and width 35 the line is 560 px; in a pane of 1000 px the room at each side is 220 px; in a pane of 500 px it is 32 px and the line 436 px; with width 0 in a pane of 1000 px the line is 936 px.
23. `theNewSettingsHaveTheirDefaults`: with no setting, `editor.slash_menu`, `appearance.link_preview` and `appearance.section_bar` are `true` and `editor.focus` is `false`. A vault that sets each is followed, and none is listed as refused. `editor.slash_menu = "yes"` is as not there.
24. `aSaveOnThePhoneRefusesAChangedFile` (Android): the editor opened `a`, the file is now `b`: saving `c` writes nothing and the file is `b`. With the file still `a`, the file becomes `c`.

## Where it stands

| Part | macOS | Linux | Android |
| --- | --- | --- | --- |
| Reader in the new look | done | done | done (its own Compose reader: headings, callouts, highlights, italic and bold) |
| `appearance.dark_palette` | done | done | done (Settings, Reading, Soft dark colours) |
| New default size and measure | done | done | not done: the phone has its own text scale and line width |
| Editor: sizes, weights, italic, line height | done | done | not started |
| Editor: highlight, callout, quote, list markers and current line in the new look | done | done (a quote and a callout are a tint behind each line, since the view draws no panel) | not started |
| Marks of one element at a time | done | done | not started |
| Format bar | done (not in Neovim mode) | done (not in Neovim mode) | not started |
| Caret in the accent, with a glide | done (Neovim mode's cursor slides already) | done (drawn by the editor over the text; GTK's own caret is clear) | not started |
| Part 2: editor, the task box | not started | done (no status menu on a right click: the editor has none; in a Persian line whose syntax starts Latin the box is where GTK puts the marks) | not started |
| Part 2: editor, date chips | not started | done (a date in a line with Persian before it is drawn by GTK's bidi order, `30-10-2026`: a tag cannot isolate it) | not started |
| Part 2: editor, a table as a grid | not started | done, as near as GtkTextView goes: the cells are in the code font, lined up by the formatter's spaces, so a column is as wide as its widest cell as written (a cell's hidden marks and smaller code are given back as padding; a Persian or wide-letter cell is not exact, and a row typed in is lined up at the next Tab or Enter); a table wider than the measure is not a grid (the view cannot scroll one part sideways); the colons of the delimiter row do not set a column's side; `\|` is shown as written; Up and Down pass over the delimiter row, which Left and Right reach; a table wholly inside a selection stays drawn | not in part 2 |
| Part 2: editor, callouts and code blocks as the reader draws them | not started | done (code in a callout keeps part 1's tint; a code line that starts Persian is left-aligned but still reads right to left; a quote is still a tint) | not started (the callout; the code block's panel is not in part 2) |
| Part 2: editor, the slash bar | not started (a menu today, not filtered) | done (an item with no Lucide drawing in the reader's bundle has a few letters for its icon) | not in part 2 |
| Part 2: editor, focus mode (`editor.focus`) | not started (it dims today, in the faint colour, as its own key) | done (a veil in the page's colour at 55% over the other paragraphs, which is each colour at 45%; a picture under a dimmed line is not dimmed) | not in part 2 |
| Part 2: link preview, reader and editor | not started (the reader has a card after 450 ms) | editor: done (the body is the editor's own view of the lines, so the head stays while it scrolls); reader: not started (a card after 450 ms) | out of scope |
| Part 2: reader, section bar | not started | not started | not started |
| Part 2: reader, a table header that stays, and the sort | not started | not started | not started |
| Part 2: reader, footnote popover | not started (a card on hover today) | not started (a card on hover today) | not started |
| Part 2: reader, the soft change | not started | not started (reading and editing crossfade today) | not started |
| Part 2: Linux, the caret in the accent and the editor's measure | does not apply | done | does not apply |
| Part 2: Android, the editor with part 1, the task box and date chips | does not apply | does not apply | not started (the phone has no editor of a note's text yet) |
| One palette file with generators | not started | not started | not started |
