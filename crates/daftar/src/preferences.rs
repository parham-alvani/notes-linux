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
    /// The notes being edited, opened again with the editor as now set.
    pub editor: Box<dyn Fn()>,
    /// The lists, after the hidden files changed.
    pub hidden: Box<dyn Fn()>,
    /// The notifications planned again, as now set.
    pub notifications: Box<dyn Fn()>,
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
            // The borrow ends before the notice, which may read the settings.
            let result = settings.borrow_mut().set(key, value);
            if let Err(error) = result {
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
    page.add(&editor_group(&settings, write.clone(), hooks.clone()));

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
    page.add(&files_group(&settings, write.clone(), hooks.clone()));
    page.add(&notifications_group(&settings, write.clone(), hooks.clone()));
    page.add(&capture_group(&settings, write.clone()));
    page.add(&wakatime_group(&settings, write.clone()));

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

/// Settings › Files: the names hidden in any folder, and showing them anyway.
fn files_group(
    settings: &Rc<RefCell<AppSettings>>,
    write: impl Fn(&str, Value) + Clone + 'static,
    hooks: Rc<Hooks>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Files")
        .description("Files with these names, in any folder, and Obsidian's excluded files are not in the file tree, quick open, search or the graph. A link to one still opens it.")
        .build();
    let file = &settings.borrow().file;
    let names = file.strings("files.hidden").unwrap_or_else(|| daftar_core::HIDDEN_NAMES.map(String::from).to_vec());
    let row = adw::EntryRow::builder().title("Hidden files, with commas between them").show_apply_button(true).build();
    row.set_text(&names.join(", "));
    let show = adw::SwitchRow::builder().title("Show hidden files").active(file.bool("files.show_hidden").unwrap_or(false)).build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        row.connect_apply(move |row| {
            let names: Vec<String> = row.text().split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
            write("files.hidden", Value::Strings(names));
            (hooks.hidden)();
        });
    }
    show.connect_active_notify(move |row| {
        write("files.show_hidden", Value::Bool(row.is_active()));
        (hooks.hidden)();
    });
    group.add(&row);
    group.add(&show);
    group
}

/// What `editor.nvim_config` may be, and how each reads.
const NVIM_CONFIGS: [(&str, &str); 2] = [("user", "Mine (init.lua)"), ("clean", "None (--clean)")];

/// Settings › Editor: Neovim mode, the nvim it runs and the config it loads,
/// as the Mac's Settings › Editor has them.
fn editor_group(
    settings: &Rc<RefCell<AppSettings>>,
    write: impl Fn(&str, Value) + Clone + 'static,
    hooks: Rc<Hooks>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Editor")
        .description("Your Neovim owns the buffer and writes the file; the editor shows it, with its colors and right-to-left text. Your config sees g:daftar = 1. Ctrl shortcuts stay with the app.")
        .build();
    let file = &settings.borrow().file;
    let live = adw::SwitchRow::builder()
        .title("Live preview")
        .subtitle("Away from the cursor's line the marks hide, so the note reads as it will look")
        .active(file.bool("editor.live_preview").unwrap_or(true))
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        live.connect_active_notify(move |row| {
            write("editor.live_preview", Value::Bool(row.is_active()));
            (hooks.editor)();
        });
    }
    group.add(&live);
    let vim = adw::SwitchRow::builder().title("Edit with Neovim").active(file.bool("editor.vim_mode").unwrap_or(false)).build();
    let path = adw::EntryRow::builder().title("nvim (empty finds it on $PATH)").show_apply_button(true).build();
    path.set_text(&file.string("editor.nvim").unwrap_or_default());
    let found = adw::ActionRow::builder().title("Runs").build();
    let show_found = {
        let found = found.clone();
        move |configured: &str| match daftar_core::NeovimClient::locate(configured) {
            Some(nvim) => {
                found.set_subtitle(&nvim.to_string_lossy());
                found.remove_css_class("error");
            }
            None => {
                found.set_subtitle("nvim is not found: pacman -S neovim");
                found.add_css_class("error");
            }
        }
    };
    show_found(&path.text());
    // A path of the person's own, as the file may name, is kept as a third choice.
    let configured = file.string("editor.nvim_config").unwrap_or_else(|| "user".into());
    let mut choices: Vec<(String, String)> = NVIM_CONFIGS.iter().map(|(key, name)| (key.to_string(), name.to_string())).collect();
    if !NVIM_CONFIGS.iter().any(|(key, _)| *key == configured) {
        choices.push((configured.clone(), configured.clone()));
    }
    let names: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
    let config = adw::ComboRow::builder()
        .title("Configuration")
        .model(&gtk::StringList::new(&names))
        .selected(choices.iter().position(|(key, _)| *key == configured).unwrap_or(0) as u32)
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        vim.connect_active_notify(move |row| {
            write("editor.vim_mode", Value::Bool(row.is_active()));
            (hooks.editor)();
        });
    }
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        path.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            show_found(&text);
            write("editor.nvim", Value::String(text));
            (hooks.editor)();
        });
    }
    config.connect_selected_notify(move |row| {
        let Some((key, _)) = choices.get(row.selected() as usize) else { return };
        write("editor.nvim_config", Value::String(key.clone()));
        (hooks.editor)();
    });
    group.add(&vim);
    group.add(&path);
    group.add(&found);
    group.add(&config);
    group
}

/// Settings › Notifications: a meeting before it starts, and the morning's
/// digest of the tasks due, as `[notifications]` in the file has them.
fn notifications_group(
    settings: &Rc<RefCell<AppSettings>>,
    write: impl Fn(&str, Value) + Clone + 'static,
    hooks: Rc<Hooks>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Notifications")
        .description("Meetings come from the week notes in Calendar/; the digest lists the tasks due and late.")
        .build();
    let file = settings.borrow().file.clone();
    let meetings = adw::SwitchRow::builder().title("Before a meeting").active(file.bool("notifications.meetings").unwrap_or(true)).build();
    let lead = adw::SpinRow::with_range(0.0, 120.0, 1.0);
    lead.set_title("Minutes before");
    lead.set_value(file.int("notifications.meeting_lead").unwrap_or(5).max(0) as f64);
    let digest = adw::SwitchRow::builder().title("Morning digest").active(file.bool("notifications.digest").unwrap_or(true)).build();
    let time = adw::EntryRow::builder().title("Digest at (HH:MM)").show_apply_button(true).build();
    time.set_text(&file.string("notifications.digest_time").unwrap_or_else(|| "09:00".into()));
    let replan = Rc::new(move || (hooks.notifications)());
    {
        let (write, replan, lead) = (write.clone(), replan.clone(), lead.clone());
        meetings.connect_active_notify(move |row| {
            lead.set_sensitive(row.is_active());
            write("notifications.meetings", Value::Bool(row.is_active()));
            replan();
        });
    }
    {
        let (write, replan) = (write.clone(), replan.clone());
        lead.connect_value_notify(move |row| {
            write("notifications.meeting_lead", Value::Int(row.value() as i64));
            replan();
        });
    }
    {
        let (write, replan, time) = (write.clone(), replan.clone(), time.clone());
        digest.connect_active_notify(move |row| {
            time.set_sensitive(row.is_active());
            write("notifications.digest", Value::Bool(row.is_active()));
            replan();
        });
    }
    time.connect_apply(move |row| {
        let text = row.text().trim().to_string();
        if chrono::NaiveTime::parse_from_str(&text, "%H:%M").is_err() {
            row.add_css_class("error");
            return;
        }
        row.remove_css_class("error");
        write("notifications.digest_time", Value::String(text));
        replan();
    });
    lead.set_sensitive(meetings.is_active());
    time.set_sensitive(digest.is_active());
    group.add(&meetings);
    group.add(&lead);
    group.add(&digest);
    group.add(&time);
    group
}

/// Settings › Quick Capture: the note a capture goes to, and the heading.
fn capture_group(settings: &Rc<RefCell<AppSettings>>, write: impl Fn(&str, Value) + Clone + 'static) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Quick Capture")
        .description(format!(
            "daftar --capture opens it from any app: bind it in the compositor, and float the window titled “{}”. “daily” is today's daily note; a path such as Inbox.md names one note. Return adds the line, Ctrl+Return adds it as a task.",
            crate::capture::TITLE
        ))
        .build();
    for (key, title, default) in [("capture.target", "Note", "daily"), ("capture.heading", "Under the heading (empty is the end)", "")] {
        let row = adw::EntryRow::builder().title(title).show_apply_button(true).build();
        row.set_text(&settings.borrow().file.string(key).unwrap_or_else(|| default.into()));
        let write = write.clone();
        row.connect_apply(move |row| write(key, Value::String(row.text().trim().to_string())));
        group.add(&row);
    }
    group
}

/// Settings › WakaTime: off at first, as on the Mac.
fn wakatime_group(settings: &Rc<RefCell<AppSettings>>, write: impl Fn(&str, Value) + Clone + 'static) -> adw::PreferencesGroup {
    let configured = settings.borrow().file.string("wakatime.cli").unwrap_or_default();
    let description = match crate::wakatime::cli(&configured) {
        Some(_) => "Uses wakatime-cli and the key in ~/.wakatime.cfg. The note's path and the vault's name leave this computer.",
        None => {
            "wakatime-cli is not installed: yay -S wakatime, or install a WakaTime editor plugin. The key is read from ~/.wakatime.cfg."
        }
    };
    let group = adw::PreferencesGroup::builder().title("WakaTime").description(description).build();
    let send = adw::SwitchRow::builder()
        .title("Send reading and writing time to WakaTime")
        .active(settings.borrow().file.bool("wakatime.enabled").unwrap_or(false))
        .build();
    send.connect_active_notify(move |row| write("wakatime.enabled", Value::Bool(row.is_active())));
    group.add(&send);
    group
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
