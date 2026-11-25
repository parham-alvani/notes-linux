//! The settings a vault kept in Obsidian's folder, `.obsidian/`, now in
//! places of Daftar's own (`docs/specs/no-obsidian-folder.md`): a key of the
//! vault's `.daftar.toml`, or a file in `.daftar/` at the vault's root for
//! data too long for a settings file.
//!
//! Each setting is read from its new place first, and from `.obsidian/`
//! only when the new place is not there at all, never merged from both.
//! A file is written where it is read from, and never made in an
//! `.obsidian/` folder a vault does not have.

use std::path::{Path, PathBuf};

use crate::{ConfigFile, Value, VaultSettings};

/// The vault's `.daftar.toml`, read for the values it may hold.
#[derive(Debug, Clone)]
pub struct OwnSettings {
    file: ConfigFile,
    root: Option<PathBuf>,
}

impl OwnSettings {
    /// The folder of Daftar's own files at the vault's root.
    pub const FOLDER: &'static str = ".daftar";
    /// Obsidian's folder, read while the new place is not there.
    pub const OBSIDIAN: &'static str = ".obsidian";

    /// The settings of [text], the vault's `.daftar.toml`, for the vault at
    /// [root] (with none, a path is judged by its text alone).
    pub fn new(text: &str, root: Option<&Path>) -> OwnSettings {
        OwnSettings { file: ConfigFile::new(text), root: root.map(Path::to_path_buf) }
    }

    /// The settings of the vault at [root], read now.
    pub fn read(root: &Path) -> OwnSettings {
        OwnSettings::new(&std::fs::read_to_string(root.join(VaultSettings::FILE)).unwrap_or_default(), Some(root))
    }

    /// [key]'s value when the vault sets it to one it may hold.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.file.get(key).filter(|value| crate::settings::holds(key, value, self.root.as_deref()))
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.get(key) {
            Some(Value::String(text)) => Some(text),
            _ => None,
        }
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(flag)) => Some(flag),
            _ => None,
        }
    }

    pub fn strings(&self, key: &str) -> Option<Vec<String>> {
        match self.get(key) {
            Some(Value::Strings(list)) => Some(list),
            _ => None,
        }
    }

    /// Every key under [prefix] (`tasks.statuses.`) the file writes, in its
    /// order, with what follows the prefix, whatever its value.
    pub fn keys_under(&self, prefix: &str) -> Vec<(String, Option<Value>)> {
        self.file.keys().into_iter().filter_map(|(key, value)| Some((key.strip_prefix(prefix)?.to_string(), value))).collect()
    }

    /// Whether the file sets any of [keys].
    pub fn sets_any(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| self.file.get(key).is_some())
    }

    /// The file a setting of `.daftar/` ([name]) is read from and written
    /// to: the new one when it is there; else Obsidian's ([old], a path
    /// under `.obsidian/`) when it is there, or when the vault has an
    /// `.obsidian/` folder at all, as before; else the new one, so no
    /// `.obsidian/` is ever made.
    pub fn file(root: &Path, name: &str, old: &str) -> PathBuf {
        let new = root.join(Self::FOLDER).join(name);
        let obsidian = root.join(Self::OBSIDIAN);
        if new.exists() || !obsidian.is_dir() { new } else { obsidian.join(old) }
    }

    /// The file of a setting that is only read: the new one when it is
    /// there, else Obsidian's.
    pub fn read_from(root: &Path, name: &str, old: &str) -> PathBuf {
        let new = root.join(Self::FOLDER).join(name);
        if new.exists() { new } else { root.join(Self::OBSIDIAN).join(old) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_place_wins() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join(".obsidian/app.json"), r#"{"attachmentFolderPath": "uploads"}"#).unwrap();
        std::fs::write(root.join(".daftar.toml"), "[files]\nattachments = \"media\"\n").unwrap();
        assert_eq!(crate::Vault::attachment_folder(root), "media");
        // With no files.attachments, Obsidian's.
        std::fs::write(root.join(".daftar.toml"), "[reader]\nfolded = true\n").unwrap();
        assert_eq!(crate::Vault::attachment_folder(root), "uploads");
        // With neither, the default: the vault's root.
        std::fs::remove_file(root.join(".obsidian/app.json")).unwrap();
        assert_eq!(crate::Vault::attachment_folder(root), "");
        // A path out of the vault is as not there.
        std::fs::write(root.join(".daftar.toml"), "[files]\nattachments = \"../out\"\n").unwrap();
        assert_eq!(OwnSettings::read(root).string("files.attachments"), None);
    }

    #[test]
    fn a_file_is_written_where_it_is_read() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        // No `.obsidian/` at all: the new place, so none is made.
        assert_eq!(OwnSettings::file(root, "icons.json", "plugins/iconic/data.json"), root.join(".daftar/icons.json"));
        // A vault with `.obsidian/` and no `.daftar/` file: Obsidian's, as before.
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        assert_eq!(OwnSettings::file(root, "icons.json", "plugins/iconic/data.json"), root.join(".obsidian/plugins/iconic/data.json"));
        // Once the new file is there, it.
        std::fs::create_dir_all(root.join(".daftar")).unwrap();
        std::fs::write(root.join(".daftar/icons.json"), "{}").unwrap();
        assert_eq!(OwnSettings::file(root, "icons.json", "plugins/iconic/data.json"), root.join(".daftar/icons.json"));
    }
}
