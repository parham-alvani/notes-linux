//! The app's settings as a TOML file the person can read, edit and keep in
//! their dotfiles: `$XDG_CONFIG_HOME/daftar/config.toml`, else
//! `~/.config/daftar/config.toml`.
//!
//! A small subset of TOML: `[section]` headers and `key = value`, where a
//! value is a quoted string, an integer or a boolean. That is every setting
//! the app has. A change from the app rewrites one value on its own line, so
//! the person's comments and order stay as they wrote them.
//!
//! The file is the same one Daftar for macOS reads, so a dotfiles repository
//! can carry one file to both: a key means the same thing in both apps, and
//! each app leaves alone the keys it does not have.

use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Int(i64),
    Bool(bool),
}

impl Value {
    fn toml(&self) -> String {
        match self {
            Value::String(text) => format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Int(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    text: String,
}

impl ConfigFile {
    pub fn new(text: impl Into<String>) -> ConfigFile {
        ConfigFile { text: text.into() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn default_path() -> PathBuf {
        let base = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
        base.join("daftar/config.toml")
    }

    /// `section.key`, or `key` for the top of the file.
    pub fn get(&self, path: &str) -> Option<Value> {
        let (section, key) = split(path);
        let mut current = "";
        for line in self.text.split('\n') {
            let trimmed = line.trim();
            if let Some(header) = header(trimmed) {
                current = header;
                continue;
            }
            if current != section {
                continue;
            }
            if let Some((name, raw)) = assignment(trimmed)
                && name == key
            {
                return value(raw);
            }
        }
        None
    }

    pub fn string(&self, path: &str) -> Option<String> {
        match self.get(path) {
            Some(Value::String(text)) => Some(text),
            _ => None,
        }
    }

    pub fn int(&self, path: &str) -> Option<i64> {
        match self.get(path) {
            Some(Value::Int(number)) => Some(number),
            _ => None,
        }
    }

    pub fn bool(&self, path: &str) -> Option<bool> {
        match self.get(path) {
            Some(Value::Bool(flag)) => Some(flag),
            _ => None,
        }
    }

    /// The vault's folder, with `~` for the home folder.
    pub fn vault(&self) -> Option<PathBuf> {
        let text = self.string("vault")?;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let home = home();
        Some(match text.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if text == "~" => home,
            None => PathBuf::from(text),
        })
    }

    /// Sets one value: in place when the key is there, else at the end of its
    /// section, else in a new section at the end of the file.
    pub fn set(&mut self, path: &str, new: Value) {
        let (section, key) = split(path);
        let mut lines: Vec<String> = self.text.split('\n').map(String::from).collect();
        let mut current = String::new();
        let mut last_in_section: Option<isize> = section.is_empty().then_some(-1);
        let mut section_seen = section.is_empty();
        for index in 0..lines.len() {
            let line = lines[index].clone();
            let trimmed = line.trim();
            if let Some(header) = header(trimmed) {
                current = header.to_string();
                if header == section {
                    section_seen = true;
                    last_in_section = Some(index as isize);
                }
                continue;
            }
            if current != section {
                continue;
            }
            if !trimmed.is_empty() {
                last_in_section = Some(index as isize);
            }
            if let Some((name, _)) = assignment(trimmed)
                && name == key
            {
                let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let comment = trailing_comment(&line).map(|c| format!(" {c}")).unwrap_or_default();
                lines[index] = format!("{indent}{key} = {}{comment}", new.toml());
                self.text = lines.join("\n");
                return;
            }
        }
        let entry = format!("{key} = {}", new.toml());
        match last_in_section {
            Some(at) if section_seen => lines.insert((at + 1) as usize, entry),
            _ => {
                while lines.last().is_some_and(|l| l.is_empty()) {
                    lines.pop();
                }
                lines.extend([String::new(), format!("[{section}]"), entry]);
            }
        }
        self.text = lines.join("\n");
        if !self.text.ends_with('\n') {
            self.text.push('\n');
        }
    }

    /// The file a new install starts from: every setting, at its default,
    /// with what it does.
    pub const TEMPLATE: &str = r#"# Daftar. These are all of the app's settings; the app reads this file
# again whenever it changes, and writes back only the value that a change
# in its settings touches. Daftar for macOS reads the same file: keys one
# app does not have are left alone by it.

# The folder of the Obsidian vault.
vault = ""

[appearance]
# "system", "light" (Daftar's Daylight palette) or "dark" (naz).
theme = "system"
# 14, as the vault's Obsidian snippet sets it.
text_size = 14
# The font of notes, in the reader and the editor: a font family. Persian
# always uses Vazirmatn.
font = "Inter"
# Code and the editor's markup.
code_font = "JetBrains Mono"

[editor]
# Edit with your own Neovim: it owns the buffer and the file, and the
# editor shows it. Needs `nvim` (pacman -S neovim).
vim_mode = false
# Empty finds nvim on $PATH.
nvim = ""
# "user" loads your init.lua (with g:daftar set); "clean" loads none;
# a path, as "~/.config/daftar/init.lua", loads that file instead.
nvim_config = "user"

[git]
# Minutes between commit-pull-push runs; 0 is off. Keep it off while
# Obsidian's git plugin syncs the same folder.
sync_interval = 0
# Pull when the app starts.
pull_on_launch = true

[notifications]
meetings = true
# Minutes before a meeting.
meeting_lead = 5
digest = true
# The morning digest of the tasks due, as HH:MM.
digest_time = "09:00"

[terminal]
# Empty is the login shell in $SHELL.
shell = ""
font_size = 13

[home]
# The picture behind the home page, which a tab with no note shows: an
# absolute path, a ~ path or a path in the vault, of a PNG, JPEG or WebP
# file. Empty is no picture. The rest of the page follows Obsidian's
# Beautitab plugin settings in the vault.
image = ""

[capture]
# Quick capture (`daftar --capture`, for a bind in the compositor): the
# line goes into a note. "daily" is today's daily note, made from its
# template when it is not there, or a note's path in the vault.
target = "daily"
# A heading to add the line under, at the end of its section, made at the
# end of the note when it is missing; empty is the end of the note.
heading = ""

[wakatime]
# Send the time spent in each note to WakaTime, with wakatime-cli and the
# key in ~/.wakatime.cfg, as Obsidian's WakaTime plugin does. The note's
# path and the vault's name leave this computer.
enabled = false
# Empty finds wakatime-cli in ~/.wakatime or on $PATH.
cli = ""
"#;
}

fn split(path: &str) -> (&str, &str) {
    match path.rsplit_once('.') {
        Some((section, key)) => (section, key),
        None => ("", path),
    }
}

fn header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(rest[..end].trim())
}

fn assignment(line: &str) -> Option<(&str, &str)> {
    if line.starts_with('#') {
        return None;
    }
    let (key, raw) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then_some((key, raw.trim()))
}

fn value(raw: &str) -> Option<Value> {
    if let Some(rest) = raw.strip_prefix('"') {
        let mut out = String::new();
        let mut escaped = false;
        for c in rest.chars() {
            if escaped {
                out.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return Some(Value::String(out));
            } else {
                out.push(c);
            }
        }
        return None;
    }
    let bare = raw.split('#').next().unwrap_or("").trim();
    match bare {
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        _ => bare.replace('_', "").parse().ok().map(Value::Int),
    }
}

/// A `# comment` after the value, outside any quotes.
fn trailing_comment(line: &str) -> Option<&str> {
    let equals = line.find('=')?;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, c) in line[equals + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return Some(&line[equals + 1 + offset..]),
            _ => {}
        }
    }
    None
}

/// The person's home folder: `$HOME`, or `%USERPROFILE%` on Windows, where
/// the command line runs too; the config file is at the same place in it.
fn home() -> PathBuf {
    env::home_dir().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_template() {
        let file = ConfigFile::new(ConfigFile::TEMPLATE);
        assert_eq!(file.string("vault").as_deref(), Some(""));
        assert_eq!(file.string("appearance.theme").as_deref(), Some("system"));
        assert_eq!(file.int("appearance.text_size"), Some(14));
        assert_eq!(file.string("appearance.code_font").as_deref(), Some("JetBrains Mono"));
        assert_eq!(file.bool("git.pull_on_launch"), Some(true));
        assert_eq!(file.string("notifications.digest_time").as_deref(), Some("09:00"));
    }

    #[test]
    fn set_keeps_comments_and_order() {
        let mut file = ConfigFile::new("# mine\nvault = \"~/a\"\n\n[git]\nsync_interval = 0 # off\n");
        file.set("git.sync_interval", Value::Int(5));
        file.set("vault", Value::String("/x \"y\"".into()));
        file.set("git.pull_on_launch", Value::Bool(false));
        file.set("terminal.font_size", Value::Int(14));
        assert_eq!(
            file.text(),
            "# mine\nvault = \"/x \\\"y\\\"\"\n\n[git]\nsync_interval = 5 # off\npull_on_launch = false\n\n[terminal]\nfont_size = 14\n"
        );
        assert_eq!(file.string("vault").as_deref(), Some("/x \"y\""));
        assert_eq!(file.int("git.sync_interval"), Some(5));
        assert_eq!(file.bool("git.pull_on_launch"), Some(false));
    }

    #[test]
    fn a_key_in_another_section_is_another_key() {
        let file = ConfigFile::new("[a]\nsize = 1\n[b]\nsize = 2\n");
        assert_eq!(file.int("a.size"), Some(1));
        assert_eq!(file.int("b.size"), Some(2));
        assert_eq!(file.int("size"), None);
    }

    #[test]
    fn a_mac_only_key_is_left_alone() {
        let mut file = ConfigFile::new("[appearance]\ndock_badge = true\ntheme = \"dark\"\n");
        file.set("appearance.theme", Value::String("light".into()));
        assert_eq!(file.text(), "[appearance]\ndock_badge = true\ntheme = \"light\"\n");
    }
}
