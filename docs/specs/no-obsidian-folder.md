# Daftar's own settings in place of `.obsidian/`

Shared by Daftar for Linux (Rust, `notes-linux`, with `daftar-cli`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`).

The apps began as readers of an Obsidian vault, so a few of their settings are read from Obsidian's own folder, `.obsidian/`, in the files of Obsidian and its plugins. Obsidian is no longer used, and a vault should not need its folder. Each of those settings now has a home of Daftar's own: a section of the vault's `.daftar.toml` (`vault-config.md`), or a file in the new folder `.daftar/` at the vault's root for data too long for a settings file.

## 1. Where each setting goes

| Today, in `.obsidian/` | Now | Shape |
| --- | --- | --- |
| `app.json` `attachmentFolderPath` | `files.attachments` | a path in the vault: `"uploads"` |
| `app.json` `userIgnoreFilters` | `files.ignore` | a list of strings, the same filters: `["uploads/"]` |
| `daily-notes.json` `format`, `folder`, `template` | `daily.format`, `daily.folder`, `daily.template` | the same strings: `"YYYY-[W]ww"`, `""`, `""` |
| `templates.json` `folder` | `templates.folder` | a path in the vault |
| `types.json` `types` | the section `[properties.types]` | one `key = "type"` line for each property: `aliases = "aliases"`, `job_next = "datetime"` |
| `plugins/obsidian-tasks-plugin/data.json` `statusSettings` | one section `[tasks.statuses.<id>]` for each status that is not one of the four every vault has | `symbol = "r"`, `name = "In Review"`, `next = "x"`, `type = "IN_PROGRESS"`; `<id>` as a rule's (letters, digits, `_`, `-`) |
| `plugins/beautitab/data.json` | `home.greeting`, `home.clock` (`"12-hour"`, `"24-hour"` or `"system"`), `home.show_time`, `home.show_greeting`, `home.show_search`, `home.show_recent`, `home.show_bookmarks`, `home.show_quote`; the picture stays `home.image` | the plugin's values under these names |
| `plugins/beautitab/data.json` `customQuotes` | `.daftar/quotes.json` | `[{"text": "…", "author": "…"}]` |
| `plugins/iconic/data.json` | `.daftar/icons.json` | the same JSON as Iconic's file (`fileIcons`, `folderIcons`, `folderRules`, `fileRules`), so it moves as it is |
| `bookmarks.json` | `.daftar/bookmarks.json` | the same JSON as Obsidian's |

The four statuses every vault has (` ` Todo, `x` Done, `/` In Progress, `-` Cancelled) need no section; a section with one of their symbols changes its name or next symbol.

A vault may set every key above: none starts a program or reaches outside the vault, and the paths are checked as in-vault paths. `daftar-cli config` lists them.

## 2. Reading: the new place first, then `.obsidian/`

For each setting an app reads the new place. When the new place is not there at all (the key is not set, or the `.daftar/` file does not exist), it reads `.obsidian/` as before, so an Obsidian vault and a vault not moved yet still work. A setting is never merged from both: the new place wins whole. For the statuses, any `[tasks.statuses.*]` section means the plugin's file is not read.

## 3. Writing

Where an app writes one of these today (an icon set from the sidebar, a bookmark, a status), it writes the new place when the vault has it (`.daftar/icons.json` exists, `.daftar.toml` has `[tasks.statuses.*]`), else the old place, as before. It never creates a file in `.obsidian/` in a vault that has none.

## 4. Moving a vault: `daftar-cli migrate-obsidian`

`daftar-cli migrate-obsidian` writes each setting of `.obsidian/` to its new place, leaves a key already set in `.daftar.toml` as it is, and prints what it moved. With `--remove` it then deletes `.obsidian/`, but only when every file it read moved and no file it does not know is left (it names any such file and stops). With `--dry-run` it prints what it would do and changes nothing. It writes no setting with the value the app takes anyway when the key is missing.

## 5. The folder's own index

`.daftar/` is skipped by the note index, the search, the audit and the file tree, as `.obsidian/` is.

## 6. Tests every app must have

1. `theNewPlaceWins`: with `files.attachments = "media"` in `.daftar.toml` and `attachmentFolderPath = "uploads"` in `.obsidian/app.json`, the attachments folder is `media`; with no `files.attachments`, it is `uploads`; with neither, the default.
2. `statusesFromTheSettings`: `[tasks.statuses.in_review]` with `symbol = "r"`, `name = "In Review"`, `next = "x"`, `type = "IN_PROGRESS"` gives the five statuses with `r` In Review, and the plugin's file is not read.
3. `dailyNotesFromTheSettings`: `[daily]` with `format = "YYYY-[W]ww"` names the note of 2026-11-25 `2026-W48`.
4. `iconsFromTheFolder`: `.daftar/icons.json` with Iconic's JSON gives the same icons as the same JSON in `.obsidian/plugins/iconic/data.json`; with both, `.daftar/` wins.
5. `homeFromTheSettings`: `home.greeting = "Hello, Parham."`, `home.clock = "12-hour"` and `.daftar/quotes.json` with one quote give the home page those values.
6. `theFolderIsSkipped`: a note in `.daftar/` is not in the index.
7. (`daftar-cli`) `migrateMovesEverything`: on a copy of a vault with each `.obsidian/` file above, `migrate-obsidian --remove` writes the sections and files, deletes `.obsidian/`, and a second run says there is nothing to move; with an unknown file in `.obsidian/` it stops and names it.
