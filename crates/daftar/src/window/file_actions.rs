//! The window's file commands, bookmarks and reading places: the actions
//! the menus, the sidebar's context menu and the keys run, each on the open
//! note when it names no path.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use super::Window;
use crate::files::{self, Action, Done};
use crate::store::VaultStore;

impl Window {
    /// After a vault is open: its icons and bookmarks in the sidebar, and the
    /// reading cursor kept as the note's place.
    pub(super) fn after_load(&self, store: &Rc<VaultStore>) {
        self.sidebar.set_iconic(store.iconic.borrow().clone());
        self.sidebar.set_tree(store.index().tree());
        self.refresh_marks();
        let places = store.places.clone();
        self.reader.connect_cursor(move |path, line| places.set(path, line));
    }

    /// The bookmarks again, as the files are now.
    pub(super) fn refresh_marks(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let entries = store.bookmarks.borrow().entries();
        self.sidebar.set_bookmarks(&entries, &store.index().files);
    }

    /// Where the note at [path] was being read, when nothing names a place.
    pub(super) fn place(&self, path: &str) -> Option<usize> {
        self.store.borrow().as_ref().and_then(|store| store.places.get(path))
    }

    /// The note on screen, in the reader or the editor.
    fn current(&self) -> Option<String> {
        match self.stack.visible_child_name().as_deref() {
            Some("reader") => self.reader.shown().map(|shown| shown.path),
            Some("editor") => self.editor.path(),
            _ => None,
        }
    }

    /// A file command, on [target] or else the note on screen.
    fn file_action(&self, name: &str, target: String) {
        let Some(store) = self.store.borrow().clone() else { return };
        let target = Some(target).filter(|t| !t.is_empty());
        let current = self.current();
        let folder_of_current = current.as_deref().map(files::parent).unwrap_or("").to_string();
        let action = match name {
            "new-note" => Action::NewNote { folder: target.unwrap_or(folder_of_current) },
            "new-folder" => Action::NewFolder { parent: target.unwrap_or(folder_of_current) },
            _ => {
                let Some(path) = target.or(current) else {
                    self.toasts.add_toast(adw::Toast::new("Open a note first, or right-click a file in the sidebar."));
                    return;
                };
                match name {
                    "rename" => Action::Rename { path },
                    "move" => Action::Move { path },
                    "trash" => Action::Trash { path },
                    _ => return,
                }
            }
        };
        // What the editor holds is on disk before a file moves.
        if !matches!(action, Action::NewNote { .. } | Action::NewFolder { .. }) && !self.leave_editor() {
            return;
        }
        let (this, toasts) = (self.clone(), self.toasts.clone());
        files::present(
            &self.window,
            action,
            store,
            move |done| this.files_done(done),
            move |error| toasts.add_toast(adw::Toast::new(&error)),
        );
    }

    /// After a file command: the index, the tree and the page follow, and a
    /// new note opens in the editor, as on the Mac.
    fn files_done(&self, done: Done) {
        let Some(store) = self.store.borrow().clone() else { return };
        let shown = self.reader.shown().map(|shown| shown.path);
        self.sidebar.set_iconic(store.iconic.borrow().clone());
        self.files_changed(done.changed.clone());
        self.refresh_marks();
        if done.moves.is_empty() {
            if let Some(path) = done.open {
                self.open(&path, None);
                self.toggle_edit();
            }
            return;
        }
        // The note on screen moved: it stays on screen, under its new name.
        if let Some(moved) = shown.and_then(|path| done.moves.get(&path).cloned()) {
            self.go(&moved, None, None, false);
        }
        if done.moves.len() > 1 {
            self.toasts.add_toast(adw::Toast::new(&format!("Moved {} files; the links to them follow.", done.moves.len())));
        }
    }

    /// Ctrl+D: bookmarks [target] or the note on screen, or takes it away.
    fn bookmark(&self, target: String) {
        let Some(store) = self.store.borrow().clone() else { return };
        let Some(path) = Some(target).filter(|t| !t.is_empty()).or_else(|| self.current()) else { return };
        match files::toggle_bookmark(&store, &path) {
            Ok(on) => {
                let title = daftar_core::Vault::title(&path);
                self.toasts.add_toast(adw::Toast::new(&if on {
                    format!("Bookmarked {title}.")
                } else {
                    format!("Removed the bookmark of {title}.")
                }));
                store.wrote();
            }
            Err(error) => self.toasts.add_toast(adw::Toast::new(&format!("The bookmarks were not saved: {error}"))),
        }
        self.refresh_marks();
    }

    pub(super) fn add_file_actions(&self) {
        for name in ["new-note", "new-folder", "rename", "move", "trash"] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let this = self.clone();
            action.connect_activate(move |action, target| {
                let target = target.and_then(|t| t.get::<String>()).unwrap_or_default();
                this.file_action(&action.name(), target);
            });
            self.window.add_action(&action);
        }
        let bookmark = gio::SimpleAction::new("bookmark", Some(glib::VariantTy::STRING));
        let this = self.clone();
        bookmark.connect_activate(move |_, target| this.bookmark(target.and_then(|t| t.get::<String>()).unwrap_or_default()));
        self.window.add_action(&bookmark);
        // The places are written before the window goes.
        let store = self.store.clone();
        self.window.connect_close_request(move |_| {
            if let Some(store) = store.borrow().as_ref() {
                store.places.flush();
            }
            glib::Propagation::Proceed
        });
    }
}
