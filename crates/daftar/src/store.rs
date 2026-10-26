//! The open vault: its folder, its index and its search, and the one way the
//! app writes a file.

use std::cell::{Ref, RefCell};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use daftar_core::{Day, Failure, SearchIndex, Vault, VaultEdits, VaultIndex};

pub struct VaultStore {
    pub root: PathBuf,
    index: RefCell<VaultIndex>,
    pub search: Arc<SearchIndex>,
    /// Told after the app wrote a note, for the sync that follows an edit.
    wrote: RefCell<Option<Box<dyn Fn()>>>,
}

impl VaultStore {
    pub fn new(root: PathBuf, index: VaultIndex, search: Arc<SearchIndex>) -> VaultStore {
        VaultStore { root, index: RefCell::new(index), search, wrote: RefCell::new(None) }
    }

    pub fn connect_wrote(&self, wrote: impl Fn() + 'static) {
        *self.wrote.borrow_mut() = Some(Box::new(wrote));
    }

    pub fn index(&self) -> Ref<'_, VaultIndex> {
        self.index.borrow()
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
        if let Some(wrote) = self.wrote.borrow().as_ref() {
            wrote();
        }
        Ok(())
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
