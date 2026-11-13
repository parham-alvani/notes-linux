//! The Tasks plugin's "edit task", as a dialog: the text, the status, the
//! reason, the priority, the dates and the repeat rule, each in its color. The line is
//! read from the file when the dialog opens and again before it is written,
//! and the save is refused when it changed meanwhile. The Mac's
//! `TaskEditSheet`. With `present_new` it makes a task instead (New Task,
//! Ctrl+Alt+T): the same form, with the note and the section it goes in,
//! and today as its created date.

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

const RULES: [&str; 7] =
    ["every day", "every week", "every 2 weeks", "every month", "every jalali month", "every year", "every week when done"];

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

/// What the dialog is for: a task on a line, or a new one.
enum Mode {
    Edit { path: String, line: usize },
    New { path: String, heading: Option<usize> },
}

/// Opens the dialog over [parent]; [saved] runs after a write went through.
pub fn present(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, path: &str, line: usize, saved: impl Fn() + 'static) {
    open(parent, store, Mode::Edit { path: path.to_string(), line }, Rc::new(|| {}), Rc::new(move |_: &str| saved()));
}

/// New Task over [parent], for the note at [path] ("" for none chosen yet)
/// at the end of the section of the heading on [heading]. [save_first]
/// runs before the write, so the editors' text is on disk; [saved] gets
/// the note written.
pub fn present_new(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<VaultStore>,
    path: &str,
    heading: Option<usize>,
    save_first: impl Fn() + 'static,
    saved: impl Fn(&str) + 'static,
) {
    open(parent, store, Mode::New { path: path.to_string(), heading }, Rc::new(save_first), Rc::new(saved));
}

/// The notes a new task can go in: those with tasks first, then the rest,
/// each by path. Not the week notes, which the phone writes whole, nor the
/// hidden notes.
fn note_choices(store: &VaultStore) -> Vec<String> {
    let index = store.index();
    let mut notes: Vec<(bool, String)> = index
        .notes
        .values()
        .filter(|n| VaultStore::takes_tasks(&n.path) && !store.is_hidden(&n.path))
        .map(|n| (n.parsed.tasks.is_empty(), n.path.clone()))
        .collect();
    notes.sort();
    notes.into_iter().map(|(_, path)| path).collect()
}

/// A note's headings for the Section list: their lines, and how they are shown.
fn section_choices(store: &VaultStore, path: &str) -> (Vec<Option<usize>>, Vec<String>) {
    let headings = daftar_core::Sections::headings(&store.text(path).unwrap_or_default());
    let mut lines = vec![None];
    let mut names = vec!["End of the Note".to_string()];
    for heading in headings {
        lines.push(Some(heading.line));
        names.push(format!("{}{}", "    ".repeat(heading.level.saturating_sub(1)), heading.text));
    }
    (lines, names)
}

#[expect(clippy::too_many_lines, reason = "one form, built top to bottom as it is shown")]
fn open(parent: &impl IsA<gtk::Widget>, store: Rc<VaultStore>, mode: Mode, save_first: Rc<dyn Fn()>, saved: Rc<dyn Fn(&str)>) {
    let new = matches!(mode, Mode::New { .. });
    let (path, line) = match &mode {
        Mode::Edit { path, line } => (path.clone(), *line),
        Mode::New { path, .. } => (path.clone(), 0),
    };
    let path = path.as_str();
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
    let title = if new { "New Task" } else { "Edit Task" };
    titles.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["title-2"]).build());
    let subtitle = gtk::Label::builder()
        .label(if new { String::new() } else { format!("{}, line {}", Vault::title(path), line + 1) })
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    titles.append(&subtitle);
    head.append(&titles);
    content.append(&head);

    let dialog = adw::Dialog::builder().title(title).content_width(680).child(&content).build();
    let error = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["t-red", "tint"]).visible(false).build();
    let read_now =
        if new { Ok(TaskFields { created: Some(Day::today().to_string()), ..TaskFields::default() }) } else { read(&store, path, line) };
    let before = match read_now {
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
    let postponable =
        !new && !TaskLine::is_closed(before.status) && (before.due.is_some() || before.scheduled.is_some() || before.start.is_some());
    let save = gtk::Button::builder().label(if new { "Add Task" } else { "Save" }).css_classes(["suggested-action", "pill"]).build();
    // A new task's note, and the line of its section's heading.
    let notes = if new { note_choices(&store) } else { Vec::new() };
    let target = Rc::new(RefCell::new(match &mode {
        Mode::New { path, heading } => (Some(path.clone()).filter(|p| notes.contains(p)), *heading),
        Mode::Edit { .. } => (None, None),
    }));
    let refresh = {
        let (fields, before, save, target, subtitle) = (fields.clone(), before.clone(), save.clone(), target.clone(), subtitle);
        Rc::new(move || {
            let now = fields.borrow();
            let filled = !now.text.trim().is_empty();
            if new {
                let note = target.borrow().0.clone();
                subtitle.set_label(
                    &note.as_deref().map_or_else(|| "Pick the note it goes in".to_string(), |p| format!("In {}", Vault::title(p))),
                );
                save.set_sensitive(filled && note.is_some());
            } else {
                save.set_sensitive(filled && *now != before);
            }
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

    // Where a new task goes: a note, searched by its path, and a section of it.
    if new {
        let note = gtk::DropDown::from_strings(&notes.iter().map(String::as_str).collect::<Vec<_>>());
        note.set_enable_search(true);
        note.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        note.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string")));
        note.set_hexpand(true);
        let section = gtk::DropDown::from_strings(&["End of the Note"]);
        let section_lines = Rc::new(RefCell::new(vec![None]));
        let fill = {
            let (store, section, section_lines, target) = (store.clone(), section.clone(), section_lines.clone(), target.clone());
            move |path: &str, wanted: Option<usize>| {
                let (lines, names) = section_choices(&store, path);
                section.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
                let at = lines.iter().position(|l| *l == wanted).unwrap_or(0);
                section.set_selected(u32::try_from(at).unwrap_or(0));
                target.borrow_mut().1 = lines[at];
                *section_lines.borrow_mut() = lines;
            }
        };
        let (start, heading) = target.borrow().clone();
        if let Some(start) = start.as_deref()
            && let Some(at) = notes.iter().position(|p| p == start)
        {
            note.set_selected(u32::try_from(at).unwrap_or(0));
            fill(start, heading);
        } else {
            note.set_selected(gtk::INVALID_LIST_POSITION);
        }
        {
            let (target, refresh) = (target.clone(), refresh.clone());
            note.connect_selected_notify(move |note| {
                let chosen = usize::try_from(note.selected()).ok().and_then(|i| notes.get(i)).cloned();
                if let Some(path) = &chosen {
                    fill(path, None);
                }
                target.borrow_mut().0 = chosen;
                refresh();
            });
        }
        {
            let target = target.clone();
            section.connect_selected_notify(move |section| {
                let at = usize::try_from(section.selected()).unwrap_or(0);
                target.borrow_mut().1 = section_lines.borrow().get(at).copied().flatten();
            });
        }
        content.append(&row("Note", &note));
        content.append(&row("Section", &section));
    }

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
    let date_fields: [(&str, &str, Field); 4] = [
        ("📅 Due", "t-accent", |f| &mut f.due),
        ("⏳ Scheduled", "t-blue", |f| &mut f.scheduled),
        ("🛫 Start", "t-purple", |f| &mut f.start),
        ("➕ Created", "t-green", |f| &mut f.created),
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
        .tooltip_text("Daftar repeats only plain intervals, such as every 2 weeks or every jalali month; Obsidian reads the rest.")
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
    let (store_for_postpone, saved_for_postpone) = (store.clone(), saved.clone());
    {
        let (dialog, path, error, saved) = (dialog.clone(), path.to_string(), error, saved.clone());
        save.connect_clicked(move |_| {
            let now = fields.borrow().clone();
            if new {
                let (Some(note), heading) = target.borrow().clone() else { return };
                save_first();
                // A heading that went from the note while the form was open: the end of the note.
                let headings = daftar_core::Sections::headings(&store.text(&note).unwrap_or_default());
                let heading = heading.filter(|line| headings.iter().any(|h| h.line == *line));
                if store.add_task(&note, heading, &now) == Ok(()) {
                    dialog.close();
                    saved(&note);
                } else {
                    error.set_label("The task could not be added to that note.");
                    error.set_visible(true);
                }
                return;
            }
            if store.edit_task(&path, line, &before, &now) == Ok(()) {
                dialog.close();
                saved(&path);
            } else {
                error.set_label("The note changed before the save. Nothing was written.");
                error.set_visible(true);
            }
        });
    }
    refresh();
    // "Postpone", the Tasks plugin's menu, for an open task with a date.
    let postpone = postponable.then(|| {
        let saved = saved_for_postpone.clone();
        let path_ = path.to_string();
        postpone_button(&dialog, &store_for_postpone, path, line, Rc::new(move || saved(&path_)))
    });
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
    let popover = gtk::Popover::new();
    let picker = {
        let (entry, popover) = (entry.clone(), popover.clone());
        crate::day_picker::DayPicker::new(move |day| {
            entry.set_text(&day.to_string());
            popover.popdown();
        })
    };
    popover.set_child(Some(&picker.widget));
    {
        let entry = entry.clone();
        popover.connect_show(move |_| picker.select(parse(&entry.text())));
    }
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
        let (entry, pick, changed) = (entry, pick, changed);
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
