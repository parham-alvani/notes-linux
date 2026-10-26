//! Where each note was being read (`ReadingPlaces`), kept across launches so
//! a note opens where it was left: the Mac's `PlaceBook`. One file for each
//! vault in `$XDG_STATE_HOME/daftar`, written a second after the last change
//! and when the window closes.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::glib;

use daftar_core::ReadingPlaces;

pub struct PlaceBook {
    places: RefCell<ReadingPlaces>,
    file: PathBuf,
    pending: RefCell<Option<glib::SourceId>>,
}

impl PlaceBook {
    /// The places of the vault at [root].
    pub fn load(root: &Path) -> std::rc::Rc<PlaceBook> {
        let file = state_file(root, "places");
        let places = fs::read_to_string(&file).map(|data| ReadingPlaces::from_data(&data)).unwrap_or_else(|_| ReadingPlaces::new());
        std::rc::Rc::new(PlaceBook { places: RefCell::new(places), file, pending: RefCell::new(None) })
    }

    pub fn get(&self, path: &str) -> Option<usize> {
        self.places.borrow().get(path)
    }

    /// The notes read last, newest first.
    pub fn recently_read(&self, limit: usize) -> Vec<String> {
        self.places.borrow().recently_read(limit)
    }

    pub fn set(self: &std::rc::Rc<Self>, path: &str, line: usize) {
        if self.get(path) == Some(line) {
            return;
        }
        self.places.borrow_mut().set(path, line);
        self.later();
    }

    /// Files moved: each place goes with its note.
    pub fn moved(self: &std::rc::Rc<Self>, moves: &BTreeMap<String, String>) {
        self.places.borrow_mut().moved(moves);
        self.later();
    }

    /// A file or a folder went to the Trash.
    pub fn forget(self: &std::rc::Rc<Self>, path: &str) {
        self.places.borrow_mut().forget(path);
        self.later();
    }

    fn later(self: &std::rc::Rc<Self>) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
        // The timer needs the main context; a thread without it never waits.
        let context = glib::MainContext::default();
        let Ok(_owner) = context.acquire() else { return };
        let this = std::rc::Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(1), move || {
            if let Some(this) = this.upgrade() {
                this.pending.take();
                this.write();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    /// Now, if anything waits to be written.
    pub fn flush(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
            self.write();
        }
    }

    fn write(&self) {
        if let Some(folder) = self.file.parent() {
            let _ = fs::create_dir_all(folder);
        }
        let temporary = self.file.with_extension("json.tmp");
        if fs::write(&temporary, self.places.borrow().data()).is_ok() {
            let _ = fs::rename(&temporary, &self.file);
        }
    }
}

/// The file of [kind] for the vault at [root] in Daftar's state folder:
/// `places-<hash>.json`.
pub fn state_file(root: &Path, kind: &str) -> PathBuf {
    state_dir().join("daftar").join(format!("{kind}-{:016x}.json", fnv(&root.to_string_lossy())))
}

/// `$XDG_STATE_HOME`, else `~/.local/state`, as the XDG spec has it.
fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local/state"))
}

/// FNV-1a, for a short stable file name from the vault's path.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_per_vault() {
        assert_ne!(fnv("/home/a/vault"), fnv("/home/a/vault2"));
        assert_eq!(fnv(""), 0xcbf29ce484222325);
    }
}
