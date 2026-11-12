# Specs shared by the three apps

Daftar for Linux (this repository), Daftar for macOS (`notes-mac`) and Daftar for Android (`notes-droid`) share no code, but they share rules. A rule that more than one app keeps is written down here first, then built in each app against the same text and the same test cases, so the three can be compared line by line.

A spec says what the Markdown looks like, how each app reads and writes it, what changes it on each platform, how it is shown, and which tests every app must have. Everything stays plain Markdown that Obsidian and its Tasks plugin still read.

| Spec | Released in |
| --- | --- |
| [Task reasons and custom statuses](task-reasons-and-statuses.md) | Linux 0.6.0, macOS 0.7.0, Android 0.57.0 |
| [Outline: closing headings and sorting the headings under one](outline-closing-and-heading-sort.md) | macOS (`723e125`), Android (`ebb7aef`), Linux 0.7.3 |
| [Properties: the front matter as a form](properties.md) | macOS (`4c26615`), Linux 0.8.1, Android 0.61.0 |
| [Export: a note as a PDF, by `daftar-cli export`](export.md) | Linux 0.9.0, macOS 0.14.0, Android 0.61.0 (its own PDF writer, without logos, math, Mermaid or footnotes) |
| [Repeating by the Jalali calendar](jalali-recurrence.md) | Linux 0.13.0, macOS 0.15.0, Android 0.60.0 |
| [The holidays of a place, and the day picker](calendar-events.md) | Linux 0.14.0, macOS 0.16.0; not Android yet |
| [Pictures from the web, only when asked](remote-images.md) | Linux 0.18.0, macOS 0.16.0, Android 0.61.0 |
| [New Task, and the created date in the task form](new-task.md) | macOS 0.19.0, Linux 0.22.0, Android 0.62.0 |
| [daftar-cli for a program: `--json` and `fmt --stdin`](cli-json.md) | not released yet (Linux's CLI; macOS runs it) |
| [The Problems panel, and formatting a note on leaving the editor](problems-and-format.md) | not released yet (macOS first) |
| [Review Cards: the drills in the app](drill-review.md) | not released yet (macOS first) |
| [The job search's board](job-board.md) | not released yet (macOS first) |
| [Search by meaning and related notes](meaning-search.md) | not released yet (macOS first) |
| [The weekly review page](weekly-review.md) | not released yet (macOS first) |
| [Send to Daftar from other apps](send-to-daftar.md) | not released yet (macOS first) |
