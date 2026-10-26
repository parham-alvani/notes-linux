//! The vault's tasks as a board: columns by status, by day or by file. A card
//! dropped on a status or a day column changes that task's line, and only it:
//! to done the task is ticked as a click ticks it, with its `✅` date and,
//! for a recurring one, the next occurrence.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::day::Day;
use crate::task_line::{ParsedTask, TaskLine};
use crate::text::standard_compare;
use crate::vault::{Vault, VaultTask};
use crate::vault_edits::{Failure, VaultEdits};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grouping {
    Status,
    Day,
    File,
}

impl Grouping {
    pub const ALL: [Grouping; 3] = [Grouping::Status, Grouping::Day, Grouping::File];

    pub fn name(self) -> &'static str {
        match self {
            Grouping::Status => "status",
            Grouping::Day => "day",
            Grouping::File => "file",
        }
    }
}

/// What a column writes on a task dropped on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Drop {
    Status(char),
    /// The due date; `None` takes it away.
    Due(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub id: String,
    pub title: String,
    pub tasks: Vec<VaultTask>,
    /// `None` for a column nothing can be dropped on, as Overdue.
    pub drop: Option<Drop>,
}

pub struct TaskBoard;

impl TaskBoard {
    /// The board of [tasks]. Done and cancelled tasks show when they closed
    /// within [closed_within] days, or all of them when it is `None`.
    pub fn columns(tasks: &[VaultTask], by: Grouping, today: Day, closed_within: Option<i64>) -> Vec<Column> {
        let column =
            |id: &str, title: &str, tasks: Vec<VaultTask>, drop: Option<Drop>| Column { id: id.into(), title: title.into(), tasks, drop };
        match by {
            Grouping::Status => {
                let since = closed_within.map(|days| today.adding_days(-days));
                let recent = |date: &Option<String>| match since {
                    None => true,
                    Some(since) => date.as_deref().and_then(Day::parse).is_some_and(|d| d >= since),
                };
                let (mut todo, mut doing, mut done, mut cancelled) = (vec![], vec![], vec![], vec![]);
                for task in tasks {
                    match task.task.status {
                        '/' => doing.push(task.clone()),
                        'x' | 'X' => {
                            if recent(&task.task.done) {
                                done.push(task.clone())
                            }
                        }
                        '-' => {
                            if recent(&task.task.cancelled) {
                                cancelled.push(task.clone())
                            }
                        }
                        _ => todo.push(task.clone()),
                    }
                }
                todo.sort_by(open);
                doing.sort_by(open);
                done.sort_by(|a, b| closed(&a.task.done, &b.task.done, a, b));
                cancelled.sort_by(|a, b| closed(&a.task.cancelled, &b.task.cancelled, a, b));
                vec![
                    column("todo", "To Do", todo, Some(Drop::Status(' '))),
                    column("doing", "In Progress", doing, Some(Drop::Status('/'))),
                    column("done", "Done", done, Some(Drop::Status('x'))),
                    column("cancelled", "Cancelled", cancelled, Some(Drop::Status('-'))),
                ]
            }
            Grouping::Day => {
                let (tomorrow, week) = (today.adding_days(1), today.adding_days(7));
                let mut columns: BTreeMap<&str, Vec<VaultTask>> = BTreeMap::new();
                for task in tasks.iter().filter(|t| t.task.is_open()) {
                    let key = match task.task.day().and_then(Day::parse) {
                        Some(day) if day < today => "overdue",
                        Some(day) if day == today => "today",
                        Some(day) if day == tomorrow => "tomorrow",
                        Some(day) if day <= week => "week",
                        Some(_) => "later",
                        None => "none",
                    };
                    columns.entry(key).or_default().push(task.clone());
                }
                [
                    ("overdue", "Overdue", None),
                    ("today", "Today", Some(Drop::Due(Some(today.to_string())))),
                    ("tomorrow", "Tomorrow", Some(Drop::Due(Some(tomorrow.to_string())))),
                    ("week", "Next 7 Days", Some(Drop::Due(Some(week.to_string())))),
                    ("later", "Later", None),
                    ("none", "No Date", Some(Drop::Due(None))),
                ]
                .into_iter()
                .map(|(id, title, drop)| {
                    let mut tasks = columns.remove(id).unwrap_or_default();
                    tasks.sort_by(open);
                    column(id, title, tasks, drop)
                })
                .collect()
            }
            Grouping::File => {
                let mut by_file: BTreeMap<&str, Vec<VaultTask>> = BTreeMap::new();
                for task in tasks.iter().filter(|t| t.task.is_open()) {
                    by_file.entry(&task.path).or_default().push(task.clone());
                }
                let mut paths: Vec<&str> = by_file.keys().copied().collect();
                paths.sort_by(|a, b| standard_compare(&Vault::title(a), &Vault::title(b)));
                paths
                    .into_iter()
                    .map(|path| {
                        let mut tasks = by_file.remove(path).unwrap_or_default();
                        tasks.sort_by(open);
                        column(path, &Vault::title(path), tasks, None)
                    })
                    .collect()
            }
        }
    }

    /// The lines a task line becomes when it is dropped on [drop]; `None` when
    /// it is not a task, or nothing changes.
    pub fn moved(raw: &str, drop: &Drop, today: Day) -> Option<Vec<String>> {
        let mut fields = TaskLine::fields(raw)?;
        match drop {
            Drop::Status(status) => {
                let status = *status;
                let was_closed = TaskLine::is_closed(fields.status);
                if status == fields.status || (status == 'x' && fields.status == 'X') {
                    return None;
                }
                // Done from open: the tick a click makes, recurrence and all.
                if status == 'x' && !was_closed {
                    return TaskLine::toggled(raw, &today.to_string());
                }
                fields.status = status;
                fields.done = (status == 'x').then(|| today.to_string());
                fields.cancelled = (status == '-').then(|| today.to_string());
            }
            Drop::Due(day) => {
                if fields.due == *day {
                    return None;
                }
                fields.due = day.clone();
            }
        }
        TaskLine::edited(raw, &fields).map(|line| vec![line])
    }
}

/// The order of open tasks: by day, the ones with none last, then by
/// priority, then where they are.
fn open(a: &VaultTask, b: &VaultTask) -> Ordering {
    let (da, db) = (a.task.day().unwrap_or("9999"), b.task.day().unwrap_or("9999"));
    da.cmp(db)
        .then_with(|| rank(a.task.priority.as_deref()).cmp(&rank(b.task.priority.as_deref())))
        .then_with(|| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)))
}

/// The last one closed first.
fn closed(a: &Option<String>, b: &Option<String>, x: &VaultTask, y: &VaultTask) -> Ordering {
    let (a, b) = (a.as_deref().unwrap_or(""), b.as_deref().unwrap_or(""));
    b.cmp(a).then_with(|| (&x.path, x.task.line).cmp(&(&y.path, y.task.line)))
}

fn rank(priority: Option<&str>) -> u8 {
    match priority {
        Some("🔺") => 0,
        Some("⏫") => 1,
        Some("🔼") => 2,
        Some("🔽") => 4,
        Some("⏬") => 5,
        _ => 3,
    }
}

impl VaultEdits {
    /// The task on [line] of [text] dropped on a board column. [task] is
    /// what the card showed, whose status and words the line must still hold.
    pub fn move_task(text: &str, line: usize, task: &ParsedTask, drop: &Drop, today: Day) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let raw = lines.get(line).ok_or(Failure::TaskMoved)?;
        let crlf = raw.ends_with('\r');
        let clean = raw.strip_suffix('\r').unwrap_or(raw).to_string();
        let now = TaskLine::parse(&clean, line, "").ok_or(Failure::TaskMoved)?;
        if now.status != task.status || now.text != task.text {
            return Err(Failure::TaskMoved);
        }
        let Some(replacement) = TaskBoard::moved(&clean, drop, today) else { return Ok(text.to_string()) };
        lines.splice(line..=line, replacement.into_iter().map(|l| if crlf { l + "\r" } else { l }));
        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> Day {
        Day::new(2026, 10, 25).unwrap()
    }

    fn task_at(raw: &str, path: &str, line: usize) -> VaultTask {
        VaultTask { path: path.into(), task: TaskLine::parse(raw, line, "Kit").unwrap() }
    }

    fn task(raw: &str) -> VaultTask {
        task_at(raw, "Tasks/Work.md", 0)
    }

    fn texts(column: &Column) -> Vec<&str> {
        column.tasks.iter().map(|t| t.task.text.as_str()).collect()
    }

    #[test]
    fn status_columns_keep_recently_closed_only() {
        let tasks = [
            task_at("- [ ] Write 📅 2026-10-27", "Tasks/Work.md", 0),
            task_at("- [ ] Urgent ⏫ 📅 2026-10-27", "Tasks/Work.md", 1),
            task_at("- [/] Build", "Tasks/Work.md", 2),
            task_at("- [x] Shipped ✅ 2026-10-20", "Tasks/Work.md", 3),
            task_at("- [x] Long ago ✅ 2026-01-01", "Tasks/Work.md", 4),
            task_at("- [-] Dropped ❌ 2026-10-24", "Tasks/Work.md", 5),
        ];
        let columns = TaskBoard::columns(&tasks, Grouping::Status, today(), Some(14));
        assert_eq!(columns.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["todo", "doing", "done", "cancelled"]);
        // The same day: the higher priority first.
        assert_eq!(texts(&columns[0]), ["Urgent", "Write"]);
        assert_eq!(texts(&columns[1]), ["Build"]);
        assert_eq!(texts(&columns[2]), ["Shipped"]);
        assert_eq!(texts(&columns[3]), ["Dropped"]);
        assert_eq!(TaskBoard::columns(&tasks, Grouping::Status, today(), None)[2].tasks.len(), 2);
    }

    #[test]
    fn day_columns() {
        let tasks = [
            task("- [ ] Late 📅 2026-10-20"),
            task("- [ ] Now ⏳ 2026-10-25"),
            task("- [ ] Next 📅 2026-10-26"),
            task("- [ ] Soon 📅 2026-11-01"),
            task("- [ ] Far 📅 2026-12-01"),
            task("- [ ] Whenever"),
            task("- [x] Done 📅 2026-10-25"),
        ];
        let columns = TaskBoard::columns(&tasks, Grouping::Day, today(), Some(14));
        let all: Vec<Vec<&str>> = columns.iter().map(texts).collect();
        assert_eq!(all, [vec!["Late"], vec!["Now"], vec!["Next"], vec!["Soon"], vec!["Far"], vec!["Whenever"]]);
        assert!(columns[0].drop.is_none() && columns[4].drop.is_none());
        assert_eq!(columns[1].drop, Some(Drop::Due(Some("2026-10-25".into()))));
        assert_eq!(columns[5].drop, Some(Drop::Due(None)));
    }

    #[test]
    fn file_columns() {
        let tasks =
            [task_at("- [ ] B", "Tasks/Zeta.md", 0), task_at("- [ ] A", "Tasks/Alpha.md", 0), task_at("- [x] C", "Tasks/Beta.md", 0)];
        let titles: Vec<String> = TaskBoard::columns(&tasks, Grouping::File, today(), Some(14)).into_iter().map(|c| c.title).collect();
        assert_eq!(titles, ["Alpha", "Zeta"]);
    }

    #[test]
    fn a_drop_writes_the_line() {
        let date = today().to_string();
        let moved = |raw: &str, drop: Drop| TaskBoard::moved(raw, &drop, today());
        let one = |s: String| Some(vec![s]);
        assert_eq!(moved("- [ ] Write 📅 2026-10-27", Drop::Status('/')), one("- [/] Write 📅 2026-10-27".into()));
        assert_eq!(moved("- [/] Write", Drop::Status('x')), one(format!("- [x] Write ✅ {date}")));
        assert_eq!(moved("- [ ] Write", Drop::Status('-')), one(format!("- [-] Write ❌ {date}")));
        assert_eq!(moved("- [x] Write ✅ 2026-10-20", Drop::Status(' ')), one("- [ ] Write".into()));
        assert_eq!(moved("- [x] Write ✅ 2026-10-20", Drop::Status('/')), one("- [/] Write".into()));
        assert_eq!(moved("- [ ] Write", Drop::Status(' ')), None);
        assert_eq!(moved("- [ ] Write 📅 2026-10-27", Drop::Due(Some("2026-10-25".into()))), one("- [ ] Write 📅 2026-10-25".into()));
        assert_eq!(moved("- [ ] Write 📅 2026-10-27", Drop::Due(None)), one("- [ ] Write".into()));
        // A recurring task is ticked as a click ticks it: the next one comes above.
        assert_eq!(
            moved("- [ ] Water 🔁 every week 📅 2026-10-25", Drop::Status('x')),
            Some(vec!["- [ ] Water 🔁 every week 📅 2026-11-01".to_string(), format!("- [x] Water 🔁 every week 📅 2026-10-25 ✅ {date}")])
        );
    }

    #[test]
    fn a_drop_refuses_a_line_that_changed() {
        let text = "# Kit\n- [ ] Write\n- [ ] Read\n";
        let shown = task("- [ ] Write").task;
        assert_eq!(VaultEdits::move_task(text, 1, &shown, &Drop::Status('/'), today()).unwrap(), "# Kit\n- [/] Write\n- [ ] Read\n");
        assert_eq!(VaultEdits::move_task(text, 2, &shown, &Drop::Status('/'), today()), Err(Failure::TaskMoved));
    }
}
