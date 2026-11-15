//! `daftar-cli ics`: the vault's dates as an iCalendar file
//! (`daftar_core::ICalendar`), for a calendar app to import or subscribe
//! to. The open tasks with a 📅 (else ⏳) day in the days ahead, all day,
//! or at their time when the text starts with one (`TaskTime`); and the job
//! search's booked events (`job_next`), with the company and its stage. The
//! week notes' meetings are left out: they came from a calendar already.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Duration, NaiveDate, Utc};

use daftar_core::{Day, ICalendar, IcsEvent, IcsTime, JobSearch, TaskLine, TaskTime, VaultTask};

/// How long a booked job event is taken to last, in minutes.
const JOB_MINUTES: i64 = 60;

/// The events of [tasks] and [jobs] from [today] for [days] days: the
/// closed tasks left out, and the ones of a day before [today] too (a
/// calendar is no list of what is late).
pub fn events(tasks: &[VaultTask], jobs: &JobSearch, today: Day, days: i64) -> Vec<IcsEvent> {
    let first = today.to_string();
    let last = today.adding_days(days.max(1)).to_string();
    let mut tasks: Vec<&VaultTask> = tasks
        .iter()
        .filter(|t| !TaskLine::is_closed(t.task.status))
        .filter(|t| t.task.day().is_some_and(|d| d >= first.as_str() && d < last.as_str()))
        .collect();
    tasks.sort_by(|a, b| (a.task.day(), &a.path, a.task.line).cmp(&(b.task.day(), &b.path, b.task.line)));
    let mut events = Vec::new();
    // The same text twice in a note: the second gets `2-` before its UID.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for task in tasks {
        let Some(day) = task.task.day().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else { continue };
        let uid = ICalendar::uid(&[&task.path, &task.task.text]);
        let count = seen.entry(uid.clone()).or_default();
        *count += 1;
        let uid = if *count == 1 { uid } else { format!("{count}-{uid}") };
        let midnight = day.and_hms_opt(0, 0, 0).unwrap_or_default();
        let (start, end, summary) = match TaskTime::parse(&task.task.text) {
            Some(time) => (
                IcsTime::Local(midnight + Duration::minutes(i64::from(time.start))),
                IcsTime::Local(midnight + Duration::minutes(i64::from(time.end))),
                if time.text.is_empty() { task.task.text.clone() } else { time.text },
            ),
            None => (IcsTime::Day(day), IcsTime::Day(day + Duration::days(1)), task.task.text.clone()),
        };
        let mut description = task.path.clone();
        if !task.task.section.is_empty() {
            description.push_str(&format!(" › {}", task.task.section));
        }
        if task.task.status == '/' {
            description.push_str("\nIn progress");
        }
        for (mark, name, value) in [("⏳", "scheduled", &task.task.scheduled), ("📅", "due", &task.task.due)] {
            if let Some(value) = value {
                description.push_str(&format!("\n{mark} {name} {value}"));
            }
        }
        events.push(IcsEvent { uid, summary, description, categories: vec!["Task".into()], start, end });
    }

    let from = today.date().and_hms_opt(0, 0, 0).unwrap_or_default();
    for application in jobs.upcoming(from, Some(from + Duration::days(days.max(1)))) {
        let Some(next) = application.next else { continue };
        let (start, end) = if application.next_has_time {
            (IcsTime::Local(next), IcsTime::Local(next + Duration::minutes(JOB_MINUTES)))
        } else {
            (IcsTime::Day(next.date()), IcsTime::Day(next.date() + Duration::days(1)))
        };
        let mut summary = format!("{} ({})", application.company, application.stage);
        if let Some(step) = &application.next_step {
            summary.push_str(&format!(": {step}"));
        }
        let mut description = application.path.clone();
        if let Some(prep) = &application.prep {
            description.push_str(&format!("\nPrep: {prep}"));
        }
        events.push(IcsEvent {
            // One event an application: a new `job_next` moves it.
            uid: ICalendar::uid(&[&application.path, "job_next"]),
            summary,
            description,
            categories: vec!["Job search".into()],
            start,
            end,
        });
    }
    events
}

/// The calendar of [events] made at [stamp].
pub fn calendar(events: &[IcsEvent], stamp: DateTime<Utc>) -> String {
    ICalendar::write("Daftar", events, stamp)
}

/// Writes [text] to [output] through a temporary file and a rename, so a
/// calendar app reading it never sees half of it; else to stdout.
pub fn put(text: &str, output: Option<&Path>) -> Result<(), String> {
    let Some(output) = output else {
        let mut out = std::io::stdout().lock();
        return match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.to_string()),
            _ => Ok(()),
        };
    };
    let name = output.file_name().ok_or_else(|| format!("{} is not a file", output.display()))?;
    let temporary = output.with_file_name(format!(".{}.tmp", name.to_string_lossy()));
    fs::write(&temporary, text).map_err(|e| format!("{}: {e}", temporary.display()))?;
    fs::rename(&temporary, output).map_err(|e| format!("{}: {e}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::{JobKeys, Vault};

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "Tasks.md",
            "# Home\n\n- [ ] Pay rent, then call; done 📅 2026-11-16\n- [ ] 09:30-10:15 Dentist ⏳ 2026-11-17\n- [ ] Late 📅 2026-11-01\n- [x] Done 📅 2026-11-16\n- [ ] Far 📅 2027-03-01\n- [ ] Pay rent, then call; done 📅 2026-11-18\n",
        );
        write("Calendar/2026-W47.md", "# Week 47\n\nTimes are UTC.\n\n## Monday 16 November\n\n- 10:00–11:00 Planning\n");
        write("Companies/Acme/Acme.md", "---\njob_stage: screening\njob_next: 2026-11-17T15:30\njob_next_step: \"Recruiter call\"\n---\n");
        write("Companies/Acme/Interview Preparation.md", "# Prep\n");
        write("Companies/Globex/Globex.md", "---\njob_stage: applied\njob_next: 2026-11-20\n---\n");
        root
    }

    fn made(root: &Path) -> Vec<IcsEvent> {
        let index = Vault::index(root, None);
        let jobs = JobSearch::new(&index, &JobKeys::default(), |_| true);
        events(&index.tasks(), &jobs, Day::parse("2026-11-15").unwrap(), 30)
    }

    #[test]
    fn dated_tasks_and_job_events_are_the_events() {
        let root = vault();
        let events = made(root.path());
        let summaries: Vec<&str> = events.iter().map(|e| e.summary.as_str()).collect();
        assert_eq!(
            summaries,
            ["Pay rent, then call; done", "Dentist", "Pay rent, then call; done", "Acme (screening): Recruiter call", "Globex (applied)"]
        );
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!((events[0].start, events[0].end), (IcsTime::Day(day("2026-11-16")), IcsTime::Day(day("2026-11-17"))));
        assert_eq!(events[0].description, "Tasks.md › Home\n📅 due 2026-11-16");
        let at = |d: &str, h, m| IcsTime::Local(day(d).and_hms_opt(h, m, 0).unwrap());
        assert_eq!((events[1].start, events[1].end), (at("2026-11-17", 9, 30), at("2026-11-17", 10, 15)));
        assert_eq!((events[3].start, events[3].end), (at("2026-11-17", 15, 30), at("2026-11-17", 16, 30)));
        assert_eq!(events[3].description, "Companies/Acme/Acme.md\nPrep: Companies/Acme/Interview Preparation.md");
        assert_eq!(events[4].start, IcsTime::Day(day("2026-11-20")));
        // The week note's meeting is not one of them.
        assert!(!summaries.contains(&"Planning"));
    }

    #[test]
    fn a_tasks_uid_follows_its_text_not_its_line_or_dates() {
        let root = vault();
        let before = made(root.path());
        // A line added above every task, and the dentist moved a day.
        let text = fs::read_to_string(root.path().join("Tasks.md")).unwrap();
        let text = text.replace("# Home\n", "# Home\n\n- [ ] New\n").replace("⏳ 2026-11-17", "⏳ 2026-11-18");
        fs::write(root.path().join("Tasks.md"), text).unwrap();
        let after = made(root.path());
        let uid = |events: &[IcsEvent], summary: &str| events.iter().find(|e| e.summary == summary).unwrap().uid.clone();
        assert_eq!(uid(&before, "Dentist"), uid(&after, "Dentist"));
        assert_eq!(uid(&before, "Globex (applied)"), uid(&after, "Globex (applied)"));
        // The same text twice in a note: two UIDs, in the order of their days.
        assert_eq!(before[0].uid, ICalendar::uid(&["Tasks.md", "Pay rent, then call; done"]));
        assert_eq!(before[2].uid, format!("2-{}", before[0].uid));
    }

    #[test]
    fn the_file_is_written_whole() {
        let root = vault();
        let stamp = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap().and_hms_opt(8, 0, 0).unwrap().and_utc();
        let text = calendar(&made(root.path()), stamp);
        assert!(text.contains("SUMMARY:Pay rent\\, then call\\; done\r\n"));
        assert!(text.contains("DTSTART:20261117T093000\r\n"));
        let output = root.path().join("out/daftar.ics");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        put(&text, Some(&output)).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), text);
        assert_eq!(fs::read_dir(output.parent().unwrap()).unwrap().count(), 1);
    }
}
