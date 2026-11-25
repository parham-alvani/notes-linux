//! Moving a vault's settings out of Obsidian's folder
//! (`docs/specs/no-obsidian-folder.md`, section 4): each setting of
//! `.obsidian/` is written to its new place, a key of `.daftar.toml` or a
//! file of `.daftar/`. A key the vault's file already sets is left as it
//! is, and a value the app takes anyway when the key is missing is not
//! written. `daftar-cli migrate-obsidian` runs it.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value as Json;

use crate::config::key_name;
use crate::home::Beautitab;
use crate::{ConfigFile, Failure, Iconic, OwnSettings, TaskStatuses, Value, VaultEdits, VaultSettings};

/// The files of `.obsidian/` the move reads, by their path in it.
pub const KNOWN: [&str; 8] = [
    "app.json",
    "daily-notes.json",
    "templates.json",
    "types.json",
    "bookmarks.json",
    "plugins/obsidian-tasks-plugin/data.json",
    "plugins/beautitab/data.json",
    "plugins/iconic/data.json",
];

/// What the move does, worked out before anything is written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Migration {
    /// The keys to write to `.daftar.toml`, in order.
    pub settings: Vec<(String, Value)>,
    /// The files to make in `.daftar/`, by name, with their text.
    pub files: Vec<(String, String)>,
    /// The keys `.daftar.toml` sets already, left as they are.
    pub kept: Vec<String>,
    /// The files of `.obsidian/` read, by their path in the vault.
    pub read: Vec<String>,
    /// What a file read holds that has no place in Daftar (Obsidian's own
    /// switches): the file, and the keys.
    pub left_out: Vec<(String, Vec<String>)>,
    /// The files of `.obsidian/` the move does not know, by their path in
    /// the vault: `--remove` stops on them.
    pub unknown: Vec<String>,
}

impl Migration {
    /// What moving the vault at [root] would do.
    pub fn plan(root: &Path) -> Migration {
        let obsidian = root.join(OwnSettings::OBSIDIAN);
        let mut plan = Migration::default();
        if !obsidian.is_dir() {
            return plan;
        }
        let toml = fs::read_to_string(root.join(VaultSettings::FILE)).unwrap_or_default();
        let mut writer = Writer { file: ConfigFile::new(toml), plan: &mut plan };
        for path in files_under(&obsidian) {
            let known = KNOWN.contains(&path.as_str());
            let full = format!("{}/{path}", OwnSettings::OBSIDIAN);
            if !known {
                writer.plan.unknown.push(full);
                continue;
            }
            let text = fs::read_to_string(obsidian.join(&path)).unwrap_or_default();
            writer.plan.read.push(full.clone());
            let json: Json = serde_json::from_str(&text).unwrap_or(Json::Null);
            let left = match path.as_str() {
                "app.json" => writer.app(&json),
                "daily-notes.json" => writer.daily(&json),
                "templates.json" => writer.templates(&json),
                "types.json" => writer.types(&json),
                "plugins/obsidian-tasks-plugin/data.json" => writer.statuses(&text),
                "plugins/beautitab/data.json" => writer.home(root, &text, &json),
                "plugins/iconic/data.json" => writer.copy(root, Iconic::NAME, &text),
                "bookmarks.json" => writer.copy(root, "bookmarks.json", &text),
                _ => Vec::new(),
            };
            if !left.is_empty() {
                writer.plan.left_out.push((full, left));
            }
        }
        plan
    }

    /// Whether there is nothing to write.
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty() && self.files.is_empty()
    }

    /// Writes the move into the vault at [root]: the keys into
    /// `.daftar.toml` (read again at the write, every other line kept
    /// byte for byte), and the files into `.daftar/`, never over one that
    /// is there. Refused in a read-only clone.
    pub fn apply(&self, root: &Path) -> Result<(), Failure> {
        VaultEdits::writable(root)?;
        if !self.settings.is_empty() {
            let with = |text: &str| {
                let mut file = ConfigFile::new(text);
                for (key, value) in &self.settings {
                    if file.get(key).is_none() {
                        file.set(key, value.clone());
                    }
                }
                file.text().to_string()
            };
            if root.join(VaultSettings::FILE).exists() {
                VaultEdits::apply(VaultSettings::FILE, root, |text| Ok(with(text)))?;
            } else {
                VaultEdits::create(VaultSettings::FILE, root, with("").trim_start()).map_err(|e| Failure::Unwritable(e.to_string()))?;
            }
        }
        for (name, text) in &self.files {
            let path = format!("{}/{name}", OwnSettings::FOLDER);
            match VaultEdits::create(&path, root, text) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(Failure::Unwritable(error.to_string())),
            }
        }
        Ok(())
    }

    /// Deletes `.obsidian/` from the vault at [root], once every file in
    /// it was read and moved: refused, with the files named, while one it
    /// does not know is there.
    pub fn remove(&self, root: &Path) -> Result<(), String> {
        if !self.unknown.is_empty() {
            return Err(format!("{} is kept: it holds files the move does not know: {}", OwnSettings::OBSIDIAN, self.unknown.join(", ")));
        }
        VaultEdits::writable(root).map_err(|e| e.to_string())?;
        fs::remove_dir_all(root.join(OwnSettings::OBSIDIAN)).map_err(|e| e.to_string())
    }
}

/// The files under [folder], by their path in it, `/` between the parts.
fn files_under(folder: &Path) -> Vec<String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rest) = path.strip_prefix(folder) {
                found.insert(rest.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    found.into_iter().collect()
}

struct Writer<'a> {
    /// `.daftar.toml` as it is, with what the move adds.
    file: ConfigFile,
    plan: &'a mut Migration,
}

impl Writer<'_> {
    /// [key] is [value], unless the file sets it already.
    fn set(&mut self, key: &str, value: Value) {
        if self.file.get(key).is_some() {
            if !self.plan.kept.iter().any(|k| k == key) && !self.plan.settings.iter().any(|(k, _)| k == key) {
                self.plan.kept.push(key.to_string());
            }
            return;
        }
        self.file.set(key, value.clone());
        self.plan.settings.push((key.to_string(), value));
    }

    /// A text that is not empty and not [default].
    fn text(&mut self, key: &str, json: &Json, name: &str, default: &str) {
        if let Some(text) = json.get(name).and_then(Json::as_str).map(str::trim).filter(|t| !t.is_empty() && *t != default) {
            self.set(key, Value::String(text.to_string()));
        }
    }

    /// The keys of [json] not among [moved]: Obsidian's own.
    fn others(json: &Json, moved: &[&str]) -> Vec<String> {
        json.as_object().map_or_default(|object| object.keys().filter(|k| !moved.contains(&k.as_str())).cloned().collect())
    }

    fn app(&mut self, json: &Json) -> Vec<String> {
        let folder = json.get("attachmentFolderPath").and_then(Json::as_str).unwrap_or("").trim();
        if !folder.is_empty() && folder != "/" {
            self.set("files.attachments", Value::String(folder.trim_end_matches('/').to_string()));
        }
        let filters: Vec<String> = json
            .get("userIgnoreFilters")
            .and_then(Json::as_array)
            .map_or_default(|list| list.iter().filter_map(|v| v.as_str().map(String::from)).collect());
        if !filters.is_empty() {
            self.set("files.ignore", Value::Strings(filters));
        }
        Self::others(json, &["attachmentFolderPath", "userIgnoreFilters"])
    }

    fn daily(&mut self, json: &Json) -> Vec<String> {
        self.text("daily.format", json, "format", crate::DailyNotes::DEFAULT_FORMAT);
        self.text("daily.folder", json, "folder", "");
        self.text("daily.template", json, "template", "");
        Self::others(json, &["format", "folder", "template", "autorun"])
    }

    fn templates(&mut self, json: &Json) -> Vec<String> {
        self.text("templates.folder", json, "folder", "");
        self.text("templates.date_format", json, "dateFormat", "YYYY-MM-DD");
        self.text("templates.time_format", json, "timeFormat", "HH:mm");
        Self::others(json, &["folder", "dateFormat", "timeFormat"])
    }

    fn types(&mut self, json: &Json) -> Vec<String> {
        for (name, kind) in json.get("types").and_then(Json::as_object).into_iter().flatten() {
            if let Some(kind) = kind.as_str().filter(|k| !k.trim().is_empty()) {
                self.set(&format!("properties.types.{}", key_name(name)), Value::String(kind.to_string()));
            }
        }
        Self::others(json, &["types"])
    }

    /// The statuses that are not the four every vault has, each a
    /// `[tasks.statuses.<id>]` section, unless the file has any already.
    fn statuses(&mut self, text: &str) -> Vec<String> {
        let prefix = format!("{}.", TaskStatuses::SECTION);
        if self.file.keys().iter().any(|(key, _)| key.starts_with(&prefix)) {
            self.plan.kept.push(format!("{}.*", TaskStatuses::SECTION));
            return Vec::new();
        }
        let defaults = TaskStatuses::default();
        let mut ids: Vec<String> = Vec::new();
        for status in TaskStatuses::from_json(text).all {
            if defaults.all.contains(&status) {
                continue;
            }
            let mut id: String =
                status.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
            id = id.trim_matches('_').to_string();
            if id.is_empty() {
                id = "status".into();
            }
            let base = id.clone();
            let mut n = 2;
            while ids.contains(&id) {
                id = format!("{base}_{n}");
                n += 1;
            }
            ids.push(id.clone());
            let key = |name: &str| format!("{prefix}{id}.{name}");
            self.set(&key("symbol"), Value::String(status.symbol.to_string()));
            self.set(&key("name"), Value::String(status.name.clone()));
            self.set(&key("next"), Value::String(status.next.to_string()));
            self.set(&key("type"), Value::String(status.kind.type_name().to_string()));
        }
        // The plugin's other settings (its filters, its suggestions) are the plugin's, not Daftar's.
        Vec::new()
    }

    /// Beautitab's page, its quotes into `.daftar/quotes.json`, and its
    /// picture into `home.image` when the vault has none.
    fn home(&mut self, root: &Path, text: &str, json: &Json) -> Vec<String> {
        let plugin = Beautitab::from_json(text);
        let defaults = Beautitab::default();
        let wants_quote = json.get("showQuote").and_then(Json::as_bool).unwrap_or(false)
            && json.get("quoteSource").and_then(Json::as_str).is_none_or(|s| s == "My quotes");
        if !plugin.greeting_text.is_empty() {
            self.set("home.greeting", Value::String(plugin.greeting_text.clone()));
        }
        if let Some(twenty_four) = plugin.twenty_four_hour {
            self.set("home.clock", Value::String(if twenty_four { "24-hour" } else { "12-hour" }.into()));
        }
        for (key, value, default) in [
            ("home.show_time", plugin.show_time, defaults.show_time),
            ("home.show_greeting", plugin.show_greeting, defaults.show_greeting),
            ("home.show_search", plugin.show_search, defaults.show_search),
            ("home.show_recent", plugin.show_recent_files, defaults.show_recent_files),
            ("home.show_bookmarks", plugin.show_bookmarks, defaults.show_bookmarks),
            ("home.show_quote", wants_quote, defaults.show_quote),
        ] {
            if value != default {
                self.set(key, Value::Bool(value));
            }
        }
        if let Some(picture) = plugin.backgrounds.first() {
            self.set("home.image", Value::String(picture.clone()));
        }
        if !plugin.quotes.is_empty() {
            self.copy(root, Beautitab::QUOTES, &Beautitab::quotes_json(&plugin.quotes));
        }
        let moved = [
            "showTime",
            "timeFormat",
            "showGreeting",
            "greetingText",
            "showInlineSearch",
            "showRecentFiles",
            "showBookmarks",
            "showQuote",
            "quoteSource",
            "customQuotes",
            "customBackground",
        ];
        let mut left = Self::others(json, &moved);
        // More than one picture: only the first is the home page's.
        if plugin.backgrounds.len() < 2 {
            left.retain(|k| k != "localBackgrounds");
        }
        left
    }

    /// A file of Obsidian's JSON that `.daftar/` keeps as it is, unless
    /// `.daftar/` has it already.
    fn copy(&mut self, root: &Path, name: &str, text: &str) -> Vec<String> {
        if root.join(OwnSettings::FOLDER).join(name).exists() {
            self.plan.kept.push(format!("{}/{name}", OwnSettings::FOLDER));
        } else {
            self.plan.files.push((name.to_string(), text.to_string()));
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUSES: &str = r#"{"statusSettings":{"coreStatuses":[{"symbol":" ","name":"Todo","nextStatusSymbol":"x","type":"TODO"},{"symbol":"x","name":"Done","nextStatusSymbol":" ","type":"DONE"}],"customStatuses":[{"symbol":"/","name":"In Progress","nextStatusSymbol":"x","type":"IN_PROGRESS"},{"symbol":"-","name":"Cancelled","nextStatusSymbol":" ","type":"CANCELLED"},{"symbol":"r","name":"In Review","nextStatusSymbol":"x","type":"IN_PROGRESS"}]},"globalFilter":""}"#;

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in [
            (".obsidian/app.json", r#"{"vimMode": true, "attachmentFolderPath": "uploads", "userIgnoreFilters": ["uploads/"]}"#),
            (".obsidian/daily-notes.json", r#"{"format": "YYYY-[W]ww", "folder": "", "template": ""}"#),
            (".obsidian/templates.json", r#"{"folder": "Meta/Templates"}"#),
            (".obsidian/types.json", r#"{"types": {"aliases": "aliases", "job_next": "datetime", "my key": "text"}}"#),
            (".obsidian/bookmarks.json", r#"{"items": [{"type": "file", "path": "Notes/A.md"}]}"#),
            (".obsidian/plugins/obsidian-tasks-plugin/data.json", STATUSES),
            (
                ".obsidian/plugins/beautitab/data.json",
                r#"{"timeFormat": "12-hour", "greetingText": "Hello, Parham.", "showQuote": true, "quoteSource": "My quotes", "customQuotes": [{"text": "Less, but better.", "author": ""}], "customBackground": "uploads/Home.webp", "backgroundTheme": "x"}"#,
            ),
            (".obsidian/plugins/iconic/data.json", r#"{"fileIcons": {"Notes": {"icon": "lucide-book", "color": "cyan"}}}"#),
            ("Notes/A.md", "# A\n- [r] Look at it\n"),
            (".daftar.toml", "# Mine.\n[reader]\nfolded = true\n"),
        ] {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    #[test]
    fn migrate_moves_everything() {
        let root = vault();
        let root = root.path();
        let before = (
            TaskStatuses::load(root),
            crate::DailyNotes::read(root),
            Iconic::load(root).icon("Notes", true).cloned(),
            Beautitab::read(root),
            crate::Templates::read(root),
            crate::BaseEngine::types(root),
            crate::Vault::attachment_folder(root),
            crate::Bookmarks::load(root).entries(),
        );
        let plan = Migration::plan(root);
        assert!(plan.unknown.is_empty());
        assert_eq!(plan.read.len(), KNOWN.len());
        assert_eq!(
            plan.left_out,
            [
                (".obsidian/app.json".to_string(), vec!["vimMode".to_string()]),
                (".obsidian/plugins/beautitab/data.json".to_string(), vec!["backgroundTheme".to_string()])
            ]
        );
        plan.apply(root).unwrap();
        plan.remove(root).unwrap();
        assert!(!root.join(".obsidian").exists());
        let toml = fs::read_to_string(root.join(".daftar.toml")).unwrap();
        assert!(
            toml.starts_with("# Mine.\n[reader]\nfolded = true\n\n[files]\nattachments = \"uploads\"\nignore = [\"uploads/\"]\n"),
            "{toml}"
        );
        assert!(toml.contains("[properties.types]\naliases = \"aliases\"\njob_next = \"datetime\"\n\"my key\" = \"text\"\n"), "{toml}");
        assert!(
            toml.contains("[tasks.statuses.in_review]\nsymbol = \"r\"\nname = \"In Review\"\nnext = \"x\"\ntype = \"IN_PROGRESS\"\n"),
            "{toml}"
        );
        assert!(
            toml.contains("[home]\ngreeting = \"Hello, Parham.\"\nclock = \"12-hour\"\nshow_quote = true\nimage = \"uploads/Home.webp\"\n"),
            "{toml}"
        );
        assert!(!toml.contains("[daily]\nfolder") && toml.contains("[daily]\nformat = \"YYYY-[W]ww\"\n"));
        for name in ["icons.json", "bookmarks.json", "quotes.json"] {
            assert!(root.join(".daftar").join(name).is_file(), "{name}");
        }
        // Every setting reads the same as before.
        let after = (
            TaskStatuses::load(root),
            crate::DailyNotes::read(root),
            Iconic::load(root).icon("Notes", true).cloned(),
            Beautitab::read(root),
            crate::Templates::read(root),
            crate::BaseEngine::types(root),
            crate::Vault::attachment_folder(root),
            crate::Bookmarks::load(root).entries(),
        );
        assert_eq!(before, after);
        // A second run has nothing to move.
        assert!(Migration::plan(root).is_empty());
    }

    #[test]
    fn a_key_set_already_is_kept_and_an_unknown_file_stops_the_removal() {
        let root = vault();
        let root = root.path();
        fs::write(root.join(".daftar.toml"), "[files]\nattachments = \"media\"\n").unwrap();
        fs::write(root.join(".obsidian/workspace.json"), "{}").unwrap();
        let plan = Migration::plan(root);
        assert!(plan.kept.contains(&"files.attachments".to_string()));
        assert_eq!(plan.unknown, [".obsidian/workspace.json"]);
        plan.apply(root).unwrap();
        assert!(fs::read_to_string(root.join(".daftar.toml")).unwrap().starts_with("[files]\nattachments = \"media\"\nignore"));
        let refused = plan.remove(root).unwrap_err();
        assert!(refused.contains(".obsidian/workspace.json"), "{refused}");
        assert!(root.join(".obsidian").is_dir());
        // Run again, the move has nothing left to write.
        assert!(Migration::plan(root).is_empty());
    }
}
