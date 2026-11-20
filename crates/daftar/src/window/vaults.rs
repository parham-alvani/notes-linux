//! Several vaults (`docs/specs/several-vaults.md`): Switch Vault, Forget
//! Vault, and the change of vault in the running app, which is one path
//! for a vault of the list and for Open Vault…: the editors save, a sync
//! that is running finishes, the tabs of the vault being left are kept,
//! and every window is loaded again for the new folder. No restart.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use super::Window;
use crate::vault_change;

thread_local! {
    /// Every window of the app: one vault is open at a time, in all of them.
    static WINDOWS: RefCell<Vec<Window>> = const { RefCell::new(Vec::new()) };
}

/// [window] is open, and changes vault with the others.
pub(super) fn register(window: &Window) {
    WINDOWS.with(|windows| windows.borrow_mut().push(window.clone()));
}

/// [window] closed.
pub(super) fn unregister(window: &Window) {
    WINDOWS.with(|windows| windows.borrow_mut().retain(|other| other.window != window.window));
}

fn windows() -> Vec<Window> {
    WINDOWS.with(|windows| windows.borrow().clone())
}

impl Window {
    /// The change to the vault at [root], in the running app. Refused, with
    /// the vault as it is, when the folder is not there or an editor keeps
    /// its text; it waits for a sync that is running.
    pub(super) fn switch_vault(&self, root: PathBuf) {
        if let Err(why) = vault_change::may_open(&root) {
            self.toasts.add_toast(crate::toast(&why));
            return;
        }
        if self.store.borrow().as_ref().is_some_and(|store| same_folder(&store.root, &root)) {
            return;
        }
        let all = windows();
        // The editors save first: one that cannot keeps its text, and the vault stays.
        for window in &all {
            let tabs = window.tabs.borrow().clone();
            if !tabs.iter().all(|tab| window.leave_editor_in(tab)) {
                self.toasts.add_toast(crate::toast("A note was not saved, so the vault stays as it is. Save it or close it first."));
                return;
            }
        }
        // A sync that is running finishes first; the change waits for it.
        if all.iter().any(|window| window.git.is_syncing()) {
            self.toasts.add_toast(crate::toast("The sync finishes first; then the vault changes."));
            let this = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || this.switch_vault(root));
            return;
        }
        for window in &all {
            // The tabs of the vault being left are kept: this window's, which the person is in.
            if window.window == self.window {
                window.save_tabs();
            }
            window.one_tab();
            // This window opens the tabs the new vault had; it writes the vault in use.
            window.load(root.clone(), None, window.window == self.window);
        }
    }

    /// One tab with one pane, on no note: what a vault's own tabs are opened over.
    fn one_tab(&self) {
        while self.tab_view.n_pages() > 1 {
            let last = self.tab_view.nth_page(self.tab_view.n_pages() - 1);
            self.tab_view.close_page(&last);
        }
        let tab = self.tab();
        if let Some(other) = self.other_pane(&tab) {
            self.unsplit(&tab, &other);
        }
    }

    /// The vaults this computer knows, to change to one: the one in use
    /// marked, a missing one off, then Open Vault… as the last entry.
    pub(super) fn ask_vault(&self) {
        let home = glib::home_dir();
        let entries = self.settings.borrow().vaults(&home).entries();
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        let dialog = adw::Dialog::builder().title("Switch Vault").content_width(460).build();
        for entry in &entries {
            let row = adw::ActionRow::builder().title(&entry.name).subtitle(&entry.text).use_markup(false).activatable(true).build();
            let state = if entry.is_missing() {
                "Missing"
            } else if entry.in_use {
                "In use"
            } else {
                ""
            };
            row.add_suffix(&gtk::Label::builder().label(state).css_classes(["dim-label", "caption"]).build());
            row.set_sensitive(!entry.is_missing() && !entry.in_use);
            let (this, dialog, path) = (self.clone(), dialog.clone(), entry.path.clone());
            row.connect_activated(move |_| {
                dialog.close();
                this.switch_vault(path.clone());
            });
            list.append(&row);
        }
        let open = adw::ActionRow::builder().title("Open Vault…").activatable(true).build();
        open.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
        {
            let (this, dialog) = (self.clone(), dialog.clone());
            open.connect_activated(move |_| {
                dialog.close();
                this.choose_vault();
            });
        }
        list.append(&open);
        dialog.set_child(Some(&framed(&list)));
        dialog.present(Some(&self.window));
    }

    /// The vaults that can be forgotten: every one but the vault in use.
    pub(super) fn ask_forget_vault(&self) {
        let home = glib::home_dir();
        let entries: Vec<_> = self.settings.borrow().vaults(&home).entries().into_iter().filter(|entry| !entry.in_use).collect();
        if entries.is_empty() {
            self.toasts.add_toast(crate::toast("There is no other vault to forget: the vault in use stays in the list."));
            return;
        }
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        let dialog = adw::Dialog::builder().title("Forget Vault").content_width(460).build();
        for entry in &entries {
            let row = adw::ActionRow::builder().title(&entry.name).subtitle(&entry.text).use_markup(false).activatable(true).build();
            let (this, dialog, folder, name) = (self.clone(), dialog.clone(), entry.text.clone(), entry.name.clone());
            row.connect_activated(move |_| {
                dialog.close();
                this.confirm_forget_vault(&folder, &name);
            });
            list.append(&row);
        }
        dialog.set_child(Some(&framed(&list)));
        dialog.present(Some(&self.window));
    }

    /// Forget Vault, after a question: the vault at [folder] leaves the
    /// list, and no file of it is touched.
    pub(super) fn confirm_forget_vault(&self, folder: &str, name: &str) {
        let alert = adw::AlertDialog::new(
            Some(&format!("Forget {name}?")),
            Some("It leaves the list of vaults. Its folder and its notes stay as they are, and Open Vault… brings it back."),
        );
        alert.add_responses(&[("cancel", "Cancel"), ("forget", "Forget")]);
        alert.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
        alert.set_close_response("cancel");
        let (this, folder) = (self.clone(), folder.to_string());
        alert.connect_response(Some("forget"), move |_, _| {
            let forgotten = this.settings.borrow_mut().forget_vault(&folder, &glib::home_dir());
            if let Err(why) = forgotten {
                this.toasts.add_toast(crate::toast(&format!("Not forgotten: {why}.")));
            }
        });
        alert.present(Some(&self.window));
    }
}

/// [list] with room around it under the dialog's title, as its only child.
fn framed(list: &gtk::ListBox) -> adw::ToolbarView {
    let frame =
        gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(6).margin_bottom(18).margin_start(18).margin_end(18).build();
    frame.append(list);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(520)
        .child(&frame)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroller));
    view
}

/// One folder, however it is written: links are not followed.
fn same_folder(a: &Path, b: &Path) -> bool {
    let vaults = daftar_core::Vaults::new("", &glib::home_dir());
    vaults.folder(&a.to_string_lossy()) == vaults.folder(&b.to_string_lossy())
}
