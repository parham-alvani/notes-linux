//! The home page's rules, apart from its view: the port of `Home.swift`.
//! The settings of Obsidian's Beautitab plugin, the new-tab page the home
//! page replaces, the greeting, where the picture is and which notes are
//! recent.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// One of the person's own quotes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Quote {
    pub text: String,
    pub author: String,
}

/// `.obsidian/plugins/beautitab/data.json`. Every key is optional, and a
/// value of the wrong type is the default. Quotes from the web are off: only
/// "My quotes" (`customQuotes`) are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beautitab {
    pub show_time: bool,
    /// True for "24-hour", false for "12-hour", None for the system's choice.
    pub twenty_four_hour: Option<bool>,
    pub show_greeting: bool,
    /// Empty for "Good morning", "Good afternoon" or "Good evening".
    pub greeting_text: String,
    pub show_search: bool,
    pub show_recent_files: bool,
    pub show_bookmarks: bool,
    /// On only when the source is the person's own quotes and there is one.
    pub show_quote: bool,
    pub quotes: Vec<Quote>,
    /// `customBackground`, then each of `localBackgrounds`: paths or web
    /// addresses, in the plugin's order. Empty strings are dropped.
    pub backgrounds: Vec<String>,
}

impl Default for Beautitab {
    fn default() -> Beautitab {
        Beautitab {
            show_time: true,
            twenty_four_hour: None,
            show_greeting: true,
            greeting_text: String::new(),
            show_search: true,
            show_recent_files: true,
            show_bookmarks: false,
            show_quote: false,
            quotes: Vec::new(),
            backgrounds: Vec::new(),
        }
    }
}

impl Beautitab {
    pub fn from_json(json: &str) -> Beautitab {
        let mut out = Beautitab::default();
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(json) else { return out };
        // Only true and false count: a number is not a flag.
        let flag = |key: &str| root.get(key).and_then(Value::as_bool);
        let text = |key: &str| root.get(key).and_then(Value::as_str).map(|t| t.trim().to_string());
        out.show_time = flag("showTime").unwrap_or(out.show_time);
        out.twenty_four_hour = match root.get("timeFormat").and_then(Value::as_str).map(str::to_lowercase).as_deref() {
            Some("24-hour") => Some(true),
            Some("12-hour") => Some(false),
            _ => None,
        };
        out.show_greeting = flag("showGreeting").unwrap_or(out.show_greeting);
        out.greeting_text = text("greetingText").unwrap_or_default();
        out.show_search = flag("showInlineSearch").unwrap_or(out.show_search);
        out.show_recent_files = flag("showRecentFiles").unwrap_or(out.show_recent_files);
        out.show_bookmarks = flag("showBookmarks").unwrap_or(out.show_bookmarks);
        out.quotes = root
            .get("customQuotes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let text = item.get("text")?.as_str()?.trim();
                        if text.is_empty() {
                            return None;
                        }
                        let author = item.get("author").and_then(Value::as_str).unwrap_or("").trim();
                        Some(Quote { text: text.to_string(), author: author.to_string() })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let source = root.get("quoteSource").and_then(Value::as_str);
        out.show_quote = flag("showQuote").unwrap_or(false) && (source.is_none() || source == Some("My quotes")) && !out.quotes.is_empty();
        let local = root.get("localBackgrounds").and_then(Value::as_array).cloned().unwrap_or_default();
        let local = local.iter().filter_map(|item| match item {
            Value::String(text) => Some(text.clone()),
            Value::Object(object) => {
                object.get("path").or_else(|| object.get("url")).or_else(|| object.get("src")).and_then(Value::as_str).map(String::from)
            }
            _ => None,
        });
        out.backgrounds = std::iter::once(root.get("customBackground").and_then(Value::as_str).unwrap_or("").to_string())
            .chain(local)
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty())
            .collect();
        out
    }

    /// The settings of the vault at [root]; the defaults when there are none.
    pub fn read(root: &Path) -> Beautitab {
        std::fs::read_to_string(root.join(".obsidian/plugins/beautitab/data.json"))
            .map(|json| Beautitab::from_json(&json))
            .unwrap_or_default()
    }
}

/// "morning" from 5 to 12, "afternoon" to 18, else "evening".
pub fn part_of_day(hour: u32) -> &'static str {
    match hour {
        5..12 => "morning",
        12..18 => "afternoon",
        _ => "evening",
    }
}

/// The person's greeting, with `{{greeting}}` as the part of the day, or
/// "Good morning" and its kind when there is none.
pub fn greeting(text: &str, hour: u32) -> String {
    let part = part_of_day(hour);
    if text.is_empty() {
        return format!("Good {part}");
    }
    text.replace("{{greeting}}", part)
}

/// The file of the home picture: [setting] is an absolute path, a `~` path,
/// a `file://` address, or a path in the vault at [vault]. A web address is
/// None: the page never goes to the network.
pub fn picture_path(setting: &str, vault: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let text = setting.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix("file://") {
        return rest.starts_with('/').then(|| PathBuf::from(percent_decoded(rest)));
    }
    if text.contains("://") || text.starts_with("data:") {
        return None;
    }
    if text == "~" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    if text.starts_with('/') {
        return Some(PathBuf::from(text));
    }
    vault.map(|vault| vault.join(text))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && let Some(byte) = text.get(at + 1..at + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            at += 3;
            continue;
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The notes to list as recent: those opened in the window first, then the
/// last read, each once and only while it is in the vault.
pub fn recent(opened: &[String], read: &[String], exists: impl Fn(&str) -> bool, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in opened.iter().chain(read) {
        if out.len() >= limit {
            break;
        }
        if exists(path) && seen.insert(path.as_str()) {
            out.push(path.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beautitab_settings_are_read() {
        let json = r#"
            {"showTime": true, "timeFormat": "12-hour", "showGreeting": true, "greetingText": " Hello, Parham. ",
             "showInlineSearch": false, "showRecentFiles": true, "showBookmarks": true, "showQuote": true,
             "quoteSource": "My quotes", "customQuotes": [{"text": "Less, but better.", "author": "Dieter Rams"}, {"text": "  "}, {"author": "x"}, 3],
             "customBackground": "Attachments/sea.webp", "localBackgrounds": ["/tmp/a.png", {"path": "b.jpg"}, 7, ""], "plugin": {"x": 1}}
            "#;
        let settings = Beautitab::from_json(json);
        assert!(settings.show_time && settings.twenty_four_hour == Some(false));
        assert_eq!(settings.greeting_text, "Hello, Parham.");
        assert!(!settings.show_search && settings.show_recent_files && settings.show_bookmarks);
        assert!(settings.show_quote);
        assert_eq!(settings.quotes, vec![Quote { text: "Less, but better.".into(), author: "Dieter Rams".into() }]);
        assert_eq!(settings.backgrounds, vec!["Attachments/sea.webp", "/tmp/a.png", "b.jpg"]);
    }

    #[test]
    fn no_file_or_odd_values_are_the_defaults() {
        let none = Beautitab::read(Path::new("/no-vault-here/at-all"));
        assert_eq!(none, Beautitab::default());
        assert!(none.show_time && none.twenty_four_hour.is_none() && none.show_greeting && none.show_search && none.show_recent_files);
        assert!(!none.show_quote && !none.show_bookmarks);
        // Numbers are not booleans, and a string is not a list.
        let odd = Beautitab::from_json(
            r#"{"showTime": 0, "showGreeting": "no", "timeFormat": 24, "customQuotes": "x", "showQuote": 1, "greetingText": false}"#,
        );
        assert_eq!(odd, Beautitab::default());
        assert_eq!(Beautitab::from_json("[1, 2]"), Beautitab::default());
        assert_eq!(Beautitab::from_json(r#"{"timeFormat": "24-hour"}"#).twenty_four_hour, Some(true));
    }

    #[test]
    fn quotes_from_the_web_are_off() {
        let web = Beautitab::from_json(r#"{"showQuote": true, "quoteSource": "Quotable", "customQuotes": [{"text": "A"}]}"#);
        assert!(!web.show_quote);
        let empty = Beautitab::from_json(r#"{"showQuote": true, "quoteSource": "My quotes", "customQuotes": []}"#);
        assert!(!empty.show_quote);
    }

    #[test]
    fn the_greeting_follows_the_hour() {
        assert_eq!(greeting("", 6), "Good morning");
        assert_eq!(greeting("", 12), "Good afternoon");
        assert_eq!(greeting("", 23), "Good evening");
        assert_eq!(greeting("", 2), "Good evening");
        assert_eq!(greeting("Hello, Parham.", 9), "Hello, Parham.");
        assert_eq!(greeting("Good {{greeting}}, Parham", 15), "Good afternoon, Parham");
    }

    #[test]
    fn the_picture_is_absolute_home_or_in_the_vault() {
        let vault = Path::new("/home/x/org");
        let home = Path::new("/home/x");
        let at = |setting: &str, vault: Option<&Path>| picture_path(setting, vault, home);
        assert_eq!(at("", Some(vault)), None);
        assert_eq!(at("  ", Some(vault)), None);
        assert_eq!(at("/a/b.png", Some(vault)), Some(PathBuf::from("/a/b.png")));
        assert_eq!(at("~/.config/daftar/home.webp", Some(vault)), Some(PathBuf::from("/home/x/.config/daftar/home.webp")));
        assert_eq!(at("Attachments/sea.jpg", Some(vault)), Some(PathBuf::from("/home/x/org/Attachments/sea.jpg")));
        assert_eq!(at("Attachments/sea.jpg", None), None);
        assert_eq!(at("file:///a/b%20c.heic", None), Some(PathBuf::from("/a/b c.heic")));
        // The page never goes to the network.
        assert_eq!(at("https://example.com/a.png", Some(vault)), None);
        assert_eq!(at("data:image/png;base64,AAAA", Some(vault)), None);
    }

    #[test]
    fn recent_notes_are_opened_then_read_each_once() {
        let files = ["A.md", "B.md", "C.md", "D.md"];
        let exists = |path: &str| files.contains(&path);
        let strings = |items: &[&str]| items.iter().map(ToString::to_string).collect::<Vec<_>>();
        let recent = recent(&strings(&["B.md", "Gone.md"]), &strings(&["A.md", "B.md", "C.md", "D.md"]), exists, 3);
        assert_eq!(recent, vec!["B.md", "A.md", "C.md"]);
        assert!(super::recent(&[], &[], exists, 8).is_empty());
    }
}
