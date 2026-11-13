# daftar-cli for a program: `--json` and `fmt --stdin`

Made by `daftar-cli` (`notes-linux`: `report.rs`, `lint.rs`, `check.rs`, `fmt.rs`, `meaning.rs` and `main.rs` in the CLI). Daftar for macOS runs the command as a subprocess for the vault's audit, lint, formatting and search by meaning instead of porting them, so what the command prints is a contract: a change to a key, a code or an exit status here is a change in the Mac app too. Linux and Android do not run it.

## 1. Findings: `lint --json`, `check --json`, `fmt --check --json`

Each finding is one JSON object on its own line (JSON Lines) on stdout, and nothing else goes to stdout. The counts and messages stay on stderr, as without `--json`.

```json
{"tool":"lint","level":"error","path":"Folder/Note.md","line":12,"column":3,"code":"MD009/no-trailing-spaces","message":"Trailing spaces [Expected: 0 or 2; Actual: 1]","fixable":true}
```

- `tool` is `lint`, `check` or `fmt`.
- `level` is `error` or `warning`. Every `lint` and `fmt` finding is an error; `check` has both.
- `path` is the file from the vault's folder, with `/`. A `check` finding may be on a picture or an SVG, not a note (`unused-picture`, `svg-text-overlap`).
- `line` counts from 1; 0 is the whole file. For `fmt` it is the first line formatting changes.
- `column` counts characters from 1, or is `null` when the tool does not know it (always `null` for `check` and `fmt`).
- `code` is the GitHub annotation's title: `MD009/no-trailing-spaces` for lint, the kind for check (`callout-body`, `ambiguous-link`, `memory-stale-status`, …), `fmt` for fmt.
- `message` is what was found, as the plain output says it.
- `fixable` is true when `lint --fix`, `check --fix` or `fmt` would mend it: a lint problem `--fix` mends (a rule of `MarkdownLint::FIXABLE`, but not a tab in a code block, which `--fix` leaves), a check `ambiguous-link`, or a `dead-url` that has an archived copy, and every `fmt` finding.

The exit status is the one without `--json`: 0 with no problem, 1 when lint finds a problem, `fmt --check` finds a note not formatted, or check finds an error (a warning too with `--strict`), and 1 on a failure, whose one line is on stderr. The other options (`--profile`, `--memory`, `--strict`, `--urls`, `--ignore`, …) work with `--json` as without it. `fmt --json` needs `--check` and does not take `--diff`. With `check --fix --json` and `lint --fix --json`, each change made and each note that could not be written goes to stderr, not stdout.

## 2. One note

`lint`, `lint --fix`, `fmt`, `fmt --check`, `check` and `check --fix` take files and folders. A relative path is read from the current folder when it is there and in the vault, else from the vault's folder, so a program can pass `Folder/Note.md` from any folder. A path that is in neither is an error. `check` still reads the whole vault, since a link resolves against every note, but it prints and mends the findings of the notes given only; pictures and figures are the vault's, so `unused-picture` and the `--figures` findings are left out then. Given paths that hold no note of the vault (a file outside it, a hidden folder, a picture) are an error too, as `check` would otherwise check nothing and exit 0.

`fmt --stdin [--path <path in the vault>]` reads one note from stdin and prints it as `fmt` would write it with the vault's `.prettierrc`, byte for byte, and writes nothing. With `--path`, a file `fmt` leaves alone (one `.prettierignore` or `--ignore` names, or one in a hidden folder) is printed unchanged. It exits 0, and 1 on a failure such as a `.prettierrc` it cannot read.

## 3. Search: `search --json`, `related --json`

One JSON array on stdout:

```json
[{"path":"Spain/Living in Barcelona.md","line":1,"score":0.7168,"snippet":"Companion to Spain Residency Roadmap, …"}]
```

- `line` counts from 1: the first line of the nearest section for search by meaning and `related`, the first line with the first matched word for search by words (1 when only the title matched).
- `score`, rounded to four places, is the cosine of the nearest section for `--meaning-only` and `related` (from -1 to 1), the reciprocal-rank-fusion score for `--meaning` (a small number, larger is better), and `null` for search by words.
- `snippet` is the text on one line, with no match marks: the section's heading path, ` · ` and its text for meaning, the words' context for search by words.

`search --meaning`, `search --meaning-only`, `related` and `semantic-index` without `--download-model` exit with **3** when the model is not downloaded, with one line on stderr that says how to download it (`semantic-index --download-model`, 531 MB). An app offers the download on 3.

The vectors are kept by note in the cache, and every one of these commands brings them up to the notes before it answers: a note that changed is embedded again, a new one is added and a removed one dropped. So with no index yet, or an old one, the first search builds it, and takes as long as `semantic-index`. `semantic-index` does the same ahead of time. On stderr, when stderr is not a terminal, each prints `indexed N/M` (sections) at each hundredth of the work, from `indexed 0/M` to `indexed M/M`, and nothing when no note changed; on a terminal it is one line rewritten. On a vault of 2,800 notes (44,000 sections) the release build embeds everything in about 2 s.

## 4. Tests

Each app that runs the command checks that it reads these lines; the CLI's own tests (`report.rs`, `lint.rs`, `fmt.rs`, `check.rs`, `meaning.rs`, `main.rs`) check the shapes: the keys in this order, `fixable` for a rule `--fix` mends and one it does not, a path from the vault's folder, `fmt --stdin` against what `fmt` writes and an ignored path, and `needs_model` for exit status 3.
