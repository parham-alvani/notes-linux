//! The window's commands from notes-mac 0.4: the note's history, the sync
//! conflicts, read aloud and Export as PDF, each on the chosen tab's note.

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{Vault, VaultEdits};

use super::Window;
use crate::palette;

impl Window {
    pub(super) fn add_extra_actions(&self) {
        for (name, run) in [
            ("history", Window::history as fn(&Window)),
            ("conflicts", Window::conflicts),
            ("read-aloud", Window::read_aloud),
            ("export-pdf", Window::export_pdf),
            ("neovide", Window::open_in_neovide),
            ("clip", Window::ask_clip),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let this = self.clone();
            action.connect_activate(move |_, _| run(&this));
            self.window.add_action(&action);
        }
        // A sync that stopped on a conflict, or one found open, opens them: in
        // this window when the person ran it, else in the window in front.
        let this = self.clone();
        self.git.connect_stopped(move |asked| {
            let front = this.window.application().and_then(|app| app.active_window());
            if asked || front.is_none_or(|front| &front == this.window.upcast_ref::<gtk::Window>()) {
                this.conflicts();
            }
        });
    }

    /// The palette's lines for these commands.
    pub(super) fn extra_commands(&self) -> Vec<palette::Command> {
        let command = |title, group, action, keys| palette::Command { title, group, action, keys };
        let mut commands = vec![
            command(
                if crate::read_aloud::is_speaking() { "Stop Reading Aloud" } else { "Read the Note Aloud" },
                "Note",
                "win.read-aloud",
                "Ctrl+Alt+R",
            ),
            command("Export the Note as PDF", "Note", "win.export-pdf", "Ctrl+Shift+E"),
            command("Quick Capture", "File", "app.capture", ""),
            command("Clip Web Page…", "File", "win.clip", "Ctrl+Shift+L"),
            command(
                if self.hidden_settings().1 { "Hide Hidden Files" } else { "Show Hidden Files" },
                "View",
                "win.show-hidden",
                "Ctrl+Shift+.",
            ),
        ];
        if neovide().is_some() {
            commands.push(command("Open in Neovide", "Note", "win.neovide", ""));
        }
        if self.git.button.is_visible() {
            commands.push(command("Show the Note's History", "Git", "win.history", "Ctrl+Alt+Y"));
        }
        if self.git.is_rebasing() {
            commands.push(command("Resolve Sync Conflicts", "Git", "win.conflicts", ""));
        }
        commands
    }

    /// Clip Web Page…: an address, from the clipboard when it holds one,
    /// and the page as a new note in Clippings.
    fn ask_clip(&self) {
        let entry = gtk::Entry::builder().placeholder_text("https://…").activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading("Clip Web Page")
            .body("The page's text becomes a new note in Clippings, its pictures saved in the vault.")
            .extra_child(&entry)
            .default_response("clip")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("clip", "Clip")]);
        dialog.set_response_appearance("clip", adw::ResponseAppearance::Suggested);
        {
            let entry = entry.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = entry.clipboard().read_text_future().await {
                    let text = text.trim().to_string();
                    if (text.starts_with("http://") || text.starts_with("https://")) && entry.text().is_empty() {
                        entry.set_text(&text);
                    }
                }
            });
        }
        let this = self.clone();
        {
            let entry = entry.clone();
            dialog.connect_response(Some("clip"), move |_, _| {
                let url = entry.text().trim().to_string();
                if !url.is_empty() {
                    this.clip(&url);
                }
            });
        }
        dialog.present(Some(&self.window));
        entry.grab_focus();
    }

    /// The page at [url] as a new note, fetched off the window; the note opens.
    pub(crate) fn clip(&self, url: &str) {
        let Some(store) = self.store.borrow().clone() else { return };
        let (root, url) = (store.root.clone(), url.to_string());
        self.notice(&format!("Clipping {url}…"));
        let this = self.clone();
        glib::spawn_future_local(async move {
            let agent = concat!("Daftar/", env!("CARGO_PKG_VERSION"));
            let done = gio::spawn_blocking(move || {
                daftar_core::web_fetch::clip(&root, &url, "Clippings", None, agent).map(|clip| (clip.path, clip.skipped.len()))
            })
            .await
            .unwrap_or_else(|_| Err("the clip stopped".into()));
            match done {
                Ok((path, skipped)) => {
                    this.files_changed(std::collections::BTreeSet::from([path.clone()]));
                    this.open(&path, None);
                    if skipped > 0 {
                        this.notice(&format!("Clipped; {skipped} pictures stayed links."));
                    }
                }
                Err(error) => this.notice(&format!("Not clipped: {error}")),
            }
        });
    }

    /// The note in Neovide, at the line in view, with the person's own
    /// Neovim config. The editor here closes first, so one program owns the
    /// file; what Neovide writes shows in the reader as it is saved.
    fn open_in_neovide(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tab = self.tab();
        let Some(path) = tab.current().filter(|path| Vault::is_note(path)) else {
            self.notice("Open a note first.");
            return;
        };
        let line = if tab.editing() {
            let line = tab.editor.line();
            self.toggle_edit();
            if tab.editing() {
                return;
            }
            line
        } else {
            store.places.get(&path).unwrap_or(0)
        };
        self.launch_neovide(&path, line);
    }

    /// Neovide on the note at [path], its cursor on [line] (from 0), in a
    /// window of its own; the reader follows the file as Neovide writes it.
    pub(super) fn launch_neovide(&self, path: &str, line: usize) {
        let Some(store) = self.store.borrow().clone() else { return };
        let Some(neovide) = neovide() else {
            self.notice("Neovide is not installed: pacman -S neovide.");
            return;
        };
        let spawned = std::process::Command::new(neovide)
            .current_dir(&store.root)
            .arg("--")
            .arg(format!("+{}", line + 1))
            .arg(store.root.join(path))
            .spawn();
        match spawned {
            // Neovide leaves its first process at once; it is waited for off the window.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(error) => self.notice(&format!("Neovide did not start: {error}")),
        }
    }

    fn notice(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Every tab's editor saved, so a file is read as it is on disk.
    fn save_editors(&self) {
        let tabs = self.tabs.borrow().clone();
        for tab in &tabs {
            tab.editor.save();
        }
    }

    /// Ctrl+Alt+Y: the commits that changed the note, and an older text to restore.
    fn history(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let Some(path) = self.tab().current().filter(|path| Vault::is_note(path)) else {
            self.notice("Open a note first: its history shows.");
            return;
        };
        let (this, toasts) = (self.clone(), self.toasts.clone());
        let hooks = crate::history::Hooks {
            restore: Box::new(move |path, text| {
                this.save_editors();
                let Some(store) = this.store.borrow().clone() else { return Err("No vault is open.".into()) };
                let text = text.to_string();
                VaultEdits::apply(path, &store.root, |_| Ok(text)).map_err(|e| e.to_string())?;
                store.refresh(&[path.to_string()]);
                store.wrote();
                Ok(())
            }),
            notice: Box::new(move |text| toasts.add_toast(adw::Toast::new(text))),
        };
        crate::history::present(&self.window, store.root.clone(), &path, hooks);
    }

    /// The files a stopped sync waits on, with what to keep of each.
    fn conflicts(&self) {
        if !self.git.is_rebasing() {
            self.notice("No sync is stopped on a conflict.");
            return;
        }
        let (save, edit, toasts) = (self.clone(), self.clone(), self.toasts.clone());
        let hooks = crate::conflicts::Hooks {
            save: Box::new(move || save.save_editors()),
            edit: Box::new(move |path| {
                edit.open(path, None);
                if !edit.tab().editing() {
                    edit.toggle_edit();
                }
            }),
            notice: Box::new(move |text| toasts.add_toast(adw::Toast::new(text))),
        };
        crate::conflicts::present(&self.window, self.git.clone(), hooks);
    }

    /// Ctrl+Alt+R: the note read aloud from the reading cursor, or the voice stopped.
    fn read_aloud(&self) {
        if crate::read_aloud::is_speaking() {
            crate::read_aloud::stop();
            return;
        }
        let Some(store) = self.store.borrow().clone() else { return };
        let tab = self.tab();
        let Some(path) = tab.current().filter(|path| Vault::is_note(path)) else {
            self.notice("Open a note first: it is read aloud.");
            return;
        };
        // The page reads; the editor gives way to it.
        if tab.editing() {
            self.toggle_edit();
        }
        let text = store.text(&path).unwrap_or_default();
        let toasts = self.toasts.clone();
        crate::read_aloud::toggle(&path, &tab.reader.view, text, move |text| toasts.add_toast(adw::Toast::new(text)));
    }

    /// Ctrl+Shift+E: the note on screen as an A4 PDF.
    fn export_pdf(&self) {
        let tab = self.tab();
        let Some(shown) = tab.reader.shown().filter(|_| tab.showing().as_deref() == Some("reader")) else {
            self.notice("Open a note in the reader first: Export as PDF prints the page.");
            return;
        };
        let toasts = self.toasts.clone();
        crate::export::pdf(self.window.upcast_ref(), &tab.reader.view, &Vault::title(&shown.path), move |text| {
            toasts.add_toast(adw::Toast::new(text))
        });
    }
}

/// The `neovide` on $PATH, if there is one.
fn neovide() -> Option<std::path::PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("neovide")).find(|file| file.is_file())
}
