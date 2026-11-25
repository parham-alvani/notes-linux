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
| [daftar-cli for a program: `--json` and `fmt --stdin`](cli-json.md) | Linux 0.23.0 (`daftar-cli`); macOS 0.20.0 and Linux run it |
| [The Problems panel, and formatting a note on leaving the editor](problems-and-format.md) | macOS 0.20.0, Linux 0.25.0; not Android, where `daftar-cli` cannot run |
| [Review Cards: the drills in the app](drill-review.md) | macOS 0.20.0; not Linux or Android yet (`daftar-cli drill` and the MCP tools since Linux 0.20.0) |
| [The job search's board](job-board.md) | macOS 0.20.0; not Linux or Android yet |
| [Search by meaning and related notes](meaning-search.md) | Linux 0.20.0 (`daftar-cli`), macOS 0.20.0; not Android, where `daftar-cli` cannot run |
| [The weekly review page](weekly-review.md) | macOS 0.20.0; not Linux or Android yet (`daftar-cli review` writes the note) |
| [Send to Daftar from other apps](send-to-daftar.md) | macOS 0.20.0; not Linux or Android yet |
| [Today's timeline: meetings and timed tasks on one axis](today-timeline.md) | macOS 0.22.0, Linux 0.24.0, Android 0.63.0 |
| [Notes that open folded](folded-sections.md) | macOS 0.24.0, Linux 0.27.0, Android 0.64.0 (the hint under the heading, its counts in the phone's language, folds kept by position) |
| [The language server: `daftar-cli lsp`](language-server.md) | Linux 0.25.0 (`daftar-cli`); macOS runs `daftar-cli`, so it has it too; not Android, where `daftar-cli` cannot run |
| [A read-only vault, and the clone's own git identity](read-only-vault.md) | Linux 0.28.0, macOS 0.25.0, Android 0.65.0 (its own key, so not the ssh command rule; the address for GitHub hosts only) |
| [The app's settings from the vault](vault-config.md) | Linux 0.28.0, macOS 0.25.0, Android 0.65.0 (the keys it has a setting for); a setting changed in the app goes to the vault, with a table for each platform, since Linux 0.29.0, macOS 0.26.0 and Android 0.66.0 |
| [The Open Knowledge Format: concepts in the vault, and the vault as a bundle](open-knowledge-format.md) | Linux 0.28.0 (`daftar-cli` with the audit, the MCP tools and `okf export`), macOS 0.25.0 and Android 0.65.0 (sections 1 to 3) |
| [Several vaults on the desktop: a list, and a change without a restart](several-vaults.md) | Linux 0.29.0 (with `daftar-cli vaults`), macOS 0.26.0; Android has several vaults already |
| [Notification rules: an alert when a note lacks today's heading](notification-rules.md) | not released yet: Linux (the app and `daftar-cli agenda --notify`), macOS and Android are built |
| [The look of a note: reader, editor and cursors](look.md) | Part 1 in Linux 0.30.0, macOS 0.27.0 and Android 0.67.0; part 2 in Linux 0.31.0, macOS 0.28.0 and Android 0.68.0. The spec's last table says what each app still lacks |
