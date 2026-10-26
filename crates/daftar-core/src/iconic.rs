//! The icons and colours the Iconic plugin gives files and folders, read from
//! `.obsidian/plugins/iconic/data.json`, so the tree here looks like the tree
//! in Obsidian and on the phone.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use fancy_regex::Regex;
use serde_json::Value;

use crate::ordered_json::{JsonFailure, OrderedJson};
use crate::text::slugs;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Icon {
    /// `lucide-book`, or an emoji.
    pub name: Option<String>,
    /// One of Iconic's names (`orange`, `cyan`, …) or `#rrggbb`.
    pub color: Option<String>,
}

impl Icon {
    pub fn is_emoji(&self) -> bool {
        self.name.as_deref().is_some_and(|n| !n.starts_with("lucide-"))
    }
}

#[derive(Debug, Clone)]
struct Rule {
    patterns: Vec<Regex>,
    all: bool,
    icon: Icon,
}

#[derive(Debug, Clone, Default)]
pub struct Iconic {
    items: HashMap<String, Icon>,
    folder_rules: Vec<Rule>,
    file_rules: Vec<Rule>,
}

impl Iconic {
    /// The colours Iconic offers by name, in its order.
    pub const COLORS: [&'static str; 9] = ["red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray"];

    pub fn file(root: &Path) -> std::path::PathBuf {
        root.join(".obsidian/plugins/iconic/data.json")
    }

    pub fn load(root: &Path) -> Iconic {
        fs::read_to_string(Self::file(root)).map(|text| Self::parse(&text)).unwrap_or_default()
    }

    pub fn parse(data: &str) -> Iconic {
        let mut iconic = Iconic::default();
        let Ok(json) = serde_json::from_str::<Value>(data) else { return iconic };
        for key in ["fileIcons", "folderIcons"] {
            for (path, value) in json.get(key).and_then(Value::as_object).into_iter().flatten() {
                let icon = icon_of(value);
                if icon.name.is_some() || icon.color.is_some() {
                    iconic.items.insert(slugs::normalize(path), icon);
                }
            }
        }
        iconic.folder_rules = rules(json.get("folderRules"));
        iconic.file_rules = rules(json.get("fileRules"));
        iconic
    }

    /// The icon of a vault path: its own, else the first rule that matches.
    pub fn icon(&self, path: &str, folder: bool) -> Option<&Icon> {
        if let Some(own) = self.items.get(path) {
            return Some(own);
        }
        let rules = if folder { &self.folder_rules } else { &self.file_rules };
        rules
            .iter()
            .find(|rule| {
                let mut hits = rule.patterns.iter().map(|p| p.is_match(path).unwrap_or(false));
                if rule.all { hits.all(|h| h) } else { hits.any(|h| h) }
            })
            .map(|rule| &rule.icon)
    }

    /// Iconic's file with [path]'s icon set to [icon] and [color]; with both
    /// `None` the entry goes. Iconic keeps files and folders alike in
    /// `fileIcons`, and its own keys in an entry (`unsynced`) stay.
    pub fn setting(path: &str, icon: Option<&str>, color: Option<&str>, data: Option<&str>) -> Result<String, JsonFailure> {
        let mut json = match data {
            Some(text) => OrderedJson::parse(text)?,
            None => OrderedJson::Object(vec![]),
        };
        let mut icons = json.get("fileIcons").cloned().unwrap_or(OrderedJson::Object(vec![]));
        let mut entry = icons.get(path).cloned().unwrap_or(OrderedJson::Object(vec![]));
        entry.set("icon", icon.map(|i| OrderedJson::String(i.into())));
        entry.set("color", color.map(|c| OrderedJson::String(c.into())));
        let keep = entry.get("icon").is_some() || entry.get("color").is_some();
        icons.set(path, keep.then_some(entry));
        json.set("fileIcons", Some(icons));
        Ok(json.text())
    }

    /// Iconic's file after files and folders moved: each entry under its new
    /// path, where it was in the file. `None` when no entry moved.
    pub fn moving(moves: &BTreeMap<String, String>, folder: Option<(&str, &str)>, data: &str) -> Result<Option<String>, JsonFailure> {
        let mut json = OrderedJson::parse(data)?;
        let Some(OrderedJson::Object(members)) = json.get("fileIcons") else { return Ok(None) };
        let mut changed = false;
        let renamed: Vec<(String, OrderedJson)> = members
            .iter()
            .map(|(key, value)| {
                let mut new = moves.get(key).cloned();
                if new.is_none()
                    && let Some((from, to)) = folder
                    && (key == from || key.starts_with(&format!("{from}/")))
                {
                    new = Some(format!("{to}{}", &key[from.len()..]));
                }
                match new {
                    Some(new) if new != *key => {
                        changed = true;
                        (new, value.clone())
                    }
                    _ => (key.clone(), value.clone()),
                }
            })
            .collect();
        if !changed {
            return Ok(None);
        }
        json.set("fileIcons", Some(OrderedJson::Object(renamed)));
        Ok(Some(json.text()))
    }
}

fn icon_of(value: &Value) -> Icon {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    Icon { name: text("icon"), color: text("color") }
}

/// Only the rule shape this vault uses: conditions that match the path
/// against a regular expression. A rule with any other condition is skipped
/// rather than half applied.
fn rules(value: Option<&Value>) -> Vec<Rule> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rule| {
            if !rule.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
                return None;
            }
            let patterns = rule
                .get("conditions")?
                .as_array()?
                .iter()
                .map(|condition| {
                    let text = |key: &str| condition.get(key).and_then(Value::as_str);
                    if !matches!(text("source"), Some("tree" | "path")) || text("operator") != Some("matches") {
                        return None;
                    }
                    Regex::new(text("value")?).ok()
                })
                .collect::<Option<Vec<_>>>()?;
            let all = rule.get("match").and_then(Value::as_str) != Some("any");
            Some(Rule { patterns, all, icon: icon_of(rule) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{
  "biggerIcons": "mobile",
  "maxSearchResults": 50,
  "fileIcons": {
    "Banking": {
      "icon": "lucide-piggy-bank",
      "color": "green",
      "unsynced": [
        "0d3e"
      ]
    },
    "Banking/Exchanges.md": {
      "icon": "lucide-shopping-cart"
    }
  },
  "fileRules": [],
  "rememberDeletedItems": false
}"#;

    // IconicTests

    #[test]
    fn icons_and_folder_rules() {
        let json = r#"{"fileIcons": {"Banking": {"icon": "lucide-piggy-bank", "color": "green"}, "Food.md": {"icon": "🍲"}},
             "folderRules": [{"enabled": true, "match": "all", "icon": "lucide-building", "color": "cyan",
               "conditions": [{"source": "tree", "operator": "matches", "value": "^Work/[^/]*$"}]}]}"#;
        let iconic = Iconic::parse(json);
        assert_eq!(iconic.icon("Banking", true), Some(&Icon { name: Some("lucide-piggy-bank".into()), color: Some("green".into()) }));
        assert!(iconic.icon("Food.md", false).unwrap().is_emoji());
        assert_eq!(iconic.icon("Work/Acme", true).unwrap().name.as_deref(), Some("lucide-building"));
        assert_eq!(iconic.icon("Work/Acme/Team", true), None);
        assert_eq!(iconic.icon("Work/Acme", false), None);
    }

    // IconicWriteTests

    #[test]
    fn the_file_comes_back_as_it_was() {
        assert_eq!(OrderedJson::parse(FILE).unwrap().text(), FILE);
        let tricky = r#"{"a": "xé\n\"y\"", "b": [1.5, -2e3, null, true]}"#;
        // Unicode written as itself, escapes as JavaScript writes them, numbers as they were.
        let expected = "{\n  \"a\": \"xé\\n\\\"y\\\"\",\n  \"b\": [\n    1.5,\n    -2e3,\n    null,\n    true\n  ]\n}";
        assert_eq!(OrderedJson::parse(tricky).unwrap().text(), expected);
    }

    #[test]
    fn setting_an_icon_changes_its_lines_only() {
        let set = Iconic::setting("Banking/Exchanges.md", Some("lucide-euro"), Some("blue"), Some(FILE)).unwrap();
        assert_eq!(set, FILE.replace("\"icon\": \"lucide-shopping-cart\"\n", "\"icon\": \"lucide-euro\",\n      \"color\": \"blue\"\n"));
        // A new one goes last; Iconic's own keys in an entry stay.
        let added = Iconic::setting("Cars", Some("🚗"), None, Some(FILE)).unwrap();
        let json = OrderedJson::parse(&added).unwrap();
        assert_eq!(json.get("fileIcons").unwrap().keys(), ["Banking", "Banking/Exchanges.md", "Cars"]);
        let recolored =
            OrderedJson::parse(&Iconic::setting("Banking", Some("lucide-piggy-bank"), Some("pink"), Some(FILE)).unwrap()).unwrap();
        assert_eq!(recolored.get("fileIcons").unwrap().get("Banking").unwrap().keys(), ["icon", "color", "unsynced"]);
        // No icon and no colour: the entry goes.
        let removed = OrderedJson::parse(&Iconic::setting("Banking/Exchanges.md", None, None, Some(FILE)).unwrap()).unwrap();
        assert_eq!(removed.get("fileIcons").unwrap().keys(), ["Banking"]);
        assert_eq!(Iconic::parse(&added).icon("Cars", true).unwrap().name.as_deref(), Some("🚗"));
    }

    #[test]
    fn icons_follow_a_move() {
        let moved = Iconic::moving(&BTreeMap::new(), Some(("Banking", "Money/Banking")), FILE).unwrap().unwrap();
        let json = OrderedJson::parse(&moved).unwrap();
        assert_eq!(json.get("fileIcons").unwrap().keys(), ["Money/Banking", "Money/Banking/Exchanges.md"]);
        let other = BTreeMap::from([("Other.md".to_string(), "New.md".to_string())]);
        assert_eq!(Iconic::moving(&other, None, FILE).unwrap(), None);
    }
}
