//! Rules for the notes under a folder (`docs/specs/more-vault-settings.md`,
//! section 6): a `[folders."<folder>"]` section of the settings sets the
//! base direction of its notes, whether they open folded, and the template
//! a new note there starts from. The most specific folder wins, key by key.

use crate::config::{last_dot, unquoted};
use crate::{ConfigFile, Value};

/// The section every folder's rules are under.
pub const SECTION: &str = "folders";

/// The base direction of a note in the reader and the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Direction {
    /// Each paragraph's own, from its first letter.
    #[default]
    Auto,
    Rtl,
    Ltr,
}

impl Direction {
    pub fn named(name: &str) -> Option<Direction> {
        match name {
            "auto" => Some(Direction::Auto),
            "rtl" => Some(Direction::Rtl),
            "ltr" => Some(Direction::Ltr),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Direction::Auto => "auto",
            Direction::Rtl => "rtl",
            Direction::Ltr => "ltr",
        }
    }
}

/// What the folders above a note set for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderRules {
    pub direction: Direction,
    /// Over `reader.folded` when set.
    pub folded: Option<bool>,
    /// A path in the vault.
    pub template: Option<String>,
}

/// The folder and the key of `folders."<folder>".<key>`, the folder's
/// quotes taken off and with no `/` at either end.
pub fn folder_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix(SECTION)?.strip_prefix('.')?;
    let at = last_dot(rest)?;
    let folder = unquoted(&rest[..at]).trim_matches('/');
    (!folder.is_empty()).then(|| (folder, &rest[at + 1..]))
}

impl FolderRules {
    /// The rules for the note at [path] in [file], the settings in use:
    /// for each key, the folder nearest the note that sets it to a value
    /// it may have. A folder that does not exist is no error.
    pub fn of(file: &ConfigFile, path: &str) -> FolderRules {
        let mut found: Vec<(usize, &'static str, Value)> = Vec::new();
        for (key, value) in file.keys() {
            let (Some((folder, name)), Some(value)) = (folder_key(&key), value) else { continue };
            let inside = path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'));
            let name = match name {
                "direction" => "direction",
                "folded" => "folded",
                "template" => "template",
                _ => continue,
            };
            if inside && crate::settings::holds(&key, &value, None) {
                found.push((folder.len(), name, value));
            }
        }
        let nearest = |name: &str| found.iter().filter(|(_, n, _)| *n == name).max_by_key(|(depth, ..)| *depth).map(|(.., v)| v.clone());
        FolderRules {
            direction: match nearest("direction") {
                Some(Value::String(word)) => Direction::named(&word).unwrap_or_default(),
                _ => Direction::Auto,
            },
            folded: match nearest("folded") {
                Some(Value::Bool(flag)) => Some(flag),
                _ => None,
            },
            template: match nearest("template") {
                Some(Value::String(path)) if !path.trim().is_empty() => Some(path.trim().to_string()),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_folder_wins() {
        let file = ConfigFile::new(
            "[folders.\"Companies\"]\nfolded = false\ntemplate = \"Old.md\"\n\n[folders.\"Companies/Snapp\"]\ntemplate = \"T.md\"\n",
        );
        let rules = FolderRules::of(&file, "Companies/Snapp/Roles/SRE.md");
        assert_eq!(rules, FolderRules { direction: Direction::Auto, folded: Some(false), template: Some("T.md".into()) });
        assert_eq!(FolderRules::of(&file, "Personal/Diary.md"), FolderRules::default());
        // A folder's name is the whole part: `Companies2` is not under `Companies`.
        assert_eq!(FolderRules::of(&file, "Companies2/x.md"), FolderRules::default());
        // The direction, and a value a key may not have, as not there.
        let file = ConfigFile::new(
            "[folders.\"Personal\"]\ndirection = \"rtl\"\nfolded = \"yes\"\n[folders.\"Personal/English\"]\ndirection = \"ltr\"\n",
        );
        assert_eq!(FolderRules::of(&file, "Personal/Day.md").direction, Direction::Rtl);
        assert_eq!(FolderRules::of(&file, "Personal/English/Day.md").direction, Direction::Ltr);
        assert_eq!(FolderRules::of(&file, "Personal/Day.md").folded, None);
        assert_eq!(folder_key("folders.\"Companies/Snapp/\".folded"), Some(("Companies/Snapp", "folded")));
        assert_eq!(folder_key("folders.folded"), None);
    }
}
