//! The note's front matter in the inspector, as the Mac's Properties view
//! shows it above the editor: a row for each property with a control for
//! its type (a check box, a date with a calendar, a list of chips, a text),
//! a menu to rename or remove it, and a button that adds one. Each change
//! is a `win.set-property` action with a `PropertyEdit`, which the window
//! writes as the app writes any edit: the editor saved first, the file read
//! again, and refused when its front matter changed. Front matter the form
//! cannot show without a change stays YAML, and the form says why.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use daftar_core::properties::{FrontMatterForm, PropertyEdit, PropertyNames, PropertySheet, PropertyType, PropertyValue};

/// The type picked for a property added without a value, until it has one.
pub type Chosen = Rc<RefCell<HashMap<String, PropertyType>>>;

/// The form for [note]'s text; [names] are the vault's property names, the
/// most used first.
pub fn form(note: &str, types: &HashMap<String, String>, names: Rc<Vec<String>>, chosen: &Chosen) -> gtk::Box {
    let place = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    let sheet = match PropertySheet::read(note) {
        FrontMatterForm::Source(reason) => {
            place.append(&dim(&format!("The front matter stays YAML: the form cannot show it without a change. {reason}")));
            return place;
        }
        FrontMatterForm::Absent(sheet) | FrontMatterForm::Form(sheet) => sheet,
    };
    if !sheet.properties().is_empty() {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        for property in sheet.properties() {
            let kind = chosen
                .borrow()
                .get(&property.key)
                .copied()
                .filter(|_| property.value == PropertyValue::Empty)
                .unwrap_or_else(|| PropertyType::infer(&property.key, &property.value, types));
            list.append(&row(&property.key, &property.value, kind));
        }
        place.append(&list);
    }
    place.append(&adder(names, chosen));
    place
}

/// Sends [edit] to the window, which writes it.
fn send(widget: &impl IsA<gtk::Widget>, edit: &PropertyEdit) {
    let _ = widget.activate_action("win.set-property", Some(&edit.encode().to_variant()));
}

fn row(key: &str, value: &PropertyValue, kind: PropertyType) -> gtk::ListBoxRow {
    let line = gtk::Box::builder().spacing(8).margin_top(4).margin_bottom(4).margin_start(8).margin_end(4).build();
    let name = gtk::Label::builder()
        .label(key)
        .xalign(0.0)
        .width_chars(9)
        .max_width_chars(12)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .tooltip_text(format!("{key} ({})", kind.title()))
        .css_classes(["dim-label"])
        .build();
    line.append(&name);
    let control = control(key, value, kind);
    control.set_hexpand(true);
    line.append(&control);
    line.append(&menu(key));
    gtk::ListBoxRow::builder().child(&line).activatable(false).build()
}

/// The control for a value of [kind].
fn control(key: &str, value: &PropertyValue, kind: PropertyType) -> gtk::Widget {
    let text = match value {
        PropertyValue::Text(text) => text.clone(),
        PropertyValue::List(items) => items.join(", "),
        PropertyValue::Empty => String::new(),
    };
    match kind {
        PropertyType::Checkbox => {
            let check = gtk::CheckButton::builder().active(text.eq_ignore_ascii_case("true")).build();
            let key = key.to_string();
            check.connect_toggled(move |check| {
                let value = if check.is_active() { "true" } else { "false" };
                send(check, &PropertyEdit::Set(key.clone(), PropertyValue::Text(value.into())));
            });
            check.upcast()
        }
        kind if kind.is_list() => chips(key, &value.items()),
        PropertyType::Date | PropertyType::Datetime => {
            let field = gtk::Box::builder().spacing(4).build();
            let entry = entry(key, &text, if kind == PropertyType::Date { "YYYY-MM-DD" } else { "YYYY-MM-DDTHH:MM" });
            entry.set_hexpand(true);
            entry.set_width_chars(if kind == PropertyType::Date { 10 } else { 16 });
            field.append(&entry);
            let calendar = gtk::Calendar::new();
            if let Some(date) = PropertyType::date(&text)
                && let Ok(day) = glib::DateTime::from_local(
                    chrono::Datelike::year(&date),
                    i32::try_from(chrono::Datelike::month(&date)).unwrap_or(1),
                    i32::try_from(chrono::Datelike::day(&date)).unwrap_or(1),
                    0,
                    0,
                    0.0,
                )
            {
                calendar.select_day(&day);
            }
            let popover = gtk::Popover::builder().child(&calendar).build();
            let button = gtk::MenuButton::builder()
                .icon_name("x-office-calendar-symbolic")
                .popover(&popover)
                .css_classes(["flat"])
                .tooltip_text("Pick a day")
                .build();
            let (key, time) = (key.to_string(), kind == PropertyType::Datetime);
            calendar.connect_day_selected(move |calendar| {
                let day = calendar.date();
                let picked = format!("{:04}-{:02}-{:02}", day.year(), day.month(), day.day_of_month());
                // A date and time keeps its time; a new one starts at nine.
                let value = if time {
                    let clock = text.split_once(['T', ' ']).map_or("09:00", |(_, t)| t);
                    format!("{picked}T{}", &clock[..clock.len().min(5)])
                } else {
                    picked
                };
                popover.popdown();
                send(calendar, &PropertyEdit::Set(key.clone(), PropertyValue::Text(value)));
            });
            field.append(&button);
            field.upcast()
        }
        PropertyType::Number => {
            let entry = entry(key, &text, "0");
            entry.set_input_purpose(gtk::InputPurpose::Number);
            entry.upcast()
        }
        _ => entry(key, &text, "Empty").upcast(),
    }
}

/// A text field that sets [key] on Enter or when it loses the focus, if its text changed.
fn entry(key: &str, text: &str, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder().text(text).placeholder_text(placeholder).build();
    let shown = Rc::new(RefCell::new(text.to_string()));
    let commit = {
        let key = key.to_string();
        move |entry: &gtk::Entry| {
            let now = entry.text().to_string();
            if *shown.borrow() == now {
                return;
            }
            shown.borrow_mut().clone_from(&now);
            send(entry, &PropertyEdit::Set(key.clone(), PropertyValue::Text(now)));
        }
    };
    let on_enter = commit.clone();
    entry.connect_activate(move |entry| on_enter(entry));
    let focus = gtk::EventControllerFocus::new();
    let weak = entry.downgrade();
    focus.connect_leave(move |_| {
        if let Some(entry) = weak.upgrade() {
            commit(&entry);
        }
    });
    entry.add_controller(focus);
    entry
}

/// A list's items as chips, each with a button that takes it away, and a field that adds one.
fn chips(key: &str, items: &[String]) -> gtk::Widget {
    let flow =
        gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).column_spacing(4).row_spacing(4).max_children_per_line(8).build();
    for (at, item) in items.iter().enumerate() {
        let chip = gtk::Box::builder().spacing(2).css_classes(["card"]).build();
        chip.append(
            &gtk::Label::builder().label(item).margin_start(6).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(20).build(),
        );
        let remove =
            gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "circular"]).tooltip_text("Remove").build();
        let (key, items) = (key.to_string(), items.to_vec());
        remove.connect_clicked(move |button| {
            let mut left = items.clone();
            left.remove(at);
            send(button, &PropertyEdit::Set(key.clone(), PropertyValue::List(left)));
        });
        chip.append(&remove);
        flow.append(&chip);
    }
    let add = gtk::Entry::builder().placeholder_text("Add…").width_chars(8).build();
    let (key, items) = (key.to_string(), items.to_vec());
    add.connect_activate(move |add| {
        let item = add.text().trim().to_string();
        if item.is_empty() || items.contains(&item) {
            return;
        }
        let mut more = items.clone();
        more.push(item);
        send(add, &PropertyEdit::Set(key.clone(), PropertyValue::List(more)));
    });
    flow.append(&add);
    flow.upcast()
}

/// Rename and Remove, in a menu at the row's end.
fn menu(key: &str) -> gtk::MenuButton {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    let name = gtk::Entry::builder().text(key).tooltip_text("Rename: a new name, then Enter").build();
    panel.append(&name);
    let remove = gtk::Button::builder().label("Remove Property").css_classes(["destructive-action"]).build();
    panel.append(&remove);
    let popover = gtk::Popover::builder().child(&panel).build();
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .popover(&popover)
        .css_classes(["flat"])
        .tooltip_text("Rename or remove")
        .valign(gtk::Align::Center)
        .build();
    let old = key.to_string();
    let to_close = popover.clone();
    name.connect_activate(move |name| {
        let new = name.text().trim().to_string();
        to_close.popdown();
        if !new.is_empty() && new != old {
            send(name, &PropertyEdit::Rename(old.clone(), new));
        }
    });
    let key = key.to_string();
    remove.connect_clicked(move |remove| {
        popover.popdown();
        send(remove, &PropertyEdit::Remove(key.clone()));
    });
    button
}

/// Add Property: a name, with the vault's names that match it to pick, and a type.
fn adder(names: Rc<Vec<String>>, chosen: &Chosen) -> gtk::Widget {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    let name = gtk::Entry::builder().placeholder_text("Name").build();
    panel.append(&name);
    let suggestions = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    panel.append(&suggestions);
    let titles: Vec<&str> = PropertyType::ALL.iter().map(|t| t.title()).collect();
    let kind = gtk::DropDown::from_strings(&titles);
    panel.append(&kind);
    let add = gtk::Button::builder().label("Add").css_classes(["suggested-action"]).build();
    panel.append(&add);
    let popover = gtk::Popover::builder().child(&panel).build();
    let button = gtk::MenuButton::builder()
        .child(&adw::ButtonContent::builder().icon_name("list-add-symbolic").label("Add Property").build())
        .popover(&popover)
        .css_classes(["flat"])
        .halign(gtk::Align::Start)
        .tooltip_text("Add a property to the note's front matter")
        .build();

    let fill = {
        let name = name.clone();
        move || {
            while let Some(child) = suggestions.first_child() {
                suggestions.remove(&child);
            }
            for found in PropertyNames::complete(&name.text(), &names, 6) {
                let pick = gtk::Button::builder().label(&found).css_classes(["flat"]).halign(gtk::Align::Start).build();
                let name = name.clone();
                pick.connect_clicked(move |_| {
                    name.set_text(&found);
                    name.set_position(-1);
                });
                suggestions.append(&pick);
            }
        }
    };
    fill();
    name.connect_changed(move |_| fill());
    let create = {
        let (name, chosen) = (name.clone(), chosen.clone());
        move || {
            let key = name.text().trim().to_string();
            if key.is_empty() {
                return;
            }
            let picked = PropertyType::ALL.get(kind.selected() as usize).copied().unwrap_or(PropertyType::Text);
            let value = if picked == PropertyType::Checkbox { PropertyValue::Text("false".into()) } else { PropertyValue::Empty };
            chosen.borrow_mut().insert(key.clone(), picked);
            popover.popdown();
            name.set_text("");
            send(&name, &PropertyEdit::Add(key, value));
        }
    };
    let on_enter = create.clone();
    name.connect_activate(move |_| on_enter());
    add.connect_clicked(move |_| create());
    button.upcast()
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim-label"]).build()
}
