//! The app's settings, all in `~/.config/daftar/config.toml`, with what
//! the open vault's `.daftar.toml` may set on top (`docs/specs/vault-config.md`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use daftar_core::jalali::Jalali;
use daftar_core::{Calendar, CalendarEvent, CalendarEvents, ConfigFile, Day, Happening, Place, Settings, Value};

/// `appearance.persian_calendar`, for the pages that show a date and do not
/// hold the settings: the home page, Today, a daily note's subtitle.
static PERSIAN_CALENDAR: AtomicBool = AtomicBool::new(false);

/// [day] in the Solar Hijri calendar, as Persian says it, when the settings
/// want it beside the Gregorian date.
pub fn persian_date(day: Day) -> Option<String> {
    PERSIAN_CALENDAR.load(Ordering::Relaxed).then(|| Jalali::long(day))
}

/// `calendar.place`: 0 is none, else a [Place] and one.
static PLACE: AtomicU8 = AtomicU8::new(0);

const PLACES: [Place; 3] = [Place::Spain, Place::Catalonia, Place::Barcelona];

/// The calendar the day picker shows: the Jalali one with the Persian date.
pub fn calendar() -> Calendar {
    if PERSIAN_CALENDAR.load(Ordering::Relaxed) { Calendar::Jalali } else { Calendar::Gregorian }
}

fn place() -> Option<Place> {
    PLACES.get(usize::from(PLACE.load(Ordering::Relaxed)).checked_sub(1)?).copied()
}

/// The holidays and feasts of `calendar.place` from [from] to [to].
pub fn events(from: Day, to: Day) -> Vec<Happening> {
    place().map_or_default(|place| CalendarEvents::between(place, from, to))
}

/// The holidays and feasts of `calendar.place` on [day].
pub fn events_on(day: Day) -> Vec<&'static CalendarEvent> {
    place().map_or_default(|place| CalendarEvents::on(place, day))
}

pub struct AppSettings {
    pub path: PathBuf,
    /// The settings in use: the computer's file, with the values the open
    /// vault sets in it. Every page reads this one.
    pub file: ConfigFile,
    /// The two files apart: which keys the vault set, and the computer's
    /// own text, which a change in the app is written to.
    resolved: Settings,
    /// The open vault, whose `.daftar.toml` is read, and written when a
    /// setting changes in the app.
    vault: Option<PathBuf>,
    /// Told after a setting was written to the vault's `.daftar.toml`, for
    /// the sync that follows an edit.
    pub on_vault_written: Option<std::rc::Rc<dyn Fn()>>,
}

impl AppSettings {
    /// The person's file, made from the template on the first start.
    pub fn load() -> AppSettings {
        let path = ConfigFile::default_path();
        let text = if let Ok(text) = fs::read_to_string(&path) {
            text
        } else {
            if let Some(folder) = path.parent() {
                let _ = fs::create_dir_all(folder);
            }
            let _ = fs::write(&path, ConfigFile::TEMPLATE);
            ConfigFile::TEMPLATE.to_string()
        };
        AppSettings::of(path, &text)
    }

    /// The settings of [computer], the text of the file at [path], with no
    /// vault open yet.
    pub fn of(path: PathBuf, computer: &str) -> AppSettings {
        let resolved = Settings::resolved("", computer);
        let settings = AppSettings { path, file: resolved.file().clone(), resolved, vault: None, on_vault_written: None };
        settings.publish();
        settings
    }

    /// The settings again from [computer] and the open vault's file; true
    /// when a setting in use changed.
    fn resolve(&mut self, computer: &str) -> bool {
        self.resolved = match &self.vault {
            Some(root) => Settings::of(root, computer),
            None => Settings::resolved("", computer),
        };
        if *self.resolved.file() == self.file {
            return false;
        }
        self.file = self.resolved.file().clone();
        self.publish();
        true
    }

    /// The vault that is open, whose `.daftar.toml` may set the app's
    /// settings; true when a setting in use changed with it.
    pub fn set_vault(&mut self, root: Option<&Path>) -> bool {
        self.vault = root.map(Path::to_path_buf);
        let computer = self.resolved.computer().text().to_string();
        self.resolve(&computer)
    }

    /// Whether [key] has the vault's value: its control is off, and a
    /// change to it would be the computer's, which the vault's wins over.
    pub fn is_from_vault(&self, key: &str) -> bool {
        self.resolved.is_from_vault(key)
    }

    /// The files read again when one changed on disk: the computer's, by
    /// hand, by the Mac's app through a dotfiles pull or by another window,
    /// or the vault's, by a pull or an edit; true when a setting in use is
    /// not the one this window holds.
    pub fn reload(&mut self) -> bool {
        let computer = fs::read_to_string(&self.path).unwrap_or_else(|_| self.resolved.computer().text().to_string());
        self.resolve(&computer)
    }

    /// The settings the pages read without holding them.
    fn publish(&self) {
        PERSIAN_CALENDAR.store(self.file.bool("appearance.persian_calendar").unwrap_or(false), Ordering::Relaxed);
        let place = Place::parse(&self.file.string("calendar.place").unwrap_or_default());
        PLACE.store(place.and_then(|p| PLACES.iter().position(|q| *q == p)).map_or(0, |i| i as u8 + 1), Ordering::Relaxed);
    }

    /// A setting changed in the app, written where the spec says
    /// (`docs/specs/vault-config.md`, section 5): to the vault's
    /// `.daftar.toml`, in its shared section or in `[linux]`, so the change
    /// reaches the other devices with the next sync; or to the computer's
    /// `config.toml`, for what a repository may never hold, what this
    /// device keeps, and every change while the vault cannot be written.
    /// Both files are read again first, so a change another window, the
    /// Mac's app or a pull made is kept, and only the key's line changes.
    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        let computer = self.computer_now();
        self.resolve(&computer);
        let writable = self.vault_writable();
        let target = self.resolved.set(key, value, writable);
        self.file = self.resolved.file().clone();
        self.publish();
        match target {
            daftar_core::SettingTarget::Device => self.write_computer(self.resolved.computer().text()),
            daftar_core::SettingTarget::Platform | daftar_core::SettingTarget::Shared => self.write_vault(self.resolved.vault_text()),
        }
    }

    /// Whether the app may write the open vault's `.daftar.toml`: not in a
    /// read-only clone, and not in a folder with no such file that is no
    /// repository.
    pub fn vault_writable(&self) -> bool {
        self.vault.as_deref().is_some_and(|root| {
            !daftar_core::Git::read_only(root) && (root.join(".daftar.toml").is_file() || daftar_core::Git::new(root).is_repository())
        })
    }

    /// Whether the control of [key] works: a key the vault sets can be
    /// changed only where the vault can be written.
    pub fn can_change(&self, key: &str) -> bool {
        !self.is_from_vault(key) || self.vault_writable()
    }

    /// Whether this device keeps [key] for itself (`vault_settings`).
    pub fn is_kept_on_device(&self, key: &str) -> bool {
        self.resolved.is_kept_on_device(key)
    }

    /// Keep on This Device: [key] is declined in `vault_settings`, with its
    /// value in use written to `config.toml`; the vault's line stays.
    pub fn keep_on_device(&mut self, key: &str) -> io::Result<()> {
        let computer = self.computer_now();
        self.resolve(&computer);
        self.resolved.keep_on_device(key);
        self.file = self.resolved.file().clone();
        self.publish();
        self.write_computer(self.resolved.computer().text())
    }

    /// Follow the Vault: [key] is taken off the keys this device declines.
    pub fn follow_vault(&mut self, key: &str) -> io::Result<()> {
        let computer = self.computer_now();
        self.resolve(&computer);
        self.resolved.follow_vault(key);
        self.file = self.resolved.file().clone();
        self.publish();
        self.write_computer(self.resolved.computer().text())
    }

    /// The vault's `.daftar.toml` written whole through the vault's
    /// ordinary write, which a read-only clone refuses; the sync that
    /// follows an edit hears of it.
    fn write_vault(&self, text: &str) -> io::Result<()> {
        let Some(root) = self.vault.as_deref() else { return Ok(()) };
        let failed = |why: String| io::Error::other(why);
        if root.join(".daftar.toml").is_file() {
            daftar_core::VaultEdits::apply(".daftar.toml", root, |_| Ok(text.to_string())).map_err(|e| failed(e.to_string()))?;
        } else {
            daftar_core::VaultEdits::create(".daftar.toml", root, text)?;
        }
        if let Some(written) = &self.on_vault_written {
            written();
        }
        Ok(())
    }

    /// The computer's file written whole, through a temporary file; a
    /// linked file is written where it points, and stays a link.
    fn write_computer(&self, text: &str) -> io::Result<()> {
        let target = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        let temporary = target.with_extension("toml.daftar");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, &target).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    }

    /// The computer's file as it is on disk now.
    fn computer_now(&self) -> String {
        fs::read_to_string(&self.path).unwrap_or_else(|_| self.resolved.computer().text().to_string())
    }

    /// The vaults this computer knows (`docs/specs/several-vaults.md`),
    /// for a person whose home folder is [home].
    pub fn vaults(&self, home: &Path) -> daftar_core::Vaults {
        daftar_core::Vaults::new(&self.computer_now(), home)
    }

    /// The vault at [root] was opened: it is the vault in use, and it is
    /// in the list, at the end, once. Nothing is written when the file
    /// says so already.
    pub fn record_vault(&mut self, root: &Path, home: &Path) -> io::Result<()> {
        let before = self.computer_now();
        let mut vaults = daftar_core::Vaults::new(&before, home);
        vaults.open(&root.to_string_lossy());
        if vaults.text() == before {
            return Ok(());
        }
        let text = vaults.text().to_string();
        self.resolve(&text);
        self.write_computer(&text)
    }

    /// Forget Vault: [folder] is taken out of the list; no file of it is
    /// touched. The vault in use cannot be forgotten.
    pub fn forget_vault(&mut self, folder: &str, home: &Path) -> Result<(), String> {
        let mut vaults = daftar_core::Vaults::new(&self.computer_now(), home);
        vaults.forget(folder).map_err(|refused| refused.to_string())?;
        let text = vaults.text().to_string();
        self.resolve(&text);
        self.write_computer(&text).map_err(|e| format!("the settings were not saved: {e}"))
    }

    /// The vault's folder, with `~` for the home folder.
    pub fn vault(&self) -> Option<PathBuf> {
        self.file.vault()
    }

    pub fn theme(&self) -> String {
        self.file.string("appearance.theme").unwrap_or_else(|| "system".into())
    }

    pub fn text_size(&self) -> i64 {
        self.file.int("appearance.text_size").unwrap_or(16)
    }

    /// The dark theme's palette is naz with its colour strength capped
    /// (`appearance.dark_palette = "naz-soft"`, docs/specs/look.md).
    pub fn soft_palette(&self) -> bool {
        self.file.string("appearance.dark_palette").as_deref() == Some("naz-soft")
    }

    /// The format bar shows over a selection in the editor (`editor.format_bar`).
    pub fn format_bar(&self) -> bool {
        self.file.bool("editor.format_bar").unwrap_or(true)
    }

    /// The reader's widest line, in em of the text size; 0 is the whole window.
    pub fn text_width(&self) -> i64 {
        self.file.int("appearance.text_width").unwrap_or(35).max(0)
    }

    /// The reading cursor's band on the line being read, in percent of the accent; 0 is none.
    pub fn reading_line(&self) -> i64 {
        self.file.int("appearance.reading_line").unwrap_or(18).clamp(0, 100)
    }

    /// Pictures from the web in the reader; `false` leaves each one a
    /// placeholder until it is clicked (docs/specs/remote-images.md).
    pub fn remote_images(&self) -> bool {
        self.file.bool("appearance.remote_images").unwrap_or(true)
    }

    /// Whether a note opens with its sections folded: as the vault says,
    /// always, or never (docs/specs/folded-sections.md).
    pub fn fold_sections(&self) -> daftar_core::FoldSections {
        daftar_core::FoldSections::named(&self.file.string("appearance.fold_sections").unwrap_or_default())
    }

    /// The reader's font families as CSS, Persian always cascading to Vazirmatn.
    pub fn font_css(&self) -> (String, String) {
        let font = self.file.string("appearance.font").unwrap_or_default();
        let text = match font.to_lowercase().as_str() {
            // The macOS names, from a config file kept in dotfiles for both apps.
            "" | "system" | "sf pro" | "-apple-system" => "system-ui".to_string(),
            "serif" | "new york" | "ui-serif" => "serif".to_string(),
            _ => format!("\"{font}\", system-ui"),
        };
        let code = self.file.string("appearance.code_font").unwrap_or_default();
        let mono = match code.to_lowercase().as_str() {
            "" | "sf mono" => "monospace".to_string(),
            _ => format!("\"{code}\", monospace"),
        };
        (format!("{text}, \"Persian Fallback\", sans-serif"), format!("{mono}, \"Persian Fallback\", monospace"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_setting_keeps_what_changed_on_disk_and_the_link() {
        let folder = tempfile::tempdir().unwrap();
        let real = folder.path().join("dotfiles.toml");
        fs::write(&real, "[appearance]\ntheme = \"system\"\ntext_size = 14\n").unwrap();
        let path = folder.path().join("config.toml");
        std::os::unix::fs::symlink(&real, &path).unwrap();
        let mut settings = AppSettings::of(path.clone(), &fs::read_to_string(&path).unwrap());
        // Another window, or the Mac's app through a dotfiles pull.
        fs::write(&real, "[appearance]\ntheme = \"dark\"\ntext_size = 14\n").unwrap();
        settings.set("appearance.text_size", Value::Int(16)).unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "[appearance]\ntheme = \"dark\"\ntext_size = 16\n");
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    }

    /// A home with a `config.toml`, and a vault with a `.daftar.toml` of its own.
    fn with_vault(theirs: &str, mine: &str) -> (tempfile::TempDir, PathBuf, AppSettings) {
        let folder = tempfile::tempdir().unwrap();
        let vault = folder.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join(".daftar.toml"), theirs).unwrap();
        let path = folder.path().join("config.toml");
        fs::write(&path, mine).unwrap();
        let mut settings = AppSettings::of(path, mine);
        settings.set_vault(Some(&vault));
        (folder, vault, settings)
    }

    #[test]
    fn a_setting_changed_in_the_app_goes_to_the_vault() {
        let mine = "[appearance]\ntheme = \"light\"\ntext_size = 14\n";
        let (_folder, vault, mut settings) = with_vault("# mine\n[appearance]\ntheme = \"dark\" # night\n", mine);
        let theirs = || fs::read_to_string(vault.join(".daftar.toml")).unwrap();
        let written = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = written.clone();
        settings.on_vault_written = Some(std::rc::Rc::new(move || count.set(count.get() + 1)));
        assert_eq!((settings.theme(), settings.text_size()), ("dark".to_string(), 14));
        assert!(settings.vault_writable() && settings.can_change("appearance.theme"));
        // The change goes to the vault: only the key's line, and the computer's file as it was.
        settings.set("appearance.text_size", Value::Int(16)).unwrap();
        assert_eq!(theirs(), "# mine\n[appearance]\ntheme = \"dark\" # night\ntext_size = 16\n");
        settings.set("appearance.theme", Value::String("light".into())).unwrap();
        assert_eq!(theirs(), "# mine\n[appearance]\ntheme = \"light\" # night\ntext_size = 16\n");
        settings.set("notifications.digest", Value::Bool(false)).unwrap();
        assert_eq!(theirs(), "# mine\n[appearance]\ntheme = \"light\" # night\ntext_size = 16\n\n[notifications]\ndigest = false\n");
        // A font is this platform's: it goes to `[linux]`.
        settings.set("appearance.font", Value::String("Inter".into())).unwrap();
        assert!(theirs().ends_with("\n[linux.appearance]\nfont = \"Inter\"\n"));
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), mine);
        assert_eq!((settings.theme(), settings.text_size(), written.get()), ("light".to_string(), 16, 4));
        // The file monitor reads the settings again right after: nothing changed, so nothing is drawn again.
        assert!(!settings.reload());
        // A line another device wrote meanwhile stays: the file is read again at the write.
        fs::write(vault.join(".daftar.toml"), "[appearance]\ntheme = \"dark\"\ntext_width = 60\n").unwrap();
        settings.set("appearance.text_size", Value::Int(18)).unwrap();
        assert_eq!(theirs(), "[appearance]\ntheme = \"dark\"\ntext_width = 60\ntext_size = 18\n");
        assert_eq!((settings.theme(), settings.text_width()), ("dark".to_string(), 60));
        // A vault with no `.daftar.toml` that is no repository is not written: the device is.
        fs::remove_file(vault.join(".daftar.toml")).unwrap();
        settings.set("appearance.text_size", Value::Int(12)).unwrap();
        assert!(!vault.join(".daftar.toml").exists());
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), "[appearance]\ntheme = \"light\"\ntext_size = 12\n");
    }

    #[test]
    fn what_a_vault_may_not_hold_goes_to_the_device() {
        let theirs = "# The vault's taste.\n[appearance]\ntheme = \"dark\"\ntext_size = 16\n";
        let mine = "[appearance]\ntheme = \"light\"\n";
        let (_folder, vault, mut settings) = with_vault(theirs, mine);
        let vaults_file = || fs::read_to_string(vault.join(".daftar.toml")).unwrap();
        // A program, pictures from the web turned on, a picture outside the vault, the device's own keys.
        settings.set("terminal.shell", Value::String("/bin/fish".into())).unwrap();
        settings.set("appearance.remote_images", Value::Bool(true)).unwrap();
        settings.set("home.image", Value::String("~/Pictures/home.png".into())).unwrap();
        settings.set("vault", Value::String("~/other".into())).unwrap();
        assert_eq!(vaults_file(), theirs);
        let device = fs::read_to_string(&settings.path).unwrap();
        assert!(
            device.contains("shell = \"/bin/fish\"") && device.contains("remote_images = true") && device.contains("vault = \"~/other\"")
        );
        // Every change in a read-only clone goes to the device, and the vault's file is byte for byte.
        let git = daftar_core::Git::new(&vault);
        for arguments in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["config", "commit.gpgsign", "false"],
            &["add", "-A"],
            &["commit", "-qm", "start"],
            &["config", "daftar.readonly", "true"],
        ] {
            git.run(arguments).unwrap();
        }
        assert!(!settings.vault_writable() && !settings.can_change("appearance.theme") && settings.can_change("appearance.reading_line"));
        settings.set("appearance.reading_line", Value::Int(30)).unwrap();
        settings.set("appearance.text_size", Value::Int(20)).unwrap();
        assert_eq!(vaults_file(), theirs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), "");
        assert!(fs::read_to_string(&settings.path).unwrap().contains("text_size = 20"));
        // A key the vault sets takes the change until the settings are read again; then the vault's value returns.
        assert_eq!((settings.reading_line(), settings.text_size()), (30, 20));
        assert!(settings.reload());
        assert_eq!((settings.reading_line(), settings.text_size()), (30, 16));
        // With `vault_settings = false`, in a vault that can be written, the device too.
        git.set_read_only(false).unwrap();
        fs::write(&settings.path, "vault_settings = false\n").unwrap();
        settings.reload();
        settings.set("appearance.text_size", Value::Int(22)).unwrap();
        assert_eq!(vaults_file(), theirs);
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), "vault_settings = false\n\n[appearance]\ntext_size = 22\n");
    }

    #[test]
    fn keep_on_this_device_declines_the_key() {
        let theirs = "[appearance]\ntheme = \"dark\"\n";
        let (_folder, vault, mut settings) = with_vault(theirs, "[appearance]\ntheme = \"light\"\n");
        settings.keep_on_device("appearance.theme").unwrap();
        assert_eq!(
            fs::read_to_string(&settings.path).unwrap(),
            "vault_settings = [\"appearance.theme\"]\n[appearance]\ntheme = \"dark\"\n"
        );
        assert!(settings.is_kept_on_device("appearance.theme") && !settings.is_from_vault("appearance.theme"));
        // A later change of the key writes the device, and the vault's line stays.
        settings.set("appearance.theme", Value::String("system".into())).unwrap();
        assert_eq!(fs::read_to_string(vault.join(".daftar.toml")).unwrap(), theirs);
        assert_eq!(settings.theme(), "system");
        // Follow the Vault: the key leaves the list, and the vault's value is used again.
        settings.follow_vault("appearance.theme").unwrap();
        assert_eq!(fs::read_to_string(&settings.path).unwrap(), "vault_settings = []\n[appearance]\ntheme = \"system\"\n");
        assert_eq!(settings.theme(), "dark");
        assert_eq!(fs::read_to_string(vault.join(".daftar.toml")).unwrap(), theirs);
    }

    #[test]
    fn notes_open_folded_as_the_vault_says_unless_the_app_says_otherwise() {
        use daftar_core::FoldSections;
        let of = |text: &str| AppSettings::of(PathBuf::new(), text).fold_sections();
        assert_eq!(of(""), FoldSections::Vault);
        assert_eq!(of("[appearance]\nfold_sections = \"sometimes\"\n"), FoldSections::Vault);
        assert_eq!(of("[appearance]\nfold_sections = \"vault\"\n"), FoldSections::Vault);
        assert_eq!(of("[appearance]\nfold_sections = \"always\"\n"), FoldSections::Always);
        assert_eq!(of("[appearance]\nfold_sections = \"never\"\n"), FoldSections::Never);
    }

    #[test]
    fn remote_images_are_shown_unless_turned_off() {
        let of = |text: &str| AppSettings::of(PathBuf::new(), text).remote_images();
        assert!(of(""));
        assert!(of("[appearance]\nremote_images = \"maybe\"\n"));
        assert!(of("[appearance]\nremote_images = true\n"));
        assert!(!of("[appearance]\nremote_images = false\n"));
    }
}
