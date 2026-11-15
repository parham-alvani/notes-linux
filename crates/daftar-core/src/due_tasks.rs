//! The open tasks a day asks for: the overdue, the day's own, the ones in
//! progress and the ones scheduled on a day that passed. The MCP's
//! `vault_brief` and `daftar-cli agenda` list them. No Swift twin yet.

use crate::day::Day;
use crate::task_line::TaskLine;
use crate::vault::VaultTask;

/// The days after today that `ahead` counts.
pub const AHEAD_DAYS: i64 = 7;

/// The open tasks of a day, each list in the order it is read in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DueTasks {
    /// 📅 before the day, the earliest first.
    pub overdue: Vec<VaultTask>,
    /// 📅 on the day, or ⏳ on the day and no 📅.
    pub today: Vec<VaultTask>,
    /// The status `/`.
    pub in_progress: Vec<VaultTask>,
    /// ⏳ before the day and no 📅, the oldest first.
    pub slipped: Vec<VaultTask>,
    /// How many fall (📅, else ⏳) in the `AHEAD_DAYS` after the day.
    pub ahead: usize,
}

impl DueTasks {
    /// The open ones of [tasks] (done and cancelled left out, as the
    /// vault's statuses type them) for [today].
    pub fn new(tasks: &[VaultTask], today: Day) -> DueTasks {
        let open: Vec<&VaultTask> = tasks.iter().filter(|t| !TaskLine::is_closed(t.task.status)).collect();
        let day = today.to_string();
        let day = day.as_str();
        let week = today.adding_days(AHEAD_DAYS).to_string();
        let pick =
            |keep: &dyn Fn(&VaultTask) -> bool| -> Vec<VaultTask> { open.iter().filter(|t| keep(t)).map(|t| (*t).clone()).collect() };
        let mut overdue = pick(&|t| t.task.due.as_deref().is_some_and(|d| d < day));
        overdue.sort_by(|a, b| a.task.day().cmp(&b.task.day()));
        let today = pick(&|t| t.task.due.as_deref() == Some(day) || (t.task.due.is_none() && t.task.scheduled.as_deref() == Some(day)));
        let mut slipped = pick(&|t| t.task.due.is_none() && t.task.scheduled.as_deref().is_some_and(|d| d < day));
        slipped.sort_by(|a, b| a.task.scheduled.cmp(&b.task.scheduled));
        let in_progress = pick(&|t| t.task.status == '/');
        let ahead = open.iter().filter(|t| t.task.day().is_some_and(|d| d > day && d <= week.as_str())).count();
        DueTasks { overdue, today, in_progress, slipped, ahead }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks(lines: &[&str]) -> Vec<VaultTask> {
        lines
            .iter()
            .enumerate()
            .map(|(line, text)| VaultTask { path: "Tasks.md".into(), task: TaskLine::parse(text, line, "").unwrap() })
            .collect()
    }

    fn texts(list: &[VaultTask]) -> Vec<&str> {
        list.iter().map(|t| t.task.text.as_str()).collect()
    }

    #[test]
    fn a_days_tasks_are_sorted_into_what_it_asks_for() {
        let tasks = tasks(&[
            "- [ ] Late 📅 2026-11-10",
            "- [ ] Later 📅 2026-11-01",
            "- [ ] Today 📅 2026-11-15",
            "- [ ] Planned today ⏳ 2026-11-15",
            "- [ ] Planned, due later ⏳ 2026-11-15 📅 2026-11-20",
            "- [/] Going",
            "- [ ] Slipped ⏳ 2026-11-12",
            "- [x] Done late 📅 2026-11-01",
            "- [-] Dropped today 📅 2026-11-15",
            "- [ ] Next week ⏳ 2026-11-22",
            "- [ ] Too far 📅 2026-11-23",
        ]);
        let due = DueTasks::new(&tasks, Day::parse("2026-11-15").unwrap());
        assert_eq!(texts(&due.overdue), ["Later", "Late"]);
        assert_eq!(texts(&due.today), ["Today", "Planned today"]);
        assert_eq!(texts(&due.in_progress), ["Going"]);
        assert_eq!(texts(&due.slipped), ["Slipped"]);
        // Due on the 20th and scheduled for the 22nd; the 23rd is past the week.
        assert_eq!(due.ahead, 2);
    }
}
