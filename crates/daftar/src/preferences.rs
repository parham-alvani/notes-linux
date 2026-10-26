//! Settings (Ctrl+,): the keys of `~/.config/daftar/config.toml` this app
//! has, each written back on its own line as it changes, so the file keeps
//! the person's comments and the keys only the Mac has.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::Value;

use crate::settings::AppSettings;

/// What the window does when a setting changes.
pub struct Hooks {
    pub theme: Box<dyn Fn(&str)>,
    pub reader: Box<dyn Fn()>,
    /// The home pages, after a new picture.
    pub home: Box<dyn Fn()>,
    pub git: Box<dyn Fn(bool, u32)>,
    pub notice: Box<dyn Fn(&str)>,
}

const THEMES: [(&str, &str); 3] = [("system", "Follow the System"), ("light", "Daylight"), ("dark", "Naz")];

/// [root] is the open vault's folder: a picture in it is kept as its path in the vault.
pub fn present(parent: &impl IsA<gtk::Widget>, settings: Rc<RefCell<AppSettings>>, root: Option<PathBuf>, hooks: Rc<Hooks>) {
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
    page.add(&home_group(&settings, root, write.clone(), hooks.clone()));

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

/// Settings › Home: the picture behind the home page, chosen or cleared.
fn home_group(
    settings: &Rc<RefCell<AppSettings>>,
    root: Option<PathBuf>,
    write: impl Fn(&str, Value) + Clone + 'static,
    hooks: Rc<Hooks>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Home")
        .description("The picture fills the page a tab shows with no note open: PNG, JPEG or WebP; one in the vault is kept as its path in the vault. The time, the greeting, the lists and the quotes follow the Beautitab plugin's settings in the vault.")
        .build();
    let row = adw::ActionRow::builder().title("Picture").build();
    let preview =
        gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).width_request(96).height_request(60).css_classes(["card"]).build();
    preview.set_overflow(gtk::Overflow::Hidden);
    row.add_prefix(&preview);
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    let clear = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).build();
    row.add_suffix(&choose);
    row.add_suffix(&clear);
    let show = {
        let (row, preview, clear, root) = (row.clone(), preview.clone(), clear.clone(), root.clone());
        Rc::new(move |setting: &str| {
            let path = daftar_core::home::picture_path(setting, root.as_deref(), &gtk::glib::home_dir());
            let missing = !setting.is_empty() && !path.as_deref().is_some_and(Path::is_file);
            let text = if setting.is_empty() {
                "None".to_string()
            } else if missing {
                format!("{setting} (not found)")
            } else {
                setting.to_string()
            };
            row.set_subtitle(&gtk::glib::markup_escape_text(&text));
            preview.set_filename(path.filter(|p| p.is_file()).as_deref());
            clear.set_sensitive(!setting.is_empty());
        })
    };
    show(&settings.borrow().file.string("home.image").unwrap_or_default());
    let set = {
        let show = show.clone();
        Rc::new(move |setting: String| {
            write("home.image", Value::String(setting.clone()));
            show(&setting);
            (hooks.home)();
        })
    };
    {
        let set = set.clone();
        clear.connect_clicked(move |_| set(String::new()));
    }
    choose.connect_clicked(move |button| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Pictures"));
        filter.add_pixbuf_formats();
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Choose a Picture").modal(true).filters(&filters).build();
        let (set, root) = (set.clone(), root.clone());
        dialog.open(button.root().and_downcast::<gtk::Window>().as_ref(), gtk::gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else { return };
            set(setting_for(&path, root.as_deref(), &gtk::glib::home_dir()));
        });
    });
    group.add(&row);
    group
}

/// How the config file keeps a chosen picture: its path in the vault, a
/// `~` path, or the absolute path, as the Mac keeps it.
pub fn setting_for(path: &Path, vault: Option<&Path>, home: &Path) -> String {
    if let Some(inside) = vault.and_then(|vault| path.strip_prefix(vault).ok()) {
        return inside.to_string_lossy().into_owned();
    }
    if let Ok(inside) = path.strip_prefix(home) {
        return format!("~/{}", inside.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_kept_in_the_vault_then_home_then_whole() {
        let (vault, home) = (Path::new("/home/x/org"), Path::new("/home/x"));
        assert_eq!(setting_for(Path::new("/home/x/org/Attachments/sea.jpg"), Some(vault), home), "Attachments/sea.jpg");
        assert_eq!(setting_for(Path::new("/home/x/Pictures/a.png"), Some(vault), home), "~/Pictures/a.png");
        assert_eq!(setting_for(Path::new("/srv/a.png"), None, home), "/srv/a.png");
    }
}
