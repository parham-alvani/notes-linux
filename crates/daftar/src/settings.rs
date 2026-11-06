//! The app's settings, all in `~/.config/daftar/config.toml`.

use std::fs;
use std::io;
use std::path::PathBuf;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use daftar_core::jalali::Jalali;
use daftar_core::{Calendar, CalendarEvent, CalendarEvents, ConfigFile, Day, Happening, Place, Value};

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
    place().map(|place| CalendarEvents::between(place, from, to)).unwrap_or_default()
}

/// The holidays and feasts of `calendar.place` on [day].
pub fn events_on(day: Day) -> Vec<&'static CalendarEvent> {
    place().map(|place| CalendarEvents::on(place, day)).unwrap_or_default()
}

pub struct AppSettings {
    pub path: PathBuf,
    pub file: ConfigFile,
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
        let settings = AppSettings { path, file: ConfigFile::new(text) };
        settings.publish();
        settings
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
    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        if let Ok(text) = fs::read_to_string(&self.path) {
            self.file = ConfigFile::new(text);
        }
        self.file.set(key, value);
        self.publish();
        let target = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        let temporary = target.with_extension("toml.daftar");
        fs::write(&temporary, self.file.text())?;
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
        let mut settings = AppSettings { file: ConfigFile::new(fs::read_to_string(&path).unwrap()), path: path.clone() };
        // Another window, or the Mac's app through a dotfiles pull.
        fs::write(&real, "[appearance]\ntheme = \"dark\"\ntext_size = 14\n").unwrap();
        settings.set("appearance.text_size", Value::Int(16)).unwrap();
        assert_eq!(fs::read_to_string(&real).unwrap(), "[appearance]\ntheme = \"dark\"\ntext_size = 16\n");
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    }
}
