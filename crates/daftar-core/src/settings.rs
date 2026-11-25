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
    /// The vault's table for this platform (`[linux.…]`).
    Platform(Platform),
    Computer,
    Default,
    /// A key of `.daftar.toml` the vault may not set: its value is not used.
    Refused,
}

impl Origin {
    pub fn name(self) -> &'static str {
        match self {
            Origin::Vault => "vault",
            Origin::Platform(Platform::Linux) => "vault (linux)",
            Origin::Platform(Platform::Macos) => "vault (macos)",
            Origin::Platform(Platform::Android) => "vault (android)",
            Origin::Computer => "computer",
            Origin::Default => "default",
            Origin::Refused => "refused",
        }
    }
}

/// The platforms the vault's `.daftar.toml` may hold a table for: each
/// reads its own over the shared sections, and never another's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
    Android,
}

impl Platform {
    pub const ALL: [Platform; 3] = [Platform::Linux, Platform::Macos, Platform::Android];

    /// The platform this program runs on: `daftar-cli` on a Mac reads the
    /// `[macos]` table, as Daftar for macOS does beside it. Everything else
    /// (Linux, and Windows, which has no table) reads `[linux]`.
    pub fn here() -> Platform {
        if cfg!(target_os = "macos") { Platform::Macos } else { Platform::Linux }
    }

    /// The table's name in `.daftar.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Android => "android",
        }
    }

    /// The settings only this platform has that a vault may hold: they
    /// live in the platform's table, under the name its app gives them.
    fn own_keys(self) -> &'static [&'static str] {
        match self {
            Platform::Linux => &["scratchpad.folder"],
            Platform::Macos => &["capture.hotkey"],
            Platform::Android => &[],
        }
    }

    /// The platform whose table [key] of `.daftar.toml` is in, and the key
    /// under it.
    fn of(key: &str) -> Option<(Platform, &str)> {
        Self::ALL.into_iter().find_map(|platform| Some((platform, key.strip_prefix(platform.name())?.strip_prefix('.')?)))
    }
}

/// The keys that differ by platform by nature: a font one system has and
/// another has not, a size in another unit.
const BY_PLATFORM: &[&str] = &["appearance.font", "appearance.code_font", "terminal.font_size"];

/// Where a setting changed in the app is written
/// (`docs/specs/vault-config.md`, section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// `config.toml`: what a repository may never hold, what the device
    /// declines, and every change while the vault cannot be written.
    Device,
    /// The platform's table in the vault's `.daftar.toml`.
    Platform,
    /// The vault's shared section.
    Shared,
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
    ("appearance.dark_palette", Rule::Choice(&["naz", "naz-soft"])),
    ("appearance.text_size", Rule::Int(10, 28)),
    ("appearance.font", Rule::Name),
    ("appearance.code_font", Rule::Name),
    ("appearance.text_width", Rule::Int(0, 400)),
    ("appearance.block_width", Rule::Int(0, 400)),
    ("appearance.reading_line", Rule::Int(0, 100)),
    ("appearance.fold_sections", Rule::Choice(&["vault", "always", "never"])),
    ("appearance.link_preview", Rule::Bool),
    ("appearance.section_bar", Rule::Bool),
    ("appearance.persian_calendar", Rule::Bool),
    ("appearance.remote_images", Rule::OnlyFalse),
    ("calendar.place", Rule::Place),
    ("editor.live_preview", Rule::Bool),
    ("editor.format_bar", Rule::Bool),
    ("editor.format_on_save", Rule::Bool),
    ("editor.slash_menu", Rule::Bool),
    ("editor.focus", Rule::Bool),
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
const TOP: &[&str] = &["vault", "vaults", "vault_settings"];

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
    /// The keys of [from_vault] that the platform's table sets.
    from_platform: Vec<String>,
    refused: Vec<(String, Option<Value>)>,
    platform: Platform,
}

impl Settings {
    /// The settings of [computer] (the text of `config.toml`) with what
    /// [vault] (the text of `.daftar.toml`) may set on top. A path is
    /// checked by how it is written only; `resolved_in` follows links too.
    pub fn resolved(vault: &str, computer: &str) -> Settings {
        Self::resolve(vault, computer, None, Platform::Linux)
    }

    /// As `resolved`, as [platform] reads the vault: its own table over
    /// the shared sections.
    pub fn resolved_on(platform: Platform, vault: &str, computer: &str) -> Settings {
        Self::resolve(vault, computer, None, platform)
    }

    /// As `resolved`, for the vault at [root]: a path that is a link out of
    /// the vault is refused.
    pub fn resolved_in(vault: &str, computer: &str, root: &Path) -> Settings {
        Self::resolve(vault, computer, Some(root), Platform::Linux)
    }

    /// The settings for the vault at [root], whose `.daftar.toml` is read
    /// now, with [computer] as the text of `config.toml`.
    pub fn of(root: &Path, computer: &str) -> Settings {
        let vault = std::fs::read_to_string(root.join(VaultSettings::FILE)).unwrap_or_default();
        Self::resolve(&vault, computer, Some(root), Platform::here())
    }

    /// The settings for the vault at [root] with the computer's own file.
    pub fn read(root: &Path) -> Settings {
        Self::of(root, &std::fs::read_to_string(ConfigFile::default_path()).unwrap_or_default())
    }

    fn resolve(vault: &str, computer: &str, root: Option<&Path>, platform: Platform) -> Settings {
        let computer = ConfigFile::new(computer);
        let follows = Follows::of(&computer);
        let mut file = computer.clone();
        let (mut from_vault, mut from_platform, mut refused) = (Vec::new(), Vec::new(), Vec::new());
        let keys = ConfigFile::new(vault).keys();
        // The shared sections first, then the platform's own table over them.
        for own in [false, true] {
            for (written, value) in &keys {
                let key = match Platform::of(written) {
                    Some((table, key)) if table == platform && own => key,
                    None if !own => written.as_str(),
                    // Another platform's table is never read.
                    _ => continue,
                };
                match verdict(key, value.as_ref(), root) {
                    Verdict::Used if follows.key(key) => {
                        if let Some(value) = value {
                            file.set(key, value.clone());
                            if !from_vault.iter().any(|known| known == key) {
                                from_vault.push(key.to_string());
                            }
                            if own {
                                from_platform.push(key.to_string());
                            }
                        }
                    }
                    Verdict::Used | Verdict::Skipped => {}
                    Verdict::Refused => refused.push((written.clone(), value.clone())),
                }
            }
        }
        Settings {
            vault: vault.to_string(),
            computer,
            root: root.map(Path::to_path_buf),
            file,
            from_vault,
            from_platform,
            refused,
            platform,
        }
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

    /// Whether a vault may hold [key] at all: it is on the list.
    pub fn may_hold(key: &str) -> bool {
        ALLOWED.iter().any(|(allowed, _)| *allowed == key)
    }

    /// Whether this device keeps [key] for itself: `vault_settings` is
    /// `false`, or names it.
    pub fn is_kept_on_device(&self, key: &str) -> bool {
        !Follows::of(&self.computer).key(key)
    }

    /// Where a change of [key] to [value] made in the app is written, the
    /// first that applies: the device, when a repository may never hold the
    /// key or the value, when the device keeps the key, or when the vault
    /// cannot be written ([writable] false: a read-only clone, a folder
    /// with no `.daftar.toml` that is no repository); the platform's table,
    /// when the key is there already or differs by platform by nature;
    /// else the vault's shared section.
    pub fn write_target(&self, key: &str, value: &Value, writable: bool) -> Target {
        let held = matches!(verdict(key, Some(value), self.root.as_deref()), Verdict::Used);
        if !writable || !held || self.is_kept_on_device(key) {
            return Target::Device;
        }
        let in_table = ConfigFile::new(self.vault.as_str()).get(&format!("{}.{key}", self.platform.name())).is_some();
        if in_table || BY_PLATFORM.contains(&key) || self.platform.own_keys().contains(&key) { Target::Platform } else { Target::Shared }
    }

    /// A change made in the app: [key] is [value], written where
    /// `write_target` says. Only the line of that key changes in the file
    /// it goes to, every other line stays byte for byte; a key that is not
    /// there is added at the end of its section, and a section that is not
    /// there at the end of the file. The caller writes `vault_text()` to
    /// `.daftar.toml` or `computer().text()` to `config.toml`, the one
    /// that changed. A change that goes to the device for a key the vault
    /// sets holds until the settings are read again.
    pub fn set(&mut self, key: &str, value: Value, writable: bool) -> Target {
        let target = self.write_target(key, &value, writable);
        let (mut vault, mut computer) = (ConfigFile::new(self.vault.as_str()), self.computer.clone());
        match target {
            Target::Device => computer.set(key, value.clone()),
            Target::Platform => vault.set(&format!("{}.{key}", self.platform.name()), value.clone()),
            Target::Shared => vault.set(key, value.clone()),
        }
        *self = Self::resolve(vault.text(), computer.text(), self.root.as_deref(), self.platform);
        if target == Target::Device {
            self.file.set(key, value);
        }
        target
    }

    /// Keep on This Device: [key] is added to the keys `vault_settings`
    /// declines, and its value in use is written to the device. The
    /// vault's line stays.
    pub fn keep_on_device(&mut self, key: &str) {
        let mut computer = self.computer.clone();
        let mut declined = match computer.get("vault_settings") {
            // Every key is the device's already.
            Some(Value::Bool(false)) => return,
            Some(Value::Strings(declined)) => declined,
            _ => Vec::new(),
        };
        if !declined.iter().any(|known| known == key) {
            declined.push(key.to_string());
        }
        if let Some(value) = self.file.get(key) {
            computer.set(key, value);
        }
        computer.set("vault_settings", Value::Strings(declined));
        *self = Self::resolve(&self.vault, computer.text(), self.root.as_deref(), self.platform);
    }

    /// Follow the Vault: [key] is taken off the keys `vault_settings`
    /// declines, and the vault's value is used again.
    pub fn follow_vault(&mut self, key: &str) {
        let mut computer = self.computer.clone();
        let Some(Value::Strings(mut declined)) = computer.get("vault_settings") else { return };
        declined.retain(|known| known != key);
        computer.set("vault_settings", Value::Strings(declined));
        *self = Self::resolve(&self.vault, computer.text(), self.root.as_deref(), self.platform);
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
                let from = if self.from_platform.contains(&key) {
                    Origin::Platform(self.platform)
                } else if self.is_from_vault(&key) {
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
    let rule_key = crate::notification_rules::id_of(key).and_then(|_| key.rsplit_once('.')).map(|(_, name)| name);
    let found = match rule_key {
        Some("note") => Some(Rule::InVault),
        Some("at") => Some(Rule::Time),
        Some("heading") => Some(Rule::Name),
        Some(name) if crate::notification_rules::KEYS.contains(&name) => Some(Rule::Text),
        _ => ALLOWED.iter().find(|(allowed, _)| *allowed == key).map(|(_, rule)| *rule),
    };
    let Some(rule) = found else {
        let refused =
            if section.is_empty() { TOP.contains(&key) } else { SECTIONS.contains(&section) && !OPEN_SECTIONS.contains(&section) };
        return if refused { Verdict::Refused } else { Verdict::Skipped };
    };
    let one_line = |text: &str| !text.chars().any(char::is_control);
    let fits = match (rule, value) {
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

pub(crate) fn is_time(text: &str) -> bool {
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
        assert_eq!((size.value, size.from), (Some(Value::Int(16)), Origin::Default));
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
    fn the_vault_cannot_name_vaults() {
        let computer = "vault = \"~/org\"\nvaults = [\"~/org\"]\n";
        let settings = Settings::resolved("vaults = [\"/tmp/x\"]\nvault = \"/tmp/x\"\n", computer);
        assert_eq!(settings.file().text(), computer);
        assert_eq!(settings.file().strings("vaults").unwrap(), ["~/org"]);
        assert_eq!(settings.refused(), ["vaults", "vault"]);
        let listed: Vec<Entry> = settings.entries().into_iter().filter(|e| e.from == Origin::Refused).collect();
        assert_eq!(listed[0], Entry { key: "vaults".into(), value: Some(Value::Strings(vec!["/tmp/x".into()])), from: Origin::Refused });
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

    /// docs/specs/look.md, part 2, case 23, for the keys this app has: the
    /// slash bar, focus mode, the link preview and the section bar.
    #[test]
    fn the_new_settings_have_their_defaults() {
        let template = ConfigFile::new(ConfigFile::TEMPLATE);
        assert_eq!(template.bool("editor.slash_menu"), Some(true));
        assert_eq!(template.bool("appearance.link_preview"), Some(true));
        assert_eq!(template.bool("editor.focus"), Some(false));
        // With no setting each is as not there, and the app takes its default.
        let none = Settings::resolved("", "");
        assert_eq!((none.bool("editor.slash_menu"), none.bool("appearance.link_preview"), none.bool("editor.focus")), (None, None, None));
        // A vault that sets each is followed, and none is refused.
        let vault = "[editor]\nslash_menu = false\nfocus = true\n\n[appearance]\nlink_preview = false\n";
        let settings = Settings::resolved(vault, "[editor]\nslash_menu = true\nfocus = false\n");
        assert_eq!(settings.bool("editor.slash_menu"), Some(false));
        assert_eq!(settings.bool("editor.focus"), Some(true));
        assert_eq!(settings.bool("appearance.link_preview"), Some(false));
        for key in ["editor.slash_menu", "editor.focus", "appearance.link_preview"] {
            assert!(settings.is_from_vault(key), "{key}");
        }
        assert!(settings.refused().is_empty());
        // A value a key may not have is as not there.
        let wrong = Settings::resolved("[editor]\nslash_menu = \"yes\"\n", "[editor]\nslash_menu = false\n");
        assert_eq!(wrong.bool("editor.slash_menu"), Some(false));
        assert!(!wrong.is_from_vault("editor.slash_menu") && wrong.refused().is_empty());
        // The reader's two keys: the link preview and the section bar.
        {
            let keys = ["appearance.link_preview", "appearance.section_bar"];
            let entry = |settings: &Settings, key: &str| settings.entries().into_iter().find(|e| e.key == key).unwrap();
            let none = Settings::resolved("", "");
            for key in keys {
                let found = entry(&none, key);
                assert_eq!((found.value, found.from), (Some(Value::Bool(true)), Origin::Default), "{key}");
            }
            // A vault that sets each is followed, and none is listed as refused.
            let vault = Settings::resolved("[appearance]\nlink_preview = false\nsection_bar = false\n", "");
            for key in keys {
                assert_eq!(vault.bool(key), Some(false), "{key}");
                assert!(vault.is_from_vault(key), "{key}");
            }
            assert!(vault.refused().is_empty());
            // A value that is no boolean is as not there.
            let wrong = Settings::resolved("[appearance]\nsection_bar = \"yes\"\n", "[appearance]\nsection_bar = false\n");
            assert_eq!(wrong.bool("appearance.section_bar"), Some(false));
            assert!(!wrong.is_from_vault("appearance.section_bar") && wrong.refused().is_empty());
        }
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
    fn a_setting_changed_in_the_app_goes_to_the_vault() {
        let vault = "# mine\n[appearance]\ntheme = \"dark\" # night\n";
        let mut settings = Settings::resolved(vault, COMPUTER);
        // A key that is not there is added at the end of its section.
        assert_eq!(settings.set("appearance.text_size", Value::Int(16), true), Target::Shared);
        assert_eq!(settings.vault_text(), "# mine\n[appearance]\ntheme = \"dark\" # night\ntext_size = 16\n");
        assert_eq!(settings.computer().text(), COMPUTER);
        assert_eq!(settings.int("appearance.text_size"), Some(16));
        // Only the line of the key changes, and its comment stays.
        assert_eq!(settings.set("appearance.theme", Value::String("light".into()), true), Target::Shared);
        assert_eq!(settings.vault_text(), "# mine\n[appearance]\ntheme = \"light\" # night\ntext_size = 16\n");
        // A section that is not there is added at the end of the file, with one blank line before it.
        assert_eq!(settings.set("notifications.digest", Value::Bool(false), true), Target::Shared);
        assert_eq!(
            settings.vault_text(),
            "# mine\n[appearance]\ntheme = \"light\" # night\ntext_size = 16\n\n[notifications]\ndigest = false\n"
        );
        assert_eq!(settings.computer().text(), COMPUTER);
        assert_eq!(settings.bool("notifications.digest"), Some(false));
        // A value equal to the default is still written: the vault says it on purpose.
        let mut plain = Settings::resolved("", "");
        assert_eq!(plain.set("appearance.text_size", Value::Int(14), true), Target::Shared);
        assert_eq!(plain.vault_text(), "\n[appearance]\ntext_size = 14\n");
    }

    #[test]
    fn a_platform_reads_its_own_table() {
        let vault = "[appearance]\ntext_size = 14\n[macos.appearance]\ntext_size = 15\n[android.appearance]\ntext_size = 16\n";
        let size = |platform| Settings::resolved_on(platform, vault, "[appearance]\ntext_size = 12\n").int("appearance.text_size");
        assert_eq!(size(Platform::Macos), Some(15));
        assert_eq!(size(Platform::Android), Some(16));
        assert_eq!(size(Platform::Linux), Some(14));
        // The platform's table is over the shared section, whatever the order of the file.
        let first = "[linux.appearance]\ntext_size = 18\n[appearance]\ntext_size = 14\ntheme = \"dark\"\n";
        let linux = Settings::resolved(first, "");
        assert_eq!((linux.int("appearance.text_size"), linux.string("appearance.theme").as_deref()), (Some(18), Some("dark")));
        let from = |settings: &Settings, key: &str| settings.entries().into_iter().find(|e| e.key == key).unwrap().from;
        assert_eq!(from(&linux, "appearance.text_size").name(), "vault (linux)");
        assert_eq!(from(&linux, "appearance.theme").name(), "vault");
        // The allow-list, the ranges and the path rules hold in a table as in a shared section.
        let refused =
            Settings::resolved_on(Platform::Macos, "[macos.terminal]\nshell = \"/tmp/x\"\n[macos.appearance]\ntext_size = 99\n", "");
        assert_eq!(refused.refused(), ["macos.terminal.shell"]);
        assert_eq!(refused.string("terminal.shell"), None);
        assert!(!refused.is_from_vault("appearance.text_size"));
        // Another platform's table is never read, nor named.
        let other = Settings::resolved("[macos.terminal]\nshell = \"/tmp/x\"\n[macos.appearance]\ntheme = \"dark\"\n", "");
        assert!(other.refused().is_empty() && !other.is_from_vault("appearance.theme"));
        // A device that declines a key declines it in the table too.
        let declined = Settings::resolved("[linux.appearance]\ntext_size = 18\n", "vault_settings = [\"appearance.text_size\"]\n");
        assert!(!declined.is_from_vault("appearance.text_size"));
    }

    #[test]
    fn a_font_is_written_to_the_platforms_table() {
        let mut mac = Settings::resolved_on(Platform::Macos, "[appearance]\ntext_size = 14\n", "");
        assert_eq!(mac.set("appearance.font", Value::String("SF Pro".into()), true), Target::Platform);
        assert_eq!(mac.vault_text(), "[appearance]\ntext_size = 14\n\n[macos.appearance]\nfont = \"SF Pro\"\n");
        assert_eq!(mac.string("appearance.font").as_deref(), Some("SF Pro"));
        // A key the platform's table has already changes there, not in the shared section.
        let mut mac = Settings::resolved_on(Platform::Macos, "[appearance]\ntext_size = 14\n[macos.appearance]\ntext_size = 15\n", "");
        assert_eq!(mac.set("appearance.text_size", Value::Int(17), true), Target::Platform);
        assert_eq!(mac.vault_text(), "[appearance]\ntext_size = 14\n[macos.appearance]\ntext_size = 17\n");
        // On Linux the fonts, the terminal's size and the settings only Linux has go to `[linux]`.
        let mut linux = Settings::resolved("", "");
        for (key, value) in [
            ("appearance.font", Value::String("Inter".into())),
            ("appearance.code_font", Value::String("JetBrains Mono".into())),
            ("terminal.font_size", Value::Int(12)),
            ("scratchpad.folder", Value::String("Scratch".into())),
        ] {
            assert_eq!(linux.set(key, value, true), Target::Platform, "{key}");
        }
        assert_eq!(
            linux.vault_text(),
            "\n[linux.appearance]\nfont = \"Inter\"\ncode_font = \"JetBrains Mono\"\n\n[linux.terminal]\nfont_size = 12\n\n[linux.scratchpad]\nfolder = \"Scratch\"\n"
        );
        assert_eq!(linux.set("appearance.theme", Value::String("dark".into()), true), Target::Shared);
    }

    #[cfg(unix)]
    #[test]
    fn what_a_vault_may_not_hold_goes_to_the_device() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("vault");
        std::fs::create_dir_all(root.join("uploads")).unwrap();
        std::fs::write(root.join("uploads/home.webp"), "").unwrap();
        let vault = "# The vault's taste.\n[appearance]\ntheme = \"dark\"\n";
        let computer = "vault = \"~/notes\"\n";
        let device = |key: &str, value: Value| {
            let mut settings = Settings::resolved_in(vault, computer, &root);
            assert_eq!(settings.set(key, value.clone(), true), Target::Device, "{key}");
            assert_eq!(settings.vault_text(), vault, "{key}");
            assert_eq!(settings.computer().get(key), Some(value), "{key}");
        };
        // A program, what a vault may only turn off, a file outside the vault, the device's own keys.
        device("terminal.shell", Value::String("/bin/fish".into()));
        device("editor.vim_mode", Value::Bool(true));
        device("wakatime.enabled", Value::Bool(true));
        device("appearance.remote_images", Value::Bool(true));
        device("home.image", Value::String("~/Pictures/home.png".into()));
        device("home.image", Value::String("/etc/passwd".into()));
        device("okf.actor", Value::String("parham".into()));
        device("vault", Value::String("~/other".into()));
        device("vaults", Value::Strings(vec!["~/other".into()]));
        device("vault_settings", Value::Bool(true));
        // What a vault may hold goes to it: pictures off, a picture in the vault.
        let mut settings = Settings::resolved_in(vault, computer, &root);
        assert_eq!(settings.set("appearance.remote_images", Value::Bool(false), true), Target::Shared);
        assert_eq!(settings.set("home.image", Value::String("uploads/home.webp".into()), true), Target::Shared);
        assert_eq!(settings.computer().text(), computer);
        // Every change in a read-only clone, and with `vault_settings = false`, goes to the device.
        let mut read_only = Settings::resolved_in(vault, computer, &root);
        assert_eq!(read_only.set("appearance.text_size", Value::Int(18), false), Target::Device);
        assert_eq!(read_only.set("appearance.theme", Value::String("system".into()), false), Target::Device);
        assert_eq!(read_only.vault_text(), vault);
        assert_eq!(read_only.computer().int("appearance.text_size"), Some(18));
        // A change to a key the vault sets holds until the settings are read again; then the vault's value returns.
        assert_eq!(read_only.string("appearance.theme").as_deref(), Some("system"));
        assert_eq!(Settings::resolved(vault, read_only.computer().text()).string("appearance.theme").as_deref(), Some("dark"));
        let mut declined = Settings::resolved_in(vault, "vault_settings = false\n", &root);
        assert_eq!(declined.set("appearance.text_size", Value::Int(18), true), Target::Device);
        assert_eq!(declined.vault_text(), vault);
        assert_eq!(declined.computer().text(), "vault_settings = false\n\n[appearance]\ntext_size = 18\n");
    }

    #[test]
    fn keep_on_this_device_declines_the_key() {
        let vault = "[appearance]\ntheme = \"dark\"\ntext_size = 16\n";
        let mut settings = Settings::resolved(vault, "vault = \"~/notes\"\n\n[appearance]\ntheme = \"light\"\n");
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("dark"));
        settings.keep_on_device("appearance.theme");
        // The key is declined, the value in use is the device's, and the vault's line stays.
        assert_eq!(
            settings.computer().text(),
            "vault = \"~/notes\"\nvault_settings = [\"appearance.theme\"]\n\n[appearance]\ntheme = \"dark\"\n"
        );
        assert_eq!(settings.vault_text(), vault);
        assert!(settings.is_kept_on_device("appearance.theme") && !settings.is_from_vault("appearance.theme"));
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("dark"));
        // A later change of the key writes the device.
        assert_eq!(settings.set("appearance.theme", Value::String("system".into()), true), Target::Device);
        assert_eq!(settings.vault_text(), vault);
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("system"));
        // Another key still goes to the vault.
        assert_eq!(settings.write_target("appearance.text_size", &Value::Int(18), true), Target::Shared);
        // A second key joins the list; Follow the Vault takes one off, and the vault's value is used again.
        settings.keep_on_device("appearance.text_size");
        assert_eq!(settings.computer().strings("vault_settings").unwrap(), ["appearance.theme", "appearance.text_size"]);
        settings.follow_vault("appearance.theme");
        assert_eq!(settings.computer().strings("vault_settings").unwrap(), ["appearance.text_size"]);
        assert_eq!(settings.string("appearance.theme").as_deref(), Some("dark"));
        assert!(settings.is_from_vault("appearance.theme"));
        assert_eq!(settings.vault_text(), vault);
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
