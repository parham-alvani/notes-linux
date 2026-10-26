//! One tab of a window, as each of the Mac's window tabs has: its own
//! reader, editor, Today, board and home page, its history and its title.
//! The sidebar, git, the inspector and the terminal are the window's.

use std::cell::RefCell;
use std::rc::Rc;

use crate::board::Board;
use crate::editor::Editor;
use crate::home::HomePage;
use crate::reader::Reader;
use crate::today::Today;

pub(crate) struct Tab {
    /// The tab's pages: `empty` (home), `loading`, `reader`, `today`,
    /// `board`, `editor` and `choose`; the tab view's page child.
    pub stack: gtk::Stack,
    pub reader: Reader,
    pub editor: Rc<Editor>,
    pub today: Rc<Today>,
    pub board: Rc<Board>,
    pub home: Rc<HomePage>,
    /// Where Back and Forward go, the nearest last.
    pub back: RefCell<Vec<String>>,
    pub forward: RefCell<Vec<String>>,
    /// The title and the subtitle the header shows while the tab is chosen.
    pub title: RefCell<(String, String)>,
    pub page: RefCell<Option<adw::TabPage>>,
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
}
