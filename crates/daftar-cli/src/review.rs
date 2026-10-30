//! `daftar-cli review`: a week of the vault as one note. What was done and
//! cancelled (by the tasks' ✅ and ❌ days), what is overdue, what is due in
//! the week ahead and in progress, the meetings of the week notes in
//! `Calendar/` behind and ahead, the notes git saw change, and the tasks
//! `check` would warn of. Its items are plain bullets, not task lines, so a
//! review never shows in a tasks query as a second copy of a task.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::Path;

use daftar_core::{CalendarWeek, Day, Git, ParsedTask, TaskStatuses, TaskType, Vault, VaultIndex};

use crate::check;

/// How many items a list shows before it says how many more there are.
const SHOWN: usize = 200;

/// The days a review covers, both included.
#[derive(Clone, Copy)]
pub struct Week {
    pub from: Day,
    pub to: Day,
}

impl Week {
    /// The seven days before [day]: on a Saturday, Saturday to Friday.
    pub fn before(day: Day) -> Week {
        Week { from: day.adding_days(-7), to: day.adding_days(-1) }
    }

    fn holds(self, day: &str) -> bool {
        day >= self.from.to_string().as_str() && day <= self.to.to_string().as_str()
    }

    /// The review's note, in [folder].
    pub fn path(self, folder: &str) -> String {
        let folder = folder.trim_matches('/');
        let name = format!("{} to {}.md", self.from, self.to);
        if folder.is_empty() { name } else { format!("{folder}/{name}") }
    }
}

/// A task with the note it is in.
struct Found<'a> {
    path: &'a str,
    task: &'a ParsedTask,
}

/// The review of [week] as Markdown; [today] is the day it is written,
/// and the week ahead starts there.
pub fn markdown(root: &Path, index: &VaultIndex, hidden: impl Fn(&str) -> bool, week: Week, today: Day) -> String {
    let statuses = TaskStatuses::load(root);
    let mut tasks: Vec<Found> = Vec::new();
    for (path, note) in &index.notes {
        if hidden(path) || CalendarWeek::is_week_note(path) {
            continue;
        }
        tasks.extend(note.parsed.tasks.iter().map(|task| Found { path, task }));
    }
    tasks.sort_by(|a, b| (a.path, a.task.line).cmp(&(b.path, b.task.line)));
    let kind = |t: &Found| statuses.kind(t.task.status);
    let open = |t: &&Found| matches!(kind(t), TaskType::Todo | TaskType::InProgress);
    let ahead = Week { from: today, to: today.adding_days(6) };
    let now = today.to_string();

    let done: Vec<&Found> =
        tasks.iter().filter(|t| kind(t) == TaskType::Done && t.task.done.as_deref().is_some_and(|d| week.holds(d))).collect();
    let cancelled: Vec<&Found> =
        tasks.iter().filter(|t| kind(t) == TaskType::Cancelled && t.task.cancelled.as_deref().is_some_and(|d| week.holds(d))).collect();
    let mut overdue: Vec<&Found> = tasks.iter().filter(open).filter(|t| t.task.due.as_deref().is_some_and(|d| d < now.as_str())).collect();
    overdue.sort_by(|a, b| a.task.due.cmp(&b.task.due));
    let mut coming: Vec<&Found> = tasks.iter().filter(open).filter(|t| t.task.day().is_some_and(|d| ahead.holds(d))).collect();
    coming.sort_by(|a, b| a.task.day().cmp(&b.task.day()));
    let progress: Vec<&Found> = tasks.iter().filter(|t| kind(t) == TaskType::InProgress).collect();
    let warnings: Vec<(&Found, String)> =
        tasks.iter().flat_map(|t| check::task_problems(t.task, &statuses, &now).into_iter().map(move |(_, what)| (t, what))).collect();
    let behind = meetings(root, week);
    let next = meetings(root, ahead);
    let changes = changes(root, week, &hidden);

    let mut text = String::new();
    let _ = writeln!(text, "# Week Review: {} to {}\n", week.from, week.to);
    let _ = writeln!(
        text,
        "Written by `daftar-cli review` on {today} from the tasks, the week notes in `Calendar/` and the vault's git history. It is not written over: the next review is a new note.\n"
    );
    let _ = writeln!(text, "## Summary\n");
    let hours = |m: &[Meeting]| m.iter().map(|m| m.minutes).sum::<i64>() as f64 / 60.0;
    let summary = [
        format!("{} tasks done and {} cancelled", done.len(), cancelled.len()),
        format!("{} overdue, {} due or scheduled in the week ahead, {} in progress", overdue.len(), coming.len(), progress.len()),
        format!(
            "{} meetings ({:.1} hours) this week, {} ({:.1} hours) in the week ahead",
            behind.len(),
            hours(&behind),
            next.len(),
            hours(&next)
        ),
        match &changes {
            Some(c) => format!("{} commits: {} notes added, {} changed", c.commits, c.added.len(), c.changed.len()),
            None => "no git history".to_string(),
        },
    ];
    for line in summary {
        let _ = writeln!(text, "- {line}");
    }

    section(&mut text, "Done", &done, |t| format!("✅ {}{}", t.task.text, place(t)));
    section(&mut text, "Cancelled", &cancelled, |t| {
        let why = t.task.reason.as_deref().map_or_else(|| " (no reason given)".to_string(), |r| format!(": {r}"));
        format!("❌ {}{why}{}", t.task.text, place(t))
    });
    section(&mut text, "Overdue", &overdue, |t| {
        let due = t.task.due.as_deref().unwrap_or_default();
        let days = Day::parse(due).map_or(0, |d| d.days_to(today));
        format!("📅 {due}, {days} days ago: {}{}", t.task.text, place(t))
    });
    section(&mut text, "The Week Ahead", &coming, |t| {
        let mark = if t.task.due.is_some() { "📅" } else { "⏳" };
        format!("{mark} {}: {}{}", t.task.day().unwrap_or_default(), t.task.text, place(t))
    });
    section(&mut text, "In Progress", &progress, |t| format!("🔄 {}{}", t.task.text, place(t)));

    let _ = writeln!(text, "\n## Meetings\n");
    if behind.is_empty() && next.is_empty() {
        let _ = writeln!(text, "No week notes in `Calendar/` for these days.");
    } else {
        for (title, list) in [("This week", &behind), ("The week ahead", &next)] {
            let mut by: BTreeMap<&str, (usize, i64)> = BTreeMap::new();
            for m in list {
                let entry = by.entry(m.calendar.as_str()).or_default();
                entry.0 += 1;
                entry.1 += m.minutes;
            }
            // An address in angle brackets, as the week notes write it: a bare one is MD034's.
            let parts: Vec<String> = by
                .iter()
                .map(|(calendar, (count, minutes))| {
                    let calendar = if calendar.contains('@') { format!("<{calendar}>") } else { (*calendar).to_string() };
                    format!("{calendar}: {count} ({:.1} h)", *minutes as f64 / 60.0)
                })
                .collect();
            let _ = writeln!(text, "- {title}: {}", if parts.is_empty() { "none".to_string() } else { parts.join(", ") });
        }
        // The ones that happen once in the week ahead stand out of the routine.
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for m in &next {
            *seen.entry(m.title.as_str()).or_default() += 1;
        }
        let once: Vec<&Meeting> = next.iter().filter(|m| seen[m.title.as_str()] == 1).collect();
        if !once.is_empty() {
            let _ = writeln!(text, "\nOnce in the week ahead:\n");
            for m in once {
                let _ = writeln!(text, "- {} {}: {}", m.day.date().format("%a %d %b"), m.time, m.title);
            }
        }
    }

    let _ = writeln!(text, "\n## Notes\n");
    match &changes {
        None => {
            let _ = writeln!(text, "The vault is not a git repository.");
        }
        Some(c) if c.commits == 0 => {
            let _ = writeln!(text, "No commits this week.");
        }
        Some(c) => {
            let mut folders: BTreeMap<String, usize> = BTreeMap::new();
            for path in c.added.iter().chain(&c.changed) {
                *folders.entry(path.split_once('/').map_or("(top)", |(f, _)| f).to_string()).or_default() += 1;
            }
            let mut folders: Vec<(String, usize)> = folders.into_iter().collect();
            folders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let parts: Vec<String> = folders.iter().map(|(f, n)| format!("{f} {n}")).collect();
            let _ = writeln!(text, "- By folder: {}", parts.join(", "));
            if !c.added.is_empty() {
                let _ = writeln!(text, "\nAdded:\n");
                for path in c.added.iter().take(SHOWN) {
                    let _ = writeln!(text, "- {}", link(path));
                }
                more(&mut text, c.added.len());
            }
        }
    }

    if !warnings.is_empty() {
        let _ = writeln!(text, "\n## Tasks to Mend\n");
        let _ = writeln!(text, "What `daftar-cli check` warns of: a review by date misses these.\n");
        for (t, what) in warnings.iter().take(SHOWN) {
            let _ = writeln!(text, "- {} line {}: {what}", link(t.path), t.task.line + 1);
        }
        more(&mut text, warnings.len());
    }
    text
}

/// A list of tasks under [title], by the note they are in.
fn section(text: &mut String, title: &str, tasks: &[&Found], line: impl Fn(&Found) -> String) {
    let _ = writeln!(text, "\n## {title}\n");
    if tasks.is_empty() {
        let _ = writeln!(text, "None.");
        return;
    }
    let mut by: BTreeMap<&str, Vec<&Found>> = BTreeMap::new();
    for task in tasks.iter().take(SHOWN) {
        by.entry(task.path).or_default().push(task);
    }
    let mut first = true;
    for (path, list) in by {
        let _ = writeln!(text, "{}{}:\n", if first { "" } else { "\n" }, link(path));
        first = false;
        for task in list {
            let _ = writeln!(text, "- {}", line(task));
        }
    }
    more(text, tasks.len());
}

fn more(text: &mut String, count: usize) {
    if count > SHOWN {
        let _ = writeln!(text, "\nAnd {} more.", count - SHOWN);
    }
}

/// The heading a task is under, when it says more than its note's name.
fn place(t: &Found) -> String {
    let section = t.task.section.trim();
    if section.is_empty() || section == Vault::title(t.path) { String::new() } else { format!(" ({section})") }
}

/// A link to a note by its path, showing its name, which no other note's name can take.
fn link(path: &str) -> String {
    let target = path.strip_suffix(".md").unwrap_or(path);
    format!("[[{target}|{}]]", Vault::title(path))
}

/// A meeting as a review counts it.
struct Meeting {
    day: Day,
    time: String,
    title: String,
    calendar: String,
    minutes: i64,
}

/// The meetings the week notes in `Calendar/` hold for [week], all-day ones left out.
fn meetings(root: &Path, week: Week) -> Vec<Meeting> {
    let mut notes = BTreeSet::new();
    let mut day = week.from;
    while day <= week.to {
        notes.insert(CalendarWeek::path(day));
        day = day.adding_days(1);
    }
    let mut found = Vec::new();
    for path in notes {
        let Ok(text) = fs::read_to_string(root.join(&path)) else { continue };
        for calendar_day in CalendarWeek::parse(&text, &path) {
            if !week.holds(&calendar_day.day.to_string()) {
                continue;
            }
            for m in calendar_day.meetings.iter().filter(|m| !m.all_day()) {
                let (Some(start), Some(end)) = (m.start, m.end) else { continue };
                let local = start.with_timezone(&chrono::Local);
                found.push(Meeting {
                    day: calendar_day.day,
                    time: local.format("%H:%M").to_string(),
                    title: m.title.clone(),
                    calendar: m.calendars.first().cloned().unwrap_or_else(|| "calendar".into()),
                    minutes: (end - start).num_minutes().max(0),
                });
            }
        }
    }
    found
}

/// What git saw in a week: its commits and the notes they added or changed.
struct Changes {
    commits: usize,
    added: BTreeSet<String>,
    changed: BTreeSet<String>,
}

fn changes(root: &Path, week: Week, hidden: impl Fn(&str) -> bool) -> Option<Changes> {
    let git = Git::new(root);
    if !git.is_repository() {
        return None;
    }
    let since = format!("--since={} 00:00", week.from);
    let until = format!("--until={} 00:00", week.to.adding_days(1));
    let log = git.run(&["log", &since, &until, "--name-status", "--no-renames", "--format=%x00%H"]).ok()?;
    let mut changes = Changes { commits: 0, added: BTreeSet::new(), changed: BTreeSet::new() };
    for line in log.lines() {
        if line.starts_with('\0') {
            changes.commits += 1;
            continue;
        }
        let Some((status, path)) = line.split_once('\t') else { continue };
        if !Vault::is_note(path) || hidden(path) || CalendarWeek::is_week_note(path) || path.starts_with('.') || !root.join(path).exists() {
            continue;
        }
        match status {
            "A" => {
                changes.added.insert(path.to_string());
            }
            _ => {
                changes.changed.insert(path.to_string());
            }
        }
    }
    let added = changes.added.clone();
    changes.changed.retain(|p| !added.contains(p));
    Some(changes)
}

/// Writes [text] as the review's note, never over one that is there.
pub fn write(root: &Path, path: &str, text: &str) -> Result<(), String> {
    let file = root.join(path);
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder).map_err(|e| format!("{path}: {e}"))?;
    }
    let mut made = fs::OpenOptions::new().write(true).create_new(true).open(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{path} is there already"),
        _ => format!("{path}: {e}"),
    })?;
    std::io::Write::write_all(&mut made, text.as_bytes()).map_err(|e| format!("{path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn a_week_is_the_seven_days_before() {
        let week = Week::before(Day::parse("2026-10-31").unwrap());
        assert_eq!((week.from.to_string(), week.to.to_string()), ("2026-10-24".into(), "2026-10-30".into()));
        assert_eq!(week.path("Reviews"), "Reviews/2026-10-24 to 2026-10-30.md");
        assert!(week.holds("2026-10-24") && week.holds("2026-10-30") && !week.holds("2026-10-31"));
    }

    #[test]
    fn it_reviews_the_tasks_of_the_week() {
        let dir = vault(&[
            (
                "Tasks/Work.md",
                "# Work\n\n## Deploy\n\n- [x] Ship it ✅ 2026-10-28\n- [x] Old ✅ 2026-09-01\n- [-] Drop it [reason:: no budget] ❌ 2026-10-26\n\
                 - [ ] Late 📅 2026-10-01\n- [ ] Soon ⏳ 2026-11-02\n- [/] Going\n- [x] Undated\n",
            ),
            ("Calendar/2026-W44.md", "- [x] Not a task of mine ✅ 2026-10-27\n"),
        ]);
        let index = Vault::index(dir.path(), None);
        let today = Day::parse("2026-10-31").unwrap();
        let text = markdown(dir.path(), &index, |_| false, Week::before(today), today);
        assert!(text.starts_with("# Week Review: 2026-10-24 to 2026-10-30\n"));
        assert!(text.contains("- 1 tasks done and 1 cancelled\n"), "{text}");
        assert!(text.contains("## Done\n\n[[Tasks/Work|Work]]:\n\n- ✅ Ship it (Deploy)\n"), "{text}");
        assert!(text.contains("- ❌ Drop it: no budget (Deploy)\n"));
        assert!(text.contains("- 📅 2026-10-01, 30 days ago: Late (Deploy)\n"));
        assert!(text.contains("- ⏳ 2026-11-02: Soon (Deploy)\n"));
        assert!(text.contains("- 🔄 Going (Deploy)\n"));
        assert!(text.contains("## Tasks to Mend\n"));
        assert!(text.contains("[[Tasks/Work|Work]] line 11: done with no ✅ date"));
        // Never a task line of its own, and never a week note's.
        assert!(!text.contains("- [x]") && !text.contains("- [ ]") && !text.contains("Not a task of mine"));
    }

    #[test]
    fn a_review_is_never_written_over() {
        let dir = vault(&[]);
        write(dir.path(), "Reviews/A.md", "one\n").unwrap();
        assert!(write(dir.path(), "Reviews/A.md", "two\n").is_err());
        assert_eq!(fs::read_to_string(dir.path().join("Reviews/A.md")).unwrap(), "one\n");
    }
}
