//! The app's settings, all in `~/.config/daftar/config.toml`.

use std::fs;
use std::io;
use std::path::PathBuf;

use std::sync::atomic::{AtomicBool, Ordering};

use daftar_core::jalali::Jalali;
use daftar_core::{ConfigFile, Day, Value};

/// `appearance.persian_calendar`, for the pages that show a date and do not
/// hold the settings: the home page, Today, a daily note's subtitle.
static PERSIAN_CALENDAR: AtomicBool = AtomicBool::new(false);

/// [day] in the Solar Hijri calendar, as Persian says it, when the settings
/// want it beside the Gregorian date.
pub fn persian_date(day: Day) -> Option<String> {
    PERSIAN_CALENDAR.load(Ordering::Relaxed).then(|| Jalali::long(day))
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
    }

    /// Writes one value back, keeping the rest of the file as it is.
    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        self.file.set(key, value);
        self.publish();
        fs::write(&self.path, self.file.text())
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
