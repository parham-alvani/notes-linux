# Drill Review: spaced-repetition cards from the notes, and their shared history

Shared by Daftar for Linux (Rust, `notes-linux`: `daftar-core` `drill.rs`, `daftar-cli drill`, the MCP `drill_due`, `drill_card` and `drill_rate` tools), macOS (Swift, `notes-mac`: `Drill.swift`, the Review Cards page) and Android (Kotlin, `notes-droid`). Drilling never writes a note. What was rated, and when each card is due, is Daftar's own state, kept in one file per vault that every app and the command line read and write, so a card rated in one is rated in all.

## 1. The cards

A card is a heading of a note; its section is the answer; the note's title and the headings above it are its context. Headings are found as the outline finds them: ATX `#` headings, not in the front matter, a code fence or a `%%` comment, no setext headings.

- A **part** of an answer is a heading named as one. Its name is compared lowercased, without `*`, `_` and `` ` ``, trimmed, and without a closing `:`. The parts are: example, examples, problem, the problem, test cases, tests, solution, solutions, the solution, answer, the answer, code, output, explanation, hint, hints; and any name starting with `example` or `worked example`, or ending in ` solution` or ` solutions`. A part is never a card: it is the answer of the heading above it, with its own heading line.
- A **reference** heading (related notes, related, see also, sources, references, links, further reading, compared the same way) is never a card and is left out of every answer.
- Every other heading is a card when its answer has text (a line that is not blank). Its answer is its own lines up to its first child heading, then each child that is not a card, not a reference and has text, as that child's heading line and that child's answer, in order. So a heading with only questions under it is context, not a card, and a part with an introduction is a card of the introduction.
- The answer's lines are joined with `\n`, then the line breaks at both ends and the white space at its end are taken off.
- The text above the first heading is no card.

**The id** is the note's vault path, `#`, and the heading path, outermost first, joined with `#`: `Learning/Go.md#Part 1#Q1. What is a goroutine?`. Two headings with the same id are one card, the first. Changing an answer keeps the card's history; renaming a heading above it starts the card again.

**The deck** is the notes of a folder at any depth, `Learning/Interview-Preparation` when none is named, without the folder notes (`X/X.md`, a note named as its folder) and the files the vault hides (Obsidian's excluded files and `files.hidden`). The notes are sorted by path in byte order (UTF-8), and the deck is each note's cards in the note's order.

## 2. The schedule

SuperMemo's SM-2 with four ratings: **Again** (quality 2), **Hard** (3), **Good** (4), **Easy** (5); `1`–`4` or the names in any case.

- A card never rated starts with ease 2.5, interval 0, reps 0, lapses 0.
- **Again**: interval 1, reps 0, lapses + 1, the ease kept; due tomorrow.
- Any other rating: ease + `0.1 - (5 - q) * (0.08 + (5 - q) * 0.02)`, never under 1.3, rounded to two places (half away from zero); reps + 1; the interval is 1 when reps is 1, 6 when reps is 2, and else the last interval (at least 1) times the new ease, rounded half away from zero.
- `due` is the rating's day plus the interval; `last` is the rating's day. Days are local calendar days, `YYYY-MM-DD`.

**The cards of a session** on a day: the cards due on that day or before, the longest overdue first (ties in deck order), then at most 10 never rated, in deck order; at most 100 in all (both are options on the command line). A card rated Again is asked again at the end of the session, but only its first rating is written.

**Cards due** (the count a page, the sidebar or `vault_brief` shows) are the rated cards due today or before, not the new ones. The next due day is the earliest `due` after today.

## 3. The state file

`$XDG_STATE_HOME/daftar/drill-<hash>.json`, or `~/.local/state/daftar/…` when `XDG_STATE_HOME` is unset or not an absolute path. **macOS uses the same path**, not `~/Library`, so the app and `daftar-cli` share it. `<hash>` is the 64-bit FNV-1a of the vault's path as text (offset `0xcbf29ce484222325`, prime `0x100000001b3`, over the UTF-8 bytes, without a trailing `/`), as 16 lowercase hex digits. A test points the file elsewhere (macOS: beside the test's settings; Linux: the MCP server's `drill_state`).

```json
{
  "version": 1,
  "cards": {
    "Prep/Go.md#Q1. What is a \"goroutine\"?": {
      "ease": 2.5,
      "interval": 1,
      "reps": 1,
      "lapses": 0,
      "due": "2026-11-13",
      "last": "2026-11-12"
    }
  }
}
```

- Written as serde_json's pretty printer writes it: two spaces, `version` then `cards`, the cards by id in byte order, each review's keys in the order above, a float with the shortest digits that read back and `.0` on a whole number, only `"`, `\`, and control characters escaped (`\n`, `\r`, `\t`, `\b`, `\f`, else `\u00xx`), no line break at the end.
- Read leniently: a file that is missing or not JSON is an empty state; a review missing `ease`, `interval`, `reps`, `due` or `last`, or with a count or interval that is not a whole number, is dropped; `lapses` defaults to 0; an ease under 1.3 reads as 1.3; a key written twice is the last one.
- **Writing a rating reads the file again first**, applies the rating to what is there, and writes through a temporary file (`.drill-<hash>.json.daftar-<pid>` beside it) and a rename, so a rating another app made since is kept and no reader sees half a file. A view may show the new review at once; the state it then keeps is the one written.

## 4. The review page (apps)

- **Opening**: macOS Go › Review Cards (⇧⌘R), the command palette, `daftar://open?path=drill`; Linux and Android choose theirs. It shows in the note pane, as Today does. The deck is read off the main thread; the page says it is reading until then.
- **A card**: the context (title › headings), then the question, its inline Markdown drawn; both read right to left when the question's first strong letter is (Persian). A progress bar and "n of m": the cards rated of the session's cards. A card asked again says so.
- **The answer** is hidden until Space, Return or the Show Answer button. It is drawn by the reader's own renderer from the answer's Markdown, with the note's links and pictures and each block's own direction; it ticks no task and keeps no folds, since its lines are not the file's. It is laid out before it is shown, so the flip only paints, and it is not read by a screen reader while hidden.
- **Rating**: `1`–`4` (by key position, so any keyboard layout works) or four buttons, Again, Hard, Good, Easy, each naming the interval it would give (`1 day`, `6 days`, `3 wk`, `4 mo`, `1.5 yr`); a card asked again names none. Rating goes to the next card at once; the file is written in the background, in order.
- `e` or Open Note opens the card's note at its heading. Escape leaves the page. Keys typed in a text field, a terminal or a sheet, and keys with ⌘, Ctrl or Alt, are not the page's.
- **Done**: how many were rated and forgotten, when the next cards are due (today, tomorrow, or the day and how far), and a button (Return) for another session while new cards remain. With nothing due, "No cards are due".
- **Counts**: Today shows "n cards due" (a link to the page) and the sidebar a Review Cards row with the count, only when n > 0. The state is read again when the app comes to the front, so a rating made on the command line counts.
- Motion follows Reduce Motion; every control has a name a screen reader says.

## 5. Tests every app must have

Ported by name (snake case in Rust, camelCase elsewhere), on the notes in `drill.rs`'s tests:

1. `aQuestionUnderAPartIsACardAndItsExampleIsItsAnswer`, `aProblemIsOneCardWithItsParts`, `headingsWithNothingUnderThemAreNoCards`, `partsAndReferencesAreNamedSo`.
2. `sm2SpacesGoodAnswersOutAndStartsAForgottenCardAgain`: from 2026-11-08, Good gives 1 day then 6 then 15; Easy after the second gives ease 2.6 and 16 days, Hard ease 2.36 and 14 days; Again gives interval 1, reps 0, lapses 1 and keeps the ease; Good after it gives 1 day. `theEaseNeverFallsUnder1_3`.
3. `theDueCardsComeFirstThenAFewNewOnes`: on `## A`…`## D` with C due 11-02, D due 11-06, B due 11-09, on 11-08 the session is C, D, A; with no new cards 2; with a limit of 1, one.
4. `theStateComesBackFromItsFile` (one file in its folder after a save, nothing left behind) and `aFoldersDeckLeavesItsFolderNoteOut`.
5. `theCommandLinesFileIsReadAndWrittenTheSame`: the file above, with also `Prep/Go.md#Zed` (ease 2.6) and `Prep/Go.md#پرسش دوم` (reps 0, lapses 1), as `daftar-cli` wrote them, reads to those reviews and writes back byte for byte.
6. `theStateFileIsTheCommandLines`: FNV-1a of `/home/a/vault` is `3ce90dc2d0a15db6`, of `/Users/someone/Notes/` `577cb4302b31064b`; the folder honours an absolute `XDG_STATE_HOME` only.
7. `aRatingReadsTheFileAgainFirst`: a rating written by another program after the state was loaded is still in the file after this app's rating.
8. A session asks a card forgotten on its first rating once more and writes only its first rating.
9. The robustness line: cards from made-up notes have unique ids and text in their answers, and a state file of any text reads, and what it writes reads back the same.
