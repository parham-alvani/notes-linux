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
    /// The open vault, whose `.daftar.toml` is read.
    vault: Option<PathBuf>,
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
        let settings = AppSettings { path, file: resolved.file().clone(), resolved, vault: None };
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

    /// Writes one value back, keeping the rest of the file as it is: as it
    /// is on disk now, so a change another window, the Mac's app or a
    /// dotfiles pull made is kept, not written over with what was read at
    /// launch. A linked file is written where it points, and stays a link.
    /// It is the computer's file that is written, never the vault's.
    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        let mut computer = match fs::read_to_string(&self.path) {
            Ok(text) => ConfigFile::new(text),
            Err(_) => self.resolved.computer().clone(),
        };
        computer.set(key, value.clone());
        let text = computer.text().to_string();
        self.resolve(&text);
        // A key the vault sets takes the change until the settings are read again.
        self.resolved.set(key, value);
        self.file = self.resolved.file().clone();
        self.publish();
        let target = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        let temporary = target.with_extension("toml.daftar");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, &target).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    }

    /// The vault's folder, with `~` for the home folder.
    pub fn vault(&self) -> Option<PathBuf> {
        self.file.vault()
    }

    pub fn theme(&self) -> String {
        self.file.string("appearance.theme").unwrap_or_else(|| "system".into())
    }

    pub fn text_size(&self) -> i64 {
        self.file.int("appearance.text_size").unwrap_or(14)
    }

    /// The reader's widest line, in em of the text size; 0 is the whole window.
    pub fn text_width(&self) -> i64 {
        self.file.int("appearance.text_width").unwrap_or(46).max(0)
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

    #[test]
    fn a_setting_changed_in_the_app_goes_to_the_computer() {
        let folder = tempfile::tempdir().unwrap();
        let vault = folder.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let theirs = "# The vault's taste.\n[appearance]\ntheme = \"dark\"\ntext_size = 16\n\n[terminal]\nshell = \"/tmp/x\"\n";
        fs::write(vault.join(".daftar.toml"), theirs).unwrap();
        let path = folder.path().join("config.toml");
        fs::write(&path, "[appearance]\ntheme = \"light\"\ntext_size = 14\nreading_line = 18\n").unwrap();
        let mut settings = AppSettings::of(path.clone(), &fs::read_to_string(&path).unwrap());
        assert_eq!((settings.theme(), settings.text_size()), ("light".to_string(), 14));
        // The vault that opens sets what it may.
        assert!(settings.set_vault(Some(&vault)));
        assert_eq!((settings.theme(), settings.text_size()), ("dark".to_string(), 16));
        assert!(settings.is_from_vault("appearance.text_size") && !settings.is_from_vault("appearance.reading_line"));
        assert_eq!(settings.file.string("terminal.shell"), None);
        // A change in the app is written to the computer's file, and the vault's is as it was.
        settings.set("appearance.reading_line", Value::Int(30)).unwrap();
        settings.set("appearance.text_size", Value::Int(20)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[appearance]\ntheme = \"light\"\ntext_size = 20\nreading_line = 30\n");
        assert_eq!(fs::read_to_string(vault.join(".daftar.toml")).unwrap(), theirs);
        // The change holds here until the settings are read again; then the vault's value returns.
        assert_eq!((settings.reading_line(), settings.text_size()), (30, 20));
        assert!(settings.reload());
        assert_eq!((settings.reading_line(), settings.text_size()), (30, 16));
        // The vault's file read again when it changed; with no vault, the computer's own.
        fs::write(vault.join(".daftar.toml"), "[appearance]\ntext_size = 12\n").unwrap();
        assert!(settings.reload() && !settings.reload());
        assert_eq!((settings.theme(), settings.text_size()), ("light".to_string(), 12));
        assert!(settings.set_vault(None));
        assert_eq!(settings.text_size(), 20);
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
