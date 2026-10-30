//! The Tasks plugin's "edit task", as a dialog: the text, the status, the
//! reason, the priority, the dates and the repeat rule, each in its color. The line is
//! read from the file when the dialog opens and again before it is written,
//! and the save is refused when it changed meanwhile. The Mac's
//! `TaskEditSheet`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::{BoardDrop, Day, Postpone, Recurrence, TaskFields, TaskLine, TaskStatus, TaskStatuses, TaskType, Vault, VaultTask};

use crate::store::VaultStore;

/// A status's color, by its type.
pub fn status_tint(kind: TaskType) -> &'static str {
    match kind {
        TaskType::Todo => "t-accent",
        TaskType::InProgress => "t-purple",
        TaskType::Done => "t-green",
        TaskType::Cancelled => "t-red",
        TaskType::NonTask => "t-muted",
    }
}

/// The name of a status that is not one of the four every vault has, as
/// `In Review`, in its color; `None` for those four.
pub fn status_badge(symbol: char) -> Option<gtk::Label> {
    let status = TaskStatuses::current().status(symbol);
    (!status.is_default()).then(|| gtk::Label::builder().label(&status.name).css_classes(["daftar-day", status_tint(status.kind)]).build())
}

/// The reason a task is in its status, muted, after a speech balloon.
pub fn reason_label(reason: &str, rtl: bool) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(5)
        .css_classes(["daftar-muted", "caption"])
        .halign(if rtl { gtk::Align::End } else { gtk::Align::Start })
        .build();
    row.append(&gtk::Label::new(Some("💬")));
    row.append(
        &gtk::Label::builder()
            .label(crate::today::plain(reason))
            .wrap(true)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .xalign(if rtl { 1.0 } else { 0.0 })
            .build(),
    );
    row.set_tooltip_text(Some(reason));
    row
}

/// Whether setting [to] on a task now [from] cancels it, which asks why first.
pub fn cancels(from: char, to: char) -> bool {
    let statuses = TaskStatuses::current();
    statuses.kind(to) == TaskType::Cancelled && statuses.kind(from) != TaskType::Cancelled
}

/// "Cancel Task": why, in one line that may be left empty. [done] gets the
/// reason when the task is cancelled; Escape or "Don't Cancel" leaves it as
/// it is.
pub fn ask_reason(parent: &impl IsA<gtk::Widget>, text: &str, current: Option<&str>, done: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::builder()
        .heading("Cancel Task")
        .body(format!("Why is “{}” cancelled? The reason is kept on the task, and it may be left empty.", crate::today::plain(text)))
        .close_response("back")
        .default_response("cancel")
        .build();
    dialog.add_responses(&[("back", "Don't Cancel"), ("cancel", "Cancel Task")]);
    dialog.set_response_appearance("cancel", adw::ResponseAppearance::Destructive);
    let entry = gtk::Entry::builder().placeholder_text("Reason").text(current.unwrap_or("")).activates_default(true).build();
    dialog.set_extra_child(Some(&entry));
    {
        let entry = entry.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "cancel" {
                done(entry.text().to_string());
            }
        });
    }
    dialog.present(Some(parent));
    // The dialog focuses its buttons as it opens: the reason is what is typed.
    gtk::glib::idle_add_local_once(move || {
        entry.grab_focus();
    });
}

const PRIORITIES: [(Option<&str>, &str, &str); 6] = [
    (None, "None", "t-muted"),
    (Some("⏬"), "Lowest", "t-blue"),
    (Some("🔽"), "Low", "t-blue"),
    (Some("🔼"), "Medium", "t-accent"),
    (Some("⏫"), "High", "t-red"),
    (Some("🔺"), "Highest", "t-red"),
];

const RULES: [&str; 6] = ["every day", "every week", "every 2 weeks", "every month", "every year", "every week when done"];

/// The task on [line] of the note at [path], as it is in the file now; an
/// error to show when the line is gone or no longer a task.
pub fn read(store: &VaultStore, path: &str, line: usize) -> Result<TaskFields, &'static str> {
    let text = store.text(path).ok_or("The note could not be read.")?;
    let raw = text.split('\n').nth(line).ok_or("That line is no longer in the note. It changed; open it again.")?;
    TaskLine::fields(raw.strip_suffix('\r').unwrap_or(raw)).ok_or("That line is no longer a task. The note changed; open it again.")
}

/// `today`, `in 3 days`, `2 days ago`, from today.
pub fn relative(day: Day, today: Day) -> String {
    match today.days_to(day) {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        n if n > 1 => format!("in {n} days"),
        n => format!("{} days ago", -n),
    }
}

/// Opens the dialog over [parent]; [saved] runs after a write went through.
pub fn present(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, path: &str, line: usize, saved: impl Fn() + 'static) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    let head = gtk::Box::builder().spacing(14).build();
    head.append(&gtk::Image::builder().icon_name("checkbox-checked-symbolic").pixel_size(22).css_classes(["daftar-board-icon"]).build());
    let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).valign(gtk::Align::Center).build();
    titles.append(&gtk::Label::builder().label("Edit Task").xalign(0.0).css_classes(["title-2"]).build());
    titles.append(
        &gtk::Label::builder().label(format!("{}, line {}", Vault::title(path), line + 1)).xalign(0.0).css_classes(["dim-label"]).build(),
    );
    head.append(&titles);
    content.append(&head);

    let dialog = adw::Dialog::builder().title("Edit Task").content_width(680).child(&content).build();
    let error = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["t-red", "tint"]).visible(false).build();
    let before = match read(&store, path, line) {
        Ok(fields) => fields,
        Err(message) => {
            error.set_label(message);
            error.set_visible(true);
            content.append(&error);
            let close = gtk::Button::builder().label("Close").halign(gtk::Align::End).build();
            let dialog_ = dialog.clone();
            close.connect_clicked(move |_| {
                dialog_.close();
            });
            content.append(&close);
            dialog.present(Some(parent));
            return;
        }
    };
    let fields = Rc::new(RefCell::new(before.clone()));
    let postponable = !TaskLine::is_closed(before.status) && (before.due.is_some() || before.scheduled.is_some() || before.start.is_some());
    let save = gtk::Button::builder().label("Save").css_classes(["suggested-action", "pill"]).build();
    let refresh = {
        let (fields, before, save) = (fields.clone(), before.clone(), save.clone());
        Rc::new(move || {
            let now = fields.borrow();
            save.set_sensitive(!now.text.trim().is_empty() && *now != before);
        })
    };

    // The words.
    let text = gtk::Entry::builder().text(&before.text).placeholder_text("What to do").css_classes(["title-4"]).build();
    {
        let (fields, refresh) = (fields.clone(), refresh.clone());
        text.connect_changed(move |entry| {
            fields.borrow_mut().text = entry.text().to_string();
            refresh();
        });
    }
    content.append(&text);

    // Status and priority, as rows of pills of which one is on: every
    // status the vault's Tasks plugin settings name, and the task's own.
    let reason = gtk::Entry::builder()
        .text(before.reason.as_deref().unwrap_or(""))
        .placeholder_text("Why it is cancelled, or what it waits for")
        .hexpand(true)
        .build();
    let statuses = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .max_children_per_line(8)
        .hexpand(true)
        .build();
    let mut all: Vec<TaskStatus> = TaskStatuses::current().all.clone();
    if !all.iter().any(|s| s.symbol == before.status) && before.status != 'X' {
        all.push(TaskStatuses::current().status(before.status));
    }
    let mut group: Option<gtk::ToggleButton> = None;
    for status in all {
        let symbol = status.symbol;
        let pill = gtk::ToggleButton::builder()
            .label(&status.name)
            .tooltip_text(format!("[{symbol}]"))
            .css_classes(["daftar-pill", "daftar-chip", status_tint(status.kind)])
            .active(before.status == symbol || (before.status == 'X' && symbol == 'x'))
            .build();
        pill.set_group(group.as_ref());
        group.get_or_insert_with(|| pill.clone());
        let (fields, refresh, reason, from) = (fields.clone(), refresh.clone(), reason.clone(), before.status);
        pill.connect_toggled(move |pill| {
            if pill.is_active() {
                fields.borrow_mut().status = symbol;
                refresh();
                // Cancelling asks why: the reason is the next thing to type.
                if cancels(from, symbol) {
                    reason.grab_focus();
                }
            }
        });
        statuses.insert(&pill, -1);
    }
    content.append(&row("Status", &statuses));
    {
        let (fields, refresh) = (fields.clone(), refresh.clone());
        reason.connect_changed(move |entry| {
            let text = entry.text().trim().to_string();
            fields.borrow_mut().reason = Some(text).filter(|t| !t.is_empty());
            refresh();
        });
    }
    content.append(&row("Reason", &reason));

    let priorities = gtk::Box::builder().spacing(6).build();
    let mut group: Option<gtk::ToggleButton> = None;
    for (emoji, name, tint) in PRIORITIES {
        let pill = gtk::ToggleButton::builder()
            .label(name)
            .css_classes(["daftar-pill", "daftar-chip", tint])
            .active(before.priority.as_deref() == emoji)
            .build();
        pill.set_group(group.as_ref());
        group.get_or_insert_with(|| pill.clone());
        let (fields, refresh) = (fields.clone(), refresh.clone());
        pill.connect_toggled(move |pill| {
            if pill.is_active() {
                fields.borrow_mut().priority = emoji.map(String::from);
                refresh();
            }
        });
        priorities.append(&pill);
    }
    content.append(&row("Priority", &priorities));

    // The dates: each on or off, then the day.
    let dates = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    type Field = fn(&mut TaskFields) -> &mut Option<String>;
    let date_fields: [(&str, &str, Field); 3] = [
        ("📅 Due", "t-accent", |f| &mut f.due),
        ("⏳ Scheduled", "t-blue", |f| &mut f.scheduled),
        ("🛫 Start", "t-purple", |f| &mut f.start),
    ];
    for (name, tint, field) in date_fields {
        let value = field(&mut before.clone()).clone();
        dates.append(&date_row(name, tint, value, {
            let (fields, refresh) = (fields.clone(), refresh.clone());
            move |value| {
                *field(&mut fields.borrow_mut()) = value;
                refresh();
            }
        }));
    }
    content.append(&row("Dates", &dates));

    // The repeat rule.
    let repeat = gtk::Box::builder().spacing(6).build();
    let rule = gtk::Entry::builder().text(before.recurring.as_deref().unwrap_or("")).placeholder_text("every week").hexpand(true).build();
    let warning = gtk::Image::builder()
        .icon_name("dialog-warning-symbolic")
        .css_classes(["t-yellow", "tint"])
        .tooltip_text("Daftar repeats only plain intervals, such as every 2 weeks; Obsidian reads the rest.")
        .build();
    let check = {
        let warning = warning.clone();
        move |text: &str| warning.set_visible(!text.trim().is_empty() && Recurrence::parse(text).is_none())
    };
    check(&rule.text());
    {
        let (fields, refresh) = (fields.clone(), refresh.clone());
        rule.connect_changed(move |entry| {
            let text = entry.text().to_string();
            check(&text);
            fields.borrow_mut().recurring = Some(text).filter(|t| !t.trim().is_empty());
            refresh();
        });
    }
    let menu = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    let presets = gtk::Popover::builder().child(&menu).build();
    for choice in RULES.iter().copied().chain(["Never"]) {
        let item = gtk::Button::builder().child(&gtk::Label::builder().label(choice).xalign(0.0).build()).css_classes(["flat"]).build();
        let (rule, presets) = (rule.clone(), presets.clone());
        item.connect_clicked(move |_| {
            rule.set_text(if choice == "Never" { "" } else { choice });
            presets.popdown();
        });
        menu.append(&item);
    }
    let pick = gtk::MenuButton::builder().icon_name("media-playlist-repeat-symbolic").popover(&presets).tooltip_text("Repeat").build();
    repeat.append(&rule);
    repeat.append(&warning);
    repeat.append(&pick);
    content.append(&row("Repeat", &repeat));

    content.append(&error);
    let buttons = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
    let cancel = gtk::Button::builder().label("Cancel").css_classes(["pill"]).build();
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    let saved: Rc<dyn Fn()> = Rc::new(saved);
    let (store_for_postpone, saved_for_postpone) = (store.clone(), saved.clone());
    {
        let (dialog, path, error, saved) = (dialog.clone(), path.to_string(), error, saved.clone());
        save.connect_clicked(move |_| {
            let now = fields.borrow().clone();
            if store.edit_task(&path, line, &before, &now) == Ok(()) {
                dialog.close();
                saved();
            } else {
                error.set_label("The note changed before the save. Nothing was written.");
                error.set_visible(true);
            }
        });
    }
    refresh();
    // "Postpone", the Tasks plugin's menu, for an open task with a date.
    let postpone = postponable.then(|| postpone_button(&dialog, &store_for_postpone, path, line, saved_for_postpone.clone()));
    let bar = gtk::Box::builder().spacing(8).build();
    if let Some(postpone) = &postpone {
        bar.append(postpone);
    }
    buttons.set_hexpand(true);
    buttons.append(&cancel);
    buttons.append(&save);
    bar.append(&buttons);
    content.append(&bar);
    dialog.set_default_widget(Some(&save));
    text.set_activates_default(true);
    dialog.present(Some(parent));
    text.grab_focus();
}

/// "Postpone": by a day to a month, as the Tasks plugin's menu moves the
/// task's due date, else its scheduled, else its start date. The line is
/// read again and the move refused when it changed.
fn postpone_button(dialog: &adw::Dialog, store: &Rc<VaultStore>, path: &str, line: usize, saved: Rc<dyn Fn()>) -> gtk::MenuButton {
    let choices = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    let popover = gtk::Popover::builder().child(&choices).build();
    for amount in Postpone::ALL {
        let button = gtk::Button::builder()
            .child(&gtk::Label::builder().label(format!("By {}", amount.title())).xalign(0.0).build())
            .css_classes(["flat"])
            .build();
        let (dialog, store, path, saved, popover) = (dialog.clone(), store.clone(), path.to_string(), saved.clone(), popover.clone());
        button.connect_clicked(move |_| {
            popover.popdown();
            let task = store
                .text(&path)
                .and_then(|text| text.split('\n').nth(line).and_then(|raw| TaskLine::parse(raw.trim_end_matches('\r'), line, "")));
            let moved =
                task.is_some_and(|task| store.move_task(&VaultTask { path: path.clone(), task }, &BoardDrop::Postpone(amount)).is_ok());
            dialog.close();
            if moved {
                saved();
            }
        });
        choices.append(&button);
    }
    gtk::MenuButton::builder().label("Postpone").popover(&popover).css_classes(["pill"]).build()
}

/// A title on the left, the controls on the right.
fn row(title: &str, controls: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::builder().spacing(12).build();
    row.append(
        &gtk::Label::builder()
            .label(title)
            .width_chars(8)
            .xalign(1.0)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .css_classes(["dim-label", "heading"])
            .build(),
    );
    row.append(controls);
    row
}

/// A date that is there or not: a switch, the day typed or picked on a
/// calendar, and how far it is from today. [changed] gets every new value.
fn date_row(name: &str, tint: &str, value: Option<String>, changed: impl Fn(Option<String>) + 'static) -> gtk::Box {
    let today = Day::today();
    // A day typed in the Solar Hijri calendar (`1405-08-05`) is its Gregorian day.
    let parse = |text: &str| Day::parse(text).or_else(|| daftar_core::jalali::Jalali::parse(text));
    let row = gtk::Box::builder().spacing(8).css_classes([tint]).build();
    let switch = gtk::Switch::builder().active(value.is_some()).valign(gtk::Align::Center).build();
    row.append(&switch);
    row.append(&gtk::Label::builder().label(name).width_chars(12).xalign(0.0).build());
    let entry = gtk::Entry::builder()
        .text(value.as_deref().unwrap_or(""))
        .placeholder_text("YYYY-MM-DD")
        .tooltip_text("A day, as 2026-10-27 or in the Persian calendar as 1405-08-05")
        .width_chars(11)
        .sensitive(value.is_some())
        .build();
    let calendar = gtk::Calendar::new();
    let popover = gtk::Popover::builder().child(&calendar).build();
    let pick = gtk::MenuButton::builder().icon_name("x-office-calendar-symbolic").popover(&popover).sensitive(value.is_some()).build();
    let how_far = gtk::Label::builder().css_classes(["daftar-day", tint]).build();
    row.append(&entry);
    row.append(&pick);
    row.append(&how_far);
    let changed = Rc::new(changed);
    let show = {
        let (how_far, entry) = (how_far, entry.clone());
        move |on: bool| {
            let day = on.then(|| parse(&entry.text())).flatten();
            how_far.set_label(
                &day.map(|d| match crate::settings::persian_date(d) {
                    Some(persian) => format!("{} · {persian}", relative(d, today)),
                    None => relative(d, today),
                })
                .unwrap_or_default(),
            );
            how_far.set_visible(day.is_some());
            entry.remove_css_class("error");
            if on && day.is_none() {
                entry.add_css_class("error");
            }
        }
    };
    show(value.is_some());
    {
        let (switch, changed, show) = (switch.clone(), changed.clone(), show.clone());
        entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            show(switch.is_active());
            // A day half typed is not written; the last whole one stays.
            if Day::parse(&text).is_some() {
                changed(Some(text));
            } else if let Some(day) = daftar_core::jalali::Jalali::parse(&text) {
                changed(Some(day.to_string()));
            }
        });
    }
    {
        let (entry, pick, changed) = (entry.clone(), pick, changed);
        switch.connect_active_notify(move |switch| {
            let on = switch.is_active();
            entry.set_sensitive(on);
            pick.set_sensitive(on);
            if on {
                if parse(&entry.text()).is_none() {
                    entry.set_text(&today.to_string());
                }
                changed(Some(entry.text().to_string()));
            } else {
                changed(None);
            }
            show(on);
        });
    }
    {
        let (entry, popover) = (entry, popover);
        calendar.connect_day_selected(move |calendar| {
            let date = calendar.date();
            if let Some(day) = Day::new(date.year(), date.month() as u32, date.day_of_month() as u32) {
                entry.set_text(&day.to_string());
            }
            popover.popdown();
        });
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_read_from_today() {
        let today = Day::parse("2026-10-26").unwrap();
        let day = |s| Day::parse(s).unwrap();
        assert_eq!(relative(day("2026-10-26"), today), "today");
        assert_eq!(relative(day("2026-10-27"), today), "tomorrow");
        assert_eq!(relative(day("2026-10-25"), today), "yesterday");
        assert_eq!(relative(day("2026-10-30"), today), "in 4 days");
        assert_eq!(relative(day("2026-10-20"), today), "6 days ago");
    }
}
