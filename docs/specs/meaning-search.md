# Search by meaning: notes near a query, and the notes related to one

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). The apps do not embed anything themselves: `daftar-cli` owns the model, the index and the ranking, and the apps run it and read its JSON, so the three show the same notes for the same query. Nothing is written to the vault.

## 1. The command

`daftar-cli` 0.23.0 or later. Every command takes `--vault <root>`; the query or the note comes last, after `--`, so a query that starts with `-` is words, not an option.

| What | Command |
| --- | --- |
| By meaning alone (the apps' search) | `daftar-cli --vault <root> search --meaning-only --json -n 20 -- <query>` |
| Words and meaning ranked together | `daftar-cli --vault <root> search --meaning --json -n 20 -- <query>` |
| The notes related to a note | `daftar-cli --vault <root> related --json -n 10 -- <note path>` |
| Download the model, then index | `daftar-cli --vault <root> semantic-index --download-model` |
| Bring the index up to the vault | `daftar-cli --vault <root> semantic-index` |

`search` and `related` print a JSON array, nearest first:

```json
[{"path": "Learning/Kafka.md", "line": 12, "score": 0.83, "snippet": "A log of events …"}]
```

- `path` is a vault path; `line` is the 1-based line of the section that matched (the apps open the note there, as line − 1 where they count from 0); `score` is higher for nearer; `snippet` is a few words of the section.
- The apps keep **each note once**, at its first (nearest) entry, in the command's order. An entry with an empty path, an absolute path or a `..` part is dropped. A missing `line` is 1, a missing `score` 0, a missing `snippet` empty; an unknown field is ignored.
- Output that is not a JSON array is a failure ("printed something that is not the list of notes"), never an empty list.

`related` leaves out the note itself and the notes it already links to; the apps also leave out the note itself and the hidden files (`files.hidden`, Obsidian's excluded files), as every list does.

## 2. Failures

| The command | Means | The apps show |
| --- | --- | --- |
| exits 3 | the model is not downloaded | one row: **Download the model (531 MB)** |
| exits 1 with "… which is not downloaded" on stderr (0.21) | the same | the same row |
| exits 2 with clap's `error: unexpected argument '--json'` (or `'--meaning'`, `'--meaning-only'`, `'--download-model'`) or `error: unrecognized subcommand 'related'` (or `'semantic-index'`) | `daftar-cli` is too old | one row: "Search by meaning needs daftar-cli 0.23.0 or later: brew upgrade parham-alvani/tap/daftar-cli" (the package manager's command on Linux) |
| any other usage error | | "daftar-cli: " and clap's `error:` line |
| any other failure | | the last line of stderr, without a leading "daftar-cli: " |
| is not installed | | "daftar-cli is not installed: …" with the install command |

An old `daftar-cli` is asked **once**: the apps remember the binary (its path and modification time) that was too old and do not run it again for each key until it changes (an upgrade). The notice is one row, never a dialog and never one per keystroke.

## 3. Where it shows

- **The search of the sidebar** (macOS ⇧⌘F, Linux the search pane, Android the search screen): under the notes by name and the full-text results, a section **By meaning** lists the notes the meaning search finds that the sections above do not already list. Not for an empty query or a `#tag` search.
- **The inspector** (macOS the right sidebar, Linux the side panel, Android the note's info sheet): **Related notes**, beside backlinks and unlinked mentions, for the open note. Asked again when another note opens and after each index build, not at each save.
- A hit is the note's title, its folder and the snippet (right to left when the snippet starts right to left), and opens the note at its line. VoiceOver / the screen reader says the title, then the snippet, with the hint "Near in meaning".

## 4. Never in the way of typing

- The search waits for a pause in typing (about 450 ms) before it runs, and a new key **cancels** the run before it: the process is ended (SIGTERM, then SIGKILL a second later), and its answer, if it comes, is not shown. The word search above it stays as fast as it was.
- No run, download or index build is on the UI thread. A section shows "Looking…" until its answer for the query on screen comes; an answer for an older query is never shown.

## 5. The model and the index

- **The model is never downloaded unasked.** The row "Download the model (531 MB)" runs `semantic-index --download-model` in the background. While it runs, the row is a progress bar with a **Cancel** button, from its stderr: `N% of 531 MB` is the download, `indexed N/M` and `embedding N/M sections` the index. Cancel ends the process. When it ends, the sections ask again; a failure is shown under the row.
- **The index is kept fresh in the background**, only once the model is there: after the vault changes (a save, a pull), once it has been still for `meaning.refresh_minutes` (10 at first; each change starts the wait again; 0 never), the apps run `semantic-index` at the lowest priority (macOS `.background`, Linux `nice`/idle I/O class, Android a WorkManager job that needs no battery saver), **never two at once**: a change during a build schedules one more after it. A background build does not hide the results; the index answers while it is built.
- `meaning.search = false` in `[meaning]` of the config file turns the section, the related notes and the refresh off.

## 6. Tests every app must have

1. `theJSONIsReadAsHitsEachNoteOnce`: `[{"path":"Learning/Kafka.md","line":12,…},{"path":"Learning/Kafka.md","line":40,…},{"path":"Personal/سفر.md","line":1,…}]` gives two hits, Kafka at line 12 (11 counted from 0), then the Persian note; `[]` gives none.
2. `aMissingFieldIsNotAReasonToDropTheNote` and `aPathOutsideTheVaultIsDropped` (`../secret.md`, `/etc/hosts`, `""`).
3. `whatIsNotTheListThrows`: `A.md:12 a log` and `{"path":"A.md"}` are failures.
4. `theArgumentsPutTheQueryAndTheNoteLast`: exactly the commands of section 1, and a query `-x` or a note `--help.md` comes after `--`.
5. `exitThreeIsAModelNotDownloaded`, and the 0.21 message with exit 1.
6. `anOldCLIIsToldToUpgrade`: the clap errors of section 2 give the upgrade notice; an unknown flag (`--frobnicate`) gives "daftar-cli: unexpected argument '--frobnicate' found".
7. `theProgressOfABuildIsReadFromItsLines`: `indexed 120/4300`, `embedding 43883/43883 sections`, `40% of 531 MB`; `indexed 5/0` and other lines are no progress; `indexed 9/4` is 4 of 4.
8. `anOldCLIIsAskedOnce`: against a made-up `daftar-cli` that exits 2 with the `--json` error, three searches run it once.
9. `aCancelEndsTheCommand`: a cancelled run of a command that sleeps 30 s ends within a few seconds and reports that it was cancelled.
10. The robustness line: made-up text as the command's stdout and stderr never crashes the parsers; a hit's path is never absolute, a progress is between 0 and 1.
