//! `daftar-cli similar`: pairs of notes that say nearly the same thing, so
//! that the person can merge them. The scoring is the core's
//! (`SemanticIndex::similar`, the cosine of the notes' mean vectors); here
//! are the notes it leaves out and how a pair is printed.

use std::collections::HashSet;
use std::path::Path;

use daftar_core::semantic::Similar;
use daftar_core::{CalendarWeek, ExcludedFiles, Templates, VaultIndex};

use crate::meaning::Meaning;

/// The cosine two notes' mean vectors need by default. On a vault of 2,800
/// notes the median pair is 0.49 and one pair in a thousand is over 0.85;
/// the pairs over 0.93 (about 100) are the notes on one subject written
/// twice, in two folders or under two names, with the closest siblings of a
/// series among them.
pub const THRESHOLD: f32 = 0.93;

/// What `similar` asks for.
pub struct Options<'a> {
    pub threshold: f32,
    pub limit: usize,
    /// Only the pairs with this note.
    pub note: Option<&'a str>,
    /// Only the pairs with a note under this folder.
    pub folder: Option<&'a str>,
}

/// The pairs, best first: the hidden notes, the week notes of `Calendar/`
/// and the templates are left out, since the one is written by the phone
/// and the other is meant to be copied.
pub fn pairs(meaning: &Meaning, vault: &VaultIndex, root: &Path, hidden: &ExcludedFiles, options: &Options) -> Vec<Similar> {
    let templates: HashSet<String> = Templates::read(root).templates(vault.notes.keys()).into_iter().collect();
    let keep = |path: &str| {
        vault.notes.contains_key(path) && !hidden.contains(path) && !CalendarWeek::is_week_note(path) && !templates.contains(path)
    };
    let folder = options.folder.map(|folder| format!("{}/", folder.trim_matches('/')));
    let under = |path: &str| folder.as_deref().is_none_or(|folder| folder == "/" || path.starts_with(folder));
    meaning
        .index
        .similar(keep, options.note, options.threshold)
        .into_iter()
        .filter(|pair| under(&pair.a) || under(&pair.b))
        .take(options.limit)
        .collect()
}

/// A pair as a line: the two paths and the cosine of their notes.
pub fn line(pair: &Similar) -> String {
    format!("{}\t{}\t{:.2}", pair.a, pair.b, pair.score)
}

/// A pair as `--json` prints it: `{"a", "b", "score", "a_line", "b_line",
/// "section"}`, the lines (from 1) of the two nearest sections and their
/// cosine, each cosine rounded to four places.
pub fn json(pair: &Similar) -> serde_json::Value {
    let round = |score: f32| (f64::from(score) * 10_000.0).round() / 10_000.0;
    serde_json::json!({
        "a": pair.a,
        "b": pair.b,
        "score": round(pair.score),
        "a_line": pair.a_line + 1,
        "b_line": pair.b_line + 1,
        "section": round(pair.section),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meaning::fake;
    use daftar_core::{ConfigFile, Vault};

    fn write(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[test]
    fn pairs_leave_out_week_notes_templates_and_hidden_notes() {
        let root = tempfile::tempdir().unwrap();
        let lag = "# Lag\n\nThe broker queue and its consumer lag.\n";
        for path in [
            "Work/Lag.md",
            "Notes/Consumer lag.md",
            "Calendar/2026-W46.md",
            "Templates/Incident.md",
            "Meta/Templates/Lag.md",
            ".trash/Lag.md",
        ] {
            write(root.path(), path, lag);
        }
        write(root.path(), "Garden.md", "Basil and mint.\n");
        write(root.path(), ".obsidian/templates.json", r#"{"folder":"Meta/Templates"}"#);
        let vault = Vault::index(root.path(), None);
        let mut meaning = Meaning::with(fake::load, &root.path().join(".cache/semantic.sqlite")).unwrap();
        meaning.update(root.path(), &vault, false).unwrap();
        let hidden = ExcludedFiles::from_config(root.path(), &ConfigFile::new(""), false);
        let all = Options { threshold: THRESHOLD, limit: 10, note: None, folder: None };

        let found = pairs(&meaning, &vault, root.path(), &hidden, &all);
        let mut lines: Vec<String> = found.iter().map(line).collect();
        lines.sort();
        // `Templates/` is a note here, as the vault's templates are in `Meta/Templates`.
        assert_eq!(
            lines,
            [
                "Notes/Consumer lag.md\tTemplates/Incident.md\t1.00",
                "Notes/Consumer lag.md\tWork/Lag.md\t1.00",
                "Templates/Incident.md\tWork/Lag.md\t1.00"
            ]
        );
        let row = json(&found[0]);
        assert_eq!(row.as_object().unwrap().keys().collect::<Vec<_>>(), ["a", "b", "score", "a_line", "b_line", "section"]);
        assert_eq!((row["a_line"].as_u64(), row["b_line"].as_u64(), row["score"].as_f64()), (Some(3), Some(3), Some(1.0)));

        let folder = Options { folder: Some("Work/"), ..all };
        assert_eq!(pairs(&meaning, &vault, root.path(), &hidden, &folder).len(), 2);
        let note = Options { note: Some("Work/Lag.md"), limit: 1, ..all };
        let with = pairs(&meaning, &vault, root.path(), &hidden, &note);
        assert_eq!((with.len(), with[0].a.as_str()), (1, "Work/Lag.md"));
        assert!(pairs(&meaning, &vault, root.path(), &hidden, &Options { threshold: 1.01, ..all }).is_empty());
    }
}
