//! The vaults this computer knows: `vaults` in `config.toml`, a list of
//! folders beside `vault`, the one in use (`docs/specs/several-vaults.md`).
//! A vault is one folder, compared after `~`, `.` and `..` are resolved,
//! links not followed, so the list never holds a folder twice in whatever
//! spelling. The rule is pure, over the text of `config.toml`; the app and
//! `daftar-cli` read the same list. The port of `Vaults.swift`.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::{ConfigFile, Value};

/// One vault of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultEntry {
    /// As `config.toml` writes it, `~` kept.
    pub text: String,
    /// The folder: `~`, `.` and `..` resolved, links not followed.
    pub path: PathBuf,
    /// The folder's name, with its parent when another vault has the name:
    /// `org`, or `org — ~/work`.
    pub name: String,
    pub in_use: bool,
}

impl VaultEntry {
    /// The folder is not there: on a disk that is away today, or gone. It
    /// stays in the list, and is not opened.
    pub fn is_missing(&self) -> bool {
        !self.path.is_dir()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultsFailure {
    /// The vault in use cannot be forgotten.
    InUse,
    /// No vault of the list has that name.
    Unknown(String),
    /// More than one vault has that name: each of them, as it is shown.
    Ambiguous(String, Vec<String>),
}

impl fmt::Display for VaultsFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultsFailure::InUse => write!(f, "the vault in use cannot be forgotten: change to another first"),
            VaultsFailure::Unknown(name) => write!(f, "no vault is named {name}: `daftar-cli vaults` lists them"),
            VaultsFailure::Ambiguous(name, which) => {
                write!(f, "more than one vault is named {name}: {}; give its folder", which.join(", "))
            }
        }
    }
}

impl std::error::Error for VaultsFailure {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vaults {
    file: ConfigFile,
    home: PathBuf,
}

impl Vaults {
    /// The vaults of [config], the text of `config.toml`, for a person
    /// whose home folder is [home].
    pub fn new(config: &str, home: &Path) -> Vaults {
        Vaults { file: ConfigFile::new(config), home: home.to_path_buf() }
    }

    /// The vaults of the person's own `config.toml`.
    pub fn read() -> Vaults {
        Self::new(&std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default(), &std::env::home_dir().unwrap_or_default())
    }

    /// The text of `config.toml`, after the changes made here.
    pub fn text(&self) -> &str {
        self.file.text()
    }

    /// The folder [text] names: `~` is the home folder, and `.` and `..`
    /// are resolved by the text alone; links are not followed.
    pub fn folder(&self, text: &str) -> PathBuf {
        let text = text.trim();
        let whole = match text.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None if text == "~" => self.home.clone(),
            None => PathBuf::from(text),
        };
        let mut out = PathBuf::new();
        for part in whole.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other),
            }
        }
        out
    }

    /// [folder] as the list writes a new one: under the home folder, with `~`.
    pub fn written(&self, folder: &Path) -> String {
        match folder.strip_prefix(&self.home) {
            Ok(rest) if !self.home.as_os_str().is_empty() && rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) if !self.home.as_os_str().is_empty() => format!("~/{}", rest.to_string_lossy()),
            _ => folder.to_string_lossy().into_owned(),
        }
    }

    /// The vault in use, as written; none when `vault` is empty or not there.
    pub fn in_use(&self) -> Option<String> {
        self.file.string("vault").map(|text| text.trim().to_string()).filter(|text| !text.is_empty())
    }

    /// The list, as written, in the person's order: `vaults`, each folder
    /// once, and the vault in use after them when it is not among them.
    pub fn list(&self) -> Vec<String> {
        let mut list: Vec<String> = Vec::new();
        let written = self.file.strings("vaults").unwrap_or_default();
        for text in written.into_iter().chain(self.in_use()) {
            let text = text.trim().to_string();
            if !text.is_empty() && !list.iter().any(|other| self.folder(other) == self.folder(&text)) {
                list.push(text);
            }
        }
        list
    }

    /// The vaults, each with its folder, its name as it is shown, and
    /// whether it is the one in use.
    pub fn entries(&self) -> Vec<VaultEntry> {
        let in_use = self.in_use().map(|text| self.folder(&text));
        let folders: Vec<(String, PathBuf)> =
            self.list().into_iter().map(|text| (self.folder(&text), text)).map(|(path, text)| (text, path)).collect();
        let short =
            |path: &Path| path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |name| name.to_string_lossy().into_owned());
        folders
            .iter()
            .map(|(text, path)| {
                let name = short(path);
                let shared = folders.iter().filter(|(_, other)| short(other) == name).count() > 1;
                let name = match path.parent().filter(|_| shared) {
                    Some(parent) => format!("{name} — {}", self.written(parent)),
                    None => name,
                };
                VaultEntry { text: text.clone(), path: path.clone(), name, in_use: in_use.as_deref() == Some(path) }
            })
            .collect()
    }

    /// Opening the vault at [folder] (as the person wrote it, or a folder
    /// that was chosen): it is the one in use, and it is in the list, at the
    /// end, once. A folder the list has already keeps the text it has.
    pub fn open(&mut self, folder: &str) {
        let path = self.folder(folder);
        let mut list = self.list();
        let known = list.iter().find(|other| self.folder(other) == path).cloned();
        let text = known.unwrap_or_else(|| {
            let text = if folder.trim().starts_with('~') { folder.trim().trim_end_matches('/').to_string() } else { self.written(&path) };
            list.push(text.clone());
            text
        });
        self.file.set("vault", Value::String(text));
        self.file.set("vaults", Value::Strings(list));
    }

    /// Forget Vault: [folder] is taken out of the list, and no file is
    /// touched. The vault in use cannot be forgotten.
    pub fn forget(&mut self, folder: &str) -> Result<(), VaultsFailure> {
        let path = self.folder(folder);
        if self.in_use().is_some_and(|text| self.folder(&text) == path) {
            return Err(VaultsFailure::InUse);
        }
        let list: Vec<String> = self.list().into_iter().filter(|other| self.folder(other) != path).collect();
        self.file.set("vaults", Value::Strings(list));
        Ok(())
    }

    /// The vault of the list whose folder's name is [name], when exactly
    /// one has it: what `--vault <name>` opens.
    pub fn named(&self, name: &str) -> Result<VaultEntry, VaultsFailure> {
        let short = |path: &Path| path.file_name().map(|name| name.to_string_lossy().into_owned());
        let mut found: Vec<VaultEntry> = self.entries().into_iter().filter(|entry| short(&entry.path).as_deref() == Some(name)).collect();
        match found.len() {
            0 => Err(VaultsFailure::Unknown(name.to_string())),
            1 => Ok(found.remove(0)),
            _ => Err(VaultsFailure::Ambiguous(name.to_string(), found.into_iter().map(|entry| entry.name).collect())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vaults(config: &str) -> Vaults {
        Vaults::new(config, Path::new("/home/p"))
    }

    #[test]
    fn the_vault_in_use_is_one_of_the_list() {
        let one = vaults("vault = \"~/org\"\n");
        assert_eq!(one.list(), ["~/org"]);
        assert_eq!(one.in_use().as_deref(), Some("~/org"));
        let two = vaults("vault = \"~/b\"\nvaults = [\"~/a\"]\n");
        assert_eq!(two.list(), ["~/a", "~/b"]);
        assert_eq!(two.entries().iter().map(|e| e.in_use).collect::<Vec<_>>(), [false, true]);
        let none = vaults("");
        assert!(none.list().is_empty() && none.in_use().is_none() && none.entries().is_empty());
        assert!(vaults("vault = \"\"\nvaults = []\n").list().is_empty());
        // The same folder in two spellings is one vault.
        assert_eq!(vaults("vault = \"/home/p/org\"\nvaults = [\"~/org\", \"~/org/\", \"~/a\"]\n").list(), ["~/org", "~/a"]);
    }

    #[test]
    fn opening_a_vault_adds_it_once() {
        let config = "# mine\nvault = \"~/org\"\nvaults = [\"~/org\"]\n";
        for same in ["/home/p/org", "~/org", "~/org/", "~/x/../org", "~/./org"] {
            let mut list = vaults(config);
            list.open(same);
            assert_eq!(list.text(), config, "{same}");
        }
        let mut list = vaults(config);
        list.open("~/notes");
        assert_eq!(list.list(), ["~/org", "~/notes"]);
        assert_eq!(list.in_use().as_deref(), Some("~/notes"));
        assert_eq!(list.text(), "# mine\nvault = \"~/notes\"\nvaults = [\"~/org\", \"~/notes\"]\n");
        // A change of vault moves nothing in the list.
        list.open("/home/p/org");
        assert_eq!((list.list(), list.in_use()), (vec!["~/org".to_string(), "~/notes".to_string()], Some("~/org".to_string())));
        // Forget Vault takes one out; the one in use cannot be forgotten.
        list.forget("/home/p/notes/").unwrap();
        assert_eq!(list.list(), ["~/org"]);
        assert_eq!(list.forget("~/org"), Err(VaultsFailure::InUse));
        assert_eq!(list.list(), ["~/org"]);
        // A config with a vault and no list gets the list at the first change; a chosen
        // folder under the home folder is written with `~`, and one outside it whole.
        let mut old = vaults("vault = \"~/org\"\n");
        old.open("/home/p/work/notes");
        old.open("/srv/shared");
        assert_eq!(old.list(), ["~/org", "~/work/notes", "/srv/shared"]);
        assert_eq!(old.in_use().as_deref(), Some("/srv/shared"));
    }

    #[test]
    fn two_vaults_of_one_name_show_their_parents() {
        let list = vaults("vault = \"~/org\"\nvaults = [\"~/org\", \"~/work/org\", \"~/notes\", \"/srv/org\"]\n");
        assert_eq!(list.entries().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["org — ~", "org — ~/work", "notes", "org — /srv"]);
        // A name that more than one vault has is refused, and each is named.
        let refused = list.named("org").unwrap_err();
        assert_eq!(refused, VaultsFailure::Ambiguous("org".into(), vec!["org — ~".into(), "org — ~/work".into(), "org — /srv".into()]));
        assert!(refused.to_string().contains("org — ~, org — ~/work"));
        // A name one vault has opens it.
        assert_eq!(list.named("notes").unwrap().path, Path::new("/home/p/notes"));
        assert_eq!(list.named("nowhere"), Err(VaultsFailure::Unknown("nowhere".into())));
        assert_eq!(vaults("vault = \"~/org\"\n").entries()[0].name, "org");
    }

    #[test]
    fn a_missing_vault_is_listed_and_not_opened() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join("here")).unwrap();
        let config = "vault = \"~/here\"\nvaults = [\"~/here\", \"~/away\"]\n";
        let mut list = Vaults::new(config, base.path());
        let entries = list.entries();
        assert_eq!(
            entries.iter().map(|e| (e.name.as_str(), e.is_missing(), e.in_use)).collect::<Vec<_>>(),
            [("here", false, true), ("away", true, false)]
        );
        // It stays in the list, until the person forgets it.
        assert_eq!(list.list(), ["~/here", "~/away"]);
        list.forget("~/away").unwrap();
        assert_eq!(list.list(), ["~/here"]);
    }
}
