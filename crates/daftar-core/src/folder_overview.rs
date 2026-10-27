//! A folder at a glance, with no base to write: its notes as a table, one
//! column per property its notes have (the most used first), and the same
//! as cards. The base is made in Obsidian's own YAML, so it runs where any
//! `.base` file does, and can be saved as one to change.

use std::collections::HashMap;

use crate::vault::{Vault, VaultIndex};

/// Columns past these few make the table too wide to read.
const COLUMNS: usize = 6;

pub struct FolderOverview;

impl FolderOverview {
    /// The base of [folder]'s notes, the folders in it too.
    pub fn base(index: &VaultIndex, folder: &str) -> String {
        let folder = folder.trim_matches('/');
        let within = format!("{folder}/");
        let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
        for note in index.notes.values().filter(|note| folder.is_empty() || note.path.starts_with(&within)) {
            for (at, (key, _)) in note.parsed.properties.iter().enumerate() {
                // How many notes have it, and how early it comes in them.
                let entry = counts.entry(key.as_str()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += at;
            }
        }
        let mut keys: Vec<(&str, usize, usize)> = counts
            .into_iter()
            .filter(|(key, _)| !matches!(*key, "aliases" | "alias" | "cssclasses" | "cssclass"))
            .map(|(key, (n, at))| (key, n, at))
            .collect();
        keys.sort_by(|a, b| b.1.cmp(&a.1).then((a.2 * b.1.max(1)).cmp(&(b.2 * a.1.max(1)))).then(a.0.cmp(b.0)));
        let columns: Vec<String> = keys.into_iter().take(COLUMNS).map(|(key, _, _)| yaml_key(key)).collect();
        let order = |extra: &[&str]| -> String {
            std::iter::once("file.name".to_string())
                .chain(columns.iter().cloned())
                .chain(extra.iter().map(|s| s.to_string()))
                .map(|column| format!("      - {column}\n"))
                .collect()
        };
        let filter = if folder.is_empty() { "file.ext == \"md\"".to_string() } else { format!("file.inFolder({})", quoted(folder)) };
        let name = if folder.is_empty() { "Every note".to_string() } else { Vault::title(folder) };
        format!(
            "filters:\n  and:\n    - {filter}\n    - file.ext == \"md\"\nviews:\n  - type: table\n    name: {}\n    order:\n{}    sort:\n      - property: file.mtime\n        direction: DESC\n  - type: cards\n    name: Cards\n    order:\n{}",
            quoted(&name),
            order(&["file.mtime"]),
            order(&[]),
        )
    }
}

/// A property as a base names it: `note.` before a name a bare key cannot be.
fn yaml_key(key: &str) -> String {
    if key.chars().all(|c| c.is_alphanumeric() || c == '_') { key.to_string() } else { format!("note[{}]", quoted(key)) }
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::NoteEntry;
    use crate::{Base, BaseEngine, NoteParser};

    fn index() -> VaultIndex {
        let mut index = VaultIndex::default();
        for (path, text) in [
            ("Books/Dune.md", "---\nauthor: Herbert\nrating: 5\nstatus: done\n---\n# Dune"),
            ("Books/Solaris.md", "---\nauthor: Lem\nrating: 4\n---\n"),
            ("Books/Old/Kindred.md", "---\nauthor: Butler\nyear read: 2020\n---\n"),
            ("Home.md", "---\nmood: fine\n---\n"),
        ] {
            index.notes.insert(
                path.into(),
                NoteEntry { path: path.into(), modified: std::time::SystemTime::now(), parsed: NoteParser::parse(text) },
            );
            index.files.insert(path.into());
        }
        index.derive();
        index
    }

    #[test]
    fn a_folder_is_a_table_of_its_notes_properties() {
        let base = FolderOverview::base(&index(), "Books");
        assert!(base.contains("file.inFolder(\"Books\")"), "{base}");
        let order: Vec<&str> = base
            .lines()
            .skip_while(|l| !l.contains("order:"))
            .skip(1)
            .take_while(|l| l.starts_with("      - "))
            .map(|l| l.trim_start_matches("      - "))
            .collect();
        assert_eq!(order, ["file.name", "author", "rating", "note[\"year read\"]", "status", "file.mtime"]);
        assert!(!base.contains("mood"));
        // It is a base the engine runs: the three books, newest first.
        let index = index();
        let mut engine = BaseEngine::new(&index, chrono::Local::now(), HashMap::new(), |_| None);
        let tables = engine.run(&Base::parse(&base), "Books/");
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].groups.iter().map(|g| g.rows.len()).sum::<usize>(), 3);
    }
}
