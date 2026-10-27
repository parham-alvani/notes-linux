//! Today, as on the Mac: the date, a line of chips that sum the day up, the
//! meetings from the phone's week note in `Calendar/` with the free time
//! between them, and cards of the open tasks that are overdue, due today, in
//! progress and due in the next seven days. A task's box ticks it; its text
//! opens its note at its line; a right click edits it.

use std::rc::Rc;

use chrono::{DateTime, Local, Utc};

use adw::prelude::*;

use daftar_core::text::direction;
use daftar_core::{CalendarWeek, Day, Meeting, Vault, VaultTask};

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
type Edit = dyn Fn(&str, usize);

pub struct Today {
    pub widget: gtk::ScrolledWindow,
    content: gtk::Box,
    open: Box<Open>,
    toggle: Box<Toggle>,
    edit: Box<Edit>,
}

impl Today {
    /// [open] opens a note at a line; [toggle] ticks the task on one; [edit]
    /// opens the Edit Task dialog on one.
    pub fn new(
        open: impl Fn(&str, usize) + 'static,
        toggle: impl Fn(&str, usize, char) + 'static,
        edit: impl Fn(&str, usize) + 'static,
    ) -> Rc<Today> {
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
        Rc::new(Today { widget, content, open: Box::new(open), toggle: Box::new(toggle), edit: Box::new(edit) })
    }

    /// Draws the page again from the index; [scratchpad] is the folder of
    /// the day's scratchpad notes, for the review.
    pub fn show(self: &Rc<Self>, store: &VaultStore, scratchpad: &str) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let today = Day::today();
        let groups = TaskGroups::new(store.listed_tasks(), today, 7);
        let days = store.calendar_days(today, 2);
        let day = days.iter().find(|d| d.day == today);
        let tomorrow = days.iter().find(|d| d.day == today.adding_days(1));

        let head = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let date = chrono::Local::now().format("%A %-d %B").to_string();
        head.append(&gtk::Label::builder().label(&date).xalign(0.0).css_classes(["daftar-date"]).build());
        if let Some(persian) = crate::settings::persian_date(today) {
            head.append(&gtk::Label::builder().label(&persian).xalign(0.0).css_classes(["title-3", "daftar-muted"]).build());
        }
        let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).column_spacing(8).build();
        let chip = |text: String, icon: &str, tint: &str| {
            let row = gtk::Box::builder().spacing(6).css_classes(["daftar-chip", tint]).build();
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&gtk::Label::new(Some(&text)));
            chips.insert(&row, -1);
        };
        let meetings = day.map_or(0, |d| d.meetings.len());
        chip(format!("{meetings} {}", if meetings == 1 { "meeting" } else { "meetings" }), "x-office-calendar-symbolic", "t-blue");
        if let Some(free) = day.and_then(|d| d.free.as_deref()) {
            chip(format!("free {free}"), "alarm-symbolic", "t-green");
        }
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

        let meetings_card = card("Meetings", "x-office-calendar-symbolic", "t-blue", None);
        let now = Utc::now();
        match day {
            Some(day) if day.meetings.is_empty() => meetings_card.append(&muted("No meetings today.")),
            Some(day) => {
                for meeting in &day.meetings {
                    meetings_card.append(&meeting_row(meeting, now));
                }
            }
            None => meetings_card
                .append(&muted(&format!("There is no week note for today in {}/. Daftar on the phone writes it.", CalendarWeek::FOLDER))),
        }
        if let Some(first) = tomorrow.and_then(|d| d.meetings.iter().find(|m| !m.all_day()))
            && let Some(start) = first.start
        {
            meetings_card.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            meetings_card.append(&muted(&format!("Tomorrow starts at {}: {}", clock(start), first.title)));
        }
        self.content.append(&meetings_card);

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
            card.append(&muted("No open task is due this week."));
            self.content.append(&card);
        }
        self.content.append(&self.review(store, today, scratchpad));
    }

    /// The day's review, under the tasks: today's scratchpad, and the notes
    /// changed on disk (a pull, another app) since they were read.
    fn review(self: &Rc<Self>, store: &VaultStore, today: Day, scratchpad: &str) -> gtk::Box {
        let review = card("Review", "view-list-symbolic", "t-yellow", None);
        let folder = scratchpad.trim().trim_matches('/');
        let path = if folder.is_empty() { format!("{today}.md") } else { format!("{folder}/{today}.md") };
        let first_line = store.index().files.contains(&path).then(|| {
            std::fs::read_to_string(store.root.join(&path))
                .ok()
                .and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty() && *line != "---").map(String::from))
                .unwrap_or_else(|| "Empty so far".into())
        });
        let pad = gtk::Box::builder().spacing(8).build();
        pad.append(&gtk::Image::from_icon_name("document-edit-symbolic"));
        let words = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        words.append(&gtk::Label::builder().label("Today's scratchpad").xalign(0.0).css_classes(["heading"]).build());
        words.append(&muted(first_line.as_deref().unwrap_or("Not started: Ctrl+Alt+S makes it")));
        pad.append(&words);
        review.append(&gtk::Button::builder().child(&pad).css_classes(["flat"]).action_name("win.scratchpad").build());
        let updated = store.updated_since_read();
        if updated.is_empty() {
            review.append(&muted("No note changed since you read it."));
        } else {
            let head = gtk::Box::builder().spacing(8).build();
            head.append(
                &gtk::Label::builder()
                    .label(format!("Changed since you read them · {}", updated.len()))
                    .xalign(0.0)
                    .hexpand(true)
                    .css_classes(["heading"])
                    .build(),
            );
            head.append(
                &gtk::Button::builder()
                    .label("Mark All as Read")
                    .css_classes(["flat", "caption"])
                    .action_name("win.mark-read")
                    .action_target(&"".to_variant())
                    .build(),
            );
            review.append(&head);
            for (path, _) in updated.iter().take(8) {
                let row = gtk::Box::builder().spacing(8).build();
                row.append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
                row.append(&gtk::Label::builder().label(Vault::title(path)).xalign(0.0).hexpand(true).build());
                row.append(
                    &gtk::Label::builder()
                        .label(daftar_core::text::slugs::normalize(path.rsplit_once('/').map_or("", |(f, _)| f)))
                        .css_classes(["daftar-muted", "caption"])
                        .build(),
                );
                let button = gtk::Button::builder().child(&row).css_classes(["flat"]).build();
                let (this, path) = (self.clone(), path.clone());
                button.connect_clicked(move |_| (this.open)(&path, 0));
                review.append(&button);
            }
        }
        review
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
        // A Persian task's note and dates stand under it, on the right.
        let meta =
            gtk::Box::builder().spacing(6).css_classes(["caption"]).halign(if rtl { gtk::Align::End } else { gtk::Align::Start }).build();
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
        // The Tasks plugin's Postpone, one click away, as on the Mac.
        if daftar_core::TaskLine::can_postpone(t) {
            let amounts = gtk::gio::Menu::new();
            for amount in daftar_core::Postpone::ALL {
                let item = gtk::gio::MenuItem::new(Some(&format!("Postpone by {}", amount.title())), None);
                let target = format!("{}\t{}\t{}", task.path, t.line, amount.name());
                item.set_action_and_target_value(Some("win.postpone"), Some(&target.to_variant()));
                amounts.append_item(&item);
            }
            row.append(
                &gtk::MenuButton::builder()
                    .icon_name("alarm-symbolic")
                    .menu_model(&amounts)
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "circular"])
                    .tooltip_text("Postpone")
                    .build(),
            );
        }
        let menu = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        {
            let (this, path, line) = (self.clone(), task.path.clone(), t.line);
            menu.connect_pressed(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                (this.edit)(&path, line);
            });
        }
        row.add_controller(menu);
        // From the keyboard, on a focused task: x ticks it, p postpones it a day.
        let keys = gtk::EventControllerKey::new();
        {
            let (this, path, line, status, postponable) =
                (self.clone(), task.path.clone(), t.line, t.status, daftar_core::TaskLine::can_postpone(t));
            keys.connect_key_pressed(move |controller, key, _, state| {
                if !state.is_empty() && state != gtk::gdk::ModifierType::SHIFT_MASK {
                    return gtk::glib::Propagation::Proceed;
                }
                match key {
                    gtk::gdk::Key::x => (this.toggle)(&path, line, status),
                    gtk::gdk::Key::p if postponable => {
                        let target = format!("{path}\t{line}\t{}", daftar_core::Postpone::Day.name());
                        if let Some(widget) = controller.widget() {
                            let _ = widget.activate_action("win.postpone", Some(&target.to_variant()));
                        }
                    }
                    _ => return gtk::glib::Propagation::Proceed,
                }
                gtk::glib::Propagation::Stop
            });
        }
        row.add_controller(keys);
        row.set_tooltip_text(Some("x ticks it, p postpones it a day"));
        row
    }
}

fn muted(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["daftar-muted"]).build()
}

/// `09:30` on this computer's clock.
fn clock(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

/// Where a meeting is from [now]: over, going on, or still to come.
#[derive(Debug, PartialEq, Eq)]
pub enum When {
    Past,
    Now,
    Next,
}

pub fn when(meeting: &Meeting, now: DateTime<Utc>) -> When {
    match (meeting.start, meeting.end) {
        (_, Some(end)) if end < now => When::Past,
        (Some(start), _) if start <= now => When::Now,
        _ => When::Next,
    }
}

/// `09:30–10:00`, or `All day`.
pub fn time_range(meeting: &Meeting) -> String {
    match (meeting.start, meeting.end) {
        (Some(start), Some(end)) => format!("{}–{}", clock(start), clock(end)),
        _ => "All day".into(),
    }
}

/// A meeting: a bar in its state, the time, the title with `now` or `maybe`,
/// an overlap warning, and Join while it is not over.
fn meeting_row(meeting: &Meeting, now: DateTime<Utc>) -> gtk::Box {
    let state = when(meeting, now);
    let tint = match state {
        When::Now => "t-accent",
        When::Past => "t-muted",
        When::Next => "t-blue",
    };
    let row = gtk::Box::builder().spacing(10).css_classes(["daftar-meeting", tint]).build();
    if state == When::Past {
        row.add_css_class("is-past");
    }
    row.append(&gtk::Box::builder().width_request(4).css_classes(["meeting-bar"]).build());
    row.append(
        &gtk::Label::builder()
            .label(time_range(meeting))
            .width_chars(13)
            .xalign(0.0)
            .valign(gtk::Align::Start)
            .css_classes(["tint", "numeric"])
            .build(),
    );
    let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    let title = gtk::Box::builder().spacing(6).build();
    let name = gtk::Label::builder().label(&meeting.title).xalign(0.0).wrap(true).build();
    if meeting.busy_only || state == When::Past {
        name.add_css_class("daftar-muted");
    }
    if state == When::Now {
        name.add_css_class("heading");
    }
    title.append(&name);
    if state == When::Now {
        title.append(&gtk::Label::builder().label("now").css_classes(["daftar-badge"]).build());
    }
    if meeting.tentative {
        title.append(&gtk::Label::builder().label("maybe").css_classes(["daftar-day", "t-purple"]).build());
    }
    lines.append(&title);
    if let Some(overlaps) = &meeting.overlaps {
        lines.append(
            &gtk::Label::builder().label(format!("⚠ Overlaps {overlaps}")).xalign(0.0).css_classes(["caption", "meeting-warning"]).build(),
        );
    }
    row.append(&lines);
    if let Some(join) = meeting.join.clone().filter(|url| joinable(url))
        && state != When::Past
    {
        let button = gtk::Button::builder()
            .label("Join")
            .valign(gtk::Align::Center)
            .css_classes(["pill", if state == When::Now { "daftar-join-now" } else { "daftar-join" }])
            .tooltip_text(&join)
            .build();
        button.connect_clicked(move |button| {
            let window = button.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(&join).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
        });
        row.append(&button);
    }
    row
}

/// A join link the app opens: web links only, as every link that leaves it.
fn joinable(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("").to_lowercase();
    scheme == "https" || scheme == "http"
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
pub fn priority(emoji: &str) -> Option<(&'static str, &'static str)> {
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
    fn a_meeting_is_past_now_or_next() {
        use chrono::TimeZone;
        let at = |h: u32| Utc.with_ymd_and_hms(2026, 10, 26, h, 0, 0).unwrap();
        let meeting = Meeting {
            day: Day::parse("2026-10-26").unwrap(),
            start: Some(at(10)),
            end: Some(at(11)),
            title: "Planning".into(),
            calendars: vec![],
            join: Some("https://meet.example.com/a".into()),
            tentative: false,
            busy_only: false,
            overlaps: None,
        };
        assert_eq!(when(&meeting, at(9)), When::Next);
        assert_eq!(when(&meeting, at(10)), When::Now);
        assert_eq!(when(&meeting, at(12)), When::Past);
        let all_day = Meeting { start: None, end: None, ..meeting };
        assert_eq!(when(&all_day, at(12)), When::Next);
        assert_eq!(time_range(&all_day), "All day");
        assert!(joinable("https://meet.example.com/a") && !joinable("file:///etc/passwd") && !joinable("zoommtg://x"));
    }

    #[test]
    fn links_read_as_their_labels() {
        assert_eq!(plain("Call [[Sam Lee|Sam]] about [[Plan]]"), "Call Sam about Plan");
        assert_eq!(plain("no [[end"), "no [[end");
    }
}
