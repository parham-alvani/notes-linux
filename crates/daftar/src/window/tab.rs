//! One tab of a window, as each of the Mac's window tabs has: one note
//! pane, or two side by side after Split Right (the Mac's `NotePane`s).
//! Each pane has its own reader, editor, Today, board and home page, its
//! history and its title; the commands act on the tab's focused pane. The
//! sidebar, git, the inspector and the terminal are the window's.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::board::Board;
use crate::editor::Editor;
use crate::home::HomePage;
use crate::reader::Reader;
use crate::today::Today;

/// A tab of the bar: its panes side by side in [paned], the bar's page
/// child, and the one the commands act on.
pub(crate) struct Group {
    pub paned: gtk::Paned,
    pub focused: RefCell<Weak<Tab>>,
    pub page: RefCell<Option<adw::TabPage>>,
    /// The side a closed split hid, for the next one.
    pub spare: RefCell<Option<Rc<Tab>>>,
}

impl Group {
    pub fn new() -> Rc<Group> {
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .wide_handle(true)
            // The window keeps its width: a pane gives way, not the window.
            .shrink_start_child(true)
            .shrink_end_child(true)
            .build();
        paned.add_css_class("daftar-split");
        Rc::new(Group { paned, focused: RefCell::new(Weak::new()), page: RefCell::new(None), spare: RefCell::new(None) })
    }

    pub fn is_focused(&self, pane: &Tab) -> bool {
        self.focused.borrow().upgrade().is_some_and(|focused| std::ptr::eq(Rc::as_ptr(&focused), pane))
    }

    /// Is [widget] this tab's page child?
    pub fn holds(&self, widget: Option<&gtk::Widget>) -> bool {
        widget == Some(self.paned.upcast_ref::<gtk::Widget>())
    }
}

/// A note pane: one side of a tab.
pub(crate) struct Tab {
    /// The pane's pages: `empty` (home), `loading`, `reader`, `today`,
    /// `board`, `editor` and `choose`.
    pub stack: gtk::Stack,
    pub reader: Reader,
    pub editor: Rc<Editor>,
    pub today: Rc<Today>,
    pub board: Rc<Board>,
    pub home: Rc<HomePage>,
    /// Where Back and Forward go, the nearest last.
    pub back: RefCell<Vec<String>>,
    pub forward: RefCell<Vec<String>>,
    /// The title and the subtitle the header shows while the pane has the focus.
    pub title: RefCell<(String, String)>,
    /// The tab the pane is in.
    pub group: Rc<Group>,
}

impl Tab {
    pub fn showing(&self) -> Option<String> {
        self.stack.visible_child_name().map(String::from)
    }

    pub fn editing(&self) -> bool {
        self.showing().as_deref() == Some("editor")
    }

    /// The note on screen, in the reader or the editor.
    pub fn current(&self) -> Option<String> {
        match self.showing().as_deref() {
            Some("reader") => self.reader.shown().map(|shown| shown.path),
            Some("editor") => self.editor.path(),
            _ => None,
        }
    }

    /// The note the side panels and the graph are about: the one on screen;
    /// Today and the board keep the note read before them; home has none.
    pub fn note_in_view(&self) -> Option<String> {
        self.current()
            .or_else(|| (self.showing().as_deref() != Some("empty")).then(|| self.reader.shown().map(|shown| shown.path)).flatten())
    }

    /// Is this the left pane of its tab?
    pub fn is_start(&self) -> bool {
        self.group.paned.start_child().as_ref() == Some(self.stack.upcast_ref::<gtk::Widget>())
    }
}
