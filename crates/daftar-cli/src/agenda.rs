//! `daftar-cli agenda`: a day at a glance. Today's timeline
//! (`daftar_core::Timeline`, `docs/specs/today-timeline.md`: the week
//! notes' meetings cut to the day, the tasks whose text starts with a time,
//! the free time), the tasks overdue, due and in progress
//! (`daftar_core::DueTasks`, as the MCP's `vault_brief`), and the job
//! search's events of the day with their prep notes. As plain lines, one
//! JSON object, or one line for a Waybar custom module. `--notify` is
//! `alerts.rs`, which reads the week notes through [meetings] too.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};

use daftar_core::timeline::WORKING_DAY;
use daftar_core::{Application, CalendarWeek, Day, DueTasks, JobKeys, JobSearch, Meeting, Timeline, TimelineEntry, VaultTask};

/// How near a meeting's start makes Waybar's class `soon`, in minutes.
const SOON: u32 = 15;

/// The longest meeting title in Waybar's text, in characters.
const SHORT_TITLE: usize = 24;

/// A day's agenda, gathered.
pub struct Agenda {
    pub day: Day,
    /// The time it is, when the day is today.
    pub now: Option<NaiveDateTime>,
    /// The all-day events first, then the timeline's items by start.
    pub slots: Vec<Slot>,
    /// The working day's stretches no item takes, in minutes.
    pub free: Vec<(u32, u32)>,
    pub due: DueTasks,
    /// The job search's events of the day, with the line of their `job_next`.
    pub jobs: Vec<(Application, usize)>,
}

/// A meeting or a timed task of the day.
pub struct Slot {
    /// `None` for an all-day event.
    pub time: Option<SlotTime>,
    pub what: What,
}

pub struct SlotTime {
    /// Its own times on the wall clock, `09:30`, even for a meeting the day cuts.
    pub start: String,
    pub end: String,
    /// Minutes from the day's midnight, cut to the day.
    pub from: u32,
    pub past: bool,
    pub current: bool,
}

pub enum What {
    /// The company it is about and its prep note, when it is about one.
    Meeting { title: String, tentative: bool, company: Option<(String, String)> },
    /// The task and its text without the time.
    Task(Box<VaultTask>, String),
}

impl Agenda {
    /// The agenda of [day] seen in [zone] at [now]: the week notes are read
    /// from [root], the timed and due tasks are [tasks] (the hidden ones
    /// already left out), the events [jobs]'.
    pub fn new<Tz: TimeZone>(
        root: &Path,
        tasks: &[VaultTask],
        jobs: &JobSearch,
        keys: &JobKeys,
        day: Day,
        zone: &Tz,
        now: DateTime<Utc>,
    ) -> Agenda {
        let meetings = meetings(root, day);
        let timeline = Timeline::new(day, &meetings, tasks, zone, Some(now), WORKING_DAY);
        let local_now = Some(now.with_timezone(zone).naive_local()).filter(|n| n.date() == day.date());
        let clock = |at: DateTime<Utc>| at.with_timezone(zone).naive_local().format("%H:%M").to_string();
        let company =
            |meeting: &Meeting| jobs.for_calendar_meeting(meeting, zone).map(|a| (a.company.clone(), a.prep_or_note().to_string()));
        let meeting = |m: &Meeting| What::Meeting { title: m.title.clone(), tentative: m.tentative, company: company(m) };
        let mut slots: Vec<Slot> = timeline.all_day.iter().map(|m| Slot { time: None, what: meeting(m) }).collect();
        for item in &timeline.items {
            let minute = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
            let (start, end, what) = match &item.entry {
                TimelineEntry::Meeting(m) => {
                    (m.start.map_or_else(|| minute(item.start), clock), m.end.map_or_else(|| minute(item.end), clock), meeting(m))
                }
                TimelineEntry::Task(task, text) => (minute(item.start), minute(item.end), What::Task(Box::new(task.clone()), text.clone())),
            };
            let time = SlotTime { start, end, from: item.start, past: item.past, current: item.current };
            slots.push(Slot { time: Some(time), what });
        }
        let midnight = day.date().and_hms_opt(0, 0, 0).unwrap_or_default();
        let jobs = jobs
            .upcoming(midnight, Some(midnight + Duration::days(1)))
            .into_iter()
            .map(|a| {
                let text = fs::read_to_string(root.join(&a.path)).unwrap_or_default();
                (a.clone(), JobSearch::property_line(&text, &keys.next))
            })
            .collect();
        Agenda { day, now: local_now, slots, free: timeline.free, due: DueTasks::new(tasks, day), jobs }
    }

    /// The minute of the day it is now, when the day is today.
    fn minute(&self) -> Option<u32> {
        use chrono::Timelike;
        self.now.map(|n| n.hour() * 60 + n.minute())
    }

    /// The agenda as plain lines: a line of a note starts `path:line` and a tab.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![match self.now {
            Some(now) => format!("# {}, {}", self.day.date().format("%A %Y-%m-%d"), now.format("%H:%M")),
            None => format!("# {}", self.day.date().format("%A %Y-%m-%d")),
        }];
        // The slots, the free time and now, by when they start.
        let mut timed: Vec<(u32, u8, String)> = Vec::new();
        for slot in &self.slots {
            let Some(time) = &slot.time else {
                lines.push(format!("All day\t{}", slot.what.shown()));
                continue;
            };
            let range = format!("{}–{}", time.start, time.end);
            let line = match &slot.what {
                What::Task(task, text) => format!("{}:{}\t{range}\t[{}] {text}", task.path, task.task.line + 1, task.task.status),
                What::Meeting { .. } => format!("{range}\t{}", slot.what.shown()),
            };
            timed.push((time.from, 0, line));
        }
        for (from, to) in &self.free {
            timed.push((*from, 2, format!("{}–{}\tfree {}", clock(*from), clock(*to), Timeline::duration(to - from))));
        }
        if let Some(now) = self.minute() {
            timed.push((now, 1, format!("{}\tnow", clock(now))));
        }
        // Now after what starts at its minute, the free time last; a stable
        // sort keeps the slots' order.
        timed.sort_by_key(|(at, kind, _)| (*at, *kind));
        lines.extend(timed.into_iter().map(|(_, _, line)| line));

        for (name, list) in [("Overdue", &self.due.overdue), ("Today", &self.due.today), ("In progress", &self.due.in_progress)] {
            if !list.is_empty() {
                lines.push(format!("## {name}: {}", list.len()));
                lines.extend(list.iter().map(crate::task_line));
            }
        }
        if !self.jobs.is_empty() {
            lines.push(format!("## Job events: {}", self.jobs.len()));
            lines.extend(self.jobs.iter().map(|(a, line)| job_line(a, *line)));
        }
        lines
    }

    /// The agenda as one JSON object.
    pub fn json(&self) -> Value {
        let slots: Vec<Value> = self
            .slots
            .iter()
            .map(|slot| {
                let mut row = match &slot.what {
                    What::Meeting { title, tentative, company } => json!({
                        "kind": "meeting", "title": title, "tentative": tentative,
                        "company": company.as_ref().map(|c| &c.0), "prep": company.as_ref().map(|c| &c.1),
                    }),
                    What::Task(task, text) => {
                        json!({ "kind": "task", "title": text, "path": task.path, "line": task.task.line + 1, "status": task.task.status.to_string() })
                    }
                };
                match &slot.time {
                    Some(time) => {
                        row["start"] = time.start.clone().into();
                        row["end"] = time.end.clone().into();
                        row["past"] = time.past.into();
                        row["current"] = time.current.into();
                    }
                    None => row["all_day"] = true.into(),
                }
                row
            })
            .collect();
        let tasks = |list: &[VaultTask]| -> Value { list.iter().map(|t| crate::task_json(t, None)).collect() };
        json!({
            "day": self.day.to_string(),
            "now": self.now.map(|n| n.format("%H:%M").to_string()),
            "timeline": slots,
            "free": self.free.iter().map(|(from, to)| json!({ "start": clock(*from), "end": clock(*to), "minutes": to - from })).collect::<Vec<_>>(),
            "overdue": tasks(&self.due.overdue),
            "today": tasks(&self.due.today),
            "in_progress": tasks(&self.due.in_progress),
            "jobs": self.jobs.iter().map(|(a, line)| json!({
                "path": a.path, "line": line, "when": when(a), "company": a.company, "stage": a.stage,
                "next_step": a.next_step, "prep": a.prep,
            })).collect::<Vec<_>>(),
        })
    }

    /// One line for a Waybar custom module (`"return-type": "json"`): the
    /// next meeting of the day with how long until it, else how many tasks
    /// are overdue or due; the class `soon` when a meeting is on or starts
    /// within `SOON` minutes, else `overdue` when a task is, else `clear`,
    /// and `alt` the same, for `format-icons`; the rest in the tooltip.
    /// Waybar reads both as Pango markup, so they are escaped.
    pub fn waybar(&self) -> Value {
        let now = self.minute();
        let next = self.slots.iter().find_map(|slot| match (&slot.time, &slot.what) {
            (Some(time), What::Meeting { title, company, .. }) if !time.past => Some((time, company.as_ref().map_or(title, |c| &c.0))),
            _ => None,
        });
        let due = self.due.overdue.len() + self.due.today.len();
        let (text, soon) = match (next, now) {
            (Some((time, title)), Some(now)) if time.from <= now => (format!("{} {} now", time.start, short(title)), true),
            (Some((time, title)), Some(now)) => {
                (format!("{} {} in {}", time.start, short(title), Timeline::duration(time.from - now)), time.from - now <= SOON)
            }
            (Some((time, title)), None) => (format!("{} {}", time.start, short(title)), false),
            (None, _) => (format!("{due} due"), false),
        };
        let class = if soon {
            "soon"
        } else if self.due.overdue.is_empty() {
            "clear"
        } else {
            "overdue"
        };
        let tooltip: Vec<String> = self.lines().iter().map(|l| l.replace('\t', "  ")).collect();
        json!({ "text": markup(&text), "tooltip": markup(&tooltip.join("\n")), "class": class, "alt": class })
    }
}

impl What {
    /// A meeting's title with `(maybe)`, and the company and its prep note.
    fn shown(&self) -> String {
        match self {
            What::Meeting { title, tentative, company } => {
                let mut shown = title.clone();
                if *tentative {
                    shown.push_str(" (maybe)");
                }
                if let Some((company, prep)) = company {
                    shown.push_str(&format!("\t{company}\tprep: {prep}"));
                }
                shown
            }
            What::Task(_, text) => text.clone(),
        }
    }
}

/// The meetings of [day]'s week note and of the day before's (a meeting is
/// listed on the day it starts, and the night's one runs into [day]).
pub fn meetings(root: &Path, day: Day) -> Vec<Meeting> {
    let before = day.adding_days(-1);
    let mut paths = vec![CalendarWeek::path(before), CalendarWeek::path(day)];
    paths.dedup();
    paths
        .iter()
        .filter_map(|path| fs::read_to_string(root.join(path)).ok().map(|text| CalendarWeek::parse(&text, path)))
        .flatten()
        .filter(|d| d.day == day || d.day == before)
        .flat_map(|d| d.meetings)
        .collect()
}

/// `path:line` of the event's `job_next`, its time, the company, the stage,
/// the next step and the prep note, as `jobs` prints it.
fn job_line(application: &Application, line: usize) -> String {
    format!(
        "{}:{line}\t{}\t{}\t{}\t{}\t{}",
        application.path,
        when(application),
        application.company,
        application.stage,
        application.next_step.as_deref().unwrap_or(""),
        application.prep.as_deref().unwrap_or(""),
    )
}

fn when(application: &Application) -> String {
    application.next.map_or_else(String::new, |next| next.format(if application.next_has_time { "%H:%M" } else { "all day" }).to_string())
}

fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn short(title: &str) -> String {
    if title.chars().count() <= SHORT_TITLE {
        return title.to_string();
    }
    let cut: String = title.chars().take(SHORT_TITLE - 1).collect();
    format!("{}…", cut.trim_end())
}

/// [text] as Pango markup shows it.
fn markup(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::Vault;

    /// A made-up vault on Sunday 15 November 2026: a week note with a
    /// standup, an interview with Acme and a night call from Saturday, a
    /// timed task, an overdue one and one in progress.
    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "Calendar/2026-W46.md",
            "# Week 46\n\nTimes are UTC.\n\n## Saturday 14 November\n\n- 23:00–Sun 00:30 Night call\n\n## Sunday 15 November\n\n- All day Team offsite\n- 09:00–09:15 Standup\n- 14:00–15:00 Interview with Acme & friends (<Work>)\n",
        );
        write(
            "Companies/Acme/Acme.md",
            "---\njob_stage: interviewing\njob_next: 2026-11-15T14:00\njob_next_step: \"Second round\"\n---\n# Acme\n",
        );
        write("Companies/Acme/Interview Preparation.md", "# Prep\n");
        write(
            "Tasks.md",
            "# Tasks\n\n- [ ] 11:00-11:45 Write the <draft> 📅 2026-11-15\n- [ ] Pay rent 📅 2026-11-10\n- [/] Read the paper\n- [x] Done 📅 2026-11-15\n",
        );
        root
    }

    fn agenda(root: &Path, at: &str) -> Agenda {
        let index = Vault::index(root, None);
        let keys = JobKeys::default();
        let jobs = JobSearch::new(&index, &keys, |_| true);
        let now = NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M").unwrap().and_utc();
        Agenda::new(root, &index.tasks(), &jobs, &keys, Day::parse("2026-11-15").unwrap(), &Utc, now)
    }

    #[test]
    fn a_day_reads_as_its_timeline_then_its_tasks_and_events() {
        let root = vault();
        assert_eq!(
            agenda(root.path(), "2026-11-15T13:35").lines(),
            [
                "# Sunday 2026-11-15, 13:35",
                "All day\tTeam offsite",
                "23:00–00:30\tNight call",
                "09:00–09:15\tStandup",
                "09:15–11:00\tfree 1h 45m",
                "Tasks.md:3\t11:00–11:45\t[ ] Write the <draft>",
                "11:45–14:00\tfree 2h 15m",
                "13:35\tnow",
                "14:00–15:00\tInterview with Acme & friends\tAcme\tprep: Companies/Acme/Interview Preparation.md",
                "15:00–18:00\tfree 3h",
                "## Overdue: 1",
                "Tasks.md:4\t[ ] Pay rent 📅 2026-11-10",
                "## Today: 1",
                "Tasks.md:3\t[ ] 11:00-11:45 Write the <draft> 📅 2026-11-15",
                "## In progress: 1",
                "Tasks.md:5\t[/] Read the paper",
                "## Job events: 1",
                "Companies/Acme/Acme.md:3\t14:00\tAcme\tinterviewing\tSecond round\tCompanies/Acme/Interview Preparation.md",
            ]
        );
    }

    #[test]
    fn waybar_counts_down_to_the_next_meeting() {
        let root = vault();
        let bar = agenda(root.path(), "2026-11-15T13:35").waybar();
        assert_eq!(bar["text"], "14:00 Acme in 25m");
        assert_eq!(bar["class"], "overdue");
        assert_eq!(bar["alt"], "overdue");
        let tooltip = bar["tooltip"].as_str().unwrap();
        assert!(tooltip.contains("Interview with Acme &amp; friends"), "{tooltip}");
        assert!(tooltip.contains("Write the &lt;draft&gt;") && !tooltip.contains('\t'), "{tooltip}");

        assert_eq!(agenda(root.path(), "2026-11-15T13:50").waybar()["class"], "soon");
        let on = agenda(root.path(), "2026-11-15T14:10").waybar();
        assert_eq!((on["text"].as_str(), on["class"].as_str()), (Some("14:00 Acme now"), Some("soon")));
        let after = agenda(root.path(), "2026-11-15T16:00").waybar();
        assert_eq!((after["text"].as_str(), after["class"].as_str()), (Some("2 due"), Some("overdue")));
    }

    #[test]
    fn the_json_has_every_part() {
        let root = vault();
        let json = agenda(root.path(), "2026-11-15T13:35").json();
        assert_eq!(json["day"], "2026-11-15");
        assert_eq!(json["now"], "13:35");
        assert_eq!(json["timeline"][0]["all_day"], true);
        assert_eq!(json["timeline"][1]["start"], "23:00");
        assert_eq!(json["timeline"][1]["past"], true);
        assert_eq!(json["timeline"][3]["kind"], "task");
        assert_eq!(json["timeline"][3]["line"], 3);
        assert_eq!(json["timeline"][4]["company"], "Acme");
        assert_eq!(json["free"][0]["minutes"], 105);
        assert_eq!(json["overdue"][0]["text"], "Pay rent");
        assert_eq!(json["jobs"][0]["when"], "14:00");
        // Another day has no now.
        let index = Vault::index(root.path(), None);
        let jobs = JobSearch::new(&index, &JobKeys::default(), |_| true);
        let now = NaiveDateTime::parse_from_str("2026-11-16T08:00", "%Y-%m-%dT%H:%M").unwrap().and_utc();
        let other = Agenda::new(root.path(), &index.tasks(), &jobs, &JobKeys::default(), Day::parse("2026-11-15").unwrap(), &Utc, now);
        assert_eq!(other.json()["now"], Value::Null);
        assert_eq!(other.waybar()["text"], "23:00 Night call");
    }
}
