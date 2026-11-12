# The weekly review: this week's note, or a preview to save

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). The review itself is `daftar-cli review`'s: the apps only find it, show it and ask the command to write it, so a review made in an app is the same note the vault's CI makes.

## 1. The note

`daftar-cli review --day D` writes the review of the seven days before `D` (`D − 7` to `D − 1`); with `--write` it makes `Reviews/<from> to <to>.md` and prints its vault path (`Reviews/2026-10-31 to 2026-11-06.md`), and fails with "… is there already" rather than write over a note. The vault's CI runs it every Saturday at 07:00 UTC.

- **This week's review** is the one written on the **last Saturday**, today when today is a Saturday: on Thursday 2026-11-12 it is the one written on 2026-11-07, `Reviews/2026-10-31 to 2026-11-06.md`; on Saturday 2026-11-14 it is `Reviews/2026-11-07 to 2026-11-13.md`. A path in another case (`reviews/…`) is the same note.
- **The newest review** is the note directly in `Reviews/` named `<YYYY-MM-DD> to <YYYY-MM-DD>.md` (two real days, the first not after the second) whose second day is the latest. Notes in subfolders and other names do not count.

## 2. The command

**Weekly Review**: macOS Go › Weekly Review (⌘4, beside Today ⌘1, Board ⌘2 and the periodic note ⌘3) and the command palette; Linux the `win.weekly-review` action (Ctrl+4) and the palette; Android the overflow menu of Today.

- When this week's review is in the vault, it opens like any note.
- When it is not (Saturday before CI ran, CI failed, a vault without CI), the app runs `daftar-cli --vault <root> review --day <the last Saturday>` off the UI thread and shows its Markdown **in the reader** as a preview page: drawn exactly as the note would be (links resolve from `Reviews/`), titled "Weekly Review" with "A preview of Reviews/… , not in the vault yet". While it runs, "Writing the review…".
- The preview has **Save to Reviews/** (⌘S): `review --day <same day> --write`, then the preview closes and the new note opens. The vault's write is the command's; the app takes the new file into its index (the file watcher, waited for a moment) before opening it.
- **Close** and Escape close the preview and write nothing. A link in the preview opens its note and closes the preview.
- A failure shows the command's last line of stderr in the preview (as Export as PDF does): "Reviews/… is there already", or, when the command does not know `review`, "The weekly review needs a newer daftar-cli: …".

## 3. Today

The Today page has a small **Weekly review** card linking the newest review, only when there is one. It is a link, not a preview: it never runs the command.

## 4. Tests every app must have

1. `thisWeeksReviewIsWrittenOnTheLastSaturday`: 2026-11-07 (a Saturday) → 2026-11-07; 2026-11-08, 2026-11-12, 2026-11-13 → 2026-11-07; 2026-11-14 → 2026-11-14; 2027-01-01 → 2026-12-26.
2. `aReviewCoversTheSevenDaysBefore`: written on 2026-11-07 is `Reviews/2026-10-31 to 2026-11-06.md`; with folder `Weekly`, 2026-03-07 is `Weekly/2026-02-28 to 2026-03-06.md`.
3. `thisWeeksFileIsFoundWhenItIsThere`: among `Reviews/2026-10-24 to 2026-10-30.md`, `Reviews/2026-10-31 to 2026-11-06.md`, on 2026-11-12 it is the second; on 2026-11-14 there is none; `reviews/2026-10-31 to 2026-11-06.md` is found on 2026-11-12.
4. `theNewestReviewIsTheOneThatEndsLast`: from the two above, `Reviews/Old/2027-01-01 to 2027-01-07.md`, `Reviews/Notes.md`, `Reviews/2026-13-01 to 2026-13-07.md`, `Elsewhere/2027-02-01 to 2027-02-07.md` and `Reviews/2026-11-06 to 2026-10-31.md`, the newest is `Reviews/2026-10-31 to 2026-11-06.md`; none in an empty vault.
5. `theCommandIsGivenTheDay`: `--vault /v review --day 2026-11-07`, and `--write` after it.
6. `weeklyReviewOpensThisWeeksNoteOrItsPreview`: on 2026-11-12 with the note there, it opens; on 2026-11-14, a preview of `Reviews/2026-11-07 to 2026-11-13.md`.
