# More of a vault's own settings: the working day, tasks, the digest, the job search, rules and folders

Shared by Daftar for Linux (Rust, `notes-linux`, with `daftar-cli`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Each key here is a setting of `.daftar.toml` (or `config.toml`) that a vault may set (`vault-config.md`). Each one replaces a value the apps had fixed, or a rule a person had to keep by hand.

## 1. The working day

| Key | Default | Value |
| --- | --- | --- |
| `calendar.working_day` | `"09:00-18:00"` | `HH:MM-HH:MM`, the start before the end |

The free time of Today's timeline (`today-timeline.md`) and of `daftar-cli agenda` is counted inside it. A value that does not fit is the default.

## 2. A new task's scheduled date

| Key | Default | Value |
| --- | --- | --- |
| `tasks.default_scheduled` | `0` | 0 to 365 days |

When it is more than 0, the New Task form (`new-task.md`) starts with `⏳` that many days after today, beside the `➕` created date. The person can change or clear it before saving. 0 adds no `⏳`, as before.

## 3. Which tasks the morning digest counts

| Key | Default | Value |
| --- | --- | --- |
| `notifications.digest_filter` | `[]` | a list of filter lines in the Tasks plugin's query language |

The digest counts the open tasks that are due or late, as before, and only those that every line matches: `["path does not include Elaheh"]`. A line the app cannot read is left out and `daftar-cli doctor` names it.

## 4. The job search's stages

| Key | Default | Value |
| --- | --- | --- |
| `jobs.waiting` | `["applied"]` | the `job_stage` values that wait on the company |
| `jobs.closed` | `["rejected", "withdrawn", "declined", "closed"]` | the values that end a process |

The job board, `job_pipeline` and the audit's `memory-quiet` read the stages from here. The property names (`job_stage`, `job_next`, …) do not change.

## 5. More kinds of notification rule

A rule (`notification-rules.md`) gets one more optional key, and two more kinds.

| Key | Value |
| --- | --- |
| `days` | a list of `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun` (any case); the rule rings only on those days. Absent is every day; an empty list or a word that is not a day makes the rule ignored. |
| `query` | a list of Tasks query lines, in place of `note` and `heading`: at `at`, the rule rings when the query has one or more open tasks. The body is `N tasks: ` and the first three tasks' text, as the digest writes them; a click opens Today. |
| `quiet_days` | 1 to 3650, with `note` and in place of `heading`: at `at`, the rule rings when the note has not changed for that many days. A change is the note's last commit in the vault's git history, else the file's modified time when the vault has no git. The body is `No change for N days`. |

A rule has exactly one of `heading`, `query` and `quiet_days`; with none or more than one it is ignored. `note` is required with `heading` and `quiet_days` and is not read with `query`.

- A query rule with no `title` is titled `Tasks`. Its body is `N tasks: ` and the first three tasks' text joined with `, `; it says `tasks` also when N is 1.
- A quiet rule's body gives the real number of days since the change, `No change for 15 days`. When the note does not exist, the body is `No such note: <path>`.
- `message`, when set, is the body of every kind of rule.

## 6. Rules for a folder

A section `[folders."<folder>"]` sets three things for the notes under that folder. The folder is a path in the vault, quoted, with no `/` at the end: `[folders."Personal"]`, `[folders."Companies/Snapp"]`.

| Key | Value | What it changes |
| --- | --- | --- |
| `direction` | `"rtl"`, `"ltr"` or `"auto"` (the default) | The base direction of the note in the reader and the editor. `auto` is each paragraph's own, as now. |
| `folded` | a boolean | Whether its notes open folded, over `reader.folded` (`folded-sections.md`). |
| `template` | a path in the vault | The template a new note in the folder starts from (`templates.md`, and New Note). |

The most specific folder wins, key by key: a note in `Companies/Snapp/Roles` takes `Companies/Snapp`'s `template` and `Companies`'s `folded` when only those set them. A folder that does not exist is no error.

## 7. Tests every app must have

1. `theWorkingDayIsTheSetting`: `calendar.working_day = "08:00-16:30"` gives a timeline whose free time is counted from 480 to 990 minutes; `"18:00-09:00"` is the default.
2. `aNewTaskIsScheduled`: `tasks.default_scheduled = 3` on 2026-11-25 starts the form with `➕ 2026-11-25 ⏳ 2026-11-28`; 0 starts it with `➕` only.
3. `theDigestIsFiltered`: with `notifications.digest_filter = ["path does not include Elaheh"]`, a due task in `Tasks/Elaheh Interviews.md` is not counted and one in `Tasks/Personal.md` is.
4. `theStagesAreTheSettings`: `jobs.closed = ["rejected", "ghosted"]` makes a note with `job_stage: ghosted` closed and one with `job_stage: withdrawn` open.
5. `aRuleRingsOnItsDays`: a rule with `days = ["mon", "Fri"]` is due on Monday 2026-11-23 and not on Tuesday 2026-11-24; `days = ["someday"]` is no rule.
6. `aQueryRule`: `query = ["due before tomorrow", "not done"]` with two such tasks gives the body `2 tasks: ` and their text; with none, nothing.
7. `aQuietRule`: `quiet_days = 14` on a note last changed 15 days before rings, at 13 days does not.
8. `theNearestFolderWins`: `[folders."Companies"]` `folded = false` and `[folders."Companies/Snapp"]` `template = "T.md"` give a note in `Companies/Snapp/Roles` `folded = false` and `template = "T.md"`, and a note in `Personal` neither.
