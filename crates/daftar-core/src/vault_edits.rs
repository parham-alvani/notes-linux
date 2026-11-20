//! The edits the app makes to a vault file. Each one reads the file as it is
//! now and refuses when the file no longer says what the person clicked on,
//! so a click on an old view of a note never changes the wrong line.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// The vault is a read-only clone (`daftar.readonly`): nothing in it is written.
    ReadOnly,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::TaskMoved => write!(f, "the task is not on that line any more"),
            Failure::TextMoved => write!(f, "the text is not on that line any more"),
            Failure::Unreadable => write!(f, "the note could not be read"),
            Failure::Unwritable(why) => write!(f, "the note could not be written: {why}"),
            Failure::ReadOnly => write!(f, "{}", VaultEdits::READ_ONLY),
        }
    }
}

impl std::error::Error for Failure {}

pub struct VaultEdits;

impl VaultEdits {
    /// What a refused write says, in the app and on the command line.
    pub const READ_ONLY: &'static str = "this vault is read-only (daftar.readonly)";

    /// Whether the vault at [root] may be written: refused with `ReadOnly`
    /// in a read-only clone. Every write of a vault file asks this first,
    /// before the file is read (`docs/specs/read-only-vault.md`).
    pub fn writable(root: &Path) -> Result<(), Failure> {
        if crate::Git::read_only(root) { Err(Failure::ReadOnly) } else { Ok(()) }
    }

    /// Makes the file [path] in the vault at [root] with [text], and the
    /// folders it is in: never over a file that is there (`AlreadyExists`),
    /// never outside the vault, never in a read-only clone.
    pub fn create(path: &str, root: &Path, text: &str) -> std::io::Result<()> {
        use std::io::{Error, ErrorKind, Write};
        Self::writable(root).map_err(|e| Error::new(ErrorKind::PermissionDenied, e.to_string()))?;
        let file = Self::within(root, path).ok_or_else(|| Error::new(ErrorKind::InvalidInput, format!("{path} is outside the vault")))?;
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::OpenOptions::new().write(true).create_new(true).open(&file)?.write_all(text.as_bytes())
    }

    /// Deletes the file [path] in the vault at [root]; never outside the
    /// vault, never in a read-only clone.
    pub fn delete(path: &str, root: &Path) -> Result<(), Failure> {
        Self::writable(root)?;
        let file = root.join(path);
        if path.starts_with('/') || path.split('/').any(|part| part == "..") || Self::within(root, path).is_none() {
            return Err(Failure::Unwritable(format!("{path} is outside the vault")));
        }
        fs::remove_file(&file).map_err(|e| Failure::Unwritable(e.to_string()))
    }

    /// Ticks or opens the task on [line] of [text]. [status] is what the view
    /// showed, which the line must still hold.
    pub fn toggle_task(text: &str, line: usize, status: char, expected: Option<&str>, today: Day) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let raw = lines.get(line).ok_or(Failure::TaskMoved)?;
        let crlf = raw.ends_with('\r');
        let clean = raw.strip_suffix('\r').unwrap_or(raw);
        let task = TaskLine::parse(clean, line, "").ok_or(Failure::TaskMoved)?;
        // Another task with the same status that moved onto the line is not this one.
        if task.status != status || expected.is_some_and(|text| text.trim() != task.text.trim()) {
            return Err(Failure::TaskMoved);
        }
        let replacement = TaskLine::toggled(clean, &today.to_string()).ok_or(Failure::TaskMoved)?;
        let replacement = replacement.into_iter().map(|l| if crlf { l + "\r" } else { l });
        lines.splice(line..=line, replacement);
        Ok(lines.join("\n"))
    }

    /// The task on [line] of [text] written from [fields]. [before] is the
    /// task the form was opened on, which the line must still hold. A new
    /// status is set as `TaskLine::with_status` sets it: its dates, a repeat,
    /// and the old reason gone unless the form gave a new one.
    pub fn edit_task(text: &str, line: usize, before: &TaskFields, fields: &TaskFields) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let raw = lines.get(line).ok_or(Failure::TaskMoved)?;
        let crlf = raw.ends_with('\r');
        let clean = raw.strip_suffix('\r').unwrap_or(raw);
        if TaskLine::fields(clean).as_ref() != Some(before) {
            return Err(Failure::TaskMoved);
        }
        let status = fields.status;
        let written = TaskFields { status: before.status, reason: before.reason.clone(), ..fields.clone() };
        let edited = TaskLine::edited(clean, &written).ok_or(Failure::TaskMoved)?;
        let reason = (fields.reason != before.reason).then(|| fields.reason.clone().unwrap_or_default());
        let replacement = match TaskLine::with_status(&edited, status, reason.as_deref(), &Day::today().to_string()) {
            Some(lines) => lines,
            None => vec![edited],
        };
        lines.splice(line..=line, replacement.into_iter().map(|l| if crlf { l + "\r" } else { l }));
        Ok(lines.join("\n"))
    }

    /// Applies [edit] to the file at [path] in [root], written in place
    /// through a temporary file, so a crash never leaves half a note.
    pub fn apply(path: &str, root: &Path, edit: impl FnOnce(&str) -> Result<String, Failure>) -> Result<(), Failure> {
        Self::writable(root)?;
        // A symlinked note is written where it points, so it stays a link,
        // and only while that is in the vault.
        let file = Self::within(root, path).ok_or_else(|| Failure::Unwritable(format!("{path} is outside the vault")))?;
        let text = fs::read_to_string(&file).map_err(|_| Failure::Unreadable)?;
        let result = edit(&text)?;
        if result == text {
            return Ok(());
        }
        let temporary =
            file.with_file_name(format!(".{}.daftar-{}", file.file_name().and_then(|n| n.to_str()).unwrap_or("note"), std::process::id()));
        fs::write(&temporary, result).map_err(|e| Failure::Unwritable(e.to_string()))?;
        // The note keeps who may read it.
        if let Ok(metadata) = fs::metadata(&file) {
            let _ = fs::set_permissions(&temporary, metadata.permissions());
        }
        fs::rename(&temporary, &file).map_err(|e| {
            let _ = fs::remove_file(&temporary);
            Failure::Unwritable(e.to_string())
        })
    }

    /// What `apply` would do, written nowhere: the file is read and [edit]
    /// runs on it, so a dry run says what would be refused. A read-only
    /// vault is looked at as any other.
    pub fn look(path: &str, root: &Path, edit: impl FnOnce(&str) -> Result<String, Failure>) -> Result<(), Failure> {
        let file = Self::within(root, path).ok_or_else(|| Failure::Unwritable(format!("{path} is outside the vault")))?;
        let text = fs::read_to_string(&file).map_err(|_| Failure::Unreadable)?;
        edit(&text).map(|_| ())
    }

    /// The file [path] names in the vault at [root], its links followed as
    /// far as the file or folder that is there; `None` when that leads out
    /// of the vault. A file not made yet is under the folder it would be in.
    pub fn within(root: &Path, path: &str) -> Option<PathBuf> {
        let root = root.canonicalize().ok()?;
        let mut there = root.join(path);
        let mut rest = Vec::new();
        while fs::symlink_metadata(&there).is_err() {
            rest.push(there.file_name()?.to_owned());
            there = there.parent()?.to_path_buf();
        }
        let mut resolved = there.canonicalize().ok()?;
        if !resolved.starts_with(&root) {
            return None;
        }
        for part in rest.iter().rev() {
            if part == ".." {
                return None;
            }
            resolved.push(part);
        }
        Some(resolved)
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

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_vault_is_never_written() {
        let base = tempfile::tempdir().unwrap();
        let (vault, outside) = (base.path().join("vault"), base.path().join("outside"));
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("n.md"), "- [ ] a\n").unwrap();
        std::os::unix::fs::symlink(&outside, vault.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.join("n.md"), vault.join("n.md")).unwrap();
        assert_eq!(VaultEdits::within(&vault, "linked/new.md"), None);
        assert_eq!(VaultEdits::within(&vault, "n.md"), None);
        assert!(VaultEdits::within(&vault, "Notes/new.md").is_some_and(|p| p.ends_with("Notes/new.md")));
        let toggle = |text: &str| VaultEdits::toggle_task(text, 0, ' ', None, day("2026-10-05"));
        assert!(matches!(VaultEdits::apply("linked/n.md", &vault, toggle), Err(Failure::Unwritable(_))));
        assert_eq!(fs::read_to_string(outside.join("n.md")).unwrap(), "- [ ] a\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_note_keeps_its_permissions_and_its_link() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/a.md"), "- [ ] a\n").unwrap();
        fs::set_permissions(root.path().join("real/a.md"), fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink("real/a.md", root.path().join("a.md")).unwrap();
        VaultEdits::apply("a.md", root.path(), |text| VaultEdits::toggle_task(text, 0, ' ', None, day("2026-10-05"))).unwrap();
        assert!(fs::symlink_metadata(root.path().join("a.md")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(root.path().join("real/a.md")).unwrap(), "- [x] a ✅ 2026-10-05\n");
        assert_eq!(fs::metadata(root.path().join("real/a.md")).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_read_only_vault_refuses_every_write() {
        use crate::{Bookmarks, Clock, Git, MoveFailure, QuickCapture, Vault, VaultMoves, WebClip};
        use std::collections::BTreeMap;
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("vault");
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Notes/Tasks.md"), "# Tasks\n- [ ] One\n").unwrap();
        fs::write(root.join("Inbox.md"), "# Inbox\n").unwrap();
        let git = Git::new(&root);
        for arguments in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "test@example.com"],
            &["config", "user.name", "Test"],
            &["config", "commit.gpgsign", "false"],
            &["config", "core.autocrlf", "false"],
            &["add", "-A"],
            &["commit", "-qm", "start"],
            &["config", "daftar.readonly", "true"],
        ] {
            git.run(arguments).unwrap();
        }
        let bytes = || -> BTreeMap<String, Vec<u8>> {
            walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_entry(|entry| entry.file_name() != ".git")
                .flatten()
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| (entry.path().strip_prefix(&root).unwrap().to_string_lossy().into_owned(), fs::read(entry.path()).unwrap()))
                .collect()
        };
        let before = bytes();
        assert_eq!(before.len(), 2);
        assert_eq!(VaultEdits::writable(&root), Err(Failure::ReadOnly));
        // A write: refused before the file is read, so even a note that is not there says read-only.
        assert_eq!(VaultEdits::apply("Inbox.md", &root, |_| Ok("# Changed\n".into())), Err(Failure::ReadOnly));
        assert_eq!(VaultEdits::apply("Gone.md", &root, |_| Ok("x".into())), Err(Failure::ReadOnly));
        assert_eq!(VaultEdits::apply_all(&["Inbox.md".to_string()], &root, |_| "x".into()), [Err(Failure::ReadOnly)]);
        // A task tick.
        let tick = |text: &str| VaultEdits::toggle_task(text, 1, ' ', Some("One"), day("2026-10-05"));
        assert_eq!(VaultEdits::apply("Notes/Tasks.md", &root, tick), Err(Failure::ReadOnly));
        // A capture, into a note that is there and into one it would make.
        let capture = |target: &str| {
            QuickCapture::capture(&root, target, "", "a thought", day("2026-10-05"), Clock { hour: 9, minute: 30, second: 0 })
        };
        assert_eq!(capture("Inbox.md"), Err(VaultEdits::READ_ONLY.to_string()));
        assert_eq!(capture("daily"), Err(VaultEdits::READ_ONLY.to_string()));
        // A new note.
        assert_eq!(VaultEdits::create("Notes/New.md", &root, "# New\n").unwrap_err().to_string(), VaultEdits::READ_ONLY);
        let mut files = Vault::index(&root, None).files;
        assert_eq!(WebClip::create(&root, "Clips", "Page", "md", b"# Page\n", &mut files).unwrap_err().to_string(), VaultEdits::READ_ONLY);
        // A move and a delete.
        let index = Vault::index(&root, None);
        assert_eq!(VaultMoves::perform("Inbox.md", "Notes/Inbox.md", &index, &root), Err(MoveFailure::ReadOnly));
        assert_eq!(VaultMoves::perform("Notes", "Archive", &index, &root), Err(MoveFailure::ReadOnly));
        assert_eq!(VaultEdits::delete("Inbox.md", &root), Err(Failure::ReadOnly));
        assert!(Bookmarks::default().save(&root).is_err());
        // Every file has the bytes it had, and git sees no change.
        assert_eq!(bytes(), before);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), "");
        // Turned off, the same writes go through.
        git.set_read_only(false).unwrap();
        VaultEdits::apply("Notes/Tasks.md", &root, tick).unwrap();
        VaultEdits::create("Notes/New.md", &root, "# New\n").unwrap();
        assert_eq!(VaultEdits::create("Notes/New.md", &root, "again").unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert!(VaultEdits::create("../out.md", &root, "x").is_err());
        VaultEdits::delete("Notes/New.md", &root).unwrap();
        assert!(matches!(VaultEdits::delete("../vault/Inbox.md", &root), Err(Failure::Unwritable(_))));
        assert_eq!(fs::read_to_string(root.join("Notes/Tasks.md")).unwrap(), "# Tasks\n- [x] One ✅ 2026-10-05\n");
        assert!(!root.join("Notes/New.md").exists());
    }

    #[test]
    fn a_folder_that_is_no_repository_is_written() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(VaultEdits::writable(root.path()), Ok(()));
    }

    fn day(text: &str) -> Day {
        Day::parse(text).unwrap()
    }

    #[test]
    fn toggle_checks_the_line_still_says_the_same() {
        let text = "# T\n- [ ] One\n- [ ] Two\n";
        assert_eq!(VaultEdits::toggle_task(text, 2, ' ', None, day("2026-10-05")).unwrap(), "# T\n- [ ] One\n- [x] Two ✅ 2026-10-05\n");
        assert_eq!(VaultEdits::toggle_task(text, 0, ' ', None, day("2026-10-05")), Err(Failure::TaskMoved));
        assert_eq!(VaultEdits::toggle_task(text, 1, 'x', None, day("2026-10-05")), Err(Failure::TaskMoved));
        assert_eq!(VaultEdits::toggle_task(text, 9, ' ', None, day("2026-10-05")), Err(Failure::TaskMoved));
        // The text the page showed must still be the line's.
        assert_eq!(VaultEdits::toggle_task(text, 2, ' ', Some("One"), day("2026-10-05")), Err(Failure::TaskMoved));
        assert!(VaultEdits::toggle_task(text, 2, ' ', Some("Two"), day("2026-10-05")).is_ok());
    }

    #[test]
    fn the_form_sets_a_status_and_its_reason() {
        let text = "- [/] Ship [reason:: review] 📅 2026-11-02\n";
        let before = TaskLine::fields("- [/] Ship [reason:: review] 📅 2026-11-02").unwrap();
        let today = Day::today().to_string();
        let cancel = TaskFields { status: '-', reason: Some("no budget".into()), ..before.clone() };
        assert_eq!(
            VaultEdits::edit_task(text, 0, &before, &cancel).unwrap(),
            format!("- [-] Ship [reason:: no budget] 📅 2026-11-02 ❌ {today}\n")
        );
        // A new status with the reason untouched leaves the old one behind.
        let open = TaskFields { status: ' ', ..before.clone() };
        assert_eq!(VaultEdits::edit_task(text, 0, &before, &open).unwrap(), "- [ ] Ship 📅 2026-11-02\n");
        let cleared = TaskFields { reason: None, ..before.clone() };
        assert_eq!(VaultEdits::edit_task(text, 0, &before, &cleared).unwrap(), "- [/] Ship 📅 2026-11-02\n");
    }

    #[test]
    fn windows_line_endings_survive() {
        assert_eq!(VaultEdits::toggle_task("- [ ] One\r\n", 0, ' ', None, day("2026-10-05")).unwrap(), "- [x] One ✅ 2026-10-05\r\n");
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
        VaultEdits::apply("a.md", root.path(), |text| VaultEdits::toggle_task(text, 0, ' ', None, day("2026-10-05"))).unwrap();
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
