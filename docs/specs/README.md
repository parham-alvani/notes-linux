# Specs shared by the three apps

Daftar for Linux (this repository), Daftar for macOS (`notes-mac`) and Daftar for Android (`notes-droid`) share no code, but they share rules. A rule that more than one app keeps is written down here first, then built in each app against the same text and the same test cases, so the three can be compared line by line.

A spec says what the Markdown looks like, how each app reads and writes it, what changes it on each platform, how it is shown, and which tests every app must have. Everything stays plain Markdown that Obsidian and its Tasks plugin still read.

| Spec | Released in |
| --- | --- |
| [Task reasons and custom statuses](task-reasons-and-statuses.md) | Linux 0.6.0, macOS 0.7.0, Android 0.57.0 |
| [Outline: closing headings and sorting the headings under one](outline-closing-and-heading-sort.md) | macOS (`723e125`), Android (`ebb7aef`), Linux 0.7.3 |
| [Properties: the front matter as a form](properties.md) | macOS (`4c26615`), Linux 0.8.1; Android (notes-droid#28) |
| [Export: a note as a PDF, by `daftar-cli export`](export.md) | Linux 0.9.0; macOS (notes-mac#6) |
| [Repeating by the Jalali calendar](jalali-recurrence.md) | Linux 0.13.0, macOS 0.15.0, Android 0.60.0 |
| [The holidays of a place, and the day picker](calendar-events.md) | Linux 0.14.0; macOS (notes-mac#9); not Android yet |
| [Pictures from the web, only when asked](remote-images.md) | Linux 0.18.0; macOS (notes-mac#9), Android (notes-droid#28) |
