//! Today, as on the Mac: the date, a line of chips that sum the day up, and
//! cards of the open tasks that are overdue, due today, in progress and due
//! in the next seven days. A task's box ticks it; its text opens its note at
//! its line.

use std::rc::Rc;

use adw::prelude::*;

use daftar_core::text::direction;
use daftar_core::{Day, Vault, VaultTask};

use crate::store::VaultStore;

/// Open tasks by the day they belong to: due, else scheduled.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TaskGroups {
    pub overdue: Vec<VaultTask>,
    pub today: Vec<VaultTask>,
    pub upcoming: Vec<VaultTask>,
    pub in_progress: Vec<VaultTask>,
}

impl TaskGroups {
    pub fn new(tasks: Vec<VaultTask>, today: Day, days: i64) -> TaskGroups {
        let mut groups = TaskGroups::default();
        let horizon = today.adding_days(days);
        for task in tasks.into_iter().filter(|t| t.task.is_open()) {
            let Some(day) = task.task.day().and_then(Day::parse) else {
                if task.task.status == '/' {
                    groups.in_progress.push(task);
                }
                continue;
            };
            if day < today {
                groups.overdue.push(task);
            } else if day == today {
                groups.today.push(task);
            } else if day <= horizon {
                groups.upcoming.push(task);
            }
        }
        for list in [&mut groups.overdue, &mut groups.today, &mut groups.upcoming, &mut groups.in_progress] {
            list.sort_by(|a, b| {
                (a.task.day().unwrap_or(""), &a.path, a.task.line).cmp(&(b.task.day().unwrap_or(""), &b.path, b.task.line))
            });
        }
        groups
    }
}

type Open = dyn Fn(&str, usize);
type Toggle = dyn Fn(&str, usize, char);

pub struct Today {
    pub widget: gtk::ScrolledWindow,
    content: gtk::Box,
    open: Box<Open>,
    toggle: Box<Toggle>,
}

impl Today {
    /// [open] opens a note at a line; [toggle] ticks the task on one.
    pub fn new(open: impl Fn(&str, usize) + 'static, toggle: impl Fn(&str, usize, char) + 'static) -> Rc<Today> {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(32)
            .margin_bottom(32)
            .margin_start(24)
            .margin_end(24)
            .build();
        let clamp = adw::Clamp::builder().maximum_size(760).child(&content).build();
        let widget =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).css_classes(["daftar-page"]).build();
        Rc::new(Today { widget, content, open: Box::new(open), toggle: Box::new(toggle) })
    }

    /// Draws the page again from the index.
    pub fn show(self: &Rc<Self>, store: &VaultStore) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let today = Day::today();
        let groups = TaskGroups::new(store.index().tasks(), today, 7);

        let head = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let date = chrono::Local::now().format("%A %-d %B").to_string();
        head.append(&gtk::Label::builder().label(&date).xalign(0.0).css_classes(["daftar-date"]).build());
        let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).column_spacing(8).build();
        let chip = |text: String, icon: &str, tint: &str| {
            let row = gtk::Box::builder().spacing(6).css_classes(["daftar-chip", tint]).build();
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&gtk::Label::new(Some(&text)));
            chips.insert(&row, -1);
        };
        chip(format!("{} due", groups.today.len()), "weather-clear-symbolic", "t-accent");
        if !groups.overdue.is_empty() {
            chip(format!("{} overdue", groups.overdue.len()), "dialog-warning-symbolic", "t-red");
        }
        if !groups.in_progress.is_empty() {
            chip(format!("{} in progress", groups.in_progress.len()), "content-loading-symbolic", "t-purple");
        }
        if !groups.upcoming.is_empty() {
            chip(format!("{} this week", groups.upcoming.len()), "x-office-calendar-symbolic", "t-aqua");
        }
        head.append(&chips);
        self.content.append(&head);

        let cards = [
            ("Overdue", "dialog-warning-symbolic", &groups.overdue, "t-red"),
            ("Due today", "weather-clear-symbolic", &groups.today, "t-accent"),
            ("In progress", "content-loading-symbolic", &groups.in_progress, "t-purple"),
            ("Next 7 days", "x-office-calendar-symbolic", &groups.upcoming, "t-aqua"),
        ];
        let mut any = false;
        for (title, icon, tasks, tint) in cards {
            if tasks.is_empty() {
                continue;
            }
            any = true;
            let card = card(title, icon, tint, Some(tasks.len()));
            for task in tasks {
                card.append(&self.task_row(task, tint));
            }
            self.content.append(&card);
        }
        if !any {
            let card = card("Nothing due", "emblem-ok-symbolic", "t-green", None);
            card.append(&gtk::Label::builder().label("No open task is due this week.").xalign(0.0).css_classes(["daftar-muted"]).build());
            self.content.append(&card);
        }
    }

    fn task_row(self: &Rc<Self>, task: &VaultTask, tint: &str) -> gtk::Box {
        let t = &task.task;
        let row = gtk::Box::builder().spacing(8).css_classes(["daftar-task", tint]).build();
        let check = gtk::CheckButton::builder().valign(gtk::Align::Start).tooltip_text("Complete").build();
        // The phone's squares: open, and in progress half filled.
        check.set_inconsistent(t.status == '/');
        {
            let (this, path, line, status) = (self.clone(), task.path.clone(), t.line, t.status);
            check.connect_toggled(move |_| (this.toggle)(&path, line, status));
        }
        row.append(&check);

        let text = plain(&t.text);
        let rtl = direction::starts_rtl(&text);
        let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        lines.append(
            &gtk::Label::builder()
                .label(&text)
                .wrap(true)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .xalign(if rtl { 1.0 } else { 0.0 })
                .build(),
        );
        let meta = gtk::Box::builder().spacing(6).css_classes(["caption"]).build();
        meta.append(&gtk::Label::builder().label(Vault::title(&task.path)).css_classes(["daftar-note"]).build());
        if !t.section.is_empty() {
            meta.append(
                &gtk::Label::builder()
                    .label(format!("› {}", t.section))
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["daftar-muted"])
                    .build(),
            );
        }
        if let Some(day) = t.day() {
            let mark = if t.due.is_some() { "📅" } else { "⏳" };
            meta.append(&gtk::Label::builder().label(format!("{mark} {day}")).css_classes(["daftar-day", tint]).build());
        }
        if let Some((mark, tint)) = t.priority.as_deref().and_then(priority) {
            meta.append(&gtk::Label::builder().label(mark).css_classes([tint, "tint"]).build());
        }
        lines.append(&meta);
        let button = gtk::Button::builder().child(&lines).css_classes(["flat"]).hexpand(true).build();
        {
            let (this, path, line) = (self.clone(), task.path.clone(), t.line);
            button.connect_clicked(move |_| (this.open)(&path, line));
        }
        row.append(&button);
        row
    }
}

/// A rounded group in its own color: a tinted icon, the title and a count.
fn card(title: &str, icon: &str, tint: &str, count: Option<usize>) -> gtk::Box {
    let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).css_classes(["daftar-card", tint]).build();
    let head = gtk::Box::builder().spacing(10).build();
    head.append(&gtk::Image::builder().icon_name(icon).css_classes(["card-icon"]).build());
    head.append(&gtk::Label::builder().label(title).css_classes(["card-title", "heading"]).build());
    if let Some(count) = count {
        head.append(&gtk::Label::builder().label(count.to_string()).css_classes(["daftar-count", tint, "numeric"]).build());
    }
    card.append(&head);
    card
}

/// The Tasks plugin's priority emoji as a mark and its color.
fn priority(emoji: &str) -> Option<(&'static str, &'static str)> {
    Some(match emoji {
        "🔺" | "⏫" => ("⇈", "t-red"),
        "🔼" => ("↑", "t-accent"),
        "🔽" => ("↓", "t-blue"),
        "⏬" => ("⇊", "t-blue"),
        _ => return None,
    })
}

/// `[[Target|label]]` as its label, `[[Target]]` as its name, for one line.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let Some(end) = rest[start + 2..].find("]]") else { break };
        out.push_str(&rest[..start]);
        let inner = &rest[start + 2..start + 2 + end];
        out.push_str(inner.split_once('|').map_or(inner, |(_, label)| label));
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::TaskLine;

    fn task(path: &str, raw: &str, line: usize) -> VaultTask {
        VaultTask { path: path.into(), task: TaskLine::parse(raw, line, "").unwrap() }
    }

    #[test]
    fn tasks_by_the_day_they_belong_to() {
        let today = Day::parse("2026-10-21").unwrap();
        let tasks = vec![
            task("A.md", "- [ ] Late 📅 2026-10-20", 0),
            task("A.md", "- [ ] Now ⏳ 2026-10-21", 1),
            task("B.md", "- [ ] Soon 📅 2026-10-25", 0),
            task("B.md", "- [ ] Far 📅 2026-11-30", 1),
            task("B.md", "- [/] Doing", 2),
            task("B.md", "- [x] Done 📅 2026-10-21", 3),
            task("B.md", "- [ ] Someday", 4),
        ];
        let groups = TaskGroups::new(tasks, today, 7);
        let texts = |list: &[VaultTask]| list.iter().map(|t| t.task.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&groups.overdue), ["Late"]);
        assert_eq!(texts(&groups.today), ["Now"]);
        assert_eq!(texts(&groups.upcoming), ["Soon"]);
        assert_eq!(texts(&groups.in_progress), ["Doing"]);
    }

    #[test]
    fn links_read_as_their_labels() {
        assert_eq!(plain("Call [[Sam Lee|Sam]] about [[Plan]]"), "Call Sam about Plan");
        assert_eq!(plain("no [[end"), "no [[end");
    }
}
