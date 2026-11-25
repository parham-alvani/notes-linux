//! `daftar-cli migrate-obsidian`: a vault's settings out of Obsidian's
//! folder into `.daftar.toml` and `.daftar/` (`daftar_core::ObsidianMigration`,
//! `docs/specs/no-obsidian-folder.md`, section 4), what it moved a line each;
//! `--dry-run` writes nothing, and `--remove` deletes `.obsidian/` after,
//! only when every file in it was moved.

use std::path::Path;

use daftar_core::{ObsidianMigration, Value};

/// What a run moves, or would move with [dry_run], as lines: `set` and a
/// tab and the key and its value, `file` and a file of `.daftar/`, `kept`
/// and a key set already, `left out` and a file's keys that are Obsidian's
/// own, `unknown` and a file the move does not know; then `removed` with
/// [remove].
pub fn run(root: &Path, dry_run: bool, remove: bool) -> Result<Vec<String>, String> {
    let plan = ObsidianMigration::plan(root);
    if plan.read.is_empty() && plan.unknown.is_empty() {
        return Ok(vec!["nothing to move: the vault has no .obsidian/".into()]);
    }
    let mut lines = Vec::new();
    let verb = |done: &str, would: &str| if dry_run { would.to_string() } else { done.to_string() };
    for (key, value) in &plan.settings {
        lines.push(format!("{}\t{key} = {}", verb("set", "would set"), shown(value)));
    }
    for (name, _) in &plan.files {
        lines.push(format!("{}\t.daftar/{name}", verb("wrote", "would write")));
    }
    for key in &plan.kept {
        lines.push(format!("kept\t{key}"));
    }
    for (file, keys) in &plan.left_out {
        lines.push(format!("left out\t{file}\t{}", keys.join(", ")));
    }
    for file in &plan.unknown {
        lines.push(format!("unknown\t{file}"));
    }
    if plan.is_empty() {
        lines.push("nothing to move: every setting is in its place".into());
    }
    if dry_run {
        if remove {
            lines.push(if plan.unknown.is_empty() {
                "would remove\t.obsidian/".into()
            } else {
                "would keep\t.obsidian/, for the files it does not know".into()
            });
        }
        return Ok(lines);
    }
    plan.apply(root).map_err(|e| e.to_string())?;
    if remove {
        if let Err(refused) = plan.remove(root) {
            // What moved is said before why the folder stays.
            for line in &lines {
                out!("{line}");
            }
            return Err(refused);
        }
        lines.push("removed\t.obsidian/".into());
    }
    Ok(lines)
}

/// A value as the file writes it, a long list cut short.
fn shown(value: &Value) -> String {
    let text = value.toml();
    if text.chars().count() > 80 { format!("{}…", text.chars().take(79).collect::<String>()) } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in [
            (".obsidian/app.json", r#"{"attachmentFolderPath": "uploads", "userIgnoreFilters": ["uploads/"]}"#),
            (".obsidian/daily-notes.json", r#"{"format": "YYYY-[W]ww", "folder": "", "template": ""}"#),
            (".obsidian/templates.json", r#"{"folder": ""}"#),
            (".obsidian/types.json", r#"{"types": {"job_next": "datetime"}}"#),
            (".obsidian/bookmarks.json", r#"{"items": []}"#),
            (
                ".obsidian/plugins/obsidian-tasks-plugin/data.json",
                r#"{"statusSettings":{"customStatuses":[{"symbol":"r","name":"In Review","nextStatusSymbol":"x","type":"IN_PROGRESS"}]}}"#,
            ),
            (".obsidian/plugins/beautitab/data.json", r#"{"greetingText": "Hello.", "customQuotes": [{"text": "Q"}], "showQuote": true}"#),
            (".obsidian/plugins/iconic/data.json", r#"{"fileIcons": {}}"#),
            ("Notes/A.md", "# A\n"),
        ] {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    #[test]
    fn migrate_moves_everything() {
        let root = vault();
        let root = root.path();
        // A dry run says what it would do and writes nothing.
        let dry = run(root, true, true).unwrap();
        assert!(dry.contains(&"would set\tfiles.attachments = \"uploads\"".to_string()), "{dry:?}");
        assert!(dry.contains(&"would remove\t.obsidian/".to_string()));
        assert!(!root.join(".daftar.toml").exists() && !root.join(".daftar").exists());
        // The move, then the folder goes.
        let moved = run(root, false, true).unwrap();
        assert!(moved.contains(&"set\tdaily.format = \"YYYY-[W]ww\"".to_string()), "{moved:?}");
        assert_eq!(moved.last().map(String::as_str), Some("removed\t.obsidian/"));
        assert!(!root.join(".obsidian").exists());
        let toml = fs::read_to_string(root.join(".daftar.toml")).unwrap();
        assert!(toml.starts_with("[files]\nattachments = \"uploads\"\n"), "{toml}");
        assert!(toml.contains("[tasks.statuses.in_review]\nsymbol = \"r\""), "{toml}");
        for name in ["icons.json", "bookmarks.json", "quotes.json"] {
            assert!(root.join(".daftar").join(name).is_file(), "{name}");
        }
        assert_eq!(daftar_core::TaskStatuses::load(root).status('r').name, "In Review");
        assert_eq!(daftar_core::DailyNotes::read(root).format, "YYYY-[W]ww");
        // A second run has nothing to move.
        assert_eq!(run(root, false, true).unwrap(), ["nothing to move: the vault has no .obsidian/"]);
    }

    #[test]
    fn an_unknown_file_stops_the_removal() {
        let root = vault();
        let root = root.path();
        fs::write(root.join(".obsidian/plugins/obsidian-git-askpass.sh"), "#!/bin/sh\n").unwrap();
        let refused = run(root, false, true).unwrap_err();
        assert!(refused.contains(".obsidian/plugins/obsidian-git-askpass.sh"), "{refused}");
        assert!(root.join(".obsidian").is_dir());
        // The settings moved all the same, and a run again finds nothing more.
        assert!(root.join(".daftar.toml").is_file());
        let again = run(root, false, false).unwrap();
        assert!(again.contains(&"nothing to move: every setting is in its place".to_string()), "{again:?}");
    }
}
