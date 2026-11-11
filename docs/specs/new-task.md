# New Task: a task made from a form, and its created date

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Everything stays plain Markdown that the Obsidian Tasks plugin (emoji format) still reads.

New Task is the Edit Task form for a task that is not there yet: the same text, status, reason, priority, dates and repeat rule, with two more fields, the note the task goes in and the section of it. The form's dates include **Created** (➕), on a new task and on an existing one alike.

## 1. The line written

A new task is one line, written from the form's fields by `TaskLine.written` (Swift), `TaskLine::written` (Rust) and its Kotlin port:

```markdown
- [ ] Call Sara about the trip ⏫ 🔁 every week ➕ 2026-11-11 ⏳ 2026-11-14
```

- `- [<status>] `, then the text, then the signifiers in the Tasks plugin's order, as Edit Task writes them: the `[reason:: …]` field, the priority, `🔁 rule`, `🏁` `⛔` `🆔`, then the dates `➕ 🛫 ⏳ 📅 ❌ ✅`, each with its `YYYY-MM-DD`. It is exactly what Edit Task's `edited` writes for the line `- [<status>]`.
- The text is trimmed, and its line breaks become single spaces: a task is one line. No text, no task: nothing is written and the form cannot be saved.
- A status that cannot stand in a box (a line break, `]`) is written as ` `.
- **Created is today by default** (`➕ <today>`), as the Tasks plugin's "set created date" does. The person may change it or turn it off. No other date is set by default.

## 2. Where it goes

The task goes in a note of the vault, at the end of a **section**, chosen by the heading's line, not its name (two `## Work` headings are two sections):

- The section of a heading runs to the next heading of its level or above, so its subsections are part of it: the task goes after the section's last line that is not blank, subsections included. A section with nothing in it gets the task right under its heading.
- With no heading chosen, or no heading on that line any more (the note changed while the form was open), the task goes at the end of the note, after its last line that is not blank. Blank lines at the end of the note stay after it.
- The blank-line rules are quick capture's (`QuickCapture.append`): a blank line goes before the task when the line above would take it in, that is a paragraph, or text that `---` above would make a heading of. After a list item, or right under a heading, none is added.
- A note written with CRLF gets a CRLF line.
- Every line the note had stays, in order. The headings are found as the outline finds them: not in the front matter, a code fence or a `%%` comment.

Writing: the editors save first (on macOS `VaultStore.afterEditors`, on Linux `save_editors`), then the file is read again at the write and the task is added to what is there now, through the app's ordinary write (`VaultEdits`). The note on screen, in the reader or the editor, shows it after.

## 3. Where the form starts

- **Note**: the note on screen. With none (home, Today, the board), the scratchpad when it is there (macOS: `scratchpad.note`; Linux: today's note in `scratchpad.folder`); else no note, and the form asks for one. The note can be changed in the form, by searching the vault's notes, those with tasks first.
- **Section**: the heading whose section the cursor is in (in the editor its cursor, in the reader the reading place), or "End of the Note" above the first heading. Changing the note sets it to "End of the Note".

Opening: macOS File › New Task… (⌥⌘T) and the command palette; Linux the `win.new-task` action, Ctrl+Alt+T, and the palette; Android from the task lists.

## 4. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin):

1. `aNewTaskIsWrittenFromItsFields`: empty fields give no line; `"  Call Sara\n about the trip "` gives `- [ ] Call Sara about the trip`; with ⏫, created `2026-11-11`, scheduled `2026-11-14` and `every week` it is `- [ ] Call Sara about the trip ⏫ 🔁 every week ➕ 2026-11-11 ⏳ 2026-11-14`, which reads back as the same text and dates; status `/` gives `- [/] Call…`.
2. `theCreatedDateCanBeChanged`: `- [ ] Pay rent ➕ 2026-11-01 📅 2026-11-05` with created `2026-10-28` becomes `- [ ] Pay rent ➕ 2026-10-28 📅 2026-11-05`, and with no created date `- [ ] Pay rent 📅 2026-11-05`.
3. `goesAtTheEndOfTheSectionOnALine`, on `# Week\n## Work\n- [ ] A\n\n## Home\n- [ ] B\n\n## Work\n- [ ] C\n`:
    - heading line 7 puts `- [ ] New` after `- [ ] C`;
    - heading line 1 puts it after `- [ ] A`, before the blank line;
    - `## Inbox\n\n## Later\n` at line 0 gives `## Inbox\n- [ ] New\n\n## Later\n`;
    - line 2 (no heading there) and no line both put it at the end of the note.
4. The robustness line: for made-up notes, with no line, lines 0 and 3, and the lines of the first two headings, every line of the note stays, in order, and the new task is there once.
