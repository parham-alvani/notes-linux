//! Settings (Ctrl+,): the keys of `~/.config/daftar/config.toml` this app
//! has, each written back on its own line as it changes, so the file keeps
//! the person's comments and the keys only the Mac has.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::Value;

use crate::settings::AppSettings;

/// What the window does when a setting changes.
pub struct Hooks {
    pub theme: Box<dyn Fn(&str)>,
    pub reader: Box<dyn Fn()>,
    pub git: Box<dyn Fn(bool, u32)>,
    pub notice: Box<dyn Fn(&str)>,
}

const THEMES: [(&str, &str); 3] = [("system", "Follow the System"), ("light", "Daylight"), ("dark", "Naz")];

pub fn present(parent: &impl IsA<gtk::Widget>, settings: Rc<RefCell<AppSettings>>, hooks: Rc<Hooks>) {
    let dialog = adw::PreferencesDialog::builder().title("Settings").build();
    let page = adw::PreferencesPage::new();
    dialog.add(&page);
    let write = {
        let (settings, hooks) = (settings.clone(), hooks.clone());
        move |key: &str, value: Value| {
            if let Err(error) = settings.borrow_mut().set(key, value) {
                (hooks.notice)(&format!("The settings were not saved: {error}"));
            }
        }
    };

    let looks = adw::PreferencesGroup::builder().title("Appearance").build();
    let theme = adw::ComboRow::builder()
        .title("Theme")
        .model(&gtk::StringList::new(&THEMES.map(|(_, name)| name)))
        .selected(THEMES.iter().position(|(key, _)| *key == settings.borrow().theme()).unwrap_or(0) as u32)
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        theme.connect_selected_notify(move |row| {
            let key = THEMES[row.selected() as usize].0;
            write("appearance.theme", Value::String(key.into()));
            (hooks.theme)(key);
        });
    }
    looks.add(&theme);
    let size = adw::SpinRow::with_range(10.0, 28.0, 1.0);
    size.set_title("Text size");
    size.set_value(settings.borrow().text_size() as f64);
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        size.connect_value_notify(move |row| {
            write("appearance.text_size", Value::Int(row.value() as i64));
            (hooks.reader)();
        });
    }
    looks.add(&size);
    for (key, title) in [("appearance.font", "Font"), ("appearance.code_font", "Code font")] {
        let row = adw::EntryRow::builder().title(title).show_apply_button(true).build();
        row.set_text(&settings.borrow().file.string(key).unwrap_or_default());
        let (write, hooks) = (write.clone(), hooks.clone());
        row.connect_apply(move |row| {
            write(key, Value::String(row.text().trim().to_string()));
            (hooks.reader)();
        });
        looks.add(&row);
    }
    page.add(&looks);

    let git = adw::PreferencesGroup::builder()
        .title("Git")
        .description("Keep automatic sync off while Obsidian's git plugin syncs the same folder.")
        .build();
    let pull = adw::SwitchRow::builder()
        .title("Pull when Daftar starts")
        .active(settings.borrow().file.bool("git.pull_on_launch").unwrap_or(true))
        .build();
    let interval = adw::SpinRow::with_range(0.0, 1440.0, 5.0);
    interval.set_title("Sync every");
    interval.set_subtitle("Minutes between commit, pull and push; 0 is off");
    interval.set_value(settings.borrow().file.int("git.sync_interval").unwrap_or(0).max(0) as f64);
    let changed = {
        let (pull, interval, write, hooks) = (pull.clone(), interval.clone(), write.clone(), hooks.clone());
        Rc::new(move || {
            write("git.pull_on_launch", Value::Bool(pull.is_active()));
            write("git.sync_interval", Value::Int(interval.value() as i64));
            (hooks.git)(pull.is_active(), interval.value() as u32);
        })
    };
    {
        let changed = changed.clone();
        pull.connect_active_notify(move |_| changed());
    }
    interval.connect_value_notify(move |_| changed());
    git.add(&pull);
    git.add(&interval);
    page.add(&git);

    let file = adw::PreferencesGroup::builder()
        .title("The settings file")
        .description("Daftar for macOS reads the same file; a key one app does not have is left alone by the other.")
        .build();
    let path = settings.borrow().path.clone();
    let open = adw::ActionRow::builder().title(path.to_string_lossy()).activatable(true).build();
    open.add_suffix(&gtk::Image::from_icon_name("document-open-symbolic"));
    open.connect_activated(move |row| {
        let launcher = gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(&path)));
        launcher.launch(row.root().and_downcast::<gtk::Window>().as_ref(), gtk::gio::Cancellable::NONE, |_| {});
    });
    file.add(&open);
    page.add(&file);

    dialog.present(Some(parent));
}
