//! Quick capture, as the Mac's: a small window with one line; Return adds it
//! to the capture note (`[capture]` in the config file, today's daily note
//! at first), Ctrl+Return adds it as a task, Escape closes. The Mac's
//! global hot key is the compositor's bind here: `daftar --capture` opens
//! the window from any app, in the running Daftar or on its own. A draft
//! closed unsent is there when the window opens again.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use daftar_core::{Clock, Day, QuickCapture};

use crate::settings::AppSettings;

/// The window's title, for a compositor rule that floats it.
pub const TITLE: &str = "Quick Capture";

thread_local! {
    static DRAFT: RefCell<String> = const { RefCell::new(String::new()) };
    static OPEN: RefCell<Option<glib::WeakRef<adw::Window>>> = const { RefCell::new(None) };
}

/// Opens the capture window, or closes it when it has the keys.
pub fn toggle(app: &adw::Application) {
    let open = OPEN.with(|open| open.borrow().as_ref().and_then(glib::WeakRef::upgrade));
    match open {
        Some(window) if window.is_active() => window.close(),
        Some(window) => window.present(),
        None => present(app),
    }
}

fn present(app: &adw::Application) {
    let settings = AppSettings::load();
    let root = settings.vault().filter(|root| root.is_dir());
    let target = settings.file.string("capture.target").unwrap_or_else(|| "daily".into());
    let heading = settings.file.string("capture.heading").unwrap_or_default();

    let window = adw::Window::builder().application(app).title(TITLE).default_width(560).resizable(false).build();
    let entry = gtk::Entry::builder().placeholder_text("Capture a thought").hexpand(true).css_classes(["capture-field"]).build();
    entry.set_text(&DRAFT.with(|draft| draft.borrow().clone()));
    let icon = gtk::Image::builder().icon_name("document-edit-symbolic").pixel_size(24).css_classes(["capture-icon"]).build();
    let task = gtk::CheckButton::with_label("Task");
    let goes = match &root {
        Some(root) => {
            let path = QuickCapture::target_path(root, &target, Day::today());
            let heading = heading.trim().trim_start_matches('#').trim();
            if heading.is_empty() { format!("→ {path}") } else { format!("→ {path} › {heading}") }
        }
        None => "No vault is open. Choose one in Daftar first.".into(),
    };
    let target_label = gtk::Label::builder()
        .label(&goes)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .css_classes(["dim-label"])
        .build();
    let as_task = gtk::Button::builder().label("Add as Task  Ctrl+↩").css_classes(["flat"]).build();
    let error = gtk::Label::builder().xalign(0.0).wrap(true).visible(false).css_classes(["capture-error"]).build();

    let line = gtk::Box::builder().spacing(10).build();
    line.append(&icon);
    line.append(&entry);
    let below = gtk::Box::builder().spacing(8).build();
    below.append(&task);
    below.append(&target_label);
    below.append(&as_task);
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["daftar-capture"]).build();
    content.append(&line);
    content.append(&below);
    content.append(&error);
    window.set_content(Some(&content));

    let add = {
        let (window, entry, task, error) = (window.clone(), entry.clone(), task, error);
        let (root, target, heading) = (root, target, heading);
        move |as_task: bool| {
            let Some(line) = QuickCapture::line(&entry.text(), as_task || task.is_active()) else { return };
            let Some(root) = root.clone() else {
                error.set_label("No vault is open.");
                error.set_visible(true);
                return;
            };
            // Cleared at once, so a second Return adds nothing twice.
            let draft = entry.text().to_string();
            entry.set_text("");
            entry.set_sensitive(false);
            let (window, entry, error, target, heading) = (window.clone(), entry.clone(), error.clone(), target.clone(), heading.clone());
            glib::spawn_future_local(async move {
                let written =
                    gio::spawn_blocking(move || QuickCapture::capture(&root, &target, &heading, &line, Day::today(), Clock::now())).await;
                entry.set_sensitive(true);
                match written {
                    Ok(Ok(_)) => {
                        DRAFT.with(|kept| kept.borrow_mut().clear());
                        window.close();
                    }
                    Ok(Err(message)) => {
                        entry.set_text(&draft);
                        error.set_label(&message);
                        error.set_visible(true);
                        entry.grab_focus();
                    }
                    Err(_) => entry.set_text(&draft),
                }
            });
        }
    };
    {
        let add = add.clone();
        entry.connect_activate(move |_| add(false));
    }
    {
        let add = add.clone();
        as_task.connect_clicked(move |_| add(true));
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (window, add) = (window.clone(), add);
        keys.connect_key_pressed(move |_, key, _, modifiers| match key {
            gdk::Key::Escape => {
                window.close();
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if modifiers.contains(gdk::ModifierType::CONTROL_MASK) => {
                add(true);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    window.add_controller(keys);
    {
        // The draft stays for the next time.
        let entry = entry.clone();
        window.connect_close_request(move |_| {
            DRAFT.with(|draft| *draft.borrow_mut() = entry.text().to_string());
            glib::Propagation::Proceed
        });
    }
    OPEN.with(|open| *open.borrow_mut() = Some(window.downgrade()));
    window.present();
    entry.grab_focus();
}
