//! Obsidian's core Templates plugin: the folder of `.obsidian/templates.json`
//! and the variables a template is filled with. `{{title}}` is the new
//! note's name, `{{date}}` and `{{time}}` are now in the plugin's formats,
//! and `{{date:FORMAT}}` and `{{time:FORMAT}}` take a moment format of their
//! own. A variable the plugin does not know stays as it is.

use std::path::Path;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::day::Day;
use crate::periodic_notes::{Clock, MomentFormat};
use crate::text::{standard_compare, trim};
use crate::vault::Vault;

static VARIABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{\s*(title|date|time)\s*(?::([^}]*))?\}\}").unwrap());

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Templates {
    /// Vault-relative, empty when no folder is set.
    pub folder: String,
    pub date_format: String,
    pub time_format: String,
}

impl Default for Templates {
    fn default() -> Templates {
        Templates::new("", "", "")
    }
}

impl Templates {
    pub fn new(folder: &str, date_format: &str, time_format: &str) -> Templates {
        Templates {
            folder: trim(folder).trim_matches('/').to_string(),
            date_format: if date_format.is_empty() { "YYYY-MM-DD" } else { date_format }.to_string(),
            time_format: if time_format.is_empty() { "HH:mm" } else { time_format }.to_string(),
        }
    }

    pub fn from_json(json: &str) -> Templates {
        let root: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let get = |key: &str| root.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
        Templates::new(get("folder"), get("dateFormat"), get("timeFormat"))
    }

    pub fn read(root: &Path) -> Templates {
        std::fs::read_to_string(root.join(".obsidian/templates.json")).map(|t| Templates::from_json(&t)).unwrap_or_default()
    }

    /// The notes of [paths] that are templates: those in the folder, or,
    /// with no folder set, those in a top folder named `Templates`.
    pub fn templates<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> Vec<String> {
        let prefix = format!("{}/", if self.folder.is_empty() { "Templates" } else { &self.folder });
        let mut found: Vec<String> = paths.into_iter().filter(|p| p.starts_with(&prefix) && Vault::is_note(p)).cloned().collect();
        found.sort_by(|a, b| standard_compare(a, b));
        found
    }

    /// [template] with its variables filled for a note named [title], made on [day] at [clock].
    pub fn fill(&self, template: &str, title: &str, day: Day, clock: Clock) -> String {
        VARIABLE
            .replace_all(template, |c: &Captures| {
                let format = c.get(2).map(|f| trim(f.as_str()).to_string());
                match &c[1] {
                    "title" => title.to_string(),
                    "date" => MomentFormat::format(format.as_deref().unwrap_or(&self.date_format), day, clock),
                    _ => MomentFormat::format(format.as_deref().unwrap_or(&self.time_format), day, clock),
                }
            })
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_are_filled() {
        let templates = Templates::from_json(r#"{"folder":"Meta/Templates","dateFormat":"DD.MM.YYYY"}"#);
        let filled = templates.fill(
            "# {{title}}\nMade {{date}} at {{ time }}, week {{date:ww}}, {{time:h A}}. {{other}}",
            "Plan",
            Day::new(2026, 10, 26).unwrap(),
            Clock::new(9, 30, 0),
        );
        assert_eq!(filled, "# Plan\nMade 26.10.2026 at 09:30, week 44, 9 AM. {{other}}");
    }

    #[test]
    fn the_templates_are_the_notes_of_the_folder() {
        let paths: Vec<String> =
            ["Meta/Templates/Week.md", "Meta/Templates/b.png", "Meta/Templates/Day.md", "Templates/Other.md", "Plan.md"]
                .map(String::from)
                .to_vec();
        assert_eq!(Templates::new("Meta/Templates/", "", "").templates(&paths), ["Meta/Templates/Day.md", "Meta/Templates/Week.md"]);
        assert_eq!(Templates::default().templates(&paths), ["Templates/Other.md"]);
    }
}
