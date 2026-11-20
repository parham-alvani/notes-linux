//! The app's settings with the vault's word on top: the committed
//! `.daftar.toml` at a vault's root may hold the same sections and keys as
//! the computer's `config.toml`, so a clone is set up by its first pull
//! (`docs/specs/vault-config.md`).
//!
//! For each key the vault's value is used when the vault may set it and
//! the computer follows the vault, else the computer's, else the default.
//! A repository is text other people can change, so the keys a vault may
//! set are a list: it never chooses a program to run, a file outside the
//! vault, or what leaves the computer. The port of `Settings.swift`.

use std::path::{Path, PathBuf};

use crate::{ConfigFile, Value, VaultEdits, VaultSettings};

/// Where a setting's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Vault,
    Computer,
    Default,
    /// A key of `.daftar.toml` the vault may not set: its value is not used.
    Refused,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Vault => "vault",
            Origin::Computer => "computer",
            Origin::Default => "default",
            Origin::Refused => "refused",
        }
    }
}

/// One line of `daftar-cli config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: Option<Value>,
    pub from: Origin,
}

/// What a vault may say for a key.
#[derive(Clone, Copy)]
enum Rule {
    Bool,
    /// A whole number from the first to the second.
    Int(i64, i64),
    /// Any text of one line.
    Text,
    /// Text of one line that says something: a font, a name.
    Name,
    /// `calendar.place`: a place the app knows, or empty for none.
    Place,
    /// `capture.target`: `daily`, or a path in the vault.
    Target,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// A list of strings.
    List,
    /// `HH:MM`.
    Time,
    /// Only `false`: a vault can turn it off, never on.
    OnlyFalse,
    /// A path in the vault, or empty for none: relative, with no `..`,
    /// and not a link out.
    InVault,
}

/// The keys a vault may set, each with what its value must be.
const ALLOWED: &[(&str, Rule)] = &[
    ("appearance.theme", Rule::Choice(&["system", "light", "dark"])),
    ("appearance.text_size", Rule::Int(10, 28)),
    ("appearance.font", Rule::Name),
    ("appearance.code_font", Rule::Name),
    ("appearance.text_width", Rule::Int(0, 400)),
    ("appearance.reading_line", Rule::Int(0, 100)),
    ("appearance.fold_sections", Rule::Choice(&["vault", "always", "never"])),
    ("appearance.persian_calendar", Rule::Bool),
    ("appearance.remote_images", Rule::OnlyFalse),
    ("calendar.place", Rule::Place),
    ("editor.live_preview", Rule::Bool),
    ("editor.format_on_save", Rule::Bool),
    ("git.sync_interval", Rule::Int(0, 10_080)),
    ("git.pull_on_launch", Rule::Bool),
    ("git.name", Rule::Name),
    ("git.email", Rule::Name),
    ("notifications.meetings", Rule::Bool),
    ("notifications.meeting_lead", Rule::Int(0, 1440)),
    ("notifications.digest", Rule::Bool),
    ("notifications.digest_time", Rule::Time),
    ("terminal.font_size", Rule::Int(9, 24)),
    ("home.image", Rule::InVault),
    ("export.logo", Rule::InVault),
    ("export.footer", Rule::Text),
    ("export.contents", Rule::Bool),
    ("export.landscape", Rule::Bool),
    ("scratchpad.folder", Rule::InVault),
    ("scratchpad.note", Rule::InVault),
    ("capture.hotkey", Rule::Text),
    ("capture.target", Rule::Target),
    ("capture.heading", Rule::Text),
    ("files.hidden", Rule::List),
    ("files.show_hidden", Rule::Bool),
];

/// The keys at the top of `config.toml`, never the vault's to say.
const TOP: &[&str] = &["vault", "vault_settings"];

/// The sections of the app's own settings. A key in one of them that is
/// not on the list is refused, by name: it names or starts a program
/// (`editor.nvim`, `terminal.shell`), sends the notes' paths to a service
/// (`wakatime.*`), or is another app's own. Every other section (`[check]`,
/// `[audit]`, `[reader]`, `[agents]`, `[okf]`) is the vault's own.
const SECTIONS: &[&str] = &[
    "appearance",
    "calendar",
    "editor",
    "git",
    "notifications",
    "terminal",
    "scratchpad",
    "capture",
    "meaning",
    "home",
    "files",
    "wakatime",
    "export",
];

/// The sections a vault may set whole (`notifications.*`, `scratchpad.*`,
/// `capture.*`): a key there this app does not know is left alone.
const OPEN_SECTIONS: &[&str] = &["notifications", "scratchpad", "capture"];

/// The keys only a vault has: the computer's own author is git's.
const VAULT_ONLY: &[&str] = &["git.name", "git.email"];

enum Verdict {
    Used,
    /// As not there: the wrong type, out of range, or a key not known.
    Skipped,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    vault: String,
    computer: ConfigFile,
    root: Option<PathBuf>,
    /// The computer's file with the vault's values set in it.
    file: ConfigFile,
    from_vault: Vec<String>,
    refused: Vec<(String, Option<Value>)>,
}

impl Settings {
    /// The settings of [computer] (the text of `config.toml`) with what
    /// [vault] (the text of `.daftar.toml`) may set on top. A path is
    /// checked by how it is written only; `resolved_in` follows links too.
    pub fn resolved(vault: &str, computer: &str) -> Settings {
        Self::resolve(vault, computer, None)
    }

    /// As `resolved`, for the vault at [root]: a path that is a link out of
    /// the vault is refused.
    pub fn resolved_in(vault: &str, computer: &str, root: &Path) -> Settings {
        Self::resolve(vault, computer, Some(root))
    }

    /// The settings for the vault at [root], whose `.daftar.toml` is read
    /// now, with [computer] as the text of `config.toml`.
    pub fn of(root: &Path, computer: &str) -> Settings {
        let vault = std::fs::read_to_string(root.join(VaultSettings::FILE)).unwrap_or_default();
        Self::resolve(&vault, computer, Some(root))
    }

    /// The settings for the vault at [root] with the computer's own file.
    pub fn read(root: &Path) -> Settings {
        Self::of(root, &std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default())
    }

    fn resolve(vault: &str, computer: &str, root: Option<&Path>) -> Settings {
        let computer = ConfigFile::new(computer);
        let follows = Follows::of(&computer);
        let mut file = computer.clone();
        let (mut from_vault, mut refused) = (Vec::new(), Vec::new());
        for (key, value) in ConfigFile::new(vault).keys() {
            match verdict(&key, value.as_ref(), root) {
                Verdict::Used if follows.key(&key) => {
                    if let Some(value) = value {
                        file.set(&key, value);
                        from_vault.push(key);
                    }
                }
                Verdict::Used | Verdict::Skipped => {}
                Verdict::Refused => refused.push((key, value)),
            }
        }
        Settings { vault: vault.to_string(), computer, root: root.map(Path::to_path_buf), file, from_vault, refused }
    }

    /// The settings in use, as one file: the computer's, with the vault's
    /// values in it.
    pub fn file(&self) -> &ConfigFile {
        &self.file
    }

    /// The computer's own file, which a change in the app is written to.
    pub fn computer(&self) -> &ConfigFile {
        &self.computer
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.file.get(key)
    }

    pub fn string(&self, key: &str) -> Option<String> {
        self.file.string(key)
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.file.int(key)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.file.bool(key)
    }

    /// Whether [key] has the vault's value, so its control is off.
    pub fn is_from_vault(&self, key: &str) -> bool {
        self.from_vault.iter().any(|k| k == key)
    }

    /// The keys of `.daftar.toml` the vault may not set, as it wrote them.
    pub fn refused(&self) -> Vec<&str> {
        self.refused.iter().map(|(key, _)| key.as_str()).collect()
    }

    /// A change made in the app: it goes to the computer's file, never to
    /// the vault's. The caller writes `computer().text()` to `config.toml`.
    /// A change to a key the vault sets (the zoom, the export's choices)
    /// holds until the settings are read again; then the vault's value
    /// returns.
    pub fn set(&mut self, key: &str, value: Value) {
        let mut computer = self.computer.clone();
        computer.set(key, value.clone());
        *self = Self::resolve(&self.vault, computer.text(), self.root.as_deref());
        self.file.set(key, value);
    }

    /// The text of the vault's file these settings were made from.
    pub fn vault_text(&self) -> &str {
        &self.vault
    }

    /// The author the vault gives a clone with none of its own: `git.name`
    /// and `git.email`, each on its own.
    pub fn author(&self) -> (Option<String>, Option<String>) {
        let of = |key: &str| self.is_from_vault(key).then(|| self.file.string(key)).flatten().filter(|v| !v.trim().is_empty());
        (of("git.name"), of("git.email"))
    }

    /// Every key with its value and where it came from: the template's
    /// keys in its order, the keys only a vault has, then each key the
    /// vault was refused, with the value it wrote.
    pub fn entries(&self) -> Vec<Entry> {
        let template = ConfigFile::new(ConfigFile::TEMPLATE);
        let mut keys: Vec<String> = template.keys().into_iter().map(|(key, _)| key).collect();
        for key in VAULT_ONLY {
            let at = keys.iter().rposition(|k| k.starts_with("git.")).map_or(keys.len(), |i| i + 1);
            keys.insert(at, (*key).to_string());
        }
        for (key, _) in self.file.keys() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let mut entries: Vec<Entry> = keys
            .into_iter()
            .map(|key| {
                let from = if self.is_from_vault(&key) {
                    Origin::Vault
                } else if self.computer.get(&key).is_some() {
                    Origin::Computer
                } else {
                    Origin::Default
                };
                let value = self.file.get(&key).or_else(|| template.get(&key));
                Entry { key, value, from }
            })
            .collect();
        entries.extend(self.refused.iter().map(|(key, value)| Entry { key: key.clone(), value: value.clone(), from: Origin::Refused }));
        entries
    }
}

/// `vault_settings` in `config.toml`: whether this computer follows the
/// vault, and the keys it declines.
struct Follows {
    any: bool,
    declined: Vec<String>,
}

impl Follows {
    fn of(computer: &ConfigFile) -> Follows {
        match computer.get("vault_settings") {
            Some(Value::Bool(false)) => Follows { any: false, declined: Vec::new() },
            Some(Value::Strings(declined)) => Follows { any: true, declined },
            _ => Follows { any: true, declined: Vec::new() },
        }
    }

    fn key(&self, key: &str) -> bool {
        self.any && !self.declined.iter().any(|k| k == key)
    }
}

fn verdict(key: &str, value: Option<&Value>, root: Option<&Path>) -> Verdict {
    let section = key.rsplit_once('.').map_or("", |(section, _)| section);
    let Some((_, rule)) = ALLOWED.iter().find(|(allowed, _)| *allowed == key) else {
        let refused =
            if section.is_empty() { TOP.contains(&key) } else { SECTIONS.contains(&section) && !OPEN_SECTIONS.contains(&section) };
        return if refused { Verdict::Refused } else { Verdict::Skipped };
    };
    let one_line = |text: &str| !text.chars().any(char::is_control);
    let fits = match (*rule, value) {
        (Rule::Bool, Some(Value::Bool(_))) => true,
        (Rule::OnlyFalse, Some(Value::Bool(on))) => !on,
        (Rule::Int(low, high), Some(Value::Int(number))) => (low..=high).contains(number),
        (Rule::Text, Some(Value::String(text))) => one_line(text),
        (Rule::Name, Some(Value::String(text))) => one_line(text) && !text.trim().is_empty(),
        (Rule::Place, Some(Value::String(text))) => text.is_empty() || crate::Place::parse(text).is_some(),
        (Rule::Choice(words), Some(Value::String(word))) => words.contains(&word.as_str()),
        (Rule::List, Some(Value::Strings(list))) => list.iter().all(|item| one_line(item)),
        (Rule::Time, Some(Value::String(time))) => is_time(time),
        (Rule::InVault, Some(Value::String(path))) => in_vault(path, root),
        (Rule::Target, Some(Value::String(path))) => path == "daily" || (!path.trim().is_empty() && in_vault(path, root)),
        _ => false,
    };
    if fits { Verdict::Used } else { Verdict::Skipped }
}

fn is_time(text: &str) -> bool {
    let Some((hour, minute)) = text.split_once(':') else { return false };
    let number = |part: &str, below: u32| part.len() == 2 && part.parse::<u32>().is_ok_and(|n| n < below);
    number(hour, 24) && number(minute, 60)
}

/// A path the vault may name: relative, with no `..`, and, in the vault at
/// [root], not a link that leaves it once resolved. Empty is none.
fn in_vault(path: &str, root: Option<&Path>) -> bool {
    let path = path.trim();
    if path.is_empty() {
        return true;
    }
    if path.starts_with(['/', '~', '\\']) || path.contains(':') || path.split(['/', '\\']).any(|part| part == "..") {
        return false;
    }
    root.is_none_or(|root| VaultEdits::within(root, path).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPUTER: &str = "vault = \"~/notes\"\n\n[appearance]\ntheme = \"light\"\ntext_size = 14\nremote_images = true\n";

    #[test]
    fn the_vaults_value_wins() {
        let settings = Settings::resolved("[appearance]\ntext_size = 16\n", COMPUTER);
        assert_eq!(settings.int("appearance.text_size"), Some(16));
        assert!(settings.is_from_vault("appearance.text_size"));
        // With no vault key, the computer's.
        let settings = Settings::resolved("[reader]\nfolded = true\n", COMPUTER);
        assert_eq!(settings.int("appearance.text_size"), Some(14));
        assert!(!settings.is_from_vault("appearance.text_size"));
        // With neither, the default.
        let settings = Settings::resolved("", "");
        let size = settings.entries().into_iter().find(|e| e.key == "appearance.text_size").unwrap();
        assert_eq!((size.value, size.from), (Some(Value::Int(14)), Origin::Default));
        let from = |settings: &Settings, key: &str| settings.entries().into_iter().find(|e| e.key == key).unwrap().from;
        assert_eq!(from(&Settings::resolved("[appearance]\ntext_size = 16\n", COMPUTER), "appearance.text_size"), Origin::Vault);
        assert_eq!(from(&Settings::resolved("", COMPUTER), "appearance.text_size"), Origin::Computer);
    }

    #[test]
    fn a_computer_can_decline() {
        let vault = "[appearance]\ntext_size = 16\ntheme = \"dark\"\n\n[reader]\nfolded = true\n";
        let none = Settings::resolved(vault, &format!("vault_settings = false\n{COMPUTER}"));
        assert_eq!(none.int("appearance.text_size"), Some(14));
        assert_eq!(none.string("appearance.theme").as_deref(), Some("light"));
        let some = Settings::resolved(vault, &format!("vault_settings = [\"appearance.text_size\"]\n{COMPUTER}"));
        assert_eq!(some.int("appearance.text_size"), Some(14));
        assert_eq!(some.string("appearance.theme").as_deref(), Some("dark"));
        // What the vault says about itself is not an app setting.
        assert!(VaultSettings::new(none.vault_text()).folded);
        assert!(VaultSettings::new(some.vault_text()).folded);
        // Not set, or `true`, follows the vault.
        assert_eq!(Settings::resolved(vault, &format!("vault_settings = true\n{COMPUTER}")).int("appearance.text_size"), Some(16));
    }

    #[test]
    fn the_vault_starts_no_program() {
        let computer = "vault = \"~/notes\"\nvault_settings = true\n\n[editor]\nnvim = \"\"\nnvim_config = \"user\"\nvim_mode = false\nneovide = false\n\n[terminal]\nshell = \"\"\n\n[wakatime]\nenabled = false\n";
        let vault = "vault = \"/tmp\"\nvault_settings = true\n\n[terminal]\nshell = \"/tmp/x\"\n\n[editor]\nnvim = \"/tmp/x\"\nnvim_config = \"x.lua\"\nvim_mode = true\nneovide = true\n\n[wakatime]\nenabled = true\n";
        let settings = Settings::resolved(vault, computer);
        assert_eq!(settings.file().text(), computer);
        assert_eq!(
            settings.refused(),
            [
                "vault",
                "vault_settings",
                "terminal.shell",
                "editor.nvim",
                "editor.nvim_config",
                "editor.vim_mode",
                "editor.neovide",
                "wakatime.enabled"
            ]
        );
        let listed: Vec<Entry> = settings.entries().into_iter().filter(|e| e.from == Origin::Refused).collect();
        assert_eq!(listed.len(), 8);
        assert_eq!(listed[2], Entry { key: "terminal.shell".into(), value: Some(Value::String("/tmp/x".into())), from: Origin::Refused });
        // The vault's own sections, an unknown key at the top, and a key this app does not
        // know under a section a vault may set whole, are neither used nor refused.
        let own = "memory = true\n\n[check]\norphans = true\n\n[okf]\nactor = \"x\"\n\n[notifications]\nsound = true\n\n[capture]\nlater = \"x\"\n";
        let own = Settings::resolved(own, computer);
        assert!(own.refused().is_empty());
        assert_eq!(own.file().text(), computer);
        // A key under one of the app's own sections that is not on the list is refused: another app's too.
        let other = Settings::resolved("[appearance]\ndock_badge = true\n\n[meaning]\nmodel = \"x\"\n", computer);
        assert_eq!(other.refused(), ["appearance.dock_badge", "meaning.model"]);
        assert_eq!(other.file().text(), computer);
    }

    #[test]
    fn the_vault_only_turns_web_pictures_off() {
        let of = |computer: bool, vault: bool| {
            Settings::resolved(&format!("[appearance]\nremote_images = {vault}\n"), &format!("[appearance]\nremote_images = {computer}\n"))
        };
        assert_eq!(of(true, false).bool("appearance.remote_images"), Some(false));
        assert_eq!(of(false, true).bool("appearance.remote_images"), Some(false));
        assert_eq!(of(true, true).bool("appearance.remote_images"), Some(true));
        assert!(of(false, true).refused().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_path_from_the_vault_stays_in_it() {
        let base = tempfile::tempdir().unwrap();
        let (root, outside) = (base.path().join("vault"), base.path().join("outside"));
        std::fs::create_dir_all(root.join("uploads")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("uploads/home.webp"), "").unwrap();
        std::fs::write(outside.join("x.png"), "").unwrap();
        std::os::unix::fs::symlink(outside.join("x.png"), root.join("uploads/link.png")).unwrap();
        let computer = "[home]\nimage = \"~/mine.png\"\n";
        let of = |path: &str| Settings::resolved_in(&format!("[home]\nimage = \"{path}\"\n"), computer, &root);
        let inside = of("uploads/home.webp");
        assert_eq!(inside.string("home.image").as_deref(), Some("uploads/home.webp"));
        assert!(inside.refused().is_empty());
        for path in ["/etc/passwd", "~/x.png", "../x.png", "uploads/../../x.png", "uploads/link.png"] {
            let settings = of(path);
            assert_eq!(settings.string("home.image").as_deref(), Some("~/mine.png"), "{path}");
            assert!(settings.refused().is_empty(), "{path}");
        }
        // The capture note and the scratchpad are in the vault by the same rule.
        let capture = Settings::resolved_in(
            "[capture]\ntarget = \"../out.md\"\nheading = \"../Ideas\"\n\n[scratchpad]\nfolder = \"Scratch\"\n",
            "",
            &root,
        );
        assert_eq!(capture.string("capture.target"), None);
        // An empty path is none, and `daily` is today's note.
        assert_eq!(Settings::resolved_in("[home]\nimage = \"\"\n", computer, &root).string("home.image").as_deref(), Some(""));
        assert_eq!(Settings::resolved_in("[capture]\ntarget = \"daily\"\n", "", &root).string("capture.target").as_deref(), Some("daily"));
        assert_eq!(capture.string("capture.heading").as_deref(), Some("../Ideas"));
        assert_eq!(capture.string("scratchpad.folder").as_deref(), Some("Scratch"));
    }

    #[test]
    fn a_wrong_value_is_as_not_there() {
        for vault in ["[appearance]\ntext_size = \"big\"\n", "[appearance]\ntext_size = -3\n", "[appearance]\ntext_size = 400\n"] {
            let settings = Settings::resolved(vault, COMPUTER);
            assert_eq!(settings.int("appearance.text_size"), Some(14), "{vault}");
            assert!(settings.refused().is_empty());
        }
        // The ranges are the same in every app.
        for (vault, key) in [
            ("[appearance]\ntext_width = 401\n", "appearance.text_width"),
            ("[git]\nsync_interval = 10081\n", "git.sync_interval"),
            ("[notifications]\nmeeting_lead = 1441\n", "notifications.meeting_lead"),
            ("[terminal]\nfont_size = 8\n", "terminal.font_size"),
            ("[terminal]\nfont_size = 25\n", "terminal.font_size"),
            ("[appearance]\nfont = \" \"\n", "appearance.font"),
            ("[calendar]\nplace = \"atlantis\"\n", "calendar.place"),
        ] {
            assert!(!Settings::resolved(vault, "").is_from_vault(key), "{vault}");
        }
        for (vault, key) in [
            ("[appearance]\ntext_width = 400\n", "appearance.text_width"),
            ("[git]\nsync_interval = 10080\n", "git.sync_interval"),
            ("[terminal]\nfont_size = 24\n", "terminal.font_size"),
            ("[calendar]\nplace = \"barcelona\"\n", "calendar.place"),
            ("[calendar]\nplace = \"\"\n", "calendar.place"),
        ] {
            assert!(Settings::resolved(vault, "").is_from_vault(key), "{vault}");
        }
        let settings = Settings::resolved("[appearance]\ntheme = \"neon\"\n\n[notifications]\ndigest_time = \"25:00\"\n", COMPUTER);
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("light"));
        assert_eq!(settings.string("notifications.digest_time"), None);
    }

    #[test]
    fn a_setting_changed_in_the_app_goes_to_the_computer() {
        let vault = "# The vault's taste.\n[appearance]\ntheme = \"dark\"\n";
        let mut settings = Settings::resolved(vault, COMPUTER);
        settings.set("appearance.text_size", Value::Int(18));
        assert_eq!(settings.computer().int("appearance.text_size"), Some(18));
        assert_eq!(settings.int("appearance.text_size"), Some(18));
        assert_eq!(settings.vault_text(), vault);
        // A change to a key the vault sets is written to the computer's file and holds
        // until the settings are read again; then the vault's value returns.
        settings.set("appearance.theme", Value::String("system".into()));
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("system"));
        assert_eq!(settings.computer().string("appearance.theme").as_deref(), Some("system"));
        assert_eq!(settings.vault_text(), vault);
        let again = Settings::resolved(vault, settings.computer().text());
        assert_eq!(again.string("appearance.theme").as_deref(), Some("dark"));
    }

    #[test]
    fn the_vault_names_an_author() {
        let settings = Settings::resolved("[git]\nname = \"Notes\"\nemail = \"notes@example.com\"\n", "[git]\nname = \"Mine\"\n");
        assert_eq!(settings.author(), (Some("Notes".into()), Some("notes@example.com".into())));
        // The computer's file has no author of its own: git's config is the computer's.
        assert_eq!(Settings::resolved("", "[git]\nname = \"Mine\"\n").author(), (None, None));
        assert_eq!(Settings::resolved("[git]\nname = \"Notes\"\n", "vault_settings = false\n").author(), (None, None));
        let keys: Vec<String> = Settings::resolved("", "").entries().into_iter().map(|e| e.key).collect();
        let at = keys.iter().position(|k| k == "git.pull_on_launch").unwrap();
        assert_eq!(keys[at + 1..at + 3], ["git.name", "git.email"]);
    }
}
