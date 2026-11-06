# Daftar task rules: repeating by the Jalali calendar

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). A bill paid in Iran falls on a day of the Solar Hijri (Jalali) month, the 10th of every month, say. That day moves against the Gregorian month by up to two days through the year, so `🔁 every month` on a Gregorian date drifts off it. A Jalali rule repeats on the same Jalali day.

The dates on the task stay Gregorian, as every other date in the vault is: only the step between them is taken in the Jalali calendar. Obsidian's Tasks plugin does not know these rules; Obsidian is no longer what writes the vault.

## 1. The rule

```markdown
- [ ] Pay the Mellat instalment 🔁 every jalali month ⏳ 2026-12-31
- [ ] Renew the car insurance 🔁 every jalali year when done 📅 2027-03-20
- [ ] Pay the quarterly fee 🔁 every 3 jalali months ⏳ 2026-12-01
```

Reading, case-insensitively, after trimming and taking a trailing `when done` off as the plain intervals do:

- `every [N] jalali (month|year)[s]`: `N` is a positive whole number, 1 when it is not written, at most 10,000, the limit of every rule, plain intervals too: `every 10001 days` and `every 999999999 years` are refused, so a tick never overflows. Whitespace between the words is one or more spaces.
- `jalali` with `day` or `week` is refused: a day and a week are the same in both calendars, so `every day` and `every week` say it. Anything else (`every jalali`, `every 0 jalali months`, `every jalali month on the 10th`, `every persian month`) is refused too.
- A refused rule is what it is today: the tick completes the task without a next one, the edit form shows its warning, and `daftar-cli check` reports `task-repeat`.
- In code it is a `Recurrence` whose unit is one of two new ones, `JalaliMonth` and `JalaliYear` (`jalali_month` / `JALALI_MONTH` in each language's style). The existing four units are untouched.

## 2. A step

From a Gregorian day `D`, a step of `N` Jalali months:

1. Convert `D` to the Jalali day `(y, m, d)`.
2. Add `N` months to `(y, m)` (`N × 12` for years), carrying into the year.
3. Clamp `d` to the length of the new month: months 1 to 6 have 31 days, 7 to 11 have 30, and 12 (Esfand) has 30 in a leap year and 29 otherwise.
4. Convert back to the Gregorian day.

Everything else is the plain intervals' rule, unchanged: the step starts from the leading date (📅, else ⏳, else 🛫; or today with `when done`), steps are chained until the date is after today (a task ticked late does not come back overdue, however late: days and weeks jump there in one step, and months and years stop after 4,000 steps with no next occurrence rather than an overdue one), and every date of the task moves by the same number of days as the leading one. A chained step clamps from the day it reached, as `every month` does: a 31st that went to a 30th stays on the 30th.

The conversion is jalaali-js's arithmetic (Borkowski's leap years, as Iran's calendar sets them), good for Jalali years 1 to 3177. Linux has it as `daftar_core::jalali` (`jalali.rs`); macOS and Android port that file, with its tests, so the three agree day for day. Do not use a platform's Persian calendar (Foundation's `.persian`, ICU's `PersianCalendar`): their leap years can differ from Iran's, and the three apps must give the same date. A step outside the calendar's range leaves the task completed without a next one, never a crash.

## 3. Editing

The repeat presets of the edit form (Linux `task_edit.rs`, macOS `TaskEditSheet.swift`, and Android where it has them) gain `every jalali month`, after `every month`. The warning for a rule the app will not repeat reads: "Daftar repeats only plain intervals, such as every 2 weeks or every jalali month; Obsidian reads the rest."

## 4. Tests every app must have

The Jalali conversion (ported with the module):

- 2026-03-21 is 1405-01-01, 2024-03-20 is 1403-01-01, 2026-10-27 is 1405-08-05, 2025-03-20 is 1403-12-30, 1979-02-11 is 1357-11-22.
- Every third day from 1990-01-01, 20,000 times, goes to Jalali and back to itself. 1404-12-30 is not a day; 1403-12-30 is 2025-03-20.

Reading the rule:

| Rule | Reads as |
| --- | --- |
| `every jalali month` | 1 Jalali month |
| `Every 3 Jalali Months when done` | 3 Jalali months, when done |
| `every jalali year` | 1 Jalali year |
| `every 2 jalali years` | 2 Jalali years |
| `every jalali week`, `every jalali day`, `every jalali`, `every 0 jalali months`, `every jalali month on the 10th` | refused |

Ticking (today in brackets; the first line is the next one the tick leaves above):

| Task | Next |
| --- | --- |
| `- [ ] Pay 🔁 every jalali month ⏳ 2026-12-31` [2026-12-31] | `- [ ] Pay 🔁 every jalali month ⏳ 2027-01-30` (Dey 10 → Bahman 10; `every month` would give 01-31) |
| `- [ ] Pay 🔁 every jalali month ⏳ 2026-09-22` [2026-09-22] | `⏳ 2026-10-22` (Shahrivar 31 → Mehr 30) |
| `- [ ] Pay 🔁 every jalali year ⏳ 2025-03-20` [2025-03-20] | `⏳ 2026-03-20` (1403-12-30 → 1404-12-29, 1404 not leap) |
| `- [ ] Pay 🔁 every jalali month ⏳ 2026-09-01` [2026-11-03] | `⏳ 2026-12-01` (Shahrivar 10, stepped past Mehr 10 and Aban 10 to Azar 10) |
| `- [ ] Pay 🔁 every jalali month when done ⏳ 2026-10-01` [2026-11-01] | `⏳ 2026-12-01` (from today, Aban 10, to Azar 10) |
| `- [ ] Pay 🔁 every jalali month ➕ 2026-12-01 ⏳ 2026-12-31 📅 2027-01-02` [2026-12-31] | `➕ 2026-12-01 ⏳ 2027-01-30 📅 2027-02-01` (📅 leads, Dey 12 → Bahman 12, 30 days; ⏳ moves by the same 30; ➕ is never moved) |
| `- [ ] Pay 🔁 every 3 jalali months ⏳ 2026-12-31` [2026-12-31] | `⏳ 2027-03-30` (Dey 10 → Farvardin 10, 1406) |

The completed copy below is the plain intervals' (`- [x] … ✅ today`). Linux also checks that `daftar-cli check` reports no `task-repeat` for `🔁 every jalali month` and still reports one for `🔁 every jalali week`.

Limits, for every rule (added after the first release, in every app):

| Task | Today | Next |
| --- | --- | --- |
| `- [ ] Far 🔁 every 999999999 years 📅 2026-01-01` | 2026-01-01 | none: ticked as `- [x] … ✅ 2026-01-01` |
| `- [ ] Water 🔁 every day 📅 1990-01-01` | 2026-11-06 | `📅 2026-11-07` |
| `- [ ] Bins 🔁 every 2 weeks 📅 1990-01-01` | 2026-11-06 | `📅 2026-11-16` |
| `- [ ] Rent 🔁 every month 📅 1990-01-31` | 2026-11-06 | `📅 2026-11-28` |
