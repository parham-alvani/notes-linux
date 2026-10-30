//! Where each note was being read and what of it was folded (`ReadingPlaces`), kept across launches so
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
        let places = fs::read_to_string(&file).map_or_else(|_| ReadingPlaces::new(), |data| ReadingPlaces::from_data(&data));
        std::rc::Rc::new(PlaceBook { places: RefCell::new(places), file, pending: RefCell::new(None) })
    }

    pub fn get(&self, path: &str) -> Option<usize> {
        self.places.borrow().get(path)
    }

    /// The notes read last, newest first.
    pub fn recently_read(&self, limit: usize) -> Vec<String> {
        self.places.borrow().recently_read(limit)
    }

    /// When the note at [path] was last on screen, in seconds since 1970.
    pub fn last_read(&self, path: &str) -> Option<i64> {
        self.places.borrow().last_read(path)
    }

    /// The note at [path] read now: a change on disk before this is not news.
    pub fn mark_read(self: &std::rc::Rc<Self>, path: &str) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        if self.last_read(path) == Some(now) {
            return;
        }
        self.places.borrow_mut().mark_read(path, now);
        self.later();
    }

    pub fn set(self: &std::rc::Rc<Self>, path: &str, line: usize) {
        if self.get(path) == Some(line) {
            return;
        }
        self.places.borrow_mut().set(path, line);
        self.later();
    }

    /// The sections folded shut in [path], as `level:text`.
    pub fn folds(&self, path: &str) -> Vec<String> {
        self.places.borrow().folds(path)
    }

    pub fn set_folds(self: &std::rc::Rc<Self>, path: &str, folds: Vec<String>) {
        if self.folds(path) == folds {
            return;
        }
        self.places.borrow_mut().set_folds(path, folds);
        self.later();
    }

    /// The headings closed in [path]'s outline, as `level:text`.
    pub fn closed(&self, path: &str) -> Vec<String> {
        self.places.borrow().closed(path)
    }

    pub fn set_closed(self: &std::rc::Rc<Self>, path: &str, closed: Vec<String>) {
        if self.closed(path) == closed {
            return;
        }
        self.places.borrow_mut().set_closed(path, closed);
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

pub use daftar_core::reading_places::state_file;
