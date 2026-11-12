# Job Board: the job search as columns of applications

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). The vault stays plain Markdown with front matter that Obsidian reads; the board adds no file and no property of its own.

The job search is already data in `daftar-core` (`job_search.rs`): every note with a `job_stage` property is an application, read by `daftar-cli jobs` and the MCP's `job_pipeline`. The Job Board shows the same applications as a board, the booked events ahead, and the `memory-quiet` warning of `daftar-cli check`. Dragging a card to another column changes its note's `job_stage`, and nothing else.

## 1. What an application is

An application is a note, not hidden (Obsidian's excluded files and `files.hidden` stay off the board, as off every list), whose front matter has a non-empty `job_stage`. The keys are case-insensitive; the value is trimmed and read in lower case. From the front matter:

| Property | Use |
| --- | --- |
| `job_stage` | The stage, its column. |
| `job_applied` | The day applied; orders a column. |
| `job_roles` | The roles, a list; the card shows the first and how many more (`DevOps Engineer +1`). |
| `job_next` | The next booked event: `YYYY-MM-DDTHH:MM`, with `:SS` or a space instead of `T`, or a day alone `YYYY-MM-DD`. Anything else is no event. |
| `job_next_step` | What the event is (`Recruiter call with Lucy, 30 min`). |
| `job_channel` | The channel: the text before ` (`, trimmed (`Kulturo (Someone, LinkedIn)` is `Kulturo`), else `unknown`. |

- **The company** is the note's name, its **logo** the folder's `logo.svg`, `logo.png`, `logo.webp`, `logo.jpg` or `logo.jpeg` (in that order, any case), as the file tree draws it; with none, a generic building icon.
- **The preparation note** is `Interview Preparation.md` in the note's folder, when that file exists and is not the note itself.
- **A time has no zone in the note: it is Europe/Madrid's clock.** `2026-11-13T15:30` is 15:30 in Madrid, shown and compared as such whatever the device's zone. A day alone is ahead all that day.
- **Waiting** stages: `applied`. **Closed** stages: `rejected`, `withdrawn`, `declined`, `closed`. Every other stage is **going**. An application got past applied when its stage is neither waiting nor closed, or it is closed and has a preparation note.

## 2. The board

- **Columns, going stages first**, in a search's order: `applied`, `screening`, `interviewing`, `offer`, `accepted`, each a column even when empty, so a card can be dropped on it. A going stage the order does not know (`take-home`) is a column after them, by name.
- **Closed stages fold into one column at the end**: its title and total count, then a row for each closed stage (`rejected`, `withdrawn`, `declined`, `closed`) with its count, each row a place to drop a card. A click on it, or the header's **Closed** toggle, unfolds it into a column for each closed stage; the toggle folds them again.
- **In a column**, an application with a booked event comes first, the soonest first; then the newest `job_applied` first; then by company name, then by path.
- **A card** shows the logo, the company, the role, the channel, the next booked event with its time in the app's date style (`Fri, Nov 13 at 3:30 PM`, or the day alone), the next step under it, and a **Preparation** link that opens the preparation note. An event that has passed is marked "passed" in red: the note should say what happened. A going card whose note has gone quiet (§4) has a **Quiet N days** mark, with the audit's message as its tooltip.
- **A click** (or Return on a focused card) opens the company's note.
- **Above the columns**, the events booked from now to the end of the 14th day after today (the midnight after it, as `daftar-cli jobs --upcoming 14`), in time order: time, logo, company, next step, Preparation link. With none, a line says nothing is booked.
- **The header** counts the going and closed applications and the events booked in 14 days, and **Channels** shows each channel's applications, how many got past applied and the rate, the most used first (as `daftar-cli jobs --channels`).
- **Today** shows the next booked event (from now on, with no limit of days) in a card, with its Preparation link, and how many more are booked, which opens the board.

Opening: macOS Go › Job Search (⇧⌘J), the command palette and `daftar://open?path=jobs`; Linux and Android choose their own place and shortcut, and keep the palette entry's name, "Job Search".

## 3. Moving a card

Dropping a card on another column, or the card's "Move to" menu, sets its note's `job_stage` to that column's stage:

- **One line changes.** The property is written by the properties form's writer: the `job_stage` line keeps its key as written (`Job_Stage` stays `Job_Stage`) and its quotes (`"applied"` becomes `"screening"`); every other byte of the note stays. Front matter the form cannot write back the same (comments, anchors, nested maps, block scalars) is not written, and the app says why.
- **Nothing else is written**: not `Tasks/Interviews.md`, not the note's history or state paragraph, not `job_next`. Recording what happened is the person's.
- **The editors save first** and the file is read again at the write (macOS `VaultStore.afterEditors`; Linux `save_editors`). The write is refused, with a notice, when the note's stage on disk is no longer the stage the card showed.
- The card moves at once; the board reads the vault again when the file watcher sees the write. Dropping a card on its own column writes nothing.

## 4. Gone quiet

The `memory-quiet` rule of `daftar-cli check` (`memory.rs`), for the going cards only (a closed note is not read):

- **Rules** come from the vault's `.daftar.toml`: `active` (`key=value` pairs, those of `[check]` and `[audit]` together) and `quiet_days` (`[audit]`'s, else `[check]`'s). With no file or no such keys: `job_stage=applied`, `job_stage=screening`, `job_stage=interviewing` and 30 days.
- A note is **going** when a front-matter line is `key: value` with a pair of `active` (the value trimmed, its quotes taken off, compared as written).
- Its **newest date** is the latest `YYYY-MM-DD` that starts a front-matter value, a `**Where it stands (YYYY-MM-DD):**` paragraph, or a history entry `**YYYY-MM-DD…` at the start of a line (no letter or digit right after the date).
- It is **quiet** when it is going, has a newest date, and that date is more than `quiet_days` days before today (Madrid). The mark says the days; the message is the audit's: `job_stage: screening, and nothing in the note is dated after 2026-10-01 (42 days): record what happened, or close it`.

## 5. Reading

The applications are read off the main thread from the vault's index, and again whenever the index changes (a save, a pull, a file the watcher sees). The quiet rule reads the going notes' text from disk in the same pass.

## 6. Accessibility and motion

- A card is **one element** for VoiceOver, a button, named in words: `Perk, Screening, DevOps Engineer and Staff Software Engineer, via Ashby, next: Recruiter call on Fri, Nov 13 at 3:30 PM, quiet for 42 days, has a preparation note`. Its actions: open the note (the default), Open the Preparation Note, and **Move to <Stage>** for every other column, closed ones too. A move is announced (`Perk moved to Interviewing`).
- A column's header says its stage and count (`Screening, 1 application`); the folded column says `Closed, 71 applications`.
- With Reduce Motion, a move, a drop target's highlight and the unfolding of the closed columns do not animate.

## 7. Tests every app must have

The cases of `job_search.rs`, `jobs.rs` and `memory.rs`, ported by name, and the board's own:

1. `aChannelIsTheTextBeforeItsParenthesis`, `everyNoteWithAStageIsAnApplication`, `stagesAreCountedInASearchsOrder`, `aClosedApplicationGotPastAppliedOnlyWithAPrepNote`, `upcomingEventsAreTheBookedOnesAheadSoonestFirst`, `aMeetingIsMatchedByItsTimeOrTheCompanyItNames`, `theKeysAreParameters`, `aPropertysLineIsFoundInTheFrontMatter`, as in `job_search.rs`; `anEventWithinDaysEndsAtTheMidnightAfter` from `jobs.rs` (an event at `2026-11-10T15:30` seen at `2026-11-08T09:00` is outside 1 day and inside 2).
2. `everyNoteWithAStageIsAnApplication` also checks the logo: `logo.svg` before `logo.png`, matched in any case (`LOGO.svg`), none when the folder has none.
3. `aTimeIsReadWithOrWithoutItsClock`: `2026-11-10T15:30` has a time; `2026-11-10 15:30:20` has its seconds; `"2026-11-10"` is a day; `2026-11-10T25:00`, `2026-02-30` and `soon` are nothing; `2026-11-10T15:30` is `2026-11-10T14:30:00Z`.
4. `theBoardHasAColumnForEachGoingStageAndTheClosedAfter`, on the `job_search.rs` fixture: going `applied, screening, interviewing, offer, accepted`; closed `rejected, withdrawn, declined, closed`; `applied` holds Initech, Stark; `rejected` holds Umbrella (booked) before Hooli; an unknown going stage is the last going column.
5. `aStageIsWrittenAsTheFormWritesAProperty`: `---\naliases: [Acme Inc]\njob_stage: "applied"\njob_roles:\n  - Engineer\n---\n# Acme\njob_stage: applied\n` moved from `applied` to `screening` changes only the front matter's `job_stage` line, to `job_stage: "screening"`; moving it again from `applied` is refused; a note with no front matter is refused; `Job_Stage: Applied` becomes `Job_Stage: offer`; front matter with an anchor is refused as unwritable.
6. `aGoingNoteWithNothingNewIsQuiet` (from `memory.rs`, `today` 2026-11-06, `job_stage=applied`, 30 days): `---\njob_stage: applied\njob_applied: 2026-09-01\n---\n**2026-09-20:** No answer.\n` is quiet at line 2, newest 2026-09-20, 47 days; an entry of 2026-10-20, or the stage `rejected`, is not quiet; a note with no date is not; a `**Where it stands (2026-11-01):**` paragraph makes it not quiet; `**2026-11-01x**` is no entry.
7. `theRulesComeFromTheVaultsAuditFile`: no file gives the defaults; `[audit]` with `active = ["job_stage=applied", "job_stage=screening"]` and `quiet_days = 21` gives those; a file without them gives the defaults.
8. `onlyGoingApplicationsAreReadForQuiet`: a closed application's note is never read.
9. A store test: with an editor holding unsaved text, a move waits for its save, writes only the stage, and keeps what was typed; a second move from the old stage is refused with a notice and writes nothing.
10. The robustness line: for made-up notes, reading times, property lines and the quiet rule never fail, and a stage set changes at most one line, the `job_stage` one, keeping the line count.
