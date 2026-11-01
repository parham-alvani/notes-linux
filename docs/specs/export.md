# Daftar export: a note as a PDF, by `daftar-cli export`

Made by `daftar-cli export` (`notes-linux`, `typst_note.rs` in the core and `export.rs` and `export.typ` in the CLI). The apps do not make the PDF themselves: Export as PDF runs this command, so a script, CI and every app make the same file. Linux runs it from Ctrl+Shift+E (`crates/daftar/src/export.rs`); the Mac's ⇧⌘E runs it the same way (`PDFExport.swift`), and Android has no export.

## 1. The command

- `daftar-cli export <note> [-o <file>] [--logo <file>] [--no-folder-logo]`, with the global `--vault`. The note is its vault path or its name, as the other commands take it. The PDF goes to `-o`, else `<title>.pdf` in the current folder; the command prints its path.
- It is written through a temporary file and a rename, as every write of Daftar's. It never writes in the vault unless `-o` points there.
- Typst is inside the binary, with its fonts: Vazirmatn (Regular and Bold) for the text, DejaVu Sans Mono for code and symbols, New Computer Modern Math for math. So are MiTeX's Typst scope, for math, and a Mermaid renderer. Nothing is installed or fetched, and the same note makes the same bytes on every computer on one day: the PDF's date is the day of the export, with no time. `$SOURCE_DATE_EPOCH` (seconds) sets that day, for a build that must make the same bytes on another day.
- A failure (a logo that is not a file, a note that is not there, Typst's own error) is one line on stderr and a non-zero exit, and no PDF is written.

## 2. Settings

In `~/.config/daftar/config.toml`, the file both apps read:

```toml
[export]
logo = ""     # an SVG or PNG at the head of every PDF; `~` is the home folder
footer = ""   # the words at the foot of every page, beside the page count
```

`--logo` takes the place of `export.logo` for one run.

## 3. The page

- A4, the reader's Daylight colors. A bar along the top edge of every page.
- Paragraphs, list items and quotes are justified, as TeX sets them, with English words hyphenated; a Persian paragraph is justified from the right. A table's cells and code are not.
- The head of the first page: the person's logo (`export.logo`), the note's title (its file name) and under it the folder's name and `Changed` and the day the note last changed (its last commit, else its file's time), then the folder's logo. A line under them.
- The folder's logo is the `logo.svg` or `logo.png` of the folder nearest the note, going up; the vault's top is never one. In a vault of companies, `Companies/Acme/Platform/Queue.md` gets `Companies/Acme/logo.svg` and the subtitle `Acme`. `--no-folder-logo` leaves it out.
- Every page after the first: the title and the subtitle at its head. Every page: `export.footer`, `Exported` and the day of the export, and `n / N` at its foot.
- A first `# Heading` that says the title again is left out: the head has it.

## 4. The note

- Headings, paragraphs, bold, emphasis (in the reader's color: Vazirmatn has no italic), strike, inline code, fenced code (highlighted by its language), tables with their alignment, ordered and unordered lists, quotes, rules, footnotes (at the foot of their page) and web links (which stay links).
- `[[Note]]`, `[[Note|text]]` and `[[Note#Heading]]` are their text in the link color (`Note › Heading`); they do not link.
- `![[picture.png]]`, `![[picture.png|300]]` (a width in pixels) and `![](path)` draw the picture when the vault has it, never wider than the page. An embed of a note or of anything else is its name, after `↪`.
- `> [!kind] Title` is a box in the kind's color (tip and success green, warning and question amber, danger and bug red, example purple, quote grey, the rest blue), with its title or the kind's name.
- A task (`- [ ]`, `- [x]`, and any other status, `[/]`, `[-]`) is its status in a box: done is ticked and greyed, cancelled greyed. The Tasks plugin's signs are words (`📅` due, `✅` done, `⏫` high, …); other emoji are left out, as no font of the PDF has them.
- `==text==` is marked, `%%comments%%` are left out (not in code), and the front matter is left out.
- A paragraph, heading, list or table whose first strong letter is Persian, Arabic or Hebrew is set right to left; a title in Persian too.
- `$math$` and `$$math$$` are set as math: MiTeX turns the TeX into Typst's math, in New Computer Modern Math, left to right in a Persian paragraph too. TeX that MiTeX or Typst cannot read (a command neither knows) stays its TeX as code, and the rest of the note is made as before.
- A ` ```mermaid ` block is drawn (mermaid-rs-renderer, in Vazirmatn, centred and never wider than the page). Its labels are sized by the renderer's own table, never by the fonts the computer has, so the bytes stay the same. A block that does not parse stays its text as code.

## 5. In the apps

- Export as PDF works on the note on screen, in the reader or the editor; the editor saves first, as the command reads the file.
- The app asks where to save, then runs `daftar-cli --vault <vault> export <note> -o <file>`: the `daftar-cli` beside the app's own binary (the packages install them together), else the one on `$PATH`; on the Mac, else Homebrew's or Cargo's (`/opt/homebrew/bin`, `/usr/local/bin`, `~/.cargo/bin`, `~/.local/bin`), as an app opened from the Finder has no Homebrew on its `$PATH`. It says "Saved <name>." when the command succeeds, and the command's last line of stderr when it fails. When there is no `daftar-cli`, it says so.

## 6. Tests

`text_goes_in_as_strings`, `a_string_escapes_what_typst_reads`, `headings_emphasis_and_code`, `wiki_links_are_their_text`, `pictures_are_drawn_and_numbered_once`, `tasks_keep_their_status_and_signs_become_words`, `a_callout_is_a_box_with_its_title`, `persian_is_set_right_to_left`, `front_matter_and_comments_are_left_out`, `highlights_tables_and_footnotes`, `math_and_mermaid_are_numbered_for_the_cli` and `the_nearest_folder_with_a_logo` in the core; `a_note_with_both_logos_and_a_picture_is_a_pdf` (and that it makes the same bytes twice), `the_day_is_source_date_epochs_else_today`, `math_and_diagrams_are_drawn_and_what_does_not_parse_stays_text` and `a_logo_that_is_not_there_is_refused` in the CLI.
