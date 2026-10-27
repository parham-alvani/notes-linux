//! Split Right (Ctrl+\), as on the Mac: a second note pane beside the note
//! in the same tab, with its own note, page and history. The focused pane is
//! the one the header, the commands, quick open, Back and Forward and the
//! inspector follow; a click or the keys in a pane focus it. Alt+click on a
//! link opens it in the other pane, making the split when there is none.
//! The editor saves when its pane loses the focus and takes the file when
//! the pane gets it back, so one note edited in both panes loses nothing; in
//! Neovim mode only one pane edits.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::Vault;

use super::{Tab, Window};

impl Window {
    pub(super) fn add_split_actions(&self) {
        let split = gio::SimpleAction::new("split", None);
        let this = self.clone();
        split.connect_activate(move |_, _| this.toggle_split());
        self.window.add_action(&split);
        let other = gio::SimpleAction::new("other-pane", None);
        let this = self.clone();
        other.connect_activate(move |_, _| {
            let tab = this.tab();
            if let Some(other) = this.other_pane(&tab) {
                this.focus_pane(&other, true);
            }
        });
        self.window.add_action(&other);
        let open_split = gio::SimpleAction::new("open-split", Some(glib::VariantTy::STRING));
        let this = self.clone();
        open_split.connect_activate(move |_, target| {
            if let Some(path) = target.and_then(|t| t.get::<String>()) {
                this.open_in_other_pane(&this.tab(), &path, None);
            }
        });
        self.window.add_action(&open_split);
    }

    /// The pane beside [tab], when its tab is split.
    pub(super) fn other_pane(&self, tab: &Rc<Tab>) -> Option<Rc<Tab>> {
        self.panes_of(&tab.group).into_iter().find(|other| !Rc::ptr_eq(other, tab))
    }

    /// Ctrl+\: a second pane beside the focused one, on the same note, with
    /// the focus; or, when split, back to one pane, the focused one.
    fn toggle_split(&self) {
        let tab = self.tab();
        if self.other_pane(&tab).is_some() {
            self.close_split(&tab);
        } else {
            let pane = self.split_pane(&tab);
            match tab.current() {
                Some(path) => self.go_in(&pane, &path, None, None, false),
                None => self.show_home(&pane),
            }
            self.focus_pane(&pane, true);
        }
    }

    /// The pane beside [tab], made empty when there is none.
    pub(super) fn split_pane(&self, tab: &Rc<Tab>) -> Rc<Tab> {
        if let Some(other) = self.other_pane(tab) {
            return other;
        }
        let paned = &tab.group.paned;
        let spare = tab.group.spare.borrow_mut().take();
        let pane = match spare {
            // The pane a closed split hid, with no history.
            Some(pane) => {
                pane.back.borrow_mut().clear();
                pane.forward.borrow_mut().clear();
                pane.stack.set_visible(true);
                self.tabs.borrow_mut().push(pane.clone());
                pane
            }
            None => {
                let pane = self.new_pane(&tab.group);
                paned.set_end_child(Some(&pane.stack));
                pane
            }
        };
        // Half and half, as the Mac draws its panes. A split put back at
        // launch has no width yet: halved at its first frame with one.
        let width = paned.width();
        if width > 0 {
            paned.set_position(width / 2);
        } else {
            paned.add_tick_callback(|paned, _| {
                let width = paned.width();
                if width <= 0 {
                    return glib::ControlFlow::Continue;
                }
                paned.set_position(width / 2);
                glib::ControlFlow::Break
            });
        }
        self.start_pane(&pane);
        self.mark_focus(tab);
        pane
    }

    /// One pane again: [keep] stays with its note and history. The other
    /// one's editor saves first, and stays when the save was refused.
    fn close_split(&self, keep: &Rc<Tab>) {
        let Some(other) = self.other_pane(keep) else { return };
        if !self.leave_editor_in(&other) {
            self.toasts.add_toast(adw::Toast::new("The other note changed on disk; the split stays so the text is not lost."));
            return;
        }
        crate::read_aloud::left(&other.reader.view, None);
        self.tabs.borrow_mut().retain(|tab| !Rc::ptr_eq(tab, &other));
        // Hidden and kept for the next split, not taken out: a web view
        // taken out of the window spins the main loop.
        other.stack.set_visible(false);
        other.stack.remove_css_class("daftar-pane-focused");
        *keep.group.spare.borrow_mut() = Some(other);
        self.focus_pane(keep, true);
    }

    /// Alt+click or Open in Split: [path] in the pane beside [from], which a
    /// split makes when there is none. The focus stays where it was.
    pub(super) fn open_in_other_pane(&self, from: &Rc<Tab>, path: &str, anchor: Option<&str>) {
        if self.store.borrow().is_none() {
            return;
        }
        if !Vault::opens_in_app(path) {
            self.go_in(from, path, anchor, None, true);
            return;
        }
        let target = self.split_pane(from);
        self.go_in(&target, path, anchor, None, true);
    }

    /// [tab] is the pane the commands act on from now on; with [grab], the
    /// keys go to it too.
    pub(super) fn focus_pane(&self, tab: &Rc<Tab>, grab: bool) {
        let before = tab.group.focused.borrow().upgrade();
        *tab.group.focused.borrow_mut() = Rc::downgrade(tab);
        self.mark_focus(tab);
        if before.is_some_and(|before| !Rc::ptr_eq(&before, tab)) {
            let (title, subtitle) = tab.title.borrow().clone();
            self.set_titles(tab, &title, &subtitle);
            if self.is_chosen(tab) {
                self.tab_selected();
            }
        }
        if grab {
            match tab.showing().as_deref() {
                Some("reader") => {
                    tab.reader.view.grab_focus();
                }
                Some("empty") => tab.home.focus(),
                _ => {
                    tab.stack.child_focus(gtk::DirectionType::TabForward);
                }
            }
        }
    }

    /// The focused pane of a split tab is marked; one pane alone is not.
    fn mark_focus(&self, tab: &Rc<Tab>) {
        let panes = self.panes_of(&tab.group);
        for pane in &panes {
            if panes.len() > 1 && tab.group.is_focused(pane) {
                pane.stack.add_css_class("daftar-pane-focused");
            } else {
                pane.stack.remove_css_class("daftar-pane-focused");
            }
        }
    }

    /// The keys or a click going into [tab]'s pane focus it; leaving it
    /// saves its editor, and coming back takes what the other pane saved.
    pub(super) fn follow_focus(&self, tab: &Rc<Tab>) {
        let focus = gtk::EventControllerFocus::new();
        let (this, weak) = (self.clone(), Rc::downgrade(tab));
        focus.connect_enter(move |_| {
            let Some(tab) = weak.upgrade() else { return };
            if this.other_pane(&tab).is_none() {
                return;
            }
            if !tab.group.is_focused(&tab) {
                this.focus_pane(&tab, false);
            }
            if tab.editing() {
                tab.editor.changed_on_disk();
            }
        });
        let weak = Rc::downgrade(tab);
        focus.connect_leave(move |_| {
            if let Some(tab) = weak.upgrade().filter(|tab| tab.editing()) {
                tab.editor.save();
            }
        });
        tab.stack.add_controller(focus);
        // A click on a page that takes no focus, as Today's or the home page's
        // background, focuses its pane too.
        let click = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        let (this, weak) = (self.clone(), Rc::downgrade(tab));
        click.connect_pressed(move |_, _, _, _| {
            if let Some(tab) = weak.upgrade().filter(|tab| !tab.group.is_focused(tab)) {
                this.focus_pane(&tab, false);
            }
        });
        tab.stack.add_controller(click);
    }

    /// Neovim mode: a pane that starts editing sends the other one back to
    /// the reader, so two Neovims never own one file. False when the other
    /// editor could not save.
    pub(super) fn one_editor(&self, tab: &Rc<Tab>) -> bool {
        if !self.settings.borrow().file.bool("editor.vim_mode").unwrap_or(false) {
            return true;
        }
        let Some(other) = self.other_pane(tab).filter(|other| other.editing()) else { return true };
        let line = other.editor.line();
        let path = other.editor.path();
        if !self.leave_editor_in(&other) {
            return false;
        }
        if let Some(path) = path {
            other.stack.set_visible_child_name("reader");
            other.reader.show(crate::reader::Request { path, heading: None, line: Some(line) }, false);
        }
        true
    }
}
