//! Settings (Ctrl+,): the keys of `~/.config/daftar/config.toml` this app
//! has, each written back on its own line as it changes, so the file keeps
//! the person's comments and the keys only the Mac has.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::{FoldSections, Git, Value};

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
    /// The vault was made read-only, or not any more.
    pub read_only: Box<dyn Fn()>,
    /// Switch Vault: the change to the vault at a folder, in the running app.
    pub switch_vault: Box<dyn Fn(PathBuf)>,
    /// Forget Vault, after a question: the folder as the list writes it, and its name.
    pub forget_vault: Forget,
}

/// Forget Vault: the folder as the list writes it, and the vault's name.
pub type Forget = Box<dyn Fn(&str, &str)>;

/// What a setting the vault holds says in place of its control.
const FROM_VAULT: &str = "From the vault";
const FROM_VAULT_TIP: &str = "The vault's .daftar.toml sets this, and the vault cannot be written here (a read-only clone), so it is not changed. To keep your own, name the key in vault_settings in config.toml, or set vault_settings = false.";

/// A key a vault may hold: when the vault sets it, the row says "From the
/// vault", and its control works where the vault can be written (the
/// change goes to `.daftar.toml`); in a read-only clone it is off. The
/// row's context menu has Keep on This Device, which declines the key in
/// `vault_settings`, and Follow the Vault, which takes it off that list
/// (`docs/specs/vault-config.md`, sections 4 and 5).
fn from_vault(settings: &Rc<RefCell<AppSettings>>, key: &'static str, row: &impl IsA<adw::PreferencesRow>) {
    let row: adw::PreferencesRow = row.as_ref().clone();
    let action = row.clone().downcast::<adw::ActionRow>().ok();
    // What the row said of itself, before the mark.
    let own = action.as_ref().map_or_else(|| row.title().to_string(), |row| row.subtitle().unwrap_or_default().to_string());
    let mark = {
        let (settings, row) = (settings.clone(), row.clone());
        Rc::new(move || {
            let settings = settings.borrow();
            let state = if settings.is_kept_on_device(key) {
                "Kept on this device"
            } else if settings.is_from_vault(key) {
                FROM_VAULT
            } else {
                ""
            };
            let said = match (state.is_empty(), own.is_empty()) {
                (true, _) => own.clone(),
                (false, true) => state.to_string(),
                (false, false) => format!("{state} · {own}"),
            };
            match &action {
                Some(row) => row.set_subtitle(&said),
                // An entry has no second line: its name says it.
                None => row.set_title(&said),
            }
            // Off where the vault sets it and cannot be written; what else turns a row off stays.
            if !settings.can_change(key) {
                row.set_sensitive(false);
                row.set_tooltip_text(Some(FROM_VAULT_TIP));
            }
        })
    };
    mark();
    let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
    let settings = settings.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(row) = gesture.widget() else { return };
        // A key the vault sets in a clone that cannot be written is the vault's: nothing to choose.
        let kept = settings.borrow().is_kept_on_device(key);
        let button =
            gtk::Button::builder().label(if kept { "Follow the Vault" } else { "Keep on This Device" }).css_classes(["flat"]).build();
        let popover = gtk::Popover::builder().child(&button).has_arrow(true).build();
        popover.set_parent(&row);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        {
            let (settings, mark, popover) = (settings.clone(), mark.clone(), popover.clone());
            button.connect_clicked(move |_| {
                // The window follows `config.toml`, and draws with the value now in use.
                let changed = if kept { settings.borrow_mut().follow_vault(key) } else { settings.borrow_mut().keep_on_device(key) };
                if changed.is_ok() {
                    mark();
                    if settings.borrow().can_change(key) {
                        row.set_sensitive(true);
                        row.set_tooltip_text(None);
                    }
                }
                popover.popdown();
            });
        }
        popover.connect_closed(WidgetExt::unparent);
        popover.popup();
    });
    row.add_controller(click);
}

const THEMES: [(&str, &str); 3] = [("system", "Follow the System"), ("light", "Daylight"), ("dark", "Naz")];

/// `calendar.place`, as `daftar_core::Place` reads it.
const PLACES: [(&str, &str); 4] = [("", "None"), ("spain", "Spain"), ("catalonia", "Catalonia"), ("barcelona", "Barcelona")];

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
    from_vault(&settings, "appearance.theme", &theme);
    let soft = adw::SwitchRow::builder()
        .title("Soft dark palette")
        .subtitle("naz with its colour strength capped, for long sessions")
        .active(settings.borrow().soft_palette())
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        soft.connect_active_notify(move |row| {
            write("appearance.dark_palette", Value::String(if row.is_active() { "naz-soft" } else { "naz" }.into()));
            (hooks.reader)();
            (hooks.editor)();
        });
    }
    looks.add(&soft);
    from_vault(&settings, "appearance.dark_palette", &soft);
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
    from_vault(&settings, "appearance.text_size", &size);
    let band = adw::SpinRow::with_range(0.0, 100.0, 1.0);
    band.set_title("Reading line");
    band.set_subtitle("The band on the line being read in a long paragraph, in percent; 0 is none");
    band.set_value(settings.borrow().reading_line() as f64);
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        band.connect_value_notify(move |row| {
            write("appearance.reading_line", Value::Int(row.value() as i64));
            (hooks.reader)();
        });
    }
    looks.add(&band);
    from_vault(&settings, "appearance.reading_line", &band);
    let preview = adw::SwitchRow::builder()
        .title("Link preview")
        .subtitle("Resting the pointer on a link in the editor shows the note's first lines")
        .active(settings.borrow().link_preview())
        .build();
    {
        let write = write.clone();
        preview.connect_active_notify(move |row| write("appearance.link_preview", Value::Bool(row.is_active())));
    }
    looks.add(&preview);
    from_vault(&settings, "appearance.link_preview", &preview);
    for (key, title) in [("appearance.font", "Font"), ("appearance.code_font", "Code font")] {
        let row = adw::EntryRow::builder().title(title).show_apply_button(true).build();
        row.set_text(&settings.borrow().file.string(key).unwrap_or_default());
        let (write, hooks) = (write.clone(), hooks.clone());
        row.connect_apply(move |row| {
            write(key, Value::String(row.text().trim().to_string()));
            (hooks.reader)();
        });
        looks.add(&row);
        from_vault(&settings, key, &row);
    }
    let persian = adw::SwitchRow::builder()
        .title("Persian calendar")
        .subtitle("The Solar Hijri date beside the Gregorian one, and Jalali days typed in a task's dates")
        .active(settings.borrow().file.bool("appearance.persian_calendar").unwrap_or(false))
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        persian.connect_active_notify(move |row| {
            write("appearance.persian_calendar", Value::Bool(row.is_active()));
            (hooks.home)();
            (hooks.reader)();
        });
    }
    looks.add(&persian);
    from_vault(&settings, "appearance.persian_calendar", &persian);
    let remote = adw::SwitchRow::builder()
        .title("Pictures from the web")
        .subtitle("Off asks no site for a picture until you click it, so opening a note tells no one")
        .active(settings.borrow().remote_images())
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        remote.connect_active_notify(move |row| {
            write("appearance.remote_images", Value::Bool(row.is_active()));
            (hooks.reader)();
        });
    }
    looks.add(&remote);
    from_vault(&settings, "appearance.remote_images", &remote);
    let folds = adw::ComboRow::builder()
        .title("Notes open folded")
        .subtitle("Each heading says what is under it; a note you fold or open yourself stays as you left it")
        .model(&gtk::StringList::new(&["As the vault says", "Always", "Never"]))
        .selected(FoldSections::ALL.iter().position(|choice| *choice == settings.borrow().fold_sections()).unwrap_or(0) as u32)
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        folds.connect_selected_notify(move |row| {
            write("appearance.fold_sections", Value::String(FoldSections::ALL[row.selected() as usize].name().into()));
            (hooks.reader)();
        });
    }
    looks.add(&folds);
    from_vault(&settings, "appearance.fold_sections", &folds);
    let place = adw::ComboRow::builder()
        .title("Holidays")
        .subtitle("The public holidays and feasts of where you live, on Today, home and the day picker")
        .model(&gtk::StringList::new(&PLACES.map(|(_, name)| name)))
        .selected(
            PLACES
                .iter()
                .position(|(key, _)| *key == settings.borrow().file.string("calendar.place").unwrap_or_default().trim().to_lowercase())
                .unwrap_or(0) as u32,
        )
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        place.connect_selected_notify(move |row| {
            write("calendar.place", Value::String(PLACES[row.selected() as usize].0.into()));
            (hooks.home)();
        });
    }
    looks.add(&place);
    from_vault(&settings, "calendar.place", &place);
    page.add(&looks);
    page.add(&home_group(&settings, root.clone(), write.clone(), hooks.clone()));
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
    from_vault(&settings, "git.pull_on_launch", &pull);
    from_vault(&settings, "git.sync_interval", &interval);
    page.add(&git);
    // Settings › Vault: the vaults this computer knows, above the Git group of the one in use.
    page.add(&vaults_group(&settings, &dialog, &hooks));
    if let Some(group) = root.as_deref().and_then(|root| vault_git_group(root, &hooks)) {
        page.add(&group);
    }
    page.add(&files_group(&settings, write.clone(), hooks.clone()));
    page.add(&notifications_group(&settings, write.clone(), hooks));
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
    from_vault(settings, "files.hidden", &row);
    from_vault(settings, "files.show_hidden", &show);
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
    from_vault(settings, "editor.live_preview", &live);
    let bar = adw::SwitchRow::builder()
        .title("Format bar")
        .subtitle("Bold, italic, highlight, code, strike and link, over the selected text")
        .active(file.bool("editor.format_bar").unwrap_or(true))
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        bar.connect_active_notify(move |row| {
            write("editor.format_bar", Value::Bool(row.is_active()));
            (hooks.editor)();
        });
    }
    group.add(&bar);
    from_vault(settings, "editor.format_bar", &bar);
    let slash = adw::SwitchRow::builder()
        .title("Slash bar")
        .subtitle("A / at the start of a line or after a space lists things to insert, filtered as you type")
        .active(settings.borrow().slash_menu())
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        slash.connect_active_notify(move |row| {
            write("editor.slash_menu", Value::Bool(row.is_active()));
            (hooks.editor)();
        });
    }
    group.add(&slash);
    from_vault(settings, "editor.slash_menu", &slash);
    let focus = adw::SwitchRow::builder()
        .title("Focus mode")
        .subtitle("Every paragraph but the cursor's is dimmed")
        .active(settings.borrow().focus())
        .build();
    {
        let (write, hooks) = (write.clone(), hooks.clone());
        focus.connect_active_notify(move |row| {
            write("editor.focus", Value::Bool(row.is_active()));
            (hooks.editor)();
        });
    }
    group.add(&focus);
    from_vault(settings, "editor.focus", &focus);
    let format = adw::SwitchRow::builder()
        .title("Format on leaving")
        .subtitle("A note you typed in is formatted with daftar-cli fmt when you leave the editor, never while you type")
        .active(file.bool("editor.format_on_save").unwrap_or(true))
        .build();
    {
        let write = write.clone();
        format.connect_active_notify(move |row| write("editor.format_on_save", Value::Bool(row.is_active())));
    }
    group.add(&format);
    from_vault(settings, "editor.format_on_save", &format);
    let neovide = adw::SwitchRow::builder()
        .title("Edit in Neovide")
        .subtitle("Ctrl+E opens the note in Neovide's own window; the page follows what it writes")
        .active(file.bool("editor.neovide").unwrap_or(false))
        .build();
    {
        let write = write.clone();
        neovide.connect_active_notify(move |row| write("editor.neovide", Value::Bool(row.is_active())));
    }
    group.add(&neovide);
    let vim = adw::SwitchRow::builder().title("Edit with Neovim").active(file.bool("editor.vim_mode").unwrap_or(false)).build();
    let path = adw::EntryRow::builder().title("nvim (empty finds it on $PATH)").show_apply_button(true).build();
    path.set_text(&file.string("editor.nvim").unwrap_or_default());
    let found = adw::ActionRow::builder().title("Runs").build();
    let show_found = {
        let found = found.clone();
        move |configured: &str| {
            if let Some(nvim) = daftar_core::NeovimClient::locate(configured) {
                found.set_subtitle(&nvim.to_string_lossy());
                found.remove_css_class("error");
            } else {
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
    from_vault(settings, "notifications.meetings", &meetings);
    from_vault(settings, "notifications.meeting_lead", &lead);
    from_vault(settings, "notifications.digest", &digest);
    from_vault(settings, "notifications.digest_time", &time);
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
        from_vault(settings, key, &row);
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
        let (row, preview, clear, root) = (row.clone(), preview, clear.clone(), root.clone());
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
            preview.set_paintable(None::<&gtk::gdk::Paintable>);
            if let Some(path) = path.filter(|p| p.is_file()) {
                let preview = preview.downgrade();
                gtk::glib::spawn_future_local(async move {
                    let texture = crate::home::texture(path).await;
                    if let Some(preview) = preview.upgrade() {
                        preview.set_paintable(texture.as_ref());
                    }
                });
            }
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
    if settings.borrow().is_from_vault("home.image") {
        row.set_subtitle(&gtk::glib::markup_escape_text(&format!("{} · {FROM_VAULT}", row.subtitle().unwrap_or_default())));
        if !settings.borrow().can_change("home.image") {
            row.set_sensitive(false);
            row.set_tooltip_text(Some(FROM_VAULT_TIP));
        }
    }
    group
}

/// Settings › Vault: every vault of the list with Switch and Forget. The
/// vault in use cannot be forgotten, and a missing one is not opened.
fn vaults_group(settings: &Rc<RefCell<AppSettings>>, dialog: &adw::PreferencesDialog, hooks: &Rc<Hooks>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Vault")
        .description("The vaults this computer knows. Switch changes to one at once, with no restart; Open Vault… (Ctrl+Shift+O) adds one. Forget takes a vault out of this list and touches no file.")
        .build();
    for entry in settings.borrow().vaults(&gtk::glib::home_dir()).entries() {
        let state = if entry.is_missing() {
            " · missing"
        } else if entry.in_use {
            " · in use"
        } else {
            ""
        };
        let row = adw::ActionRow::builder().title(&entry.name).subtitle(format!("{}{state}", entry.text)).use_markup(false).build();
        let switch =
            gtk::Button::builder().label("Switch").valign(gtk::Align::Center).sensitive(!entry.in_use && !entry.is_missing()).build();
        let forget = gtk::Button::builder().label("Forget").valign(gtk::Align::Center).sensitive(!entry.in_use).build();
        {
            let (hooks, dialog, path) = (hooks.clone(), dialog.clone(), entry.path.clone());
            switch.connect_clicked(move |_| {
                // The page is the old vault's: it closes, and the vault changes.
                dialog.close();
                (hooks.switch_vault)(path.clone());
            });
        }
        {
            let (hooks, dialog, folder, name) = (hooks.clone(), dialog.clone(), entry.text.clone(), entry.name);
            forget.connect_clicked(move |_| {
                dialog.close();
                (hooks.forget_vault)(&folder, &name);
            });
        }
        row.add_suffix(&switch);
        row.add_suffix(&forget);
        group.add(&row);
    }
    group
}

/// Settings › Vault › Git, for the vault that is open, only when it is a
/// repository: what this clone is, in its own git config, so it is never
/// committed and a shell reads the same (docs/specs/read-only-vault.md).
/// Each field writes its key with `git config --local`; emptied, it unsets it.
fn vault_git_group(root: &Path, hooks: &Rc<Hooks>) -> Option<adw::PreferencesGroup> {
    let git = Rc::new(Git::new(root));
    if !git.is_repository() {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title("Vault · Git")
        .description("This clone's own settings, kept in its git config and never committed: a clone can be read-only, and carry its own address, author and SSH key, apart from this computer's.")
        .build();
    let failed = {
        let hooks = hooks.clone();
        move |what: &str, failure: &dyn std::fmt::Display| (hooks.notice)(&format!("{what} was not saved: {failure}"))
    };

    let read_only = adw::SwitchRow::builder()
        .title("Read-only")
        .subtitle("Daftar pulls and never writes: no edit, no commit, no push. Changes this clone already has stay as they are.")
        .active(git.is_read_only())
        .build();
    {
        let (git, hooks, failed) = (git.clone(), hooks.clone(), failed.clone());
        read_only.connect_active_notify(move |row| {
            if let Err(failure) = git.set_read_only(row.is_active()) {
                failed("Read-only", &failure);
            }
            (hooks.read_only)();
        });
    }
    group.add(&read_only);

    let address = adw::EntryRow::builder().title("Address").show_apply_button(true).build();
    address.set_text(&git.address().unwrap_or_default());
    address.set_tooltip_text(Some(
        "Where this clone pulls from and pushes to. Nothing is fetched until the next sync, which says so when the new address does not answer or holds another history.",
    ));
    {
        let (git, hooks, failed) = (git.clone(), hooks.clone(), failed.clone());
        address.connect_apply(move |row| {
            let text = row.text().trim().to_string();
            if text.is_empty() {
                // An empty address is refused, and the old one stays.
                row.set_text(&git.address().unwrap_or_default());
                (hooks.notice)("The address cannot be empty. The old one is kept.");
                return;
            }
            match git.set_address(&text) {
                Ok(()) => (hooks.notice)("The address is set. The next sync uses it."),
                Err(failure) => failed("The address", &failure),
            }
        });
    }
    group.add(&address);

    for (key, title) in [("user.name", "Name"), ("user.email", "Email")] {
        let row = adw::EntryRow::builder().title(title).show_apply_button(true).build();
        row.set_text(&git.local(key).unwrap_or_default());
        row.set_tooltip_text(Some(
            "The author of this clone's commits. Empty is the vault's git.name and git.email, else this computer's.",
        ));
        let (git, failed) = (git.clone(), failed.clone());
        row.connect_apply(move |row| {
            if let Err(failure) = git.set_local(key, Some(row.text().as_str())) {
                failed(title, &failure);
            }
        });
        group.add(&row);
    }

    let key = adw::ActionRow::builder().title("SSH key").build();
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    let generate = gtk::Button::builder()
        .label("Generate")
        .valign(gtk::Align::Center)
        .tooltip_text("A new key for this clone in ~/.ssh, with no passphrase; give its public key to the remote")
        .build();
    let clear = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).build();
    key.add_suffix(&choose);
    key.add_suffix(&generate);
    key.add_suffix(&clear);
    // A command the app did not write is shown as text, and not changed:
    // Choose, Generate and Clear are off for it.
    let show = {
        let (git, key, clear, choose, generate) = (git.clone(), key.clone(), clear.clone(), choose.clone(), generate.clone());
        Rc::new(move || {
            let command = git.local("core.sshCommand");
            let text = match &command {
                None => "None: this computer's own keys".to_string(),
                Some(command) => Git::ssh_key(command).map_or_else(|| command.clone(), |file| file.to_string_lossy().into_owned()),
            };
            key.set_subtitle(&gtk::glib::markup_escape_text(&text));
            let ours = command.as_deref().is_none_or(|command| Git::ssh_key(command).is_some());
            clear.set_sensitive(command.is_some() && ours);
            choose.set_sensitive(ours);
            generate.set_sensitive(ours);
        })
    };
    show();
    let set = {
        let (git, show, failed) = (git.clone(), show, failed.clone());
        Rc::new(move |file: Option<&Path>| {
            let command = file.map(Git::ssh_command_for_key);
            if let Err(failure) = git.set_local("core.sshCommand", command.as_deref()) {
                failed("The SSH key", &failure);
            }
            show();
        })
    };
    {
        let set = set.clone();
        clear.connect_clicked(move |_| set(None));
    }
    {
        let set = set.clone();
        choose.connect_clicked(move |button| {
            let dialog = gtk::FileDialog::builder().title("Choose a Private Key").modal(true).build();
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(gtk::glib::home_dir().join(".ssh"))));
            let set = set.clone();
            dialog.open(button.root().and_downcast::<gtk::Window>().as_ref(), gtk::gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|file| file.path()) {
                    set(Some(&path));
                }
            });
        });
    }
    {
        let hooks = hooks.clone();
        generate.connect_clicked(move |button| match git.generate_key(&gtk::glib::home_dir().join(".ssh"), &Git::host()) {
            Ok((file, public)) => {
                set(Some(&file));
                public_key(button, &file, &public);
            }
            Err(why) => (hooks.notice)(&format!("No key was made: {why}")),
        });
    }
    group.add(&key);
    Some(group)
}

/// The public key of a key just made, to give the remote, with Copy.
fn public_key(over: &impl IsA<gtk::Widget>, file: &Path, public: &str) {
    let alert = adw::AlertDialog::builder()
        .heading("The Key Is Made")
        .body(format!(
            "{} is this clone's key now. Give the remote its public key, as a deploy key; without write access, the remote refuses this clone's pushes.",
            file.display()
        ))
        .build();
    let text = gtk::Label::builder()
        .label(public)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::Char)
        .css_classes(["monospace"])
        .build();
    alert.set_extra_child(Some(&text));
    alert.add_responses(&[("close", "Close"), ("copy", "Copy")]);
    alert.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("copy"));
    let public = public.to_string();
    alert.connect_response(Some("copy"), move |alert, _| alert.clipboard().set_text(&public));
    alert.present(Some(over));
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
