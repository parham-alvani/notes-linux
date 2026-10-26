//! The open vault: its folder, its index and its search, and the one way the
//! app writes a file.

use std::cell::{Ref, RefCell};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use daftar_core::{
    BoardDrop, Bookmarks, CalendarDay, CalendarWeek, Day, Failure, Iconic, SearchIndex, TaskFields, Vault, VaultEdits, VaultIndex,
    VaultTask,
};

use crate::places::PlaceBook;

pub struct VaultStore {
    pub root: PathBuf,
    index: RefCell<VaultIndex>,
    pub search: Arc<SearchIndex>,
    /// Told after the app wrote a note, for the sync that follows an edit.
    wrote: RefCell<Option<Box<dyn Fn()>>>,
    /// Obsidian's `.obsidian/bookmarks.json`.
    pub bookmarks: RefCell<Bookmarks>,
    /// The Iconic plugin's icons and colors.
    pub iconic: RefCell<Iconic>,
    /// Where each note was being read.
    pub places: Rc<PlaceBook>,
}

impl VaultStore {
    pub fn new(root: PathBuf, index: VaultIndex, search: Arc<SearchIndex>) -> VaultStore {
        let (bookmarks, iconic, places) = (Bookmarks::load(&root), Iconic::load(&root), PlaceBook::load(&root));
        VaultStore {
            root,
            index: RefCell::new(index),
            search,
            wrote: RefCell::new(None),
            bookmarks: RefCell::new(bookmarks),
            iconic: RefCell::new(iconic),
            places,
        }
    }

    pub fn connect_wrote(&self, wrote: impl Fn() + 'static) {
        *self.wrote.borrow_mut() = Some(Box::new(wrote));
    }

    pub fn index(&self) -> Ref<'_, VaultIndex> {
        self.index.borrow()
    }

    /// Tells the sync that the app wrote to the vault.
    pub fn wrote(&self) {
        if let Some(wrote) = self.wrote.borrow().as_ref() {
            wrote();
        }
    }

    /// The text of the file at [path], when it is in the vault.
    pub fn text(&self, path: &str) -> Option<String> {
        fs::read(file_in(path, &self.root)?).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Ticks or opens a task. Only a note in the index is written, and only
    /// when its line still holds the task the page showed.
    pub fn toggle_task(&self, path: &str, line: usize, status: char) -> Result<(), Failure> {
        if !self.index().notes.contains_key(path) {
            return Err(Failure::Unreadable);
        }
        VaultEdits::apply(path, &self.root, |text| VaultEdits::toggle_task(text, line, status, Day::today()))?;
        self.refresh(&[path.to_string()]);
        self.wrote();
        Ok(())
    }

    /// A task dropped on a board column. The card's task must still be on its
    /// line, with the same status and words, or nothing is written.
    pub fn move_task(&self, task: &VaultTask, drop: &BoardDrop) -> Result<(), Failure> {
        self.write(&task.path, |text| VaultEdits::move_task(text, task.task.line, &task.task, drop, Day::today()))
    }

    /// The Edit Task dialog's save: refused when the line is no longer [before].
    pub fn edit_task(&self, path: &str, line: usize, before: &TaskFields, fields: &TaskFields) -> Result<(), Failure> {
        self.write(path, |text| VaultEdits::edit_task(text, line, before, fields))
    }

    /// One edit of a note in the index, through `VaultEdits::apply`; the index
    /// reads the note again either way, so a refused edit shows what is there.
    fn write(&self, path: &str, edit: impl FnOnce(&str) -> Result<String, Failure>) -> Result<(), Failure> {
        if !self.index().notes.contains_key(path) || file_in(path, &self.root).is_none() {
            return Err(Failure::Unreadable);
        }
        let result = VaultEdits::apply(path, &self.root, edit);
        self.refresh(&[path.to_string()]);
        if result.is_ok()
            && let Some(wrote) = self.wrote.borrow().as_ref()
        {
            wrote();
        }
        result
    }

    /// The days from [start], [count] of them, in the phone's week notes.
    pub fn calendar_days(&self, start: Day, count: i64) -> Vec<CalendarDay> {
        let mut paths: Vec<String> = Vec::new();
        for offset in 0..count {
            let path = CalendarWeek::path(start.adding_days(offset));
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let end = start.adding_days(count);
        paths
            .iter()
            .filter(|path| self.index().files.contains(*path))
            .filter_map(|path| self.text(path).map(|text| CalendarWeek::parse(&text, path)))
            .flatten()
            .filter(|day| day.day >= start && day.day < end)
            .collect()
    }

    /// Reads the files at [paths] again.
    pub fn refresh(&self, paths: &[String]) {
        let changed: BTreeSet<String> = paths.iter().cloned().collect();
        let updated = Vault::update(&self.index(), &changed, &self.root, Some(&self.search));
        *self.index.borrow_mut() = updated;
    }
}

/// [path] inside [root], or `None` for an absolute path, a `.` or `..` part,
/// or a symlink that points out of the vault.
pub fn file_in(path: &str, root: &Path) -> Option<PathBuf> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    let base = root.canonicalize().ok()?;
    let file = base.join(path);
    if file.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    // A file that is not there yet: its nearest folder that is.
    let mut existing = file.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let resolved = existing.canonicalize().ok()?;
    resolved.starts_with(&base).then_some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_cannot_leave_the_vault() {
        let outside = std::env::temp_dir().join(format!("daftar-outside-{}", std::process::id()));
        let root = outside.join("vault");
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(outside.join("secret.md"), "no").unwrap();
        fs::write(root.join("Notes/a.md"), "yes").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Notes/out")).unwrap();
        assert!(file_in("Notes/a.md", &root).is_some());
        assert!(file_in("Notes/new.md", &root).is_some());
        assert!(file_in("../secret.md", &root).is_none());
        assert!(file_in("Notes/../../secret.md", &root).is_none());
        assert!(file_in("/etc/passwd", &root).is_none());
        assert!(file_in("Notes/out/secret.md", &root).is_none());
        fs::remove_dir_all(&outside).unwrap();
    }
}
