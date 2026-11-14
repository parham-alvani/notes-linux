# Today's timeline: the day's meetings and timed tasks on one axis

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Nothing new is written to the vault: the timeline reads the week notes in `Calendar/` and the tasks, and a time on a task is plain text that Obsidian and its Tasks plugin keep as part of the task's description.

Today lists the day's meetings and its tasks apart. The timeline puts them on one vertical axis of hours: the meetings from the week note, the open tasks of the day that carry a time, the free time between them, and a line at now. It answers "what is next, and when can I do the rest" at a glance, on the phone above all.

## 1. A time on a task

A task has a time when its text (the description, without the box, the signifiers and the `[reason:: …]` field, as the task lists show it) **starts** with one, as the Obsidian Day Planner plugin writes it:

```markdown
- [ ] 09:30 Call Sara about the trip ⏳ 2026-11-14
- [ ] 14:00-15:30 Write the review 📅 2026-11-14
- [ ] 7:15 – 7:45 Run
```

- A time is `H:MM` or `HH:MM`, hours `0`–`23` and minutes `00`–`59`, in ASCII digits.
- It may be followed by an end: `-` or `–` (en dash), with at most one space on each side, then a second time. A dash not followed by a time is no range: `10:00 - later` is a time of 10:00 with the text `- later`, and `10:00-x` is no time. An end that is not a time of the day, as in `10:00-25:00`, makes the whole no time.
- The time (or the range) is followed by a space or by the end of the text; `10:00am`, `9:3` and `24:00` are no time.
- The task lasts from its start to its end; with no end, or an end that is not after the start, it lasts **30 minutes**, cut at 24:00.
- The text shown on the timeline is the rest, trimmed: `Call Sara about the trip`. Everywhere else the text is unchanged.
- A time is the viewer's wall-clock time: a task has no time zone.

`TaskTime.parse` (Swift), `TaskTime::parse` (Rust) and its Kotlin port return the start and end, in minutes from midnight, and the rest of the text; or nothing.

## 2. What is on the timeline

The timeline is for one day, in the viewer's time zone, and is built from:

- **Meetings**: every meeting with a time, from the day's week note and the day before (a meeting is listed on the day it starts, so `23:00–Sat 01:00` is on Friday's list), that shares some time with the day. Its start and end are converted to the viewer's wall clock and cut at the day's midnights: a meeting from 23:00 to 01:00 is 23:00–24:00 on its first day and 00:00–01:00 on the next. A meeting left with no time in the day is not on the timeline.
- **All-day events**: the day's own all-day meetings, above the axis, in their order in the note.
- **Tasks**: the open tasks (by their status's type, as everywhere else) whose day is the timeline's day and whose text has a time (§1). A task's day is its due date, else its scheduled date, as Today groups tasks.

Minutes are counted from the day's midnight on the wall clock: 09:30 is 570 and the end of the day is 1440, also on a day when the clocks change.

## 3. Lanes

Meetings and tasks that share time stand side by side:

1. The items are ordered by start, then by the longer first, then meetings before tasks, then in the order they came.
2. A **cluster** is a run of items in that order where each one starts before the latest end so far. An item that starts exactly when the others end (back to back) starts a new cluster.
3. In a cluster, each item takes the first lane, counting from 0, whose last item has ended by its start.
4. Every item of a cluster has `lanes`, the number of lanes that cluster used. An item is drawn in the `lane`-th of `lanes` equal columns.

## 4. The axis, free time and now

- **The working day** is 09:00 to 18:00, as the week notes count free time, unless the app knows another (`calendar.hours` on Android).
- **The axis** runs from the earliest of the working day's start, the first item's start and now, rounded down to the hour, to the latest of the working day's end, the last item's end and now, rounded up to the hour, within 00:00–24:00. With nothing in the day it is the working day.
- **Free time** is every stretch of the working day that no meeting and no task takes, of 15 minutes or more. An all-day event takes no time here, though the week note's own `Free:` line counts an all-day absence as the whole day. A tentative meeting takes its time, as in the week note's `Free:` line. Free time is computed from the timeline's own items, so a task with a time makes the free time around it shorter.
- **Now** is the minute of the wall clock when the day is today, and there is none on another day. An item that ended by now is **past**; one that started by now and has not ended is **current**.

`Timeline.duration` writes a length as the free time is labelled: `45m`, `2h`, `1h 30m`.

## 5. How it is shown

On Today, as its own card under the day's summary, and only when the day has a week note or a task with a time; the Meetings card stays as it is, with its join links and prep buttons.

- The hours are written on the left, one line across per hour, the axis's hours only.
- A **meeting** is a box in the meetings' color (blue) with its time and title. The time is the meeting's own start and end on the viewer's clock, not the times cut at midnight: the box from 00:00 to 01:00 of a meeting that began at 23:00 the day before says `23:00–01:00`. A tentative one is fainter with a dashed edge; a busy-only one is grey. Pressing it opens its join link when it has one; its tooltip or long press shows its calendars and overlaps.
- A **task** is a box in the accent color with a box to tick it (the ordinary toggle) and its text; pressing it opens its note at its line (on Android, which opens a note only at a heading, at the task's section).
- **Free time** is a faint green band behind the lanes, labelled `free 1h 30m` when it is tall enough to hold the label.
- **Now** is a red line across with a dot at the left. What is past is dimmed. On opening, the page shows Today from the top, and the timeline itself is never scrolled on its own.
- All-day events stand above the axis as chips.
- A box is at least as tall as one line of its text, so a 10-minute meeting is readable; a box made taller this way may cover the top of the next one in its lane, and the earlier box is drawn under the later.

Scale: about one point a minute on a desktop (an hour is 60 points) and 1 dp a minute on the phone.

## 6. Tests every app must have

Ported by name (snake case in Rust, camelCase in Kotlin). Times are in UTC and the viewer's zone is UTC unless a test says otherwise.

1. `aTaskTimeIsReadFromTheStartOfItsText`:
    - `09:30 Call Sara` is 570–600, `Call Sara`;
    - `9:30-10:15 Gym` is 570–615, `Gym`; `14:00 – 15:30 Review` is 840–930, `Review`; `7:15–7:45 Run` is 435–465, `Run`;
    - `23:50 Late` is 1430–1440; `12:00` alone is 720–750 with the text empty; `10:00-09:00 Back` is 600–630;
    - `10:00 - later` is 600–630, `- later`;
    - `10:00am Coffee`, `9:3 x`, `24:00 x`, `Call at 09:30`, `12:60 x`, `10:00-x`, `10:00-25:00 x` and `` are no time.
2. `meetingsAndTasksShareLanesWhenTheyOverlap`: meetings A 09:00–10:00, B 09:30–10:30, C 10:00–11:00, and the task `11:00 Write`: A is lane 0 of 2, B lane 1 of 2, C lane 0 of 2, the task lane 0 of 1. A meeting and a task both 13:00–13:30: the meeting is lane 0 and the task lane 1, of 2.
3. `theAxisCoversTheWorkingDayAndEveryItem`: with nothing, 09:00–18:00 (540–1080); with a meeting 07:45–08:20, 07:00–18:00; with a meeting 17:30–19:10, 09:00–20:00; with now at 06:10 and nothing else, 06:00–18:00.
4. `freeTimeIsWhatNothingTakes`: meetings 09:00–10:00 and 12:30–13:00 (tentative), the task `13:00-13:40 Lunch`, and the task `17:50 Call`: free time is 10:00–12:30 and 13:40–17:50; with a meeting 10:00–12:20 added, 12:20–12:30 is too short and left out.
5. `aMeetingOverMidnightIsCut`: a meeting on 2026-11-13 from 23:00 to 01:00 is 1380–1440 on 2026-11-13 and 0–60 on 2026-11-14; a meeting on 2026-11-12 is on neither 2026-11-14 nor the day after.
6. `theViewersZoneMovesAMeeting`: a meeting 08:00–09:00 UTC on 2026-11-14, seen in Asia/Tehran (UTC+03:30), is 690–750.
7. `nowMarksWhatIsPastAndCurrent`: meetings 09:00–10:00 and 10:00–11:00 with now at 10:00: the first is past, the second current, and now is 600; on another day there is no now and nothing is past.
8. `onlyTheDaysOpenTasksWithATime`: of `- [ ] 10:00 A 📅 <day>`, `- [x] 11:00 B 📅 <day>`, `- [ ] 12:00 C 📅 <day after>`, `- [ ] D 📅 <day>` and `- [ ] 13:00 E ⏳ <day>`, only A and E are on the timeline; a task with a due date the day after and a scheduled date of the day is not.
9. `aLengthIsWrittenShort`: 45 is `45m`, 120 is `2h`, 90 is `1h 30m`, 0 is `0m`.
