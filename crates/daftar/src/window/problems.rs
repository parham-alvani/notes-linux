//! The Problems panel and format on save in the window
//! (`docs/specs/problems-and-format.md`): the checks' hooks, the toolbar's
//! count, the editors' margins, Show Problems and Format Note.

use std::rc::Rc;

use adw::prelude::*;

use daftar_core::{FormatMoment, Vault};

use super::Window;
use crate::editor::Editor;
use crate::problems_view;
use crate::vault_checks::Hooks;

impl Window {
    /// What the checks ask of the window.
    pub(super) fn install_problems(&self) {
        let (save, notice, changed, wrote, settings) =
            (self.clone(), self.toasts.clone(), self.clone(), self.clone(), self.settings.clone());
        self.checks.set_hooks(Hooks {
            save_editors: Box::new(move || save.save_editors()),
            notice: Box::new(move |text| notice.add_toast(crate::toast(text))),
            changed: Box::new(move || changed.problems_changed()),
            wrote: Box::new(move |paths| {
                let Some(store) = wrote.store.borrow().clone() else { return };
                // The app's own fix does not make a note "updated since you read".
                for path in paths {
                    store.places.mark_read(path);
                }
                store.wrote();
            }),
            format_on_save: Box::new(move || settings.borrow().file.bool("editor.format_on_save").unwrap_or(true)),
        });
    }

    /// A new pane's editor: its margin shows the findings, and a note it
    /// lets go is formatted.
    pub(super) fn wire_editor(&self, editor: &Editor) {
        let checks = self.checks.clone();
        editor.set_marks_source(move |path| checks.lines(path));
        let checks = self.checks.clone();
        editor.connect_left(move |path, loaded| checks.format_on_leave(path, loaded, FormatMoment::Closed));
    }

    /// The findings or the checks' state changed.
    fn problems_changed(&self) {
        self.problems.update();
        problems_view::show_count(&self.problems_count, self.checks.counts());
        let tabs = self.tabs.borrow().clone();
        for tab in &tabs {
            if tab.editing() {
                tab.editor.refresh_marks();
            }
        }
    }

    /// Ctrl+Shift+M: the panel under the note, or away.
    pub(super) fn toggle_problems(&self) {
        if self.problems.is_open() {
            self.problems.close();
        } else {
            self.problems.open();
        }
    }

    /// A finding clicked: its note at its line, in the focused pane. An
    /// editor on that note puts its cursor there.
    pub(super) fn open_problem(&self, path: &str, line: Option<usize>) {
        let tab = self.tab();
        if tab.editing() && tab.editor.path().as_deref() == Some(path) {
            if let Some(line) = line {
                tab.editor.show_line(line);
            }
            return;
        }
        self.go(path, None, line, true);
    }

    /// The panel closed: the keys go back to the note.
    pub(super) fn problems_closed(&self) {
        let tab = self.tab();
        if tab.editing() {
            tab.editor.show_line(tab.editor.line());
        } else {
            tab.reader.view.grab_focus();
        }
    }

    /// Ctrl+Alt+Shift+F: the note on screen formatted, in the editor as one
    /// step to undo, in the reader through the editors' gate.
    pub(super) fn format_note(&self) {
        let tab = self.tab();
        match tab.showing().as_deref() {
            Some("editor") if tab.editor.path().is_some_and(|path| Vault::is_note(&path)) => tab.editor.format_note(&self.checks),
            Some("reader") if let Some(shown) = tab.reader.shown().filter(|shown| Vault::is_note(&shown.path)) => {
                self.checks.format_file_now(shown.path);
            }
            _ => self.notice("Open a note first: Format Note formats it."),
        }
    }

    /// The window closes: each note in an editor is formatted as if it were
    /// left, and the app waits for it.
    pub(super) fn format_open_notes(&self) {
        let tabs: Vec<Rc<super::Tab>> = self.tabs.borrow().clone();
        for tab in tabs.iter().filter(|tab| tab.editing()) {
            if let Some((path, loaded)) = tab.editor.leaving() {
                self.checks.format_on_leave(path, loaded, FormatMoment::Quit);
            }
        }
    }
}
