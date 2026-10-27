//! The edits the app makes to a vault file. Each one reads the file as it is
//! now and refuses when the file no longer says what the person clicked on,
//! so a click on an old view of a note never changes the wrong line.

use std::fmt;
use std::fs;
use std::path::Path;

use rayon::prelude::*;

use crate::day::Day;
use crate::task_line::{TaskFields, TaskLine};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The line is not the task that was clicked: the file changed under the view.
    TaskMoved,
    /// The text an edit was about is not where it was found any more.
    TextMoved,
    Unreadable,
    Unwritable(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::TaskMoved => write!(f, "the task is not on that line any more"),
            Failure::TextMoved => write!(f, "the text is not on that line any more"),
            Failure::Unreadable => write!(f, "the note could not be read"),
            Failure::Unwritable(why) => write!(f, "the note could not be written: {why}"),
        }
    }
}

impl std::error::Error for Failure {}

pub struct VaultEdits;

impl VaultEdits {
    /// Ticks or opens the task on [line] of [text]. [status] is what the view
    /// showed, which the line must still hold.
    pub fn toggle_task(text: &str, line: usize, status: char, today: Day) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let raw = lines.get(line).ok_or(Failure::TaskMoved)?;
        let crlf = raw.ends_with('\r');
        let clean = raw.strip_suffix('\r').unwrap_or(raw);
        let task = TaskLine::parse(clean, line, "").ok_or(Failure::TaskMoved)?;
        if task.status != status {
            return Err(Failure::TaskMoved);
        }
        let replacement = TaskLine::toggled(clean, &today.to_string()).ok_or(Failure::TaskMoved)?;
        let replacement = replacement.into_iter().map(|l| if crlf { l + "\r" } else { l });
        lines.splice(line..=line, replacement);
        Ok(lines.join("\n"))
    }

    /// The task on [line] of [text] written from [fields]. [before] is the
    /// task the form was opened on, which the line must still hold.
    pub fn edit_task(text: &str, line: usize, before: &TaskFields, fields: &TaskFields) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let raw = lines.get(line).ok_or(Failure::TaskMoved)?;
        let crlf = raw.ends_with('\r');
        let clean = raw.strip_suffix('\r').unwrap_or(raw);
        if TaskLine::fields(clean).as_ref() != Some(before) {
            return Err(Failure::TaskMoved);
        }
        let replacement = TaskLine::edited(clean, fields).ok_or(Failure::TaskMoved)?;
        lines[line] = if crlf { replacement + "\r" } else { replacement };
        Ok(lines.join("\n"))
    }

    /// Applies [edit] to the file at [path] in [root], written in place
    /// through a temporary file, so a crash never leaves half a note.
    pub fn apply(path: &str, root: &Path, edit: impl FnOnce(&str) -> Result<String, Failure>) -> Result<(), Failure> {
        let file = root.join(path);
        let text = fs::read_to_string(&file).map_err(|_| Failure::Unreadable)?;
        let result = edit(&text)?;
        if result == text {
            return Ok(());
        }
        let temporary =
            file.with_file_name(format!(".{}.daftar-{}", file.file_name().and_then(|n| n.to_str()).unwrap_or("note"), std::process::id()));
        fs::write(&temporary, result).map_err(|e| Failure::Unwritable(e.to_string()))?;
        fs::rename(&temporary, &file).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            Failure::Unwritable(e.to_string())
        })
    }

    /// [edit] applied to each of [paths], several files at a time: a tag
    /// rename can write thousands of notes. For each path, in order, whether
    /// its text changed, or what stopped it.
    pub fn apply_all(paths: &[String], root: &Path, edit: impl Fn(&str) -> String + Sync) -> Vec<Result<bool, Failure>> {
        paths
            .par_iter()
            .map(|path| {
                let mut changed = false;
                Self::apply(path, root, |text| {
                    let result = edit(text);
                    changed = result != text;
                    Ok(result)
                })
                .map(|()| changed)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        Day::parse(text).unwrap()
    }

    #[test]
    fn toggle_checks_the_line_still_says_the_same() {
        let text = "# T\n- [ ] One\n- [ ] Two\n";
        assert_eq!(VaultEdits::toggle_task(text, 2, ' ', day("2026-10-05")).unwrap(), "# T\n- [ ] One\n- [x] Two ✅ 2026-10-05\n");
        assert_eq!(VaultEdits::toggle_task(text, 0, ' ', day("2026-10-05")), Err(Failure::TaskMoved));
        assert_eq!(VaultEdits::toggle_task(text, 1, 'x', day("2026-10-05")), Err(Failure::TaskMoved));
        assert_eq!(VaultEdits::toggle_task(text, 9, ' ', day("2026-10-05")), Err(Failure::TaskMoved));
    }

    #[test]
    fn windows_line_endings_survive() {
        assert_eq!(VaultEdits::toggle_task("- [ ] One\r\n", 0, ' ', day("2026-10-05")).unwrap(), "- [x] One ✅ 2026-10-05\r\n");
    }

    #[test]
    fn the_edit_is_refused_when_the_line_changed() {
        let text = "# Tasks\n- [ ] One 📅 2026-10-30\n- [ ] Two\n";
        let before = TaskLine::fields("- [ ] One 📅 2026-10-30").unwrap();
        let mut after = before.clone();
        after.due = Some("2026-11-01".into());
        assert_eq!(VaultEdits::edit_task(text, 1, &before, &after).unwrap(), "# Tasks\n- [ ] One 📅 2026-11-01\n- [ ] Two\n");
        assert_eq!(VaultEdits::edit_task(text, 2, &before, &after), Err(Failure::TaskMoved));
    }

    #[test]
    fn apply_writes_the_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "- [ ] One\n").unwrap();
        VaultEdits::apply("a.md", root.path(), |text| VaultEdits::toggle_task(text, 0, ' ', day("2026-10-05"))).unwrap();
        assert_eq!(fs::read_to_string(root.path().join("a.md")).unwrap(), "- [x] One ✅ 2026-10-05\n");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn apply_all_says_what_changed() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.md"), "#old here").unwrap();
        fs::write(root.path().join("b.md"), "nothing").unwrap();
        let paths = ["a.md", "b.md", "gone.md"].map(String::from);
        let results = VaultEdits::apply_all(&paths, root.path(), |text| text.replace("#old", "#new"));
        assert_eq!(results, [Ok(true), Ok(false), Err(Failure::Unreadable)]);
        assert_eq!(fs::read_to_string(root.path().join("a.md")).unwrap(), "#new here");
    }
}
