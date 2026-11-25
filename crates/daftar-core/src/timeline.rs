//! Today's timeline: the day's meetings and its tasks with a time on one axis
//! of minutes, with the free time between them and now
//! (`docs/specs/today-timeline.md`). No Swift twin of this file came first:
//! the Mac's `Timeline.swift` and the phone's port follow the same spec.

use std::sync::LazyLock;

use chrono::{DateTime, TimeZone, Timelike, Utc};
use regex::Regex;

use crate::calendar_week::Meeting;
use crate::day::Day;
use crate::vault::VaultTask;

/// The minutes of a day: 24:00 is its end.
pub const DAY_MINUTES: u32 = 24 * 60;

/// How long a task with no end lasts.
pub const TASK_MINUTES: u32 = 30;

/// The shortest free time shown, as the week notes count it.
pub const SHORTEST_FREE: u32 = 15;

/// The working day the week notes count free time in: 09:00 to 18:00,
/// unless `calendar.working_day` says otherwise.
pub const WORKING_DAY: (u32, u32) = (9 * 60, 18 * 60);

/// The working day of [file], the settings in use: `calendar.working_day`
/// (`HH:MM-HH:MM`, the start before the end), else `WORKING_DAY`.
pub fn working_day(file: &crate::ConfigFile) -> (u32, u32) {
    file.string("calendar.working_day").and_then(|text| crate::settings::working_day(&text)).unwrap_or(WORKING_DAY)
}

static TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]{1,2}):([0-9]{2})(?: ?[-–] ?([0-9]{1,2}):([0-9]{2}))?(?:\s+|$)").unwrap());

/// The time a task's text starts with, as the Day Planner plugin writes it:
/// `09:30 Call Sara`, `14:00-15:30 Write`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTime {
    /// Minutes from midnight.
    pub start: u32,
    pub end: u32,
    /// The text after the time.
    pub text: String,
}

impl TaskTime {
    pub fn parse(text: &str) -> Option<TaskTime> {
        let found = TIME.captures(text)?;
        let minutes = |hour: &str, minute: &str| -> Option<u32> {
            let (hour, minute) = (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?);
            (hour < 24 && minute < 60).then_some(hour * 60 + minute)
        };
        let start = minutes(&found[1], &found[2])?;
        let end = match (found.get(3), found.get(4)) {
            (Some(hour), Some(minute)) => Some(minutes(hour.as_str(), minute.as_str())?),
            _ => None,
        };
        let end = end.filter(|end| *end > start).unwrap_or_else(|| (start + TASK_MINUTES).min(DAY_MINUTES));
        let rest = &text[found.get(0).map_or(0, |m| m.end())..];
        Some(TaskTime { start, end, text: rest.trim().to_string() })
    }
}

/// What a box on the timeline is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Meeting(Meeting),
    /// The task, and its text without the time.
    Task(VaultTask, String),
}

/// A meeting or a task, placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineItem {
    pub entry: Entry,
    /// Minutes from the day's midnight on the viewer's wall clock.
    pub start: u32,
    pub end: u32,
    /// Its column, of `lanes` equal ones.
    pub lane: usize,
    pub lanes: usize,
    /// Ended by now.
    pub past: bool,
    /// Started by now and not ended.
    pub current: bool,
}

/// One day's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    pub day: Day,
    /// The hours shown: whole hours, in minutes.
    pub axis: (u32, u32),
    /// The day's all-day events, in the note's order.
    pub all_day: Vec<Meeting>,
    pub items: Vec<TimelineItem>,
    /// The stretches of the working day nothing takes.
    pub free: Vec<(u32, u32)>,
    /// The minute it is now, when the day is today.
    pub now: Option<u32>,
}

impl Timeline {
    /// The timeline of [day] seen in [zone]: [meetings] are the day's and the
    /// day before's (a meeting is listed on the day it starts), [tasks] the
    /// vault's, [working] the working day in minutes.
    pub fn new<Tz: TimeZone>(
        day: Day,
        meetings: &[Meeting],
        tasks: &[VaultTask],
        zone: &Tz,
        now: Option<DateTime<Utc>>,
        working: (u32, u32),
    ) -> Timeline {
        let minute = |instant: DateTime<Utc>| -> u32 {
            let local = instant.with_timezone(zone).naive_local();
            match local.date().cmp(&day.date()) {
                std::cmp::Ordering::Less => 0,
                std::cmp::Ordering::Greater => DAY_MINUTES,
                std::cmp::Ordering::Equal => local.hour() * 60 + local.minute(),
            }
        };
        let all_day = meetings.iter().filter(|m| m.all_day() && m.day == day).cloned().collect();
        let mut items: Vec<TimelineItem> = Vec::new();
        let place = |entry: Entry, start: u32, end: u32| TimelineItem { entry, start, end, lane: 0, lanes: 1, past: false, current: false };
        for meeting in meetings {
            let (Some(start), Some(end)) = (meeting.start, meeting.end) else { continue };
            let (start, end) = (minute(start), minute(end));
            if end > start {
                items.push(place(Entry::Meeting(meeting.clone()), start, end));
            }
        }
        let today = day.to_string();
        for task in tasks {
            if !task.task.is_open() || task.task.day() != Some(today.as_str()) {
                continue;
            }
            if let Some(time) = TaskTime::parse(&task.task.text) {
                items.push(place(Entry::Task(task.clone(), time.text), time.start, time.end));
            }
        }
        // By start, the longer first, meetings before tasks; a stable sort
        // keeps the order they came in.
        items.sort_by_key(|item| (item.start, std::cmp::Reverse(item.end), matches!(item.entry, Entry::Task(..))));
        lay_out(&mut items);

        let now = now
            .map(|now| now.with_timezone(zone).naive_local())
            .filter(|now| now.date() == day.date())
            .map(|now| now.hour() * 60 + now.minute());
        if let Some(now) = now {
            for item in &mut items {
                item.past = item.end <= now;
                item.current = item.start <= now && now < item.end;
            }
        }

        let first = items.iter().map(|i| i.start).chain(now).chain([working.0]).min().unwrap_or(working.0);
        let last = items.iter().map(|i| i.end).chain(now).chain([working.1]).max().unwrap_or(working.1);
        let axis = (first / 60 * 60, last.div_ceil(60).saturating_mul(60).min(DAY_MINUTES));
        let free = free_time(&items, working);
        Timeline { day, axis, all_day, items, free, now }
    }

    /// `45m`, `2h`, `1h 30m`.
    pub fn duration(minutes: u32) -> String {
        match (minutes / 60, minutes % 60) {
            (0, rest) => format!("{rest}m"),
            (hours, 0) => format!("{hours}h"),
            (hours, rest) => format!("{hours}h {rest}m"),
        }
    }
}

/// Lanes for items in order: a cluster is a run that overlaps, and each item
/// takes the first lane free by its start.
fn lay_out(items: &mut [TimelineItem]) {
    let mut cluster_start = 0;
    let mut cluster_end = 0;
    let mut lane_ends: Vec<u32> = Vec::new();
    for index in 0..items.len() {
        let (start, end) = (items[index].start, items[index].end);
        if index > cluster_start && start >= cluster_end {
            for item in &mut items[cluster_start..index] {
                item.lanes = lane_ends.len();
            }
            cluster_start = index;
            lane_ends.clear();
        }
        let lane = if let Some(lane) = lane_ends.iter().position(|e| *e <= start) {
            lane_ends[lane] = end;
            lane
        } else {
            lane_ends.push(end);
            lane_ends.len() - 1
        };
        items[index].lane = lane;
        cluster_end = if index == cluster_start { end } else { cluster_end.max(end) };
    }
    let lanes = lane_ends.len();
    for item in &mut items[cluster_start..] {
        item.lanes = lanes;
    }
}

/// The stretches of [working] that no item takes, of `SHORTEST_FREE` or more.
fn free_time(items: &[TimelineItem], working: (u32, u32)) -> Vec<(u32, u32)> {
    let mut free = Vec::new();
    let mut cursor = working.0;
    let mut add = |from: u32, to: u32| {
        if to >= from + SHORTEST_FREE {
            free.push((from, to));
        }
    };
    for item in items {
        if item.start >= working.1 {
            break;
        }
        if item.start > cursor {
            add(cursor, item.start);
        }
        cursor = cursor.max(item.end);
    }
    if cursor < working.1 {
        add(cursor, working.1);
    }
    free
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_line::TaskLine;

    fn at(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
        Day::parse(day).unwrap().date().and_hms_opt(hour, minute, 0).unwrap().and_utc()
    }

    fn meeting(day: &str, from: (u32, u32), to: (u32, u32), title: &str) -> Meeting {
        let start = at(day, from.0, from.1);
        let mut end = at(day, to.0, to.1);
        if end <= start {
            end += chrono::Duration::days(1);
        }
        Meeting {
            day: Day::parse(day).unwrap(),
            start: Some(start),
            end: Some(end),
            title: title.into(),
            calendars: vec![],
            join: None,
            tentative: false,
            busy_only: false,
            overlaps: None,
        }
    }

    fn task(line: &str) -> VaultTask {
        VaultTask { path: "Tasks.md".into(), task: TaskLine::parse(line, 0, "").unwrap() }
    }

    fn timeline(day: &str, meetings: &[Meeting], tasks: &[VaultTask], now: Option<DateTime<Utc>>) -> Timeline {
        Timeline::new(Day::parse(day).unwrap(), meetings, tasks, &Utc, now, WORKING_DAY)
    }

    fn spans(timeline: &Timeline) -> Vec<(u32, u32, usize, usize)> {
        timeline.items.iter().map(|i| (i.start, i.end, i.lane, i.lanes)).collect()
    }

    const DAY: &str = "2026-11-14";

    #[test]
    fn the_working_day_is_the_setting() {
        let file = crate::ConfigFile::new("[calendar]\nworking_day = \"08:00-16:30\"\n");
        let working = working_day(&file);
        assert_eq!(working, (480, 990));
        let day = Timeline::new(Day::parse("2026-11-25").unwrap(), &[], &[], &Utc, None, working);
        assert_eq!(day.free, [(480, 990)]);
        // An end before the start, or no value, is the default.
        assert_eq!(working_day(&crate::ConfigFile::new("[calendar]\nworking_day = \"18:00-09:00\"\n")), WORKING_DAY);
        assert_eq!(working_day(&crate::ConfigFile::new("")), WORKING_DAY);
        assert_eq!(working_day(&crate::ConfigFile::new("[calendar]\nworking_day = \"9-17\"\n")), WORKING_DAY);
    }

    #[test]
    fn a_task_time_is_read_from_the_start_of_its_text() {
        let time = |text: &str| TaskTime::parse(text).map(|t| (t.start, t.end, t.text));
        assert_eq!(time("09:30 Call Sara"), Some((570, 600, "Call Sara".into())));
        assert_eq!(time("9:30-10:15 Gym"), Some((570, 615, "Gym".into())));
        assert_eq!(time("14:00 – 15:30 Review"), Some((840, 930, "Review".into())));
        assert_eq!(time("7:15–7:45 Run"), Some((435, 465, "Run".into())));
        assert_eq!(time("23:50 Late"), Some((1430, 1440, "Late".into())));
        assert_eq!(time("12:00"), Some((720, 750, String::new())));
        assert_eq!(time("10:00-09:00 Back"), Some((600, 630, "Back".into())));
        assert_eq!(time("10:00 - later"), Some((600, 630, "- later".into())));
        for none in ["10:00am Coffee", "9:3 x", "24:00 x", "Call at 09:30", "12:60 x", "10:00-x", "10:00-25:00 x", ""] {
            assert_eq!(time(none), None, "{none}");
        }
    }

    #[test]
    fn meetings_and_tasks_share_lanes_when_they_overlap() {
        let meetings = [meeting(DAY, (9, 0), (10, 0), "A"), meeting(DAY, (9, 30), (10, 30), "B"), meeting(DAY, (10, 0), (11, 0), "C")];
        let tasks = [task(&format!("- [ ] 11:00 Write 📅 {DAY}"))];
        let line = timeline(DAY, &meetings, &tasks, None);
        assert_eq!(spans(&line), [(540, 600, 0, 2), (570, 630, 1, 2), (600, 660, 0, 2), (660, 690, 0, 1)]);
        let titles: Vec<String> = line
            .items
            .iter()
            .map(|i| match &i.entry {
                Entry::Meeting(m) => m.title.clone(),
                Entry::Task(_, text) => text.clone(),
            })
            .collect();
        assert_eq!(titles, ["A", "B", "C", "Write"]);

        let both = timeline(DAY, &[meeting(DAY, (13, 0), (13, 30), "M")], &[task(&format!("- [ ] 13:00 T 📅 {DAY}"))], None);
        assert_eq!(spans(&both), [(780, 810, 0, 2), (780, 810, 1, 2)]);
        assert!(matches!(both.items[0].entry, Entry::Meeting(_)));
    }

    #[test]
    fn the_axis_covers_the_working_day_and_every_item() {
        assert_eq!(timeline(DAY, &[], &[], None).axis, (540, 1080));
        assert_eq!(timeline(DAY, &[meeting(DAY, (7, 45), (8, 20), "Early")], &[], None).axis, (420, 1080));
        assert_eq!(timeline(DAY, &[meeting(DAY, (17, 30), (19, 10), "Late")], &[], None).axis, (540, 1200));
        assert_eq!(timeline(DAY, &[], &[], Some(at(DAY, 6, 10))).axis, (360, 1080));
    }

    #[test]
    fn free_time_is_what_nothing_takes() {
        let mut tentative = meeting(DAY, (12, 30), (13, 0), "Maybe");
        tentative.tentative = true;
        let mut meetings = vec![meeting(DAY, (9, 0), (10, 0), "First"), tentative];
        let tasks = [task(&format!("- [ ] 13:00-13:40 Lunch 📅 {DAY}")), task(&format!("- [ ] 17:50 Call 📅 {DAY}"))];
        assert_eq!(timeline(DAY, &meetings, &tasks, None).free, [(600, 750), (820, 1070)]);
        meetings.push(meeting(DAY, (10, 0), (12, 20), "Long"));
        assert_eq!(timeline(DAY, &meetings, &tasks, None).free, [(820, 1070)]);
    }

    #[test]
    fn a_meeting_over_midnight_is_cut() {
        let late = [meeting("2026-11-13", (23, 0), (1, 0), "Late")];
        assert_eq!(spans(&timeline("2026-11-13", &late, &[], None)), [(1380, 1440, 0, 1)]);
        assert_eq!(spans(&timeline("2026-11-14", &late, &[], None)), [(0, 60, 0, 1)]);
        let old = [meeting("2026-11-12", (10, 0), (11, 0), "Old")];
        assert!(timeline("2026-11-14", &old, &[], None).items.is_empty());
        assert!(timeline("2026-11-15", &old, &[], None).items.is_empty());
    }

    #[test]
    fn the_viewers_zone_moves_a_meeting() {
        let line = Timeline::new(
            Day::parse(DAY).unwrap(),
            &[meeting(DAY, (8, 0), (9, 0), "Call")],
            &[],
            &chrono_tz::Asia::Tehran,
            None,
            WORKING_DAY,
        );
        assert_eq!(spans(&line), [(690, 750, 0, 1)]);
    }

    #[test]
    fn now_marks_what_is_past_and_current() {
        let meetings = [meeting(DAY, (9, 0), (10, 0), "One"), meeting(DAY, (10, 0), (11, 0), "Two")];
        let line = timeline(DAY, &meetings, &[], Some(at(DAY, 10, 0)));
        assert_eq!(line.now, Some(600));
        assert_eq!(line.items.iter().map(|i| (i.past, i.current)).collect::<Vec<_>>(), [(true, false), (false, true)]);
        let other = timeline(DAY, &meetings, &[], Some(at("2026-11-15", 10, 0)));
        assert_eq!(other.now, None);
        assert!(other.items.iter().all(|i| !i.past && !i.current));
    }

    #[test]
    fn only_the_days_open_tasks_with_a_time() {
        let tasks = [
            task(&format!("- [ ] 10:00 A 📅 {DAY}")),
            task(&format!("- [x] 11:00 B 📅 {DAY}")),
            task("- [ ] 12:00 C 📅 2026-11-15"),
            task(&format!("- [ ] D 📅 {DAY}")),
            task(&format!("- [ ] 13:00 E ⏳ {DAY}")),
            task(&format!("- [ ] 14:00 F 📅 2026-11-15 ⏳ {DAY}")),
        ];
        let line = timeline(DAY, &[], &tasks, None);
        let texts: Vec<&str> = line
            .items
            .iter()
            .filter_map(|i| match &i.entry {
                Entry::Task(_, text) => Some(text.as_str()),
                Entry::Meeting(_) => None,
            })
            .collect();
        assert_eq!(texts, ["A", "E"]);
    }

    #[test]
    fn a_length_is_written_short() {
        assert_eq!(Timeline::duration(45), "45m");
        assert_eq!(Timeline::duration(120), "2h");
        assert_eq!(Timeline::duration(90), "1h 30m");
        assert_eq!(Timeline::duration(0), "0m");
    }
}
