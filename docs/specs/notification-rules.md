# Notification rules: an alert when a note lacks today's heading

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Nothing is written to the vault: a rule is a few lines of settings, and the check only reads a note.

The meeting alerts and the morning digest are fixed. A rule is a notification the person defines: "at 21:00, if `Diet/Diary.md` has no heading for today, tell me". It is for a note kept day by day (a food diary, a journal, a log) whose day is easy to forget.

## 1. The rule

A rule is a section of the settings, `[notifications.rules.<id>]`, in the vault's `.daftar.toml` or in the computer's `config.toml`:

```toml
[notifications.rules.diary]
note = "Diet/Diary.md"
heading = "D MMM YYYY · dddd"
at = "21:00"
title = "Diary"
message = "Nothing is logged in the diary today."
```

| Key | Value | Required |
| --- | --- | --- |
| `note` | a path in the vault, relative, with no `..`, as `home.image` | yes |
| `heading` | a moment.js format of the day's heading, as `.obsidian/daily-notes.json` writes a note's name: `D MMM YYYY · dddd` is `25 Nov 2026 · Wednesday`. Text in `[ ]` is literal | yes |
| `at` | `HH:MM`, the time of the check on the device's wall clock | yes |
| `title` | the notification's title; the note's name without `.md` when absent or empty | no |
| `message` | the notification's body; when absent or empty, `No heading for today yet: <heading>`, with the day's heading | no |

- `<id>` is one or more of `A`–`Z`, `a`–`z`, `0`–`9`, `_` and `-`. A section whose id has another character is no rule.
- A rule with no `note`, `heading` or `at`, or with one that does not fit the table, is ignored as a whole; the other rules stand. A key the table does not name is left alone, so a later kind of rule can add keys.
- The rules are read in the order the file writes their sections, the vault's first. A vault's rule and a computer's rule with the same id are one rule: each key is the vault's when the vault sets it (`docs/specs/vault-config.md`), as for every other setting.
- A vault may set every key of a rule: a rule reads one note of the vault and starts nothing. `notifications.rules.<id>.note` is checked as an in-vault path, and a path that leaves the vault makes the rule ignored.
- `daftar-cli config` lists each key of each rule, with where its value came from.

Each core has the type `NotificationRule` (`notification_rules.rs`, `NotificationRule.swift`, `NotificationRule.kt`) with the rules read from the settings (`NotificationRule::all`), the expected heading of a day (`expected`), whether a note's text has it (`is_missing`), and the title and body of its alert.

## 2. The check

On a day `D`, the rule's **expected heading** is its `heading` format applied to `D` (`MomentFormat.format` / `MomentFormat::format`, with the time tokens at 00:00). The note **has today's heading** when one of its ATX headings (any level, as the note parser reads them: not inside a code fence, a `%%` comment or the front matter) has a text, trimmed, that **starts with** the expected heading. So `### 25 Nov 2026 · Wednesday` and `## 25 Nov 2026 · Wednesday (fasting)` both count; a paragraph that says `25 Nov 2026` does not.

A note that does not exist has no heading: the alert rings, which also tells of a wrong path.

At `at` on day `D`, when the note has no heading for `D`, the app sends the alert:

- **Title**: `title`, else the note's name.
- **Body**: `message`, else `No heading for today yet: 25 Nov 2026 · Wednesday`.
- **Identifier**: `rule.<id>`, so a day's alert replaces the day before's.
- **A click** opens the note.

A rule is checked on every day. It rings at most once a day.

## 3. When each app checks

| App | How |
| --- | --- |
| Linux (the app) | With the other alerts (`notifier.rs`): planned again whenever the vault changes, so a heading written before `at` takes the alert away. Rings while Daftar runs, at most ten minutes late. |
| Linux and macOS (`daftar-cli agenda --notify`) | Each run (the timer runs it every minute) sends each rule that is due: `at` has come today, no more than ten minutes ago, the note has no heading for today, and `rule.<id>.<YYYY-MM-DD>` is not in the state file of sent alerts. |
| macOS (the app) | A notification request for the next `at` per rule, planned with the other alerts whenever the vault changes; a request for a day whose heading is there already is not made, and one made is taken away when the heading appears. The system holds it while the app is closed, so a heading written on another device and not pulled yet does not take it away. |
| Android | A job at the next `at` per rule (as `TaskDigestWorker`) that reads the note when it runs, so the check is at the time of the alert, after a sync. |

## 4. Tests every app must have

1. `aRuleIsReadFromItsSection`: `[notifications.rules.diary]` with `note = "Diet/Diary.md"`, `heading = "D MMM YYYY · dddd"`, `at = "21:00"` gives one rule, id `diary`, at 21:00, title `Diary`, with no message.
2. `aRuleThatDoesNotFitIsIgnored`: with no `at`; with `at = "25:00"`; with `note = "../x.md"`; with the id `my diary` (a space): no rule. A second, good rule beside them is read.
3. `theExpectedHeadingIsTheDayInTheFormat`: `D MMM YYYY · dddd` on 2026-11-25 is `25 Nov 2026 · Wednesday`; `[Day] YYYY-MM-DD` is `Day 2026-11-25`.
4. `todaysHeadingIsFound`: on 2026-11-25, `## 24 Nov 2026 · Tuesday\n` lacks it; `### 25 Nov 2026 · Wednesday\n` has it; `## 25 Nov 2026 · Wednesday (fasting)\n` has it; `25 Nov 2026 · Wednesday\n` (a paragraph) lacks it; the heading inside a fenced code block lacks it; an empty note lacks it.
5. `theAlertSaysWhatIsMissing`: with no `message`, the body is `No heading for today yet: 25 Nov 2026 · Wednesday` and the title `Diary`; with a message and a title, those.
6. `aVaultMaySetARule`: a `.daftar.toml` with the rule of test 1 gives the rule, and `daftar-cli config` (Linux) names its keys as from the vault.
