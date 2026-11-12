# Problems and format on save: the vault's checks inside the app

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`, where `daftar-cli` cannot run: there the Problems panel and Format Note are not offered). Both features run `daftar-cli`, so the app, `just audit`, `just check`, `just fix`, `just format` and CI report and write the same thing; no app has its own linter or formatter.

The **Problems panel** lists what `just audit` and `just check` report, by note, and mends what has one answer, as `just fix` does. **Format on save** formats a note with `daftar-cli fmt`, as `just format` does, when the editor leaves it.

## 1. The command line

`daftar-cli` 0.23.0 or later. It is found as Export as PDF finds it (`export.md`): beside the app's binary, else on `$PATH`, else Homebrew's or Cargo's folders. Every run:

- has `--vault <root>` before the subcommand, and runs **in the vault's folder**, so a vault-relative path means the same note to every version;
- runs off the UI thread, with stdout and stderr read at the same time (a full pipe must not stop it), and is ended when the work that asked for it is cancelled (another vault opened, the app quits);
- writes stdin, when it has one, so that a command that ends early cannot end the app (`SIGPIPE` ignored for that pipe).

The commands, all JSON Lines on stdout:

| Purpose | Arguments after `--vault <root>` |
| --- | --- |
| Audit | `check --json`, plus `--profile audit` when the vault's `.daftar.toml` has an `[audit]` section (what `just audit` adds) |
| Lint | `lint --json` |
| Formatting | `fmt --check --json` |
| Fix lint | `lint --fix [-- <path>…]` |
| Fix the audit | `check --fix [-- <path>…]` |
| Format files | `fmt [-- <path>…]` |
| Format one text | `fmt --stdin --path=<vault-relative path>`: the note on stdin, the formatted note on stdout; writes nothing |

The paths come after `--`, and `--path=` is one argument, so a note named `-x.md` is a note. Without paths a fix is the whole vault.

**A finding** is one JSON object on a line:

```json
{"tool":"check","level":"error","path":"Folder/Note.md","line":12,"column":3,"code":"callout-body","message":"…","fixable":true}
```

- `tool` is `check`, `lint` or `fmt`; `level` is `error` or `warning`; `path` is the vault path. A line whose tool or level is another word, that has no path, or that is not JSON is skipped, never an error: a newer `daftar-cli` may say more.
- `line` is from 1; 0 (or missing, or not a whole number) is the whole file. `column` is from 1, or `null`; below 1 is `null`.
- `code` and `message` are shown as they are; `fixable` says that the tool's own fix (`lint --fix`, `check --fix`, `fmt`) mends it.
- The exit status is not read for findings: `check` fails when it finds an error, and that is a finding. A run that exits non-zero with no finding and something on stderr failed: the panel says its last stderr line.

**A `daftar-cli` too old.** clap refuses an unknown flag with exit status 2 and `error: unexpected argument '--json' found` (or `'--stdin'`, `'--path'`); a `check` that takes no files refuses the first path the same way. That is not a problem of the vault. The app says, once, "Problems and Format Note need daftar-cli 0.23.0 or later: brew upgrade parham-alvani/tap/daftar-cli." and stops asking that binary in the background until the panel is opened again. With no `daftar-cli` at all it says how to install it.

## 2. When the vault is checked

- **When a vault opens**, in the background, if it keeps `daftar-cli`'s settings: `.daftar.toml`, `.markdownlint.jsonc` or `.prettierrc` at its root. Any other vault is checked only when the panel opens.
- **When files change** (a save, a fix, a pull, another app), two seconds after the last change, once it has been checked before. A missing or too old `daftar-cli` is not asked again at each save.
- **When the panel opens**, and with its Check Again button: at once.
- **One at a time.** The three commands run one after the other, at background priority (they take every core for a moment; typing must not wait). A change while a check runs asks for one more check after it, never a second at once. Opening another vault cancels the check of the one before, and its findings go.
- The findings shown are those of the last check that finished; while a check runs, the old ones stay, with a spinner.

## 3. The panel

Opened from View › Show Problems (macOS ⇧⌘M; Linux Ctrl+Shift+M), the command palette, or the toolbar's count. It sits under the note and does not change the note's view or reload it.

- **Header:** "Problems", the counts in words ("12 errors, 671 warnings", "1 error", "No problems"), a filter (Errors *n* / Warnings *n* / All, All at first), Fix All for the vault, Check Again, and a button to close it. Escape closes it while it has the keys; it takes the keys when it opens.
- **Grouping:** by note. Notes with an error come first, then the others, each part by path in natural order. In a note, errors first, then by line, then by column, then by code. A finding two tools report the same way (same tool, path, line, column, code and message) is listed once.
- **Each note:** its title, its folder, its counts, and Fix All when one of its findings is fixable.
- **Each finding:** an icon and colour by level (red error, yellow warning) that is also said in words, "Line *n*" or "Note" for line 0, the message, the code, and Fix when it is fixable. To a screen reader a row is "Error, line 12: message. code"; Fix is "Fix *code* in *Note*".
- **A click on a finding** opens its note at its line, in the focused pane: in the reader, the reading cursor is put on that line (it marks it); when the pane is editing, the editor's cursor goes to the line's start. Line 0 opens the note where it was being read.
- **With nothing to list** the panel says why: no `daftar-cli`, too old (the notice's text), the check failed (its message), checking, or "No problems" / "No errors" / "No warnings" by the filter.
- **The toolbar** shows a count when there is any finding: the number of errors in red, or else of warnings in yellow; its spoken value is the counts in words.

## 4. Fixes

- **Fix** on a finding runs its tool's fix on its note: `lint --fix -- <path>`, `check --fix -- <path>` or `fmt -- <path>`.
- **Fix All** on a note runs all three on the note, in `just fix`'s order: `lint --fix`, `check --fix`, `fmt`. Fix All in the header runs them on the whole vault.
- **Through the editors' gate:** every open editor saves first (Neovim writes), so the fix reads what was typed and no editor writes older text over it afterwards. After the fix, the note is read again: an editor with nothing unsaved takes the new text, and an editor that was typed into meanwhile keeps its text (nothing typed is ever lost; its next save wins). The app's own fix does not make the note "updated since you read".
- One fix at a time; the Fix buttons wait while one runs. When the fixes end, the vault is checked again at once.

## 5. Marks in the note

In the editor, a line with a finding has a small dot in the margin left of the text, red when one of its findings is an error, else yellow, and its messages ("Error: message (code)", one a line) on hover. The marks are of the file as the last check read it, and move with the text at the next check after a save. They are drawn, never written: the text, its attributes, the cursor and undo do not change. (The reader may mark lines the same way; Daftar for macOS does not yet.)

## 6. Format on save

A setting, `editor.format_on_save` in the `[editor]` table of the config file, `true` at first:

```toml
[editor]
format_on_save = true
```

**When.** The editor formats a note when it **leaves** it, never while it is being typed in:

| Moment | Formats |
| --- | --- |
| The autosave a second after typing stops | never |
| The editor loses the keys and still shows the note (a click in the sidebar) | never |
| Another note opens in the same editor | with the setting, if the note was typed in |
| The editor goes away: back to the reader (macOS ⌘E), the pane, tab or window closes | with the setting, if the note was typed in |
| The app quits | with the setting, if the note was typed in |
| Edit › Format Note | always |

**Typed in** means the file on disk is not the text the editor took when it opened the note (or last took from disk). Opening an unformatted note in the editor and leaving it changes nothing. Only Markdown notes (`.md`, any case) are formatted, never a base, a canvas or a picture.

**How, on leaving.** Once the editor's text is on disk (the plain editor saves; a Neovim that is let go writes, and is waited for), the file is read, formatted with `fmt --stdin --path=<path>`, and, if the result differs and is not empty, written through the editors' gate: the editors save, the file is read again, and it is written only if it is still exactly the text that was formatted. Otherwise (typed again since, a pull, another app) nothing is written. A note left twice is formatted once after the other, never twice at once. Quit waits for the formats under way and for the notes still in an editor, within the time Quit already gives the editors to save.

**Silence.** With no `daftar-cli`, one too old, or a run that fails, leaving formats nothing and says nothing. A too old binary is not asked again at each leave.

## 7. Format Note

Edit › Format Note (macOS ⌥⇧⌘F; Linux Ctrl+Alt+Shift+F) and the command palette, on a Markdown note on screen.

- **In the plain editor:** the text on screen is formatted with `fmt --stdin`; when it comes back, if the editor still shows that note with that exact text, only the one range that differs is replaced, as **one step to undo** (named "Format Note"), the scroll holds and the cursor stays on its letter (after the change it moves with the text; inside it, it goes to the change's end). Then the note is saved. If the text changed meanwhile, nothing is replaced and the app says so.
- **In Neovim mode:** Neovim owns the buffer. The buffer is written first and the file is formatted; then only the run of lines that differs is replaced in Neovim's buffer (`nvim_buf_set_lines` on the smallest block of whole lines: lines `start..<end` become the new ones), which is one step of Neovim's `u` and keeps the cursor, marks and folds outside those lines; Neovim then writes. If the buffer is no longer the text that was formatted, nothing is replaced and the app says so.
- **In the reader:** the file is formatted through the editors' gate, as on leaving.
- A note already formatted: "*Note* is formatted already." No `daftar-cli`, too old, or a failure: one notice with the reason (the install or upgrade line, or the command's last stderr line).

## 8. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin):

1. `findingsAreReadAndWhatIsNotOneIsSkipped`: from the lines `{"tool":"check","level":"warning","path":"Notes/Plan.md","line":12,"column":null,"code":"link-heading","message":"no heading Goals","fixable":false}`, `{"tool":"lint","level":"error","path":"Notes/Plan.md","line":3,"column":5,"code":"MD034","message":"bare URL","fixable":true}`, a blank line, `daftar-cli: 2 errors…`, `{"tool":"check","level":"error","path":"Home.md","line":9,"column":1,"code":"callout-body",…}`, `{"tool":"fmt","level":"error","path":"Notes/Plan.md","line":0,"column":null,"code":"fmt","message":"not formatted","fixable":true}`, a tool `spell`, a level `info`, an object with no path, `{"tool":"lint","level":"warning","path":"Home.md","line":true,"column":-2,…}` and `{not json`: five findings; the second is line 3 column 5, fixable; the fmt one opens at no line; the last has line 0 and no column. Empty output and clap's usage error give none.
2. `notesWithErrorsComeFirstAndErrorsFirstInEach`: those findings group as `Home.md`, `Notes/Plan.md`; Plan's are `fmt`, `MD034`, `link-heading` with 2 errors and 1 warning and it is fixable; a note `A.md` with one warning goes after both; Warnings alone gives `Home.md`, `Notes/Plan.md`; the findings twice are listed once.
3. `countsAreSaidInWords`: 3 errors and 2 warnings say "3 errors, 2 warnings"; then "1 error", "1 warning", "No problems".
4. `formatOnLeaveIsDecidedByTheMoment`: for every moment, a note typed in with the setting on formats only on another note, the editor going, quit, and the command; with the setting off only on the command; a note not typed in only on the command; `Board.canvas` and `Tasks.base` never; `Folder/NOTE.MD` does.
5. `onlyTheLinesThatChangedAreReplaced`: equal texts give nothing; `# T\n*  item\ntext\n` → `# T\n- item\ntext\n` is lines 1..<2 → `["- item"]`; a removed blank line is one line out and none in; an added blank line is an empty range with `[""]`; the lines of `a\nb\n` and `a\nb` are both `["a", "b"]`; and for `x\ny\nz\n`→`x\nY\nz\n`, `""`→`a\n`, `a\n`→`""`, `1\n2\n3\n4\n`→`1\n4\n`, applying the change to the old lines gives the new.
6. `anOldCommandLineIsToldApart`: exit 2 with `error: unexpected argument '--json' found` (or `'--stdin'`) is too old; `'--bad'` is not; exit 1 with the same text is not; `error: unexpected argument 'Notes/Plan.md' found` after giving that path is too old.
7. `theCommandsAreThoseOfJustCheckAuditAndFix`: the audit for `/v` with a profile is `check --json --profile audit`, `lint --json`, `fmt --check --json`, each after `--vault /v`; the vault's fix is `lint --fix`, `check --fix`, `fmt`; `check` on `-x.md` is `check --fix -- -x.md`; formatting `-x.md` is `fmt --stdin --path=-x.md`.
8. With a stand-in `daftar-cli` (a script): the audit lists its findings and runs three commands; an old one gives the upgrade notice once and is not asked again at the next change; Fix All of a note saves the editors first and runs the three fixes in order, then checks again; leaving a note typed in formats its file; leaving a note only read, at the autosave, or with the setting off does not; with an old one leaving writes nothing and says nothing; Format Note in the editor is one step to undo, and says "formatted already" the second time; in Neovim mode the buffer takes the formatted lines and `u` undoes them, and leaving a note formats it only after Neovim wrote it.
9. The robustness line: for made-up JSON-ish text, every finding has a path, a line of at least 0 and a column of at least 1 or none, and grouping keeps each distinct finding once; for made-up notes, applying `changedLines` to the old lines gives the new.
