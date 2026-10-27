//! The window's commands from the Mac's 0.4.0: the periodic notes (Ctrl+3,
//! and Ctrl+Alt+Left and Right through them), sorting a note's tasks,
//! renaming a tag, Find and Replace in the Vault (Ctrl+Shift+H), Postpone,
//! and the notes updated since they were read.

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{BoardDrop, Day, Postpone, Vault};

use super::Window;
use crate::vault_dialogs;

impl Window {
    pub(super) fn add_vault_tools(&self) {
        let periodic = gio::SimpleAction::new("periodic", None);
        let this = self.clone();
        periodic.connect_activate(move |_, _| this.open_periodic());
        self.window.add_action(&periodic);
        for (name, step) in [("periodic-previous", -1), ("periodic-next", 1)] {
            let action = gio::SimpleAction::new(name, None);
            let this = self.clone();
            action.connect_activate(move |_, _| this.periodic_neighbour(step));
            self.window.add_action(&action);
        }
        let sort = gio::SimpleAction::new("sort-tasks", None);
        let this = self.clone();
        sort.connect_activate(move |_, _| this.sort_tasks());
        self.window.add_action(&sort);
        let rename_tag = gio::SimpleAction::new("rename-tag", Some(glib::VariantTy::STRING));
        let this = self.clone();
        rename_tag.connect_activate(move |_, target| {
            let tag = target.and_then(|t| t.get::<String>()).unwrap_or_default();
            this.rename_tag(&tag);
        });
        self.window.add_action(&rename_tag);
        let replace = gio::SimpleAction::new("replace-in-vault", None);
        let this = self.clone();
        replace.connect_activate(move |_, _| this.replace_in_vault());
        self.window.add_action(&replace);
        // `path`, a tab, the line from 0, a tab and the amount's name.
        let postpone = gio::SimpleAction::new("postpone", Some(glib::VariantTy::STRING));
        let this = self.clone();
        postpone.connect_activate(move |_, target| {
            let Some(target) = target.and_then(|t| t.get::<String>()) else { return };
            let mut parts = target.split('\t');
            if let (Some(path), Some(Ok(line)), Some(Some(amount))) =
                (parts.next(), parts.next().map(str::parse::<usize>), parts.next().map(Postpone::from_name))
            {
                this.postpone(path, line, amount);
            }
        });
        self.window.add_action(&postpone);
        // A path, or "" for every note in the list.
        let mark_read = gio::SimpleAction::new("mark-read", Some(glib::VariantTy::STRING));
        let this = self.clone();
        mark_read.connect_activate(move |_, target| {
            let path = target.and_then(|t| t.get::<String>()).unwrap_or_default();
            this.mark_read(&path);
        });
        self.window.add_action(&mark_read);
    }

    /// The commands of this file, for the command palette.
    pub(super) fn vault_tool_commands(&self) -> Vec<crate::palette::Command> {
        let command = |title, group, action, keys| crate::palette::Command { title, group, action, keys };
        let Some(store) = self.store.borrow().clone() else { return vec![] };
        let period = store.periodic.period;
        let (previous, next) = match period {
            daftar_core::Period::Day => ("Previous Day's Note", "Next Day's Note"),
            daftar_core::Period::Week => ("Previous Week's Note", "Next Week's Note"),
            daftar_core::Period::Month => ("Previous Month's Note", "Next Month's Note"),
            daftar_core::Period::Quarter => ("Previous Quarter's Note", "Next Quarter's Note"),
            daftar_core::Period::Year => ("Previous Year's Note", "Next Year's Note"),
        };
        vec![
            command(period.current(), "Go", "win.periodic", "Ctrl+3"),
            command(previous, "Go", "win.periodic-previous", "Ctrl+Alt+Left"),
            command(next, "Go", "win.periodic-next", "Ctrl+Alt+Right"),
            command("Sort Tasks (Done First)", "Note", "win.sort-tasks", ""),
            command("Rename a Tag…", "Edit", "win.rename-tag::", ""),
            command("Find and Replace in the Vault", "Edit", "win.replace-in-vault", "Ctrl+Shift+H"),
            command("Mark All as Read", "Go", "win.mark-read::", ""),
        ]
    }

    /// Ctrl+3: the periodic note of today's period, made from the Daily
    /// notes plugin's template when it is not there yet.
    fn open_periodic(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        match store.periodic_note(Day::today()) {
            Ok(path) => {
                self.sidebar.set_tree(store.index().tree());
                self.open(&path, None);
            }
            Err(error) => self.toasts.add_toast(adw::Toast::new(&format!("The note could not be made: {error}"))),
        }
    }

    /// The periodic note nearest the open one, before or after it: a
    /// journal kept in fits and starts has gaps.
    fn periodic_neighbour(&self, step: i32) {
        let Some(store) = self.store.borrow().clone() else { return };
        let Some(current) = self.tab().current() else { return };
        let found = store.periodic.nearest(&store.index().files, &current, step);
        match found {
            Some(path) => self.open(&path, None),
            None if store.periodic.period_of(&current).is_none() => {
                self.toasts.add_toast(adw::Toast::new("The open note is not a periodic note."));
            }
            None => self.toasts.add_toast(adw::Toast::new(if step < 0 { "No note before this one." } else { "No note after this one." })),
        }
    }

    /// The open note's tasks sorted as the vault's Task Sorter plugin does:
    /// in the editor as a step to undo, else in the file.
    fn sort_tasks(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tab = self.tab();
        if tab.editing() {
            if !tab.editor.sort_tasks() {
                self.toasts.add_toast(adw::Toast::new("The tasks are in order already."));
            }
            return;
        }
        let Some(path) = tab.current().filter(|p| Vault::is_note(p)) else { return };
        match store.sort_tasks(&path) {
            Ok(true) => tab.reader.reload(),
            Ok(false) => self.toasts.add_toast(adw::Toast::new("The tasks are in order already.")),
            Err(error) => self.toasts.add_toast(adw::Toast::new(&format!("The tasks could not be sorted: {error}."))),
        }
    }

    fn rename_tag(&self, tag: &str) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        let this = self.clone();
        vault_dialogs::rename_tag(&self.window, store, tag, move |text| {
            this.toasts.add_toast(adw::Toast::new(&text));
            this.redraw_page();
        });
    }

    fn replace_in_vault(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        // What the editor holds is on disk before the search reads it.
        if !self.leave_editor() {
            return;
        }
        let this = self.clone();
        vault_dialogs::replace_in_vault(&self.window, store, "", move |text| {
            this.toasts.add_toast(adw::Toast::new(&text));
            this.redraw_page();
        });
    }

    /// The task on [line] of [path] postponed by [amount], as the Tasks
    /// plugin's menu does; refused when its line changed.
    fn postpone(&self, path: &str, line: usize, amount: Postpone) {
        let Some(store) = self.store.borrow().clone() else { return };
        let task = store.index().tasks().into_iter().find(|t| t.path == path && t.task.line == line);
        match task {
            Some(task) => self.move_task(&task, &BoardDrop::Postpone(amount)),
            None => self.toasts.add_toast(adw::Toast::new("The task is not on that line any more.")),
        }
    }

    /// The note at [path] read now, or every note in the list for "".
    fn mark_read(&self, path: &str) {
        let Some(store) = self.store.borrow().clone() else { return };
        let paths: Vec<String> = match path {
            "" => store.updated_since_read().into_iter().map(|(path, _)| path).collect(),
            path => vec![path.to_string()],
        };
        for path in &paths {
            store.places.mark_read(path);
        }
        self.refresh_updated();
    }

    /// The sidebar's list of notes changed since they were read.
    pub(super) fn refresh_updated(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        self.sidebar.set_updated(&store.updated_since_read());
    }
}
