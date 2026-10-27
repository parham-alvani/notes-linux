//! The commands that write many notes or make one: `rename-tag`, `replace`,
//! `sort-tasks`, `postpone` and `daily`. Each returns the lines it prints,
//! so the tests run them on a synthetic vault. A write goes through
//! `VaultEdits`, as the app's does; `--dry-run` writes nothing.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use daftar_core::{
    BoardDrop, Clock, DailyNotes, Day, PeriodicNotes, Postpone, ReplaceOptions, ReplacePattern, TagRename, TaskLine, TaskSorter, Templates,
    Vault, VaultEdits, VaultIndex, VaultReplace,
};

/// Renames [old] and the tags under it in every note that has it; the notes
/// that changed, or would.
pub fn rename_tag(root: &Path, index: &VaultIndex, old: &str, new: &str, dry_run: bool) -> Result<Vec<String>, String> {
    for tag in [old, new] {
        if !TagRename::is_valid(tag) {
            return Err(format!("{tag} cannot be a tag: letters, digits, _, - and /, not only digits"));
        }
    }
    let paths: Vec<String> = index.notes.values().filter(|n| TagRename::has(&n.parsed.tags, old)).map(|n| n.path.clone()).collect();
    if dry_run {
        let mut changed: Vec<String> = paths
            .into_iter()
            .filter(|p| fs::read_to_string(root.join(p)).is_ok_and(|text| TagRename::rename(&text, old, new) != text))
            .collect();
        changed.sort();
        return Ok(changed);
    }
    let mut changed = Vec::new();
    for (path, outcome) in paths.iter().zip(VaultEdits::apply_all(&paths, root, |text| TagRename::rename(text, old, new))) {
        match outcome {
            Ok(true) => changed.push(path.clone()),
            Ok(false) => {}
            Err(error) => eprintln!("daftar-cli: {path}: {error}"),
        }
    }
    changed.sort();
    Ok(changed)
}

/// Every match of [find] in the notes under [folder], as `path:line` and
/// the line before and after; written unless [dry_run].
pub fn replace(
    root: &Path,
    index: &VaultIndex,
    find: &str,
    replacement: &str,
    options: ReplaceOptions,
    folder: Option<&str>,
    dry_run: bool,
) -> Result<Vec<String>, String> {
    let Some(pattern) = ReplacePattern::new(find, replacement, options).map_err(|e| e.to_string())? else {
        return Err("nothing to find".into());
    };
    let folder = folder.map(|f| f.trim_matches('/')).filter(|f| !f.is_empty());
    let mut lines = Vec::new();
    for path in index.files.iter().filter(|p| Vault::is_note(p) && folder.is_none_or(|f| p.starts_with(&format!("{f}/")))) {
        let Ok(text) = fs::read_to_string(root.join(path)) else { continue };
        let matches = VaultReplace::matches_of(&text, &pattern);
        if matches.is_empty() {
            continue;
        }
        for m in &matches {
            lines.push(format!("{path}:{}\t{}\t→\t{}", m.line + 1, m.before(), m.after()));
        }
        if !dry_run {
            let chosen: BTreeSet<usize> = matches.iter().map(|m| m.id()).collect();
            let written = VaultEdits::apply(path, root, |now| {
                VaultReplace::replace(now, &pattern, &matches, &chosen).map_err(|_| daftar_core::Failure::TextMoved)
            });
            if let Err(error) = written {
                eprintln!("daftar-cli: {path}: {error}");
            }
        }
    }
    Ok(lines)
}

/// The note's tasks sorted as the Task Sorter plugin does; whether they moved.
pub fn sort_tasks(root: &Path, path: &str, dry_run: bool) -> Result<bool, String> {
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let Some(sorted) = TaskSorter::sort(&text) else { return Ok(false) };
    if !dry_run {
        VaultEdits::apply(path, root, |now| if now == text { Ok(sorted.clone()) } else { Err(daftar_core::Failure::TextMoved) })
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// The task on line [at] (from 0) of [path] postponed by [amount]; its new line.
pub fn postpone(root: &Path, path: &str, at: usize, amount: &str, today: Day) -> Result<String, String> {
    let names: Vec<&str> = Postpone::ALL.iter().map(|p| p.name()).collect();
    let amount = Postpone::from_name(amount).ok_or(format!("postpone by {}", names.join(", ")))?;
    let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    let raw = text.split('\n').nth(at).ok_or(format!("{path} has no line {}", at + 1))?;
    let task = TaskLine::parse(raw.trim_end_matches('\r'), at, "").ok_or(format!("{path}:{} is not a task", at + 1))?;
    if !TaskLine::can_postpone(&task) {
        return Err(format!("{path}:{} is closed, or has no due, scheduled or start date", at + 1));
    }
    VaultEdits::apply(path, root, |now| VaultEdits::move_task(now, at, &task, &BoardDrop::Postpone(amount), today))
        .map_err(|e| e.to_string())?;
    let after = fs::read_to_string(root.join(path)).unwrap_or_default();
    Ok(after.split('\n').nth(at).unwrap_or_default().to_string())
}

/// The periodic note of [day]'s period, as the Daily notes plugin names
/// it, made from its template when it is not there and [create] is set.
pub fn daily(root: &Path, files: &BTreeSet<String>, day: Day, create: bool, clock: Clock) -> Result<String, String> {
    let periodic = PeriodicNotes::new(DailyNotes::read(root));
    let path = periodic.path_for(day);
    if files.contains(&path) || !create {
        return Ok(path);
    }
    let template = &periodic.config.template;
    let source = if template.is_empty() {
        String::new()
    } else {
        let file = if Vault::is_note(template) { template.clone() } else { format!("{template}.md") };
        fs::read_to_string(root.join(&file)).unwrap_or_default()
    };
    let text = Templates::read(root).fill(&source, &Vault::title(&path), day, clock);
    let file = root.join(&path);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{path}: {e}"))?;
    }
    // Never over a note that came meanwhile.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(files: &[(&str, &str)]) -> (tempfile::TempDir, VaultIndex) {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        let index = Vault::index(root.path(), None);
        (root, index)
    }

    fn read(root: &tempfile::TempDir, path: &str) -> String {
        fs::read_to_string(root.path().join(path)).unwrap()
    }

    #[test]
    fn a_tag_is_renamed_in_every_note_that_has_it() {
        let (root, index) = vault(&[("A.md", "#work/alpha here"), ("B.md", "---\ntags: [work]\n---\nbody"), ("C.md", "#home")]);
        assert_eq!(rename_tag(root.path(), &index, "work", "job", true).unwrap(), ["A.md", "B.md"]);
        assert_eq!(read(&root, "A.md"), "#work/alpha here");
        assert_eq!(rename_tag(root.path(), &index, "work", "job", false).unwrap(), ["A.md", "B.md"]);
        assert_eq!(read(&root, "A.md"), "#job/alpha here");
        assert_eq!(read(&root, "B.md"), "---\ntags: [job]\n---\nbody");
        assert!(rename_tag(root.path(), &index, "work", "12", false).is_err());
    }

    #[test]
    fn replace_lists_then_writes() {
        let (root, index) = vault(&[("Notes/A.md", "Call Ana\n`Ana` stays"), ("Other/B.md", "Ana")]);
        let lines = replace(root.path(), &index, "ana", "Sara", ReplaceOptions::default(), Some("Notes"), true).unwrap();
        assert_eq!(lines, ["Notes/A.md:1\tCall Ana\t→\tCall Sara"]);
        assert_eq!(read(&root, "Notes/A.md"), "Call Ana\n`Ana` stays");
        replace(root.path(), &index, "ana", "Sara", ReplaceOptions::default(), Some("Notes"), false).unwrap();
        assert_eq!(read(&root, "Notes/A.md"), "Call Sara\n`Ana` stays");
        assert_eq!(read(&root, "Other/B.md"), "Ana");
    }

    #[test]
    fn tasks_are_sorted_and_postponed() {
        let (root, _) = vault(&[("T.md", "- [ ] Open 📅 2026-10-30\n- [x] Done")]);
        assert!(sort_tasks(root.path(), "T.md", false).unwrap());
        assert_eq!(read(&root, "T.md"), "- [x] Done\n- [ ] Open 📅 2026-10-30");
        assert!(!sort_tasks(root.path(), "T.md", false).unwrap());
        let today = Day::new(2026, 10, 26).unwrap();
        assert_eq!(postpone(root.path(), "T.md", 1, "week", today).unwrap(), "- [ ] Open 📅 2026-11-06");
        assert!(postpone(root.path(), "T.md", 0, "day", today).is_err());
        assert!(postpone(root.path(), "T.md", 1, "fortnight", today).is_err());
    }

    #[test]
    fn the_daily_note_is_made_from_its_template() {
        let (root, index) = vault(&[("Templates/Day.md", "# {{title}}\n{{date:dddd}}")]);
        fs::write(root.path().join(".obsidian/daily-notes.json"), r#"{"folder":"Journal","template":"Templates/Day"}"#).unwrap();
        let day = Day::new(2026, 10, 26).unwrap();
        let path = daily(root.path(), &index.files, day, false, Clock::default()).unwrap();
        assert_eq!(path, "Journal/2026-10-26.md");
        assert!(!root.path().join(&path).exists());
        daily(root.path(), &index.files, day, true, Clock::default()).unwrap();
        assert_eq!(read(&root, &path), "# 2026-10-26\nMonday");
        // A note that is there is left as it is.
        fs::write(root.path().join(&path), "mine").unwrap();
        assert!(daily(root.path(), &index.files, day, true, Clock::default()).is_err());
        assert_eq!(read(&root, &path), "mine");
    }
}
