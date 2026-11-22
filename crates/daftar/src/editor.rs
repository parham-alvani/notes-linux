//! Editing a note: the markdown in a GtkSourceView, colored by the core's
//! `MarkdownHighlighter` in Daftar's palette, with the Mac editor's commands
//! on its keys: Return continues a list, Tab and Shift+Tab indent an item or
//! go to the next cell of a table, Ctrl+B, Ctrl+I and Ctrl+Shift+H mark the
//! selection, Ctrl+K links it, Ctrl+Return ticks the task on the line,
//! Alt+Up and Alt+Down move lines and Ctrl+Alt+Up and Down the section, `[[`
//! offers notes and `/` the slash bar (`editor_slash.rs`), a table is a grid
//! with Tab, Shift+Tab and Return on its cells, and a pasted picture is
//! saved in the vault's attachment folder. Focus mode dims every paragraph
//! but the cursor's (`editor_focus.rs`), and resting the pointer on a link
//! shows the note's first lines (`editor_link_card.rs`). The pictures a
//! line embeds are drawn under it, and a dropped file becomes a link
//! (`editor_media.rs`); Ctrl+Alt+X extracts the selection to a new note.
//!
//! The file is the record: what the editor loaded is kept, and a save goes
//! through only while the file on disk is still that text, so a change made
//! meanwhile by another app or `git pull` is never overwritten. A save runs a
//! second after typing stops, on Ctrl+S, and when the editor closes.
//!
//! With `editor.vim_mode`, Neovim owns the buffer and the file instead
//! (`neovim_session.rs`): keys go to it, and the view shows what it reports.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use sourceview5::prelude::*;

use daftar_core::{
    BufferMirror, Candidate, Day, EditorCommands, HtmlMarkdown, LivePreview, MarkdownTable, NoteFormat, QuickSwitcher, Return, Sections,
    Snippet, TaskSorter, Vault, VaultMoves,
};

#[path = "editor_link_card.rs"]
mod link_card;
#[path = "editor_media.rs"]
pub(crate) mod media;
#[path = "editor_problems.rs"]
mod problems;
#[path = "editor_slash.rs"]
mod slash;

use crate::editor_caret::Caret;
use crate::editor_draw::Draw;
use crate::editor_focus::Focus;
use crate::editor_popup::{Choice, Popup};
use crate::editor_style::{Metrics, Styler};
use crate::editor_view::EditorView;
use crate::neovim_session::{self, Event, MenuItem, NeovimSession};
use crate::settings::AppSettings;
use crate::store::{VaultStore, file_in};
use crate::vim_glide::{Glide, Shape};

/// How long typing must stop before the note is saved.
const AFTER_TYPING: Duration = Duration::from_secs(1);

/// What a save found.
#[derive(Debug, PartialEq, Eq)]
pub enum Saved {
    /// Nothing changed since the last save.
    Nothing,
    Written,
    /// The file on disk is not what the editor loaded; nothing was written.
    ChangedOnDisk,
    Failed(String),
}

impl Saved {
    /// The text is still only in the editor: the file changed under it, or
    /// it could not be written. Whoever was letting the note go keeps it.
    pub fn kept(&self) -> bool {
        matches!(self, Saved::ChangedOnDisk | Saved::Failed(_))
    }
}

/// The note open in the editor: its path, the text the file had when it was
/// loaded or last saved, and the text it had when the editor took it.
struct Open {
    path: String,
    file: PathBuf,
    on_disk: String,
    /// The file when the editor loaded it, or last read it again: a note
    /// whose file is not this when it is left was typed in, and is formatted
    /// (`docs/specs/problems-and-format.md` §6).
    loaded: String,
}

/// Hears a note the editor let go: its path, and the text it took.
type LeftListener = Box<dyn Fn(String, String)>;

/// What the list at the cursor is offering.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Completing {
    Nothing,
    Link,
    /// Snippets, for the `/` at this character.
    Slash(usize),
}

/// Neovim mode: the session, and the buffer as Neovim last reported it.
struct Vim {
    session: NeovimSession,
    mirror: Option<BufferMirror>,
    mode: String,
    /// The line to put the cursor on once Neovim has the file.
    start_line: usize,
    save_soon: Option<glib::SourceId>,
    /// The top line the wheel left the page at, so Neovim's answer does not
    /// pull the page to it again.
    wheel_top: Option<usize>,
    /// The command line holds a `:` command, not a search.
    command: bool,
}

/// Hears Neovim's mode as the bar names it, or `None` without Neovim.
type ModeListener = Box<dyn Fn(Option<&str>)>;

pub struct Editor {
    pub widget: gtk::Box,
    view: sourceview5::View,
    buffer: sourceview5::Buffer,
    styler: Rc<Styler>,
    popup: Rc<Popup>,
    /// The slash bar, and where a `/` was typed since the text was last styled.
    slash: Rc<slash::SlashBar>,
    slash_typed: Cell<Option<usize>>,
    /// The card of the link the pointer rests on, and what the pointer does.
    card: Rc<link_card::Card>,
    hover: link_card::Hover,
    /// Focus mode's veil over every paragraph but the cursor's.
    focus: Rc<Focus>,
    completing: Cell<Completing>,
    candidates: RefCell<Vec<Candidate>>,
    /// Where a closing bracket was put that the person did not type, so
    /// typing it steps over it.
    auto_closed: Cell<Option<usize>>,
    /// The characters changed since the colors were last brought up to date.
    dirty: Cell<Option<(usize, usize)>>,
    restyle_soon: Cell<bool>,
    /// A cursor move waiting for the idle that tints its line.
    reveal_soon: Cell<bool>,
    /// The pointer is over a task's box, where it is a hand.
    over_box: Cell<bool>,
    /// The format bar over a selection, once one was shown (docs/specs/look.md).
    format_bar: RefCell<Option<gtk::Popover>>,
    /// Edits so far, and the counts of words and characters at one of them.
    text_version: Cell<u64>,
    counted: Cell<Option<(u64, usize, usize)>>,
    open: RefCell<Option<Open>>,
    pending: RefCell<Option<glib::SourceId>>,
    settings: Rc<RefCell<AppSettings>>,
    store: RefCell<Option<Rc<VaultStore>>>,
    vim: RefCell<Option<Vim>>,
    /// Bumped at each new Neovim session, so an old one's events are ignored.
    generation: Cell<u64>,
    /// True while the view takes an edit from Neovim.
    remote: Cell<bool>,
    block_tag: gtk::TextTag,
    font_css: gtk::CssProvider,
    mode_label: gtk::Label,
    message: gtk::Label,
    /// A message of more than one line, over the bar until the next key.
    long_message: gtk::Popover,
    long_text: gtk::Label,
    glide: Rc<Glide>,
    /// The caret in the accent, in place of the view's own.
    caret: Rc<Caret>,
    /// What is drawn beside the text: panels, pills, boxes and icons.
    draw: Rc<Draw>,
    /// Over the text: Neovim's floating windows, as `vim_floats` gives them.
    stage: gtk::Overlay,
    floats: RefCell<Vec<gtk::Widget>>,
    /// Neovim's background behind the floats' rows, and between them.
    float_css: gtk::CssProvider,
    counts: gtk::Label,
    on_mode: RefCell<Option<ModeListener>>,
    pictures: media::Pictures,
    notice: Box<dyn Fn(&str)>,
    conflict: Box<dyn Fn(&str)>,
    left: RefCell<Option<LeftListener>>,
    marks: problems::Marks,
}

impl Editor {
    /// [notice] says a line to the person; [conflict] says a save was refused
    /// because the file changed on disk, and may offer `reload`.
    pub fn new(settings: Rc<RefCell<AppSettings>>, notice: impl Fn(&str) + 'static, conflict: impl Fn(&str) + 'static) -> Rc<Editor> {
        let buffer = sourceview5::Buffer::new(None);
        // The colors are the core highlighter's, as on the Mac.
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(false);
        // A view of our own, which draws the panels, the pills and the boxes
        // under and over the text (`editor_draw.rs`).
        let canvas = EditorView::new(&buffer);
        let view: sourceview5::View = canvas.clone().upcast();
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_auto_indent(true);
        view.set_indent_width(4);
        view.set_tab_width(4);
        view.set_insert_spaces_instead_of_tabs(true);
        view.set_smart_backspace(true);
        view.set_top_margin(24);
        view.set_bottom_margin(240);
        view.set_left_margin(32);
        view.set_right_margin(32);
        view.add_css_class("daftar-editor");
        // The view is as wide as the pane, so the margin's marks stay at the
        // pane's edge; the line's width is the view's own margins (`measure`).
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&view)
            .css_classes(["daftar-page"])
            .build();
        let mode_label = gtk::Label::builder().use_markup(true).visible(false).build();
        let message =
            gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["monospace"]).build();
        let counts = gtk::Label::builder().css_classes(["dim-label", "caption", "numeric"]).build();
        let bar = gtk::Box::builder().spacing(10).margin_start(12).margin_end(12).margin_top(4).margin_bottom(4).build();
        bar.append(&mode_label);
        bar.append(&message);
        bar.append(&counts);
        // Neovim's floating windows stand over the text.
        let stage = gtk::Overlay::builder().child(&scroller).vexpand(true).build();
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["daftar-page"]).build();
        widget.append(&stage);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&bar);
        let long_text = gtk::Label::builder().xalign(0.0).selectable(true).css_classes(["monospace"]).build();
        let long_message = gtk::Popover::builder()
            .child(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .propagate_natural_width(true)
                    .max_content_height(360)
                    .child(&long_text)
                    .build(),
            )
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Top)
            .css_classes(["daftar-vim-message"])
            .build();
        long_message.set_parent(&message);
        let block_tag = gtk::TextTag::builder().name("daftar:vim-block").build();
        buffer.tag_table().add(&block_tag);
        let font_css = gtk::CssProvider::new();
        #[expect(deprecated, reason = "GTK has no other way to give one view its own font")]
        view.style_context().add_provider(&font_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let styler = Styler::new(buffer.upcast_ref());
        let draw = Draw::new(&styler);
        let caret = Caret::new(&view);
        let focus = Focus::new(view.upcast_ref());
        {
            // Over the text: focus mode's veil over all that is drawn, then
            // the caret last, with nothing on it.
            let (draw, caret, styler, focus) = (draw.clone(), caret.clone(), styler.clone(), focus.clone());
            canvas.set_painter(move |view, layer, snapshot| {
                draw.paint(view, layer, snapshot);
                if layer == gtk::TextViewLayer::AboveText {
                    focus.paint(view, snapshot, &crate::editor_style::rgba("background", 1.0, styler.is_dark()));
                    caret.paint(snapshot, &crate::editor_style::rgba("accent", 1.0, styler.is_dark()));
                }
            });
        }
        let editor = Rc::new(Editor {
            widget,
            styler,
            draw,
            popup: Popup::new(&view),
            slash: slash::SlashBar::new(&view),
            slash_typed: Cell::new(None),
            card: link_card::Card::new(&view),
            hover: link_card::Hover::default(),
            focus,
            glide: Glide::new(&view),
            caret,
            stage,
            floats: RefCell::new(Vec::new()),
            float_css: {
                let provider = gtk::CssProvider::new();
                if let Some(display) = gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                }
                provider
            },
            view,
            buffer,
            completing: Cell::new(Completing::Nothing),
            candidates: RefCell::new(Vec::new()),
            auto_closed: Cell::new(None),
            dirty: Cell::new(None),
            restyle_soon: Cell::new(false),
            reveal_soon: Cell::new(false),
            over_box: Cell::new(false),
            format_bar: RefCell::new(None),
            text_version: Cell::new(0),
            counted: Cell::new(None),
            open: RefCell::new(None),
            pending: RefCell::new(None),
            settings,
            store: RefCell::new(None),
            vim: RefCell::new(None),
            generation: Cell::new(0),
            remote: Cell::new(false),
            block_tag,
            font_css,
            mode_label,
            message,
            long_message,
            long_text,
            counts,
            on_mode: RefCell::new(None),
            pictures: media::Pictures::default(),
            notice: Box::new(notice),
            conflict: Box::new(conflict),
            left: RefCell::new(None),
            marks: problems::Marks::default(),
        });
        editor.install_marks();
        install_schemes(&editor.buffer);
        editor.connect_buffer();
        editor.connect_keys();
        editor.connect_clicks();
        editor.connect_card();
        editor.connect_drop();
        editor.styler.attach(editor.view.upcast_ref());
        editor.connect_measure();
        editor.connect_pictures();
        let this = Rc::downgrade(&editor);
        editor.glide.connect_wheel_stopped(move |top| {
            let Some(this) = this.upgrade() else { return };
            let mut vim = this.vim.borrow_mut();
            if let Some(vim) = vim.as_mut().filter(|vim| vim.mirror.is_some()) {
                vim.wheel_top = Some(top);
                vim.session.set_top(top);
            }
        });
        editor
    }

    /// The vault's index, for `[[` completion and pasted pictures.
    pub fn set_store(&self, store: Rc<VaultStore>) {
        *self.store.borrow_mut() = Some(store);
    }

    /// [changed] hears the Neovim mode's name, or `None` when there is none.
    pub fn connect_mode(&self, changed: impl Fn(Option<&str>) + 'static) {
        *self.on_mode.borrow_mut() = Some(Box::new(changed));
    }

    /// Loads the note at [path] from the vault at [root], with [line] in view.
    pub fn load(self: &Rc<Self>, root: &Path, path: &str, line: Option<usize>) -> bool {
        self.save();
        self.stop_vim();
        let Some(file) = file_in(path, root) else { return false };
        let Ok(text) = fs::read_to_string(&file) else { return false };
        self.cancel_pending();
        self.stop_completion();
        self.slash_typed.set(None);
        self.close_card();
        self.auto_closed.set(None);
        self.apply_fonts();
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        self.dirty.set(None);
        // Another text: focus mode's paragraph is found anew, once the cursor is placed.
        self.focus.forget();
        self.styler.restyle_all();
        *self.open.borrow_mut() = Some(Open { path: path.to_string(), file, on_disk: text.clone(), loaded: text });
        self.refresh_marks();
        let line = line.unwrap_or(0);
        self.go_to_line(line);
        self.focus.follow(None);
        self.update_counts();
        let vim = self.settings.borrow().file.bool("editor.vim_mode").unwrap_or(false);
        if vim {
            self.start_vim(root, path, line);
        } else {
            self.message.set_label("");
            self.mode_label.set_visible(false);
            self.tell_mode(None);
        }
        true
    }

    /// Ctrl+Alt+X: the selection to extract to a new note and where it is,
    /// or why there is none.
    pub fn extract_selection(&self) -> Result<(Range<usize>, String), &'static str> {
        if self.vim.borrow().is_some() {
            return Err("Extract works in the editor without Neovim mode.");
        }
        let range = self.selection();
        if range.is_empty() {
            return Err("Select the text to extract first.");
        }
        Ok((range.clone(), self.slice(range)))
    }

    /// Puts [link] where [text] was extracted from, while it is still there;
    /// false when the note changed meanwhile.
    pub fn replace_extracted(&self, range: Range<usize>, text: &str, link: &str) -> bool {
        if self.vim.borrow().is_some() || range.end > self.buffer.char_count() as usize || self.slice(range.clone()) != text {
            return false;
        }
        let at = range.start + link.chars().count();
        self.replace(range, link, Some(at..at));
        self.view.grab_focus();
        true
    }

    /// The path of the note being edited.
    pub fn path(&self) -> Option<String> {
        self.open.borrow().as_ref().map(|open| open.path.clone())
    }

    /// The line the cursor is on, from 0, to show the same place when reading.
    pub fn line(&self) -> usize {
        self.buffer.iter_at_mark(&self.buffer.get_insert()).line().max(0) as usize
    }

    /// Writes the buffer when it changed, unless the file changed under it.
    /// In Neovim mode, Neovim writes it, and this waits for it a moment.
    /// [text] typed at the end of the note, for a test with no keyboard.
    #[cfg(test)]
    pub(crate) fn type_at_end(&self, text: &str) {
        self.buffer.insert(&mut self.buffer.end_iter(), text);
    }

    pub fn save(&self) -> Saved {
        self.cancel_pending();
        if let Some(vim) = self.vim.borrow().as_ref() {
            vim.session.save_and_wait(Duration::from_secs(2));
            return Saved::Nothing;
        }
        let mut open = self.open.borrow_mut();
        let Some(note) = open.as_mut() else { return Saved::Nothing };
        if !self.buffer.is_modified() {
            return Saved::Nothing;
        }
        let text = self.text();
        let saved = write_if_unchanged(&note.file, &note.on_disk, &text);
        match &saved {
            Saved::Written => {
                note.on_disk = text;
                self.buffer.set_modified(false);
                // The editor's own save does not make the note "updated since you read".
                let store = self.store.borrow().clone();
                if let Some(store) = store {
                    store.places.mark_read(&note.path);
                    crate::wakatime::heartbeat(&self.settings.borrow().file, &store.root, &note.path, true);
                }
            }
            Saved::ChangedOnDisk | Saved::Failed(_) | Saved::Nothing => {}
        }
        // The callbacks run with the note let go: one that reads the editor
        // would find it borrowed.
        let path = note.path.clone();
        drop(open);
        match &saved {
            Saved::ChangedOnDisk => (self.conflict)(&format!("{path} changed on disk, so your edit was not saved.")),
            Saved::Failed(error) => (self.notice)(&format!("{path} was not saved: {error}")),
            _ => {}
        }
        saved
    }

    /// True in Neovim mode, where the buffer takes Neovim's edits only: a
    /// command that edits the note saves, writes the file and has Neovim
    /// read it again.
    pub fn is_neovim(&self) -> bool {
        self.vim.borrow().is_some()
    }

    /// [left] hears each note the editor lets go, once its text is on disk,
    /// for format on save.
    pub fn connect_left(&self, left: impl Fn(String, String) + 'static) {
        *self.left.borrow_mut() = Some(Box::new(left));
    }

    /// Saves, then lets go of the note, unless the save kept the text.
    pub fn close(&self) -> Saved {
        let saved = self.save();
        if !saved.kept() {
            let left = self.leaving();
            self.release();
            if let (Some((path, loaded)), Some(listener)) = (left, self.left.borrow().as_ref()) {
                listener(path, loaded);
            }
        }
        saved
    }

    /// The note open and the text the editor took, for format on save at Quit.
    pub fn leaving(&self) -> Option<(String, String)> {
        self.open.borrow().as_ref().map(|open| (open.path.clone(), open.loaded.clone()))
    }

    /// Lets go of the note and what was typed into it since the last save:
    /// the person chose to.
    pub fn discard(&self) {
        self.cancel_pending();
        self.buffer.set_modified(false);
        self.release();
    }

    fn release(&self) {
        self.stop_vim();
        self.popup.close();
        self.clear_pictures();
        *self.open.borrow_mut() = None;
    }

    /// The file changed on disk: an editor with nothing unsaved reads it
    /// again; Neovim checks it, and keeps what it has not written.
    pub fn changed_on_disk(&self) {
        if let Some(vim) = self.vim.borrow().as_ref() {
            vim.session.check_disk();
            return;
        }
        if !self.buffer.is_modified() {
            self.reload();
        }
    }

    /// Reads the file again, where the cursor was, and drops what was not saved.
    pub fn reload(&self) {
        if let Some(vim) = self.vim.borrow().as_ref() {
            vim.session.check_disk();
            return;
        }
        self.cancel_pending();
        let Some(file) = self.open.borrow().as_ref().map(|open| open.file.clone()) else { return };
        let Ok(text) = fs::read_to_string(&file) else { return };
        if self.open.borrow().as_ref().is_some_and(|open| open.on_disk == text) && !self.buffer.is_modified() {
            return;
        }
        let line = self.line();
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        if let Some(open) = self.open.borrow_mut().as_mut() {
            open.on_disk.clone_from(&text);
            open.loaded = text;
        }
        self.focus.forget();
        if let Some(at) = self.buffer.iter_at_line(line as i32) {
            self.buffer.place_cursor(&at);
        }
        self.focus.follow(None);
        self.refresh_marks();
    }

    // The buffer

    pub(crate) fn text(&self) -> String {
        self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), true).to_string()
    }

    fn cursor(&self) -> usize {
        self.buffer.cursor_position().max(0) as usize
    }

    /// The selection in characters; empty at the cursor.
    fn selection(&self) -> Range<usize> {
        match self.buffer.selection_bounds() {
            Some((a, b)) => a.offset().min(b.offset()) as usize..a.offset().max(b.offset()) as usize,
            None => self.cursor()..self.cursor(),
        }
    }

    #[expect(clippy::iter_not_returning_iterator, reason = "a `TextIter`, as GTK names a place in the text")]
    fn iter(&self, offset: usize) -> gtk::TextIter {
        self.buffer.iter_at_offset(offset as i32)
    }

    fn slice(&self, range: Range<usize>) -> String {
        self.buffer.text(&self.iter(range.start), &self.iter(range.end), true).to_string()
    }

    /// Characters [range] become [text], as one step to undo, then [select].
    fn replace(&self, range: Range<usize>, text: &str, select: Option<Range<usize>>) {
        self.buffer.begin_user_action();
        let (mut start, mut end) = (self.iter(range.start), self.iter(range.end));
        self.buffer.delete(&mut start, &mut end);
        let mut at = self.iter(range.start);
        self.buffer.insert(&mut at, text);
        self.buffer.end_user_action();
        if let Some(select) = select {
            self.buffer.select_range(&self.iter(select.end), &self.iter(select.start));
        }
        self.view.scroll_mark_onscreen(&self.buffer.get_insert());
    }

    /// The cursor's line without its newline, in characters.
    fn current_line(&self) -> Range<usize> {
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let mut start = at;
        start.set_line_offset(0);
        let mut end = at;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        start.offset() as usize..end.offset() as usize
    }

    /// The whole lines the selection is on, with the last one's newline.
    fn selected_lines(&self) -> Range<usize> {
        let selection = self.selection();
        let mut start = self.iter(selection.start);
        start.set_line_offset(0);
        let last = if selection.end > selection.start { selection.end - 1 } else { selection.end };
        let mut end = self.iter(last);
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        if !end.is_end() {
            end.forward_char();
        }
        start.offset() as usize..end.offset() as usize
    }

    fn go_to_line(&self, line: usize) {
        let mut at = self.buffer.iter_at_line(line as i32).unwrap_or_else(|| self.buffer.start_iter());
        self.buffer.place_cursor(&at);
        let view = self.view.clone();
        glib::idle_add_local_once(move || {
            view.scroll_to_iter(&mut at, 0.1, true, 0.0, 0.3);
            view.grab_focus();
        });
    }

    /// The fonts of the settings: the note's for the text, the code font for
    /// code, at the reader's size, with Obsidian's line height.
    fn apply_fonts(&self) {
        let settings = self.settings.borrow();
        let size = settings.text_size();
        let (text, _) = settings.font_css();
        let code = settings.file.string("appearance.code_font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into());
        self.font_css.load_from_string(&format!("textview.daftar-editor {{ font-family: {text}; font-size: {size}px; }}"));
        self.styler.set_code_font(&code);
        self.styler.set_live(settings.file.bool("editor.live_preview").unwrap_or(true));
        self.styler.set_soft(settings.soft_palette());
        self.focus.set_on(settings.focus());
        // Line height 1.65, as the reader's: the space between lines and between wrapped rows.
        let gap = (size as f64 * 0.65).round() as i32;
        self.view.set_pixels_below_lines(gap);
        self.view.set_pixels_inside_wrap(gap / 2);
        drop(settings);
        self.measure();
    }

    /// The settings changed (the text size, the measure, a font, live
    /// preview): the note open follows them now, not at the next one.
    pub fn settings_changed(&self) {
        if self.open.borrow().is_some() {
            self.apply_fonts();
        }
    }

    /// The editor's line is the reader's measure (`appearance.text_width` em
    /// of the text size) in the middle of the pane: the view's margins are
    /// the room at each side, and the panels' looks follow them. Worked out
    /// when the pane's width, the text size or the setting changes, never on
    /// a key, and never while the view lays itself out.
    fn measure(&self) {
        let (size, width) = {
            let settings = self.settings.borrow();
            (settings.text_size() as f64, settings.text_width() as f64)
        };
        // The pane without the margin's marks: the room for text.
        let pane = self.view.hadjustment().map_or(0.0, |h| h.page_size());
        let room = if pane > 1.0 { LivePreview::measure(pane, size, width).room.floor() as i32 } else { 32 };
        if self.view.left_margin() != room {
            self.view.set_left_margin(room);
            self.view.set_right_margin(room);
        }
        let bracket = f64::from(self.view.create_pango_layout(Some("[")).pixel_size().0);
        // A table is a grid while its widest line fits the measure, in the code font.
        let letters = self.view.create_pango_layout(Some(&"0".repeat(20)));
        let mut face = self.view.pango_context().font_description().unwrap_or_default();
        face.set_family(&self.styler.code_font());
        letters.set_font_description(Some(&face));
        let mono = f64::from(letters.pixel_size().0) / 20.0;
        let line = if pane > 1.0 { pane - 2.0 * f64::from(room) } else { 0.0 };
        self.styler.set_metrics(Metrics { size, room, bracket, line, mono });
    }

    /// The measure follows the pane's width, and the late chips the day.
    fn connect_measure(self: &Rc<Self>) {
        if let Some(h) = self.view.hadjustment() {
            let this = Rc::downgrade(self);
            h.connect_page_size_notify(move |_| {
                let this = this.clone();
                glib::idle_add_local_once(move || {
                    if let Some(this) = this.upgrade() {
                        this.measure();
                    }
                });
            });
        }
        // "Today" is looked at once a minute, never on a key.
        let this = Rc::downgrade(self);
        glib::timeout_add_seconds_local(60, move || match this.upgrade() {
            Some(this) => {
                this.styler.follow_day();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// A click on the box of the task whose marks start at [offset]: the
    /// line becomes what the task toggle makes of it (Ctrl+Return), as one
    /// step to undo. The caret stays on its text and the page where it is;
    /// in Neovim mode the change of those lines is sent to Neovim, and a
    /// read-only vault's text is not changed.
    fn click_box(&self, offset: usize) {
        let read_only = self.store.borrow().as_ref().is_some_and(|store| daftar_core::VaultEdits::writable(&store.root).is_err());
        if read_only {
            return;
        }
        let at = self.iter(offset);
        let mut start = at;
        start.set_line_offset(0);
        let mut end = at;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let range = start.offset() as usize..end.offset() as usize;
        let Some(lines) = LivePreview::click(&self.slice(range.clone()), Day::today()) else { return };
        if let Some(vim) = self.vim.borrow().as_ref() {
            let (line, expected) = (at.line().max(0) as usize, NoteFormat::lines(&self.text()).join("\n"));
            // Neovim answers with its own lines event; nothing waits for it here.
            drop(vim.session.replace_lines(&expected, line..line + 1, lines));
            return;
        }
        let place =
            |mark: &gtk::TextMark| LivePreview::caret_after_click(self.buffer.iter_at_mark(mark).offset().max(0) as usize, &range, &lines);
        let (insert, bound) = (place(&self.buffer.get_insert()), place(&self.buffer.selection_bound()));
        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut self.iter(range.start), &lines.join("\n"));
        self.buffer.end_user_action();
        self.buffer.select_range(&self.iter(insert), &self.iter(bound));
    }

    fn connect_buffer(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.buffer.connect_insert_text(move |buffer, at, text| {
            let Some(this) = this.upgrade() else { return };
            // In Neovim mode only Neovim edits: a paste by the mouse, a drop.
            if this.vim.borrow().is_some() && !this.remote.get() {
                buffer.stop_signal_emission_by_name("insert-text");
                return;
            }
            let start = at.offset() as usize;
            this.mark_dirty(start, start + text.chars().count());
        });
        let this = Rc::downgrade(self);
        self.buffer.connect_delete_range(move |buffer, start, _| {
            let Some(this) = this.upgrade() else { return };
            if this.vim.borrow().is_some() && !this.remote.get() {
                buffer.stop_signal_emission_by_name("delete-range");
                return;
            }
            let at = start.offset() as usize;
            this.mark_dirty(at, at);
        });
        let this = Rc::downgrade(self);
        self.buffer.connect_changed(move |_| {
            if let Some(this) = this.upgrade() {
                this.text_version.set(this.text_version.get() + 1);
                this.schedule_restyle();
                if this.vim.borrow().is_none() {
                    this.schedule_save();
                }
            }
        });
        let this = Rc::downgrade(self);
        self.buffer.connect_mark_set(move |_, _, mark| {
            if let Some(this) = this.upgrade()
                && (mark.name().as_deref() == Some("insert") || mark.name().as_deref() == Some("selection_bound"))
            {
                this.schedule_reveal();
            }
        });
    }

    /// The cursor's line tinted, and its marks shown, once the key that
    /// moved it is done with.
    fn schedule_reveal(self: &Rc<Self>) {
        if self.reveal_soon.replace(true) {
            return;
        }
        let this = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(this) = this.upgrade() else { return };
            this.reveal_soon.set(false);
            let (a, b) = this.buffer.selection_bounds().map_or_else(
                || {
                    let at = this.buffer.iter_at_mark(&this.buffer.get_insert()).offset() as usize;
                    (at, at)
                },
                |(a, b)| (a.offset() as usize, b.offset() as usize),
            );
            this.styler.reveal((a, b));
            this.focus.follow(None);
            this.slash_follows_caret();
            this.update_counts();
            this.show_format_bar();
        });
    }

    /// The format bar over the start of a selection inside one paragraph:
    /// bold, italic, highlight, code, strike and a link to a note, each the
    /// command its key runs. Gone with the selection, in Neovim mode (Neovim
    /// owns the text) and with `editor.format_bar` off. Its buttons take no
    /// focus, so the keys stay in the text.
    fn show_format_bar(self: &Rc<Self>) {
        let range = self.selection();
        let wanted =
            !range.is_empty() && !self.is_neovim() && self.settings.borrow().format_bar() && !self.slice(range.clone()).contains('\n');
        if !wanted {
            if let Some(bar) = self.format_bar.borrow().as_ref() {
                bar.popdown();
            }
            return;
        }
        if self.format_bar.borrow().is_none() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            const ITEMS: [(&str, &str, Option<&str>); 6] = [
                ("B", "Bold (Ctrl+B)", Some("**")),
                ("I", "Italic (Ctrl+I)", Some("*")),
                ("==", "Highlight", Some("==")),
                ("</>", "Code", Some("`")),
                ("S", "Strikethrough", Some("~~")),
                ("[[ ]]", "Link to a note (Ctrl+K)", None),
            ];
            for (title, help, marker) in ITEMS {
                let button = gtk::Button::with_label(title);
                button.add_css_class("flat");
                button.set_tooltip_text(Some(help));
                button.set_focusable(false);
                button.set_focus_on_click(false);
                let this = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    let Some(this) = this.upgrade() else { return };
                    match marker {
                        Some(marker) => this.wrap(marker),
                        None => this.insert_link(),
                    }
                    this.view.grab_focus();
                });
                row.append(&button);
            }
            let bar = gtk::Popover::new();
            bar.set_child(Some(&row));
            bar.set_parent(&self.view);
            bar.set_autohide(false);
            bar.set_has_arrow(false);
            bar.set_can_focus(false);
            bar.set_position(gtk::PositionType::Top);
            bar.add_css_class("daftar-format-bar");
            *self.format_bar.borrow_mut() = Some(bar);
        }
        let place = self.view.iter_location(&self.iter(range.start));
        let (x, y) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, place.x(), place.y());
        if let Some(bar) = self.format_bar.borrow().as_ref() {
            bar.set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, place.height())));
            bar.popup();
        }
    }

    fn mark_dirty(&self, start: usize, end: usize) {
        let merged = match self.dirty.get() {
            Some((a, b)) => (a.min(start), b.max(end)),
            None => (start, end),
        };
        self.dirty.set(Some(merged));
    }

    /// The colors, the counts and the list at the cursor follow the text,
    /// once the key that changed it is done with.
    fn schedule_restyle(self: &Rc<Self>) {
        if self.restyle_soon.replace(true) {
            return;
        }
        let this = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(this) = this.upgrade() else { return };
            this.restyle_soon.set(false);
            if let Some((start, end)) = this.dirty.take() {
                this.styler.restyle_after(start..end);
                // Focus mode's paragraph is looked at again only where the text changed.
                if this.focus.is_on() {
                    let total = this.buffer.char_count().max(0) as usize;
                    let line = |offset: usize| this.iter(offset.min(total)).line();
                    this.focus.follow(Some(line(start)..line(end)));
                }
            }
            this.update_counts();
            if let Some(slash) = this.slash_typed.take() {
                this.maybe_slash_bar(slash);
            }
            this.follow_completion();
            this.refresh_pictures();
        });
    }

    fn update_counts(&self) {
        let selection = self.selection();
        // The note's counts, again only after an edit: a cursor move keeps them.
        let version = self.text_version.get();
        let (words, characters) = match self.counted.get() {
            Some((counted, words, characters)) if counted == version => (words, characters),
            _ => {
                let text = self.text();
                let counts = (text.split_whitespace().count(), self.buffer.char_count().max(0) as usize);
                self.counted.set(Some((version, counts.0, counts.1)));
                counts
            }
        };
        let (word, character) = (plural(words, "word"), plural(characters, "character"));
        let line = if selection.is_empty() {
            format!("{words} {word} · {characters} {character}")
        } else {
            let selected = self.slice(selection.clone());
            format!("{} of {words} {word} · {} of {characters} {character}", selected.split_whitespace().count(), selection.len())
        };
        self.counts.set_label(&line);
    }

    // Keys

    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        // Before the text view's own handling, so Return and Tab are ours.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, keycode, state| match this.upgrade() {
            Some(this) => this.key(key, keycode, state),
            None => glib::Propagation::Proceed,
        });
        self.view.add_controller(keys);
    }

    fn key(self: &Rc<Self>, key: gdk::Key, keycode: u32, state: gdk::ModifierType) -> glib::Propagation {
        let control = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let super_ = state.contains(gdk::ModifierType::SUPER_MASK);
        // A key closes the link's card, and typing under a still pointer opens none.
        self.typed_under_pointer();
        if self.vim.borrow().is_some() {
            return self.vim_key(key, keycode, control, alt, super_, shift);
        }
        if !super_ && self.slash_key(key, control, alt, shift) {
            return glib::Propagation::Stop;
        }
        if !control && !alt && self.popup.key(key) {
            return glib::Propagation::Stop;
        }
        let stop = |done: bool| if done { glib::Propagation::Stop } else { glib::Propagation::Proceed };
        let plain = !control && !alt && !super_;
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter if control && !shift && !alt => {
                self.toggle_task();
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if plain && !shift => stop(self.on_return()),
            // In a table Shift+Return is Return: a cell never holds a line break.
            gdk::Key::Return | gdk::Key::KP_Enter if plain && shift => stop(self.table_key(TableKey::Return)),
            gdk::Key::Left | gdk::Key::Right if plain && !shift => stop(self.table_arrow(key == gdk::Key::Right)),
            gdk::Key::Tab if plain && !shift => stop(self.on_tab(false)),
            gdk::Key::ISO_Left_Tab | gdk::Key::Tab if plain && shift => stop(self.on_tab(true)),
            gdk::Key::Up | gdk::Key::Down if alt && !control && shift => {
                let edit = EditorCommands::duplicate_lines(&self.text(), self.selection(), key == gdk::Key::Up);
                self.replace(edit.range, &edit.text, Some(edit.selection));
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::Down if alt && !control => {
                self.move_lines(key == gdk::Key::Up);
                glib::Propagation::Stop
            }
            gdk::Key::Up | gdk::Key::Down if alt && control && !shift => {
                self.move_section(key == gdk::Key::Up);
                glib::Propagation::Stop
            }
            gdk::Key::BackSpace if plain => stop(self.on_backspace()),
            gdk::Key::v | gdk::Key::V if control && !alt && !shift => stop(self.paste()),
            // The clipboard's plain text, never converted.
            gdk::Key::v | gdk::Key::V if control && !alt && shift => {
                self.paste_plain();
                glib::Propagation::Stop
            }
            // Ctrl+Alt+1 to Ctrl+Alt+6 make the line a heading, Ctrl+Alt+0 plain text.
            _ if control && alt && !shift && heading_level(key).is_some() => {
                let line = self.current_line();
                let headed = EditorCommands::heading(&self.slice(line.clone()), heading_level(key).unwrap_or(0));
                let end = line.start + headed.chars().count();
                self.replace(line, &headed, Some(end..end));
                glib::Propagation::Stop
            }
            _ if control && !alt => match key.to_lower() {
                gdk::Key::k if shift => {
                    let edit = EditorCommands::delete_lines(&self.text(), self.selection());
                    self.replace(edit.range, &edit.text, Some(edit.selection));
                    glib::Propagation::Stop
                }
                gdk::Key::b if !shift => {
                    self.wrap("**");
                    glib::Propagation::Stop
                }
                gdk::Key::i if !shift => {
                    self.wrap("*");
                    glib::Propagation::Stop
                }
                gdk::Key::h if shift => {
                    self.wrap("==");
                    glib::Propagation::Stop
                }
                // Strikethrough and inline code, the Mac's Shift+Cmd+X and Shift+Cmd+C.
                gdk::Key::x if shift => {
                    self.wrap("~~");
                    glib::Propagation::Stop
                }
                gdk::Key::c if shift => {
                    self.wrap("`");
                    glib::Propagation::Stop
                }
                gdk::Key::k if !shift => {
                    self.insert_link();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            },
            _ if plain => match key.to_unicode() {
                Some(c) if !c.is_control() => stop(self.on_char(c)),
                _ => glib::Propagation::Proceed,
            },
            _ => glib::Propagation::Proceed,
        }
    }

    /// Return continues a list, and on an empty item ends it. In a table it
    /// goes to the same column of the row below, with a new row after the
    /// last, and on an empty last row it ends the table
    /// (`MarkdownTable::enter`): one step to undo, with the formatting.
    fn on_return(&self) -> bool {
        // A selection in one cell does not become a line break: a cell holds none.
        if self.table_key(TableKey::Return) {
            return true;
        }
        let line = self.current_line();
        let cursor = self.selection();
        if !cursor.is_empty() {
            return false;
        }
        let text = self.slice(line.clone());
        if cursor.start != line.end {
            return false;
        }
        match EditorCommands::on_return(&text) {
            Some(Return::Insert(insert)) => {
                let at = cursor.start + insert.chars().count();
                self.replace(cursor, &insert, Some(at..at));
                true
            }
            Some(Return::EndList) => {
                self.replace(line.clone(), "", Some(line.start..line.start));
                true
            }
            None => false,
        }
    }

    /// Tab: the next cell of a table, else list items in by four spaces.
    /// Shift+Tab: the cell before, else out.
    fn on_tab(&self, back: bool) -> bool {
        if self.table_key(TableKey::Tab { back }) {
            return true;
        }
        let lines = self.selected_lines();
        let text = self.slice(lines.clone());
        let items = !text.is_empty() && text.lines().all(EditorCommands::is_list_item);
        if back {
            self.shift_lines(lines, EditorCommands::outdent);
            return true;
        }
        if !items {
            let selection = self.selection();
            let at = selection.start + EditorCommands::INDENT_UNIT.len();
            self.replace(selection, EditorCommands::INDENT_UNIT, Some(at..at));
            return true;
        }
        self.shift_lines(lines, EditorCommands::indent);
        true
    }

    fn shift_lines(&self, lines: Range<usize>, change: fn(&str) -> String) {
        let text = self.slice(lines.clone());
        let trailing = text.ends_with('\n');
        let parts: Vec<&str> = text.strip_suffix('\n').unwrap_or(&text).split('\n').collect();
        let changed = parts.iter().map(|line| change(line)).collect::<Vec<_>>().join("\n") + if trailing { "\n" } else { "" };
        let selection = self.selection();
        let delta = changed.chars().count() as isize - lines.len() as isize;
        let select = if parts.len() == 1 {
            let start = (selection.start as isize + delta).max(lines.start as isize) as usize;
            start..start + selection.len()
        } else {
            lines.start..lines.start + changed.chars().count()
        };
        self.replace(lines, &changed, Some(select));
    }

    /// Tab, Shift+Tab or Return in a table (`MarkdownTable::tab_key`,
    /// `return_key`): the table formatted and the caret in the cell the
    /// key goes to, as one step to undo. Only the lines that start with a
    /// pipe are the table's, so the text beside it stays as it is. False
    /// where the key is the text's own: on a line that is no row, in a
    /// code block or the front matter, and with a selection over a pipe or
    /// a line. The note is read only on a key in a line that starts with a pipe.
    fn table_key(&self, key: TableKey) -> bool {
        if !MarkdownTable::is_row(&self.slice(self.current_line())) {
            return false;
        }
        let (text, selection) = (self.text(), self.selection());
        let edit = match key {
            TableKey::Tab { back } => MarkdownTable::tab_key(&text, selection, back),
            TableKey::Return => MarkdownTable::return_key(&text, selection),
        };
        let Some(edit) = edit else { return false };
        self.replace(edit.range, &edit.text, Some(edit.selection));
        true
    }

    /// Left and Right in a table drawn as a grid: at the edge of a cell's
    /// text the caret goes to the next cell's text, over the pipe and the
    /// padding, which are not drawn. Elsewhere, and in a line with
    /// right-to-left text, where the arrows go by what is seen, the key is
    /// the view's own.
    fn table_arrow(&self, right: bool) -> bool {
        let at = self.cursor();
        if !self.selection().is_empty() || !self.styler.in_grid(at) {
            return false;
        }
        let line = self.current_line();
        let text = self.slice(line.clone());
        if daftar_core::text::direction::contains_rtl(&text) {
            return false;
        }
        let place = if right { MarkdownTable::right(&text, at - line.start) } else { MarkdownTable::left(&text, at - line.start) };
        let Some(place) = place else { return false };
        self.buffer.place_cursor(&self.iter(line.start + place));
        self.view.scroll_mark_onscreen(&self.buffer.get_insert());
        true
    }

    /// A click in a table drawn as a grid puts the caret at the nearest
    /// place in the cell's text, never in the padding or on a pipe, which
    /// are not drawn; in an empty cell, between its two padding spaces.
    fn click_in_grid(&self) {
        let at = self.cursor();
        if !self.selection().is_empty() || !self.styler.in_grid(at) {
            return;
        }
        let line = self.current_line();
        let place = line.start + MarkdownTable::nearest(&self.slice(line.clone()), at - line.start);
        if place != at {
            self.buffer.place_cursor(&self.iter(place));
        }
    }

    fn move_lines(&self, up: bool) {
        match EditorCommands::move_lines(&self.text(), self.selection(), up) {
            Some(edit) => self.replace(edit.range, &edit.text, Some(edit.selection)),
            None => self.view.error_bell(),
        }
    }

    /// Ctrl+Alt+Up and Down, the Mac's ⌥⌘↑ and ⌥⌘↓: the section the cursor
    /// is in goes above the previous heading of its level or below the next,
    /// as one step to undo, the cursor with it. Above the first heading the
    /// lines move as Alt+Up and Down move them.
    fn move_section(&self, up: bool) {
        let text = self.text();
        let cursor = self.cursor();
        let line = text.chars().take(cursor).filter(|c| *c == '\n').count();
        let headings = Sections::headings(&text);
        let Some(index) = Sections::heading_at(line, &headings) else { return self.move_lines(up) };
        let siblings = Sections::siblings(index, &headings);
        let at = siblings.iter().position(|s| *s == index).unwrap_or(0);
        let place = if up { at.checked_sub(1) } else { Some(at + 1) };
        let Some(moved) = place.and_then(|place| Sections::move_section(&text, index, place)) else {
            return self.view.error_bell();
        };
        let start = |text: &str, line: usize| text.split('\n').take(line).map(|l| l.chars().count() + 1).sum::<usize>();
        let shifted = (cursor + start(&moved.text, moved.to)).saturating_sub(start(&text, moved.from)).min(moved.text.chars().count());
        self.replace(0..self.buffer.char_count().max(0) as usize, &moved.text, Some(shifted..shifted));
    }

    /// The section of heading [index] at place [position] among the
    /// headings of its level, from the outline, as one step to undo; false
    /// when it is there already.
    pub fn move_section_to(&self, index: usize, position: usize) -> bool {
        let text = self.text();
        let Some(moved) = Sections::move_section(&text, index, position) else { return false };
        let cursor = self.cursor().min(moved.text.chars().count());
        self.replace(0..self.buffer.char_count().max(0) as usize, &moved.text, Some(cursor..cursor));
        true
    }

    /// Ctrl+Return: tick, open, or make a task of the line.
    fn toggle_task(&self) {
        let line = self.current_line();
        let lines = EditorCommands::toggle_task(&self.slice(line.clone()), Day::today());
        let at = line.start + lines[0].chars().count();
        self.replace(line, &lines.join("\n"), Some(at..at));
    }

    /// Wraps the selection in a marker, or takes it off when it is there.
    fn wrap(&self, marker: &str) {
        let range = self.selection();
        let selected = self.slice(range.clone());
        let width = marker.chars().count();
        if selected.starts_with(marker) && selected.ends_with(marker) && selected.chars().count() >= 2 * width {
            let inner: String = selected.chars().skip(width).take(selected.chars().count() - 2 * width).collect();
            let length = inner.chars().count();
            self.replace(range.clone(), &inner, Some(range.start..range.start + length));
        } else {
            self.replace(range.clone(), &format!("{marker}{selected}{marker}"), Some(range.start + width..range.end + width));
        }
    }

    /// Ctrl+K: `[[selection]]`, and the note completion when nothing is selected.
    fn insert_link(self: &Rc<Self>) {
        let range = self.selection();
        let selected = self.slice(range.clone());
        self.replace(range.clone(), &format!("[[{selected}]]"), Some(range.start + 2..range.end + 2));
        if range.is_empty() {
            self.complete_links();
        }
    }

    /// The marks that wrap a selection, as Obsidian has them.
    fn wrapping(c: char) -> Option<char> {
        Some(match c {
            '*' | '_' | '`' | '~' | '=' | '"' => c,
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => return None,
        })
    }

    fn closing(c: char) -> Option<char> {
        Some(match c {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => return None,
        })
    }

    /// A typed character: a selection is wrapped; an opening bracket gets its
    /// closing one; a closing bracket typed where one was put steps over it;
    /// `[[` closes the link and offers notes; `/` offers snippets. True when
    /// the character is handled here.
    fn on_char(self: &Rc<Self>, typed: char) -> bool {
        let selection = self.selection();
        // The characters around the cursor, read from the buffer: a copy of
        // the whole note at each key costs a long note's typing.
        let count = usize::try_from(self.buffer.char_count()).unwrap_or(0);
        let char_at = |offset: usize| (offset < count).then(|| self.iter(offset).char());
        if !selection.is_empty()
            && let Some(close) = Self::wrapping(typed)
            && let inner = self.slice(selection.clone())
            && !inner.contains('\n')
        {
            self.replace(selection.clone(), &format!("{typed}{inner}{close}"), Some(selection.start + 1..selection.end + 1));
            return true;
        }
        if !selection.is_empty() {
            return false;
        }
        let at = selection.start;
        if self.auto_closed.get() == Some(at) && char_at(at) == Some(typed) && [')', ']', '}'].contains(&typed) {
            self.auto_closed.set(None);
            self.buffer.place_cursor(&self.iter(at + 1));
            return true;
        }
        // `[[`: the link's own closing, and the note completion.
        if typed == '[' && at >= 1 && char_at(at - 1) == Some('[') && char_at(at) == Some(']') && self.auto_closed.get() == Some(at) {
            self.replace(at..at + 1, "[]]", Some(at + 1..at + 1));
            self.auto_closed.set(None);
            self.complete_links();
            return true;
        }
        if typed == '[' && at >= 1 && char_at(at - 1) == Some('[') {
            self.replace(at..at, "[]]", Some(at + 1..at + 1));
            self.complete_links();
            return true;
        }
        if typed == '/' {
            // Looked at once the slash is in the text and styled, so it is
            // on screen under the list. A pasted `/` never comes here.
            self.slash_typed.set(Some(at));
            return false;
        }
        let Some(close) = Self::closing(typed) else {
            // Text typed inside the pair moves its closing bracket on.
            if let Some(auto) = self.auto_closed.get() {
                self.auto_closed.set(if at <= auto { Some(auto + 1) } else { None });
            }
            return false;
        };
        // Only before a space, the end of a line or another closing mark.
        let next = char_at(at).unwrap_or('\n');
        if !matches!(next, '\n' | ' ' | '\t' | ')' | ']' | '}' | ',' | '.') {
            return false;
        }
        self.replace(at..at, &format!("{typed}{close}"), Some(at + 1..at + 1));
        self.auto_closed.set(Some(at + 1));
        true
    }

    /// Backspace in an empty pair takes both.
    fn on_backspace(&self) -> bool {
        let selection = self.selection();
        let at = selection.start;
        if selection.is_empty() && at >= 1 && self.auto_closed.get() == Some(at) {
            let pair = self.slice(at - 1..at + 1);
            let mut letters = pair.chars();
            if let (Some(open), Some(close)) = (letters.next(), letters.next())
                && Self::closing(open) == Some(close)
            {
                self.auto_closed.set(None);
                self.replace(at - 1..at + 1, "", Some(at - 1..at - 1));
                return true;
            }
        }
        if let Some(auto) = self.auto_closed.get() {
            self.auto_closed.set(if selection.is_empty() && at <= auto && at > 0 { Some(auto - 1) } else { None });
        }
        false
    }

    // `[[` and `/`

    fn cursor_rectangle(&self) -> gdk::Rectangle {
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let location = self.view.iter_location(&at);
        let (x, y) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, location.x(), location.y());
        gdk::Rectangle::new(x, y, location.width().max(1), location.height())
    }

    /// Notes for the `[[` the cursor is in, ranked as the quick switcher ranks them.
    fn complete_links(self: &Rc<Self>) {
        let Some(store) = self.store.borrow().clone() else { return };
        {
            let index = store.index();
            *self.candidates.borrow_mut() = index
                .files
                .iter()
                .map(|path| {
                    let aliases = index.notes.get(path).map_or_default(|n| n.parsed.aliases.clone());
                    Candidate::new(path, &Vault::title(path), &aliases)
                })
                .collect();
        }
        self.completing.set(Completing::Link);
        self.follow_completion();
    }

    /// The list at the cursor, for what is typed now; closed when the cursor
    /// left the `[[` or the `/`.
    fn follow_completion(self: &Rc<Self>) {
        let choices = match self.completing.get() {
            Completing::Nothing => return,
            Completing::Link => {
                let Some(query) = EditorCommands::link_query(&self.text(), self.cursor()) else {
                    return self.stop_completion();
                };
                let typed = self.slice(query);
                QuickSwitcher::rank(&typed, &self.candidates.borrow(), &[], 50)
                    .into_iter()
                    .map(|path| Choice {
                        title: glib::markup_escape_text(&Vault::title(&path)).into(),
                        subtitle: QuickSwitcher::folder(&path),
                        value: path,
                    })
                    .collect::<Vec<_>>()
            }
            Completing::Slash(slash) => return self.follow_slash(slash),
        };
        // Nothing matches what is typed yet: no list, but the `[[` still
        // listens, as more is typed.
        if choices.is_empty() {
            self.popup.close();
            return;
        }
        let this = Rc::downgrade(self);
        let kind = self.completing.get();
        let below = self.cursor_rectangle().y() < self.view.height() / 2;
        self.popup.show(self.cursor_rectangle(), below, choices, move |choice| {
            if let Some(this) = this.upgrade() {
                this.completing.set(Completing::Nothing);
                if kind == Completing::Link {
                    this.insert_completion(&choice.value);
                }
            }
        });
    }

    fn stop_completion(&self) {
        self.completing.set(Completing::Nothing);
        self.popup.close();
        self.slash.close();
    }

    /// The note picked for `[[`: its name, or its path when another note has
    /// the same name, and the cursor past the `]]`.
    fn insert_completion(self: &Rc<Self>, path: &str) {
        let Some(query) = EditorCommands::link_query(&self.text(), self.cursor()) else { return };
        let files: BTreeSet<String> = self.store.borrow().as_ref().map_or_default(|s| s.index().files.clone());
        let link = EditorCommands::link(path, &files);
        let target = link.trim_start_matches('!').trim_start_matches("[[").trim_end_matches("]]").to_string();
        let end = query.start + target.chars().count();
        self.replace(query, &target, Some(end..end));
        if self.slice(end..end + 2) == "]]" {
            self.buffer.place_cursor(&self.iter(end + 2));
        }
        self.view.grab_focus();
    }

    // Pasting

    /// Ctrl+V: a web address on selected words makes them a link to it; a
    /// picture alone is saved in the attachment folder and embedded. True
    /// when the paste is handled here.
    fn paste(self: &Rc<Self>) -> bool {
        let clipboard = self.view.clipboard();
        let formats = clipboard.formats();
        let text = formats.contain_mime_type("text/plain") || formats.contain_mime_type("text/plain;charset=utf-8");
        let picture = formats.contains_type(gdk::Texture::static_type());
        if !text && picture {
            let this = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                if let Ok(Some(texture)) = clipboard.read_texture_future().await
                    && let Some(this) = this.upgrade()
                {
                    this.paste_picture(&texture);
                }
            });
            return true;
        }
        let selection = self.selection();
        let on_words = !selection.is_empty() && !self.slice(selection.clone()).contains('\n');
        let html = formats.contain_mime_type("text/html");
        if text && (on_words || html) {
            let this = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let Ok(Some(pasted)) = clipboard.read_text_future().await else { return };
                // A web address pasted on selected words makes them a link to it.
                let address = pasted.trim().to_string();
                if on_words && EditorCommands::is_web_address(&address) {
                    let Some(this) = this.upgrade() else { return };
                    let label = this.slice(selection.clone());
                    let end = selection.start + label.chars().count() + address.chars().count() + 4;
                    this.replace(selection, &format!("[{label}]({address})"), Some(end..end));
                    return;
                }
                // A web page or a document pasted as rich text keeps its
                // headings, links and lists as Markdown. Ctrl+Shift+V pastes
                // the plain text.
                let markdown = if html { read_html(&clipboard).await.and_then(|html| HtmlMarkdown::convert(&html)) } else { None };
                let Some(this) = this.upgrade() else { return };
                let text = markdown.unwrap_or_else(|| pasted.to_string());
                let at = selection.start + text.chars().count();
                this.replace(selection, &text, Some(at..at));
            });
            return true;
        }
        false
    }

    /// Ctrl+Shift+H, which the window's shortcut gives here while editing:
    /// the selection marked `==`, or the mark taken off.
    pub fn highlight(&self) {
        self.wrap("==");
    }

    /// Ctrl+Shift+V: the clipboard's text as it is.
    fn paste_plain(self: &Rc<Self>) {
        let clipboard = self.view.clipboard();
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(Some(pasted)) = clipboard.read_text_future().await else { return };
            let Some(this) = this.upgrade() else { return };
            let selection = this.selection();
            let at = selection.start + pasted.chars().count();
            this.replace(selection, &pasted, Some(at..at));
        });
    }

    /// The note's tasks sorted as the vault's Task Sorter plugin does, as
    /// one step to undo; false when they are in order already.
    pub fn sort_tasks(&self) -> bool {
        let Some(sorted) = TaskSorter::sort(&self.text()) else { return false };
        let cursor = self.cursor().min(sorted.chars().count());
        self.replace(0..self.buffer.char_count().max(0) as usize, &sorted, Some(cursor..cursor));
        true
    }

    /// "Sort Headings Under This" on heading [index], or on the heading of
    /// the section the cursor is in, as one step to undo; else what to say.
    pub fn sort_headings(&self, index: Option<usize>) -> Result<(), String> {
        let text = self.text();
        let cursor = self.cursor();
        let line = text.chars().take(cursor).filter(|c| *c == '\n').count();
        let sorted = Sections::sort_headings(&text, index, line)?;
        let cursor = cursor.min(sorted.chars().count());
        self.replace(0..self.buffer.char_count().max(0) as usize, &sorted, Some(cursor..cursor));
        Ok(())
    }

    fn paste_picture(&self, texture: &gdk::Texture) {
        let Some(store) = self.store.borrow().clone() else { return };
        if let Err(refused) = daftar_core::VaultEdits::writable(&store.root) {
            (self.notice)(&format!("The picture could not be saved: {refused}"));
            return;
        }
        let folder = attachment_folder(&store.root);
        let name = EditorCommands::pasted_image_name(chrono::Local::now().naive_local(), "png");
        let base = name.trim_end_matches(".png");
        let path = VaultMoves::free_name(base, "png", &folder, &store.index().files);
        let Some(file) = file_in(&path, &store.root) else { return };
        let written = file.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::write(&file, texture.save_to_png_bytes()));
        if let Err(error) = written {
            (self.notice)(&format!("The picture could not be saved: {error}"));
            return;
        }
        let file_name = path.rsplit('/').next().unwrap_or(&path);
        let embed = format!("![[{file_name}]]");
        if let Some(vim) = self.vim.borrow().as_ref() {
            vim.session.paste(&embed);
            return;
        }
        let selection = self.selection();
        let at = selection.start + embed.chars().count();
        self.replace(selection, &embed, Some(at..at));
    }

    // Saving

    /// A save a second from now. The buffer is not marked modified yet when
    /// `changed` runs, so the save looks at the mark when it runs instead:
    /// a single edit after a save, as a drop is, is saved too.
    fn schedule_save(self: &Rc<Self>) {
        if self.open.borrow().is_none() {
            return;
        }
        self.cancel_pending();
        let this = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(AFTER_TYPING, move || {
            if let Some(this) = this.upgrade() {
                this.pending.take();
                this.save();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn cancel_pending(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
    }

    // Neovim mode

    fn start_vim(self: &Rc<Self>, root: &Path, path: &str, line: usize) {
        let (configured, config) = {
            let settings = self.settings.borrow();
            (
                settings.file.string("editor.nvim").unwrap_or_default(),
                settings.file.string("editor.nvim_config").unwrap_or_else(|| "user".into()),
            )
        };
        let (rows, columns) = self.screen_size();
        let Some((session, events)) = NeovimSession::start(&configured, &config, root, path, rows, columns) else {
            (self.notice)("Neovim was not found, so the note is edited without it. Set editor.nvim in the settings to its path.");
            return;
        };
        session.set_persian(&neovim_session::keyboard_langmap());
        *self.vim.borrow_mut() =
            Some(Vim { session, mirror: None, mode: "n".into(), start_line: line, save_soon: None, wheel_top: None, command: false });
        self.glide.set_on(true);
        self.glide.set_shape(Shape::Block);
        self.caret.set_on(false);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.set_bar("NEOVIM", "Starting Neovim…");
        let this = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(this) = this.upgrade() else { return };
                if this.generation.get() != generation {
                    return;
                }
                this.vim_event(event);
            }
        });
    }

    fn stop_vim(&self) {
        let Some(mut vim) = self.vim.take() else { return };
        if let Some(id) = vim.save_soon.take() {
            id.remove();
        }
        // The session goes, and with it Neovim.
        self.generation.set(self.generation.get() + 1);
        self.buffer.remove_tag(&self.block_tag, &self.buffer.start_iter(), &self.buffer.end_iter());
        self.view.set_overwrite(false);
        self.glide.set_on(false);
        self.caret.set_on(true);
        self.draw_floats(&crate::vim_floats::Floats::default());
        self.popup.close();
        self.long_message.popdown();
        self.mode_label.set_visible(false);
        self.message.set_label("");
        self.tell_mode(None);
    }

    /// Neovim's floating windows over the text: one on the editor's screen
    /// where its cells put it, one anchored to the window beside the cursor,
    /// as far from it in cells as Neovim puts it. Each is its grid's rows
    /// in the code font and Neovim's colors.
    fn draw_floats(&self, floats: &crate::vim_floats::Floats) {
        for old in self.floats.borrow_mut().drain(..) {
            self.stage.remove_overlay(&old);
        }
        if floats.views.is_empty() {
            return;
        }
        let (family, size) = {
            let settings = self.settings.borrow();
            let code = settings.file.string("appearance.code_font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into());
            (code, settings.text_size())
        };
        if let Some(background) = floats.views.iter().find_map(|view| view.background.clone()) {
            self.float_css.load_from_string(&format!(".daftar-vim-float {{ background-color: {background}; }}"));
        }
        let origin = self
            .view
            .compute_point(&self.stage, &gtk::graphene::Point::new(0.0, 0.0))
            .map_or((0.0, 0.0), |p| (f64::from(p.x()), f64::from(p.y())));
        let (cursor_x, cursor_y) = self.glide.cursor_point();
        let (bound_w, bound_h) = (f64::from(self.stage.width()), f64::from(self.stage.height()));
        for view in &floats.views {
            let markup = format!("<span font=\"{} {}px\">{}</span>", glib::markup_escape_text(&family), size, view.rows.join("\n"));
            let label = gtk::Label::builder().use_markup(true).label(&markup).xalign(0.0).yalign(0.0).build();
            let (width, height) = label.layout().pixel_size();
            let (cell_w, cell_h) = (f64::from(width) / view.width.max(1) as f64, f64::from(height) / view.height.max(1) as f64);
            let (mut x, mut y) = if view.on_editor {
                (origin.0 + f64::from(self.view.left_margin()) + view.column * cell_w, origin.1 + view.row * cell_h)
            } else {
                let (row, column) = floats.window_cursor.unwrap_or((0, 0));
                (origin.0 + cursor_x + (view.column - column as f64) * cell_w, origin.1 + cursor_y + (view.row - row as f64) * cell_h)
            };
            if view.corner.ends_with('E') {
                x -= f64::from(width);
            }
            if view.corner.starts_with('S') {
                y -= f64::from(height);
            }
            let x = x.clamp(0.0, (bound_w - f64::from(width)).max(0.0));
            let y = y.clamp(0.0, (bound_h - f64::from(height)).max(0.0));
            let frame = gtk::Box::builder()
                .halign(gtk::Align::Start)
                .valign(gtk::Align::Start)
                .margin_start(x as i32)
                .margin_top(y as i32)
                .can_target(false)
                .css_classes(["daftar-vim-float"])
                .build();
            frame.append(&label);
            self.stage.add_overlay(&frame);
            self.floats.borrow_mut().push(frame.upcast());
        }
    }

    /// Rows and columns the view shows, so `<C-d>`, `zz` and `H`/`L` move by
    /// what is on screen.
    fn screen_size(&self) -> (i64, i64) {
        let metrics = self.view.pango_context().metrics(None, None);
        let scale = f64::from(gtk::pango::SCALE);
        let line = f64::from(metrics.ascent() + metrics.descent()) / scale + f64::from(self.view.pixels_below_lines());
        let width = f64::from(metrics.approximate_char_width()) / scale;
        let (height, across) =
            (f64::from(self.view.height()), f64::from(self.view.width() - self.view.left_margin() - self.view.right_margin()));
        if height < 1.0 || line < 1.0 || width < 1.0 {
            return (40, 100);
        }
        (((height / line) as i64).max(5), ((across / width) as i64).max(20))
    }

    fn vim_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Ready { lines, eol } => {
                let mirror = BufferMirror::new(lines, eol);
                let text = mirror.text();
                if text != self.text() {
                    self.remote_edit(0..self.buffer.char_count() as usize, &text);
                }
                let start = {
                    let mut vim = self.vim.borrow_mut();
                    let Some(vim) = vim.as_mut() else { return };
                    vim.mirror = Some(mirror);
                    vim.start_line
                };
                if let Some(vim) = self.vim.borrow().as_ref() {
                    vim.session.set_cursor(start + 1, 0, true);
                    let (rows, columns) = self.screen_size();
                    vim.session.resize(rows, columns);
                }
                self.set_bar("NORMAL", "");
                self.tell_mode(Some("NORMAL"));
                self.refresh_marks();
            }
            Event::Lines { first, last, lines } => {
                let edit = {
                    let mut vim = self.vim.borrow_mut();
                    let Some(mirror) = vim.as_mut().and_then(|v| v.mirror.as_mut()) else { return };
                    mirror.apply(first, last, lines)
                };
                self.remote_edit(edit.range, &edit.text);
                self.schedule_vim_save();
            }
            Event::Cursor { mode, row, column, visual } => self.vim_cursor(mode, row, column, visual),
            Event::Scroll(top) => {
                let offset = {
                    let mut vim = self.vim.borrow_mut();
                    let Some(vim) = vim.as_mut() else { return };
                    // Where the wheel left the page: it stays there.
                    if vim.wheel_top.take() == Some(top) {
                        return;
                    }
                    vim.mirror.as_ref().map(|m| m.offset(top, 0))
                };
                if let Some(offset) = offset {
                    self.glide.scroll_to_line(offset);
                }
            }
            Event::Status(text) => {
                if let Some(vim) = self.vim.borrow_mut().as_mut() {
                    vim.command = text.starts_with(':');
                }
                self.vim_status(&text);
            }
            Event::Menu { items, selected, cmdline } => self.vim_menu(items, selected, cmdline),
            Event::MenuSelect(at) => self.popup.select(at),
            Event::MenuHide => self.popup.close(),
            Event::Floats(floats) => self.draw_floats(&floats),
            Event::Failed(error) => {
                self.stop_vim();
                (self.notice)(&format!("Neovim did not start, so the note is edited without it: {error}"));
            }
            Event::Ended => {
                self.stop_vim();
                (self.notice)("Neovim ended. The note is edited without it until it is opened again.");
            }
        }
    }

    /// An edit Neovim made, taken into the view as it is.
    fn remote_edit(&self, range: Range<usize>, text: &str) {
        self.remote.set(true);
        let (mut start, mut end) = (self.iter(range.start), self.iter(range.end));
        self.buffer.delete(&mut start, &mut end);
        let mut at = self.iter(range.start);
        self.buffer.insert(&mut at, text);
        self.buffer.set_modified(false);
        self.remote.set(false);
    }

    /// Neovim writes the file a moment after the last change, and never in the
    /// middle of an insert: a config may format on save, and text moving under
    /// the cursor while one types is worse than a late save.
    fn schedule_vim_save(self: &Rc<Self>) {
        let mut vim = self.vim.borrow_mut();
        let Some(vim) = vim.as_mut() else { return };
        if let Some(id) = vim.save_soon.take() {
            id.remove();
        }
        let this = Rc::downgrade(self);
        vim.save_soon = Some(glib::timeout_add_local(AFTER_TYPING, move || {
            let Some(this) = this.upgrade() else { return glib::ControlFlow::Break };
            let mut vim = this.vim.borrow_mut();
            let Some(vim) = vim.as_mut() else { return glib::ControlFlow::Break };
            if !neovim_session::is_normal(&vim.mode) {
                return glib::ControlFlow::Continue;
            }
            vim.session.save();
            vim.save_soon = None;
            glib::ControlFlow::Break
        }));
    }

    fn vim_cursor(&self, mode: String, row: usize, column: usize, visual: Option<(usize, usize)>) {
        let (cursor, ranges) = {
            let vim = self.vim.borrow();
            let Some(mirror) = vim.as_ref().and_then(|v| v.mirror.as_ref()) else { return };
            let cursor = mirror.offset(row, column);
            let total = self.buffer.char_count() as usize;
            let line_end = |offset: usize| {
                let mut at = self.iter(offset);
                if !at.ends_line() {
                    at.forward_to_line_end();
                }
                at.offset() as usize
            };
            let ranges: Vec<Range<usize>> = match (mode.chars().next(), visual) {
                // Visual block: the same columns on every row, one range each.
                (Some('\u{16}'), Some((v_row, v_column))) => (v_row.min(row)..=v_row.max(row))
                    .map(|line| {
                        let start = mirror.offset(line, v_column.min(column));
                        let end = mirror.offset(line, v_column.max(column));
                        start..(end + 1).min(line_end(start)).max(start)
                    })
                    .collect(),
                (Some('v'), Some((v_row, v_column))) => {
                    let anchor = mirror.offset(v_row, v_column);
                    std::iter::once(anchor.min(cursor)..(anchor.max(cursor) + 1).min(total)).collect()
                }
                (Some('V'), Some((v_row, _))) => {
                    let (first, last) = (v_row.min(row), v_row.max(row));
                    let start = mirror.offset(first, 0);
                    let end = line_end(mirror.offset(last, 0));
                    std::iter::once(start..(end + 1).min(total)).collect()
                }
                _ => vec![],
            };
            (cursor, ranges)
        };
        self.buffer.remove_tag(&self.block_tag, &self.buffer.start_iter(), &self.buffer.end_iter());
        match ranges.as_slice() {
            [one] if !mode.starts_with('\u{16}') => {
                self.buffer.select_range(&self.iter(cursor), &self.iter(if cursor == one.start { one.end } else { one.start }));
            }
            [] => self.buffer.place_cursor(&self.iter(cursor)),
            many => {
                self.block_tag.set_background_rgba(Some(&self.view.color().with_alpha(0.25)));
                for range in many {
                    self.buffer.apply_tag(&self.block_tag, &self.iter(range.start), &self.iter(range.end));
                }
                self.buffer.place_cursor(&self.iter(cursor));
            }
        }
        // A block cursor in normal mode, gliding there; while inserting, the
        // editor's own caret.
        self.glide.set_shape(Shape::of(&mode));
        self.caret.set_on(Shape::of(&mode) == Shape::Bar);
        self.glide.cursor_moved();
        let name = neovim_session::mode_name(&mode);
        if let Some(vim) = self.vim.borrow_mut().as_mut() {
            vim.mode = mode;
        }
        self.set_bar(name, &self.message.label());
        self.tell_mode(Some(name));
    }

    /// Neovim's line: the command line or a message. One of more lines shows
    /// whole over the bar until the next key, as Neovim's own would.
    fn vim_status(&self, text: &str) {
        let first = text.lines().next().unwrap_or("");
        self.message.set_label(first);
        if text.contains('\n') {
            self.long_text.set_text(text);
            self.long_message.popup();
        } else {
            self.long_message.popdown();
        }
    }

    /// Neovim's completion menu: at the cursor for the text, over the bar
    /// for the command line. A click chooses as Neovim would.
    fn vim_menu(self: &Rc<Self>, items: Vec<MenuItem>, selected: Option<usize>, cmdline: bool) {
        let choices: Vec<Choice> = items
            .iter()
            .enumerate()
            .map(|(at, item)| Choice {
                title: glib::markup_escape_text(&item.word).to_string(),
                subtitle: [item.kind.as_str(), item.menu.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("  "),
                value: at.to_string(),
            })
            .collect();
        let (at, below) = if cmdline {
            (gdk::Rectangle::new(24, self.view.height() - 8, 1, 1), false)
        } else {
            let at = self.cursor_rectangle();
            (at, at.y() + at.height() + 320 < self.view.height())
        };
        let this = Rc::downgrade(self);
        self.popup.show(at, below, choices, move |choice| {
            let Some(this) = this.upgrade() else { return };
            if let (Some(vim), Ok(index)) = (this.vim.borrow().as_ref(), choice.value.parse()) {
                vim.session.choose(index);
            }
        });
        self.popup.select(selected);
    }

    /// A key the window has a shortcut for, that Neovim has first while it
    /// edits here: Ctrl with a letter or a bracket, but for Ctrl+E (back to
    /// reading), Ctrl+S and Ctrl+Q. With Shift or Alt, it is the app's.
    pub fn takes_from_window(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let running = self.vim.borrow().as_ref().is_some_and(|v| v.mirror.is_some());
        if !running || !self.view.has_focus() {
            return false;
        }
        let only_control = state
            & (gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK)
            == gdk::ModifierType::CONTROL_MASK;
        let neovims = match key.to_lower().to_unicode() {
            Some('e' | 's' | 'q') => false,
            Some(c) => c.is_ascii_lowercase() || c == '[' || c == ']',
            None => false,
        };
        if !only_control || !neovims {
            return false;
        }
        self.vim_key(key, 0, true, false, false, false);
        true
    }

    /// Keys in Neovim mode go to Neovim, but for the window's own Ctrl+E,
    /// Ctrl+S and Ctrl+V (a paste, which Neovim takes as `nvim_paste`).
    fn vim_key(self: &Rc<Self>, key: gdk::Key, keycode: u32, control: bool, alt: bool, super_: bool, shift: bool) -> glib::Propagation {
        let running = self.vim.borrow().as_ref().is_some_and(|v| v.mirror.is_some());
        if control && !alt && !shift && matches!(key.to_lower(), gdk::Key::e | gdk::Key::s) {
            return glib::Propagation::Proceed;
        }
        if !running {
            return glib::Propagation::Stop;
        }
        if control && shift && key.to_lower() == gdk::Key::v {
            let clipboard = self.view.clipboard();
            let this = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                if let Ok(Some(text)) = clipboard.read_text_future().await
                    && let Some(this) = this.upgrade()
                    && let Some(vim) = this.vim.borrow().as_ref()
                {
                    vim.session.paste(&text);
                }
            });
            return glib::Propagation::Stop;
        }
        // A message over the bar goes with the next key, as a hit-enter prompt does.
        self.long_message.popdown();
        let key = self.vim_persian_key(key, keycode);
        let name = key.name().map_or_default(|n| n.to_string());
        if let Some(keys) = neovim_session::vim_key(&name, key.to_unicode(), control, alt, super_, shift)
            && let Some(vim) = self.vim.borrow().as_ref()
        {
            vim.session.input(&keys);
        }
        glib::Propagation::Stop
    }

    /// Persian writing in Neovim: a `:` command takes the Latin letters of
    /// the keys typed on the Persian layout (a search, `/`, keeps Persian),
    /// and on a right-to-left line the arrows move the way they point.
    fn vim_persian_key(&self, key: gdk::Key, keycode: u32) -> gdk::Key {
        let vim = self.vim.borrow();
        let Some(vim) = vim.as_ref() else { return key };
        if vim.mode.starts_with('c') && vim.command {
            return neovim_session::latin_key(key, keycode).unwrap_or(key);
        }
        let swapped = match key {
            gdk::Key::Left => gdk::Key::Right,
            gdk::Key::Right => gdk::Key::Left,
            gdk::Key::KP_Left => gdk::Key::KP_Right,
            gdk::Key::KP_Right => gdk::Key::KP_Left,
            _ => return key,
        };
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let mut end = at;
        let mut start = self.buffer.iter_at_line(at.line()).unwrap_or(at);
        start.set_line_offset(0);
        end.forward_to_line_end();
        let line = self.buffer.text(&start, &end, false);
        if daftar_core::text::direction::first_strong(&line) == Some(true) { swapped } else { key }
    }

    /// A click on a task's box changes its status, and never moves the
    /// caret or the focus: the gesture is before the view's own and keeps
    /// the click from it. With Shift, Ctrl, Alt or Super it is a click in
    /// the text. The pointer is a hand over a box. Any other click moves
    /// Neovim's cursor where it landed.
    fn connect_clicks(self: &Rc<Self>) {
        let on_box = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).propagation_phase(gtk::PropagationPhase::Capture).build();
        let this = Rc::downgrade(self);
        on_box.connect_pressed(move |gesture, _, x, y| {
            let Some(this) = this.upgrade() else { return };
            let keys = gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK;
            // A click outside the link's card closes it.
            this.close_card();
            if gesture.current_event_state().intersects(keys) {
                return;
            }
            let (bx, by) = this.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            if let Some(offset) = this.draw.box_at(this.view.upcast_ref(), bx, by) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.click_box(offset.max(0) as usize);
            }
        });
        self.view.add_controller(on_box);
        let pointer = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        let over = move |place: Option<(f64, f64)>| {
            let Some(this) = this.upgrade() else { return };
            let on = place.and_then(|(x, y)| {
                let (bx, by) = this.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
                this.draw.box_at(this.view.upcast_ref(), bx, by)
            });
            if this.draw.set_hover(on) {
                this.view.queue_draw();
            }
            if this.over_box.replace(on.is_some()) != on.is_some() {
                this.view.set_cursor_from_name(Some(if on.is_some() { "pointer" } else { "text" }));
            }
        };
        let over = Rc::new(over);
        {
            let (over, this) = (over.clone(), Rc::downgrade(self));
            pointer.connect_motion(move |pointer, x, y| {
                over(Some((x, y)));
                if let Some(this) = this.upgrade() {
                    let buttons = gdk::ModifierType::BUTTON1_MASK | gdk::ModifierType::BUTTON2_MASK | gdk::ModifierType::BUTTON3_MASK;
                    this.hover_link(Some((x, y)), pointer.current_event_state().intersects(buttons));
                }
            });
        }
        let this = Rc::downgrade(self);
        pointer.connect_leave(move |_| {
            over(None);
            if let Some(this) = this.upgrade() {
                this.hover_link(None, false);
            }
        });
        self.view.add_controller(pointer);
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_released(move |_, _, _, _| {
            let Some(this) = this.upgrade() else { return };
            let this = Rc::downgrade(&this);
            glib::idle_add_local_once(move || {
                let Some(this) = this.upgrade() else { return };
                if this.vim.borrow().is_none() {
                    this.click_in_grid();
                }
                let vim = this.vim.borrow();
                let Some(vim) = vim.as_ref() else { return };
                let Some(mirror) = vim.mirror.as_ref() else { return };
                let (row, column) = mirror.position(this.cursor());
                vim.session.set_cursor(row, column, false);
            });
        });
        self.view.add_controller(click);
    }

    /// The bar under the text: Neovim's mode as a chip, and its command line.
    fn set_bar(&self, mode: &str, message: &str) {
        let dark = adw::StyleManager::default().is_dark();
        let color = match mode {
            "INSERT" => "#20ffa0",
            "VISUAL" | "V-LINE" | "V-BLOCK" => "#f0a0f0",
            "REPLACE" => "#ff7088",
            _ => "#ffb040",
        };
        let color = if dark {
            color
        } else {
            match color {
                "#20ffa0" => "#0a7a48",
                "#f0a0f0" => "#8040a0",
                "#ff7088" => "#c01030",
                _ => "#a85200",
            }
        };
        let text = if dark { "#323232" } else { "#faf9f5" };
        self.mode_label.set_markup(&format!(
            "<span font_family=\"monospace\" weight=\"bold\" size=\"small\" background=\"{color}\" foreground=\"{text}\"> {} </span>",
            glib::markup_escape_text(mode)
        ));
        self.mode_label.set_visible(true);
        self.message.set_label(message);
    }

    fn tell_mode(&self, mode: Option<&str>) {
        if let Some(changed) = self.on_mode.borrow().as_ref() {
            changed(mode);
        }
    }
}

/// [noun] for [count] of it: "1 word", "2 words".
fn plural(count: usize, noun: &str) -> String {
    if count == 1 { noun.to_string() } else { format!("{noun}s") }
}

/// A key that is the table's with the caret in one.
#[derive(Debug, Clone, Copy)]
enum TableKey {
    Tab { back: bool },
    Return,
}

/// The snippets of the slash menu, with today's date and this minute in them.
/// The heading level a digit key with Ctrl+Alt asks for: 0 for plain text.
fn heading_level(key: gdk::Key) -> Option<usize> {
    key.to_unicode().and_then(|c| c.to_digit(10)).filter(|d| *d <= 6).map(|d| d as usize)
}

/// The clipboard's `text/html`, as a browser or an office app puts it there:
/// UTF-8, or UTF-16 with its byte order mark.
async fn read_html(clipboard: &gdk::Clipboard) -> Option<String> {
    let (stream, _) = clipboard.read_future(&["text/html"], glib::Priority::DEFAULT).await.ok()?;
    let mut bytes = Vec::new();
    loop {
        let chunk = stream.read_bytes_future(64 * 1024, glib::Priority::DEFAULT).await.ok()?;
        if chunk.is_empty() || bytes.len() > 8 * 1024 * 1024 {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => {
            String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect::<Vec<_>>())
        }
        _ => String::from_utf8_lossy(&bytes).into_owned(),
    })
}

fn slash_snippets() -> Vec<Snippet> {
    EditorCommands::slash_commands(Day::today(), &chrono::Local::now().format("%H:%M").to_string())
}

/// Where a pasted picture goes: Obsidian's attachment folder for the vault,
/// `attachmentFolderPath` in `.obsidian/app.json`; the vault's root without one.
pub fn attachment_folder(root: &Path) -> String {
    let Ok(text) = fs::read_to_string(root.join(".obsidian/app.json")) else { return String::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return String::new() };
    match json.get("attachmentFolderPath").and_then(|v| v.as_str()) {
        Some(folder) if !folder.is_empty() && folder != "/" && !folder.starts_with("./") => folder.trim_end_matches('/').to_string(),
        _ => String::new(),
    }
}

/// Writes [text] to [file] through a temporary file and a rename, only while
/// the file still holds [expected].
pub fn write_if_unchanged(file: &Path, expected: &str, text: &str) -> Saved {
    // A read-only vault is never written; git finds the clone from the note's folder.
    if file.parent().is_some_and(daftar_core::Git::read_only) {
        return Saved::Failed(daftar_core::VaultEdits::READ_ONLY.into());
    }
    match fs::read_to_string(file) {
        Ok(current) if current != expected => return Saved::ChangedOnDisk,
        Ok(_) => {}
        Err(error) => return Saved::Failed(error.to_string()),
    }
    let Some(name) = file.file_name() else { return Saved::Failed("no file name".into()) };
    let temporary = file.with_file_name(format!(".{}.daftar-tmp", name.to_string_lossy()));
    if let Err(error) = fs::write(&temporary, text) {
        return Saved::Failed(error.to_string());
    }
    // The person's permissions stay the file's.
    if let Ok(metadata) = fs::metadata(file) {
        let _ = fs::set_permissions(&temporary, metadata.permissions());
    }
    match fs::rename(&temporary, file) {
        Ok(()) => Saved::Written,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Saved::Failed(error.to_string())
        }
    }
}

/// Daftar's two schemes, written where GtkSourceView looks, and the one for
/// the current scheme chosen now and whenever it changes. They set the text,
/// the background, the selection and the cursor; the markdown's colors are
/// the highlighter's tags.
fn install_schemes(buffer: &sourceview5::Buffer) {
    let folder = glib::user_cache_dir().join("daftar").join("styles");
    let _ = fs::create_dir_all(&folder);
    for (id, name, palette) in [("daftar-naz", "Daftar naz", &NAZ), ("daftar-daylight", "Daftar Daylight", &DAYLIGHT)] {
        let _ = fs::write(folder.join(format!("{id}.xml")), scheme(id, name, palette));
    }
    let manager = sourceview5::StyleSchemeManager::default();
    manager.append_search_path(&folder.to_string_lossy());
    manager.force_rescan();
    let style = adw::StyleManager::default();
    let apply = {
        let buffer = buffer.clone();
        move |dark: bool| {
            let id = if dark { "daftar-naz" } else { "daftar-daylight" };
            buffer.set_style_scheme(sourceview5::StyleSchemeManager::default().scheme(id).as_ref());
        }
    };
    apply(style.is_dark());
    style.connect_dark_notify(move |style| apply(style.is_dark()));
}

/// background, text, accent, current line, selection.
type Palette = [&'static str; 5];
const NAZ: Palette = ["#323232", "#f5f5f0", "#ffb040", "#3a3a35", "#4a4a4a"];
const DAYLIGHT: Palette = ["#faf9f5", "#2a2a28", "#a85200", "#efede6", "#d8d5cc"];

/// A GtkSourceView style scheme in the reader's colors.
fn scheme(id: &str, name: &str, p: &Palette) -> String {
    let [background, text, accent, line, selection] = p;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="{id}" name="{name}" version="1.0">
  <style name="text" foreground="{text}" background="{background}"/>
  <style name="selection" background="{selection}"/>
  <style name="cursor" foreground="{accent}"/>
  <style name="current-line" background="{line}"/>
</style-scheme>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_never_overwrites_a_change_on_disk() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("Note.md");
        fs::write(&file, "one\n").unwrap();
        assert_eq!(write_if_unchanged(&file, "one\n", "two\n"), Saved::Written);
        assert_eq!(fs::read_to_string(&file).unwrap(), "two\n");
        // Another app wrote meanwhile: the editor's text stays out.
        fs::write(&file, "three\n").unwrap();
        assert_eq!(write_if_unchanged(&file, "two\n", "mine\n"), Saved::ChangedOnDisk);
        assert_eq!(fs::read_to_string(&file).unwrap(), "three\n");
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 1);
    }

    /// Live preview's drawn parts in a real view (docs/specs/look.md, part
    /// 2): the tags the drawing follows, a part raw with the cursor in it,
    /// the click on a box, and the measure. Needs a display: `xvfb-run -a
    /// cargo test -p daftar -- --ignored live_preview_draws`, alone, as GTK
    /// keeps to the thread that started it.
    #[test]
    #[ignore = "needs a display"]
    fn live_preview_draws_the_parts_and_a_click_is_the_toggle() {
        use crate::vault_checks::tests::{on_a_loop_of_its_own, wait};
        on_a_loop_of_its_own(|context| {
            gtk::init().unwrap();
            adw::init().unwrap();
            crate::style::install();
            let vault = tempfile::tempdir().unwrap();
            // The task's line starts at 5, the callout at 29, the code block at 46.
            let text = "# T\n\n- [ ] Buy 📅 2001-01-01\n\n> [!tip]\n> body\n\n```Rust\nlet a;\n```\n";
            fs::write(vault.path().join("Note.md"), text).unwrap();
            let settings = AppSettings::of(vault.path().join("config.toml"), daftar_core::ConfigFile::TEMPLATE);
            let editor = Editor::new(Rc::new(RefCell::new(settings)), |_| {}, |_| {});
            let window = gtk::Window::builder().default_width(1100).default_height(700).child(&editor.widget).build();
            window.present();
            assert!(editor.load(vault.path(), "Note.md", Some(0)));
            let tagged =
                |name: &str, at: usize| editor.iter(at).tags().iter().any(|tag| tag.name().is_some_and(|n| n == format!("daftar:{name}")));

            // Each part has its tag: the box over `- [ ]`, the late chip, the
            // callout's panel with its icon where `[` is, the code block's
            // panel with its fence hidden and its language word.
            assert!(wait(context, || tagged("box:5", 5)), "the box is drawn");
            assert!(tagged("box:5", 9) && !tagged("box:5", 10), "over the five marks and no more");
            assert!(tagged("chip:red:late", 15) && tagged("chip:red:late", 26) && !tagged("chip:red:late", 27));
            assert!(tagged("chip-gap", 14), "the room before the pill is in the space");
            assert!(
                tagged("panel:callout:chartreuse", 29) && tagged("panel:callout:chartreuse", 40) && !tagged("panel:callout:chartreuse", 45)
            );
            assert!(tagged("panel-top", 29) && tagged("panel-bottom", 38) && !tagged("panel-bottom", 29));
            assert!(tagged("hidden", 29) && tagged("callout-icon", 31) && tagged("hidden", 32));
            assert!(tagged("panel:code", 46) && tagged("panel:code", 60) && tagged("panel-top", 46) && tagged("panel-bottom:code", 61));
            assert!(tagged("hidden", 46) && tagged("language:orange", 49) && tagged("code-body", 54) && tagged("hidden", 61));
            assert_eq!(editor.styler.drawn().len(), 6, "a box, a chip, a callout's icon and panel, a code panel, a first line");

            // The cursor in the box shows it raw, and drawn again when it leaves;
            // on the callout's first line its marks show.
            editor.buffer.place_cursor(&editor.iter(7));
            assert!(wait(context, || !tagged("box:5", 5)), "raw with the cursor inside");
            assert!(tagged("chip:red:late", 15), "the chip of the same line stays drawn");
            editor.buffer.place_cursor(&editor.iter(20));
            assert!(wait(context, || tagged("box:5", 5) && !tagged("chip:red:late", 15)));
            editor.buffer.place_cursor(&editor.iter(33));
            assert!(wait(context, || !tagged("callout-icon", 31) && !tagged("hidden", 29) && tagged("chip:red:late", 15)));
            assert!(tagged("panel:callout:chartreuse", 29), "the panel stays");

            // A click on the box is the task toggle: one step to undo, the
            // cursor and the selection where they were, the file not written.
            editor.buffer.select_range(&editor.iter(13), &editor.iter(11));
            editor.click_box(5);
            let done = format!("- [x] Buy 📅 2001-01-01 ✅ {}", Day::today());
            assert_eq!(editor.text().lines().nth(2), Some(done.as_str()));
            assert_eq!(editor.selection(), 11..13);
            assert_eq!(editor.cursor(), 13);
            assert_eq!(fs::read_to_string(vault.path().join("Note.md")).unwrap(), text, "the save is the editor's ordinary one");
            assert!(wait(context, || tagged("chip:green:closed", 28)), "the done date is a chip, weaker on a closed task");
            editor.buffer.undo();
            assert_eq!(editor.text(), text, "one undo takes the click back");

            // The line is the measure, 35 em of 16 px, in the middle of the pane.
            let pane = editor.view.hadjustment().unwrap().page_size();
            assert!(wait(context, || editor.view.left_margin() > 32));
            assert!((pane - 2.0 * f64::from(editor.view.left_margin()) - 560.0).abs() <= 1.0, "{pane}");
            assert_eq!(editor.view.left_margin(), editor.view.right_margin());
            assert!(editor.view.is_cursor_visible(), "the arrow keys move the cursor of a view that has one");
            editor.discard();
            window.destroy();
        });
    }

    /// The table's grid and its keys, the slash bar, focus mode and the
    /// link preview in a real view (docs/specs/look.md, part 2). Needs a
    /// display: `xvfb-run -a cargo test -p daftar -- --ignored
    /// the_grid_the_slash_bar`, alone, as GTK keeps to the thread that
    /// started it.
    #[test]
    #[ignore = "needs a display"]
    fn the_grid_the_slash_bar_focus_mode_and_the_link_card_in_a_view() {
        use crate::vault_checks::tests::{on_a_loop_of_its_own, wait};
        on_a_loop_of_its_own(|context| {
            gtk::init().unwrap();
            adw::init().unwrap();
            crate::style::install();
            let vault = tempfile::tempdir().unwrap();
            let text = "Intro with [[Plan#Next|the plan]] and [[Nowhere]].\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n\nlast\n\n| X | Y |\n| --- | --- |\n| **b** | c |\n";
            fs::write(vault.path().join("Note.md"), text).unwrap();
            fs::create_dir_all(vault.path().join("Work")).unwrap();
            fs::write(vault.path().join("Work/Plan.md"), "# Plan\n\nfirst\n\n## Next\n\n- [ ] do it\nthen rest\n").unwrap();
            let index = Vault::index(vault.path(), None);
            let store = Rc::new(VaultStore::new(vault.path().to_path_buf(), index, std::sync::Arc::new(daftar_core::SearchIndex::new())));
            let settings = Rc::new(RefCell::new(AppSettings::of(vault.path().join("config.toml"), daftar_core::ConfigFile::TEMPLATE)));
            let editor = Editor::new(settings.clone(), |_| {}, |_| {});
            editor.set_store(store);
            let window = gtk::Window::builder().default_width(1100).default_height(700).child(&editor.widget).build();
            window.present();
            // A note opened with focus mode on has its first paragraph
            // bright, not the whole note.
            assert!(!settings.borrow().focus(), "`editor.focus` is false with no setting");
            settings.borrow_mut().file.set("editor.focus", daftar_core::Value::Bool(true));
            assert!(editor.load(vault.path(), "Note.md", Some(0)));
            assert_eq!(editor.focus.bright_lines(), Some((0, 0)));
            settings.borrow_mut().file.set("editor.focus", daftar_core::Value::Bool(false));
            editor.settings_changed();
            // Where [needle] starts in the editor's text, in characters.
            let at = |needle: &str| {
                let text = editor.text();
                text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?} is in {text:?}"))].chars().count()
            };
            let tagged =
                |name: &str, at: usize| editor.iter(at).tags().iter().any(|tag| tag.name().is_some_and(|n| n == format!("daftar:{name}")));
            let none = gdk::ModifierType::empty();
            let key = |key: gdk::Key, state: gdk::ModifierType| editor.key(key, 0, state) == glib::Propagation::Stop;
            // Keys as the view gets them: one the editor leaves alone goes into the text.
            let typed = |letters: &str| {
                for letter in letters.chars() {
                    let name = match letter {
                        '/' => "slash".to_string(),
                        ' ' => "space".to_string(),
                        letter => letter.to_string(),
                    };
                    if !key(gdk::Key::from_name(name).unwrap(), none) {
                        editor.buffer.insert_at_cursor(&letter.to_string());
                    }
                }
            };
            // The loop runs for this long, for what a timer does.
            let spin = |ms: u64| {
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(ms) {
                    while context.iteration(false) {}
                    std::thread::sleep(Duration::from_millis(5));
                }
            };

            // The table is a grid: its rows one run, the header, the
            // delimiter row at no height, each pipe clear, the last row marked.
            let (head, rule, one) = (at("| A |"), at("| --- |"), at("| 1 |"));
            assert!(wait(context, || tagged("grid", head)), "the table is a grid");
            assert!(
                tagged("grid-head", head + 2)
                    && tagged("grid-pipe", head)
                    && tagged("grid-pipe", head + 4)
                    && !tagged("grid-pipe", head + 2)
            );
            assert!(tagged("grid-rule", rule) && tagged("grid", one) && tagged("grid-last", at("| 3 |")) && !tagged("grid-last", one));
            assert!(!tagged("grid", at("last")) && editor.styler.in_grid(one) && !editor.styler.in_grid(0));

            // The caret in a cell keeps the grid; on the delimiter row, and
            // with a selection that reaches outside one cell, the table is raw.
            editor.buffer.place_cursor(&editor.iter(one + 2));
            spin(50);
            assert!(tagged("grid", head) && tagged("grid-pipe", one), "a cell is edited in place");
            editor.buffer.place_cursor(&editor.iter(rule + 2));
            assert!(wait(context, || !tagged("grid", head)), "raw with the caret on the delimiter row");
            assert!(!tagged("grid-rule", rule) && !tagged("grid-pipe", head) && !tagged("grid", at("| 3 |")));
            editor.buffer.place_cursor(&editor.iter(one + 2));
            assert!(wait(context, || tagged("grid", head) && tagged("grid-rule", rule)), "drawn again when the caret leaves");
            editor.buffer.select_range(&editor.iter(one + 2), &editor.iter(one + 6));
            assert!(wait(context, || !tagged("grid", head)), "raw with a selection over two cells");
            editor.buffer.select_range(&editor.iter(one + 2), &editor.iter(one + 3));
            assert!(wait(context, || tagged("grid", head)), "a selection inside one cell keeps the grid");

            // A cell has part 1's marks rule: `**` shows with the caret in
            // the bold text and hides when it leaves. What the hidden marks
            // lost of the column's width is given back in the cell's padding.
            let bold = at("| **b**");
            let padded = |at: usize| editor.iter(at).tags().iter().any(|tag| tag.name().is_some_and(|n| n.starts_with("daftar:grid-pad:")));
            assert!(wait(context, || tagged("hidden", bold + 2) && tagged("hidden", bold + 6) && padded(bold + 7)), "hidden, and padded");
            editor.buffer.place_cursor(&editor.iter(bold + 5));
            assert!(wait(context, || !tagged("hidden", bold + 2) && !padded(bold + 7)), "the marks show with the caret in the element");
            assert!(tagged("grid", bold) && tagged("grid-pipe", bold));
            editor.buffer.place_cursor(&editor.iter(bold + 11));
            assert!(wait(context, || tagged("hidden", bold + 2) && padded(bold + 7)), "and hide when it leaves, in the same row");

            // Right at the end of a cell's text goes to the next cell's text, and Left back.
            editor.buffer.place_cursor(&editor.iter(one + 3));
            assert!(key(gdk::Key::Right, none));
            assert_eq!(editor.cursor(), one + 6);
            assert!(key(gdk::Key::Left, none));
            assert_eq!(editor.cursor(), one + 3);
            assert!(!key(gdk::Key::Left, none), "inside the text the key is the view's");

            // Return goes to the same column of the row below, with the
            // table formatted, as one step to undo.
            assert!(key(gdk::Key::Return, none));
            assert!(editor.text().contains("| A   | B   |\n| --- | --- |\n| 1   | 2   |\n| 3   | 4   |\n"));
            assert_eq!(editor.cursor(), at("| 3  ") + 2);
            editor.buffer.undo();
            assert_eq!(editor.text(), text, "one undo takes the move and its formatting back");
            // Shift+Return is Return. On the last row a new row is added,
            // the caret in its same column, and Return on it ends the table.
            editor.buffer.place_cursor(&editor.iter(at("| 3 |") + 6));
            assert!(key(gdk::Key::Return, gdk::ModifierType::SHIFT_MASK));
            assert!(editor.text().contains("| 3   | 4   |\n|     |     |\n"));
            let added = at("|     |     |");
            assert_eq!(editor.cursor(), added + 8);
            assert!(wait(context, || tagged("grid-last", added) && !tagged("grid-last", at("| 3  "))), "the box ends under the new row");
            assert!(key(gdk::Key::Return, none));
            assert!(editor.text().contains("| 3   | 4   |\n\n") && !editor.text().contains("|     |"), "{}", editor.text());
            // Tab from the header's last cell passes the delimiter row.
            editor.buffer.place_cursor(&editor.iter(at("| A  ") + 8));
            assert!(key(gdk::Key::Tab, none));
            assert_eq!(editor.cursor(), at("| 1  ") + 2);
            // Outside a table Shift+Return is the view's own key.
            editor.buffer.place_cursor(&editor.iter(at("last")));
            assert!(!key(gdk::Key::Return, gdk::ModifierType::SHIFT_MASK));

            // Focus mode: the caret's paragraph is the bright one. The table
            // is lines 2 to 5; a blank line is alone; a selection keeps
            // every paragraph it touches.
            assert!(!editor.focus.is_on() && editor.focus.bright_lines().is_none());
            settings.borrow_mut().file.set("editor.focus", daftar_core::Value::Bool(true));
            editor.settings_changed();
            assert!(editor.focus.is_on());
            editor.buffer.place_cursor(&editor.iter(at("| 1  ") + 2));
            assert!(wait(context, || editor.focus.bright_lines() == Some((2, 5))), "{:?}", editor.focus.bright_lines());
            editor.buffer.place_cursor(&editor.iter(3));
            assert!(wait(context, || editor.focus.bright_lines() == Some((0, 0))));
            let blank = at("| 3  ") + 15;
            editor.buffer.place_cursor(&editor.iter(blank));
            assert!(wait(context, || editor.focus.bright_lines() == Some((7, 7))), "{:?}", editor.focus.bright_lines());
            // A blank line typed in is still its own paragraph, between blank lines.
            editor.buffer.insert_at_cursor("x");
            spin(50);
            assert_eq!(editor.focus.bright_lines(), Some((7, 7)));
            editor.buffer.undo();
            editor.buffer.select_range(&editor.iter(3), &editor.iter(at("| 1  ") + 2));
            assert!(wait(context, || editor.focus.bright_lines() == Some((0, 5))), "{:?}", editor.focus.bright_lines());
            // A line break in a paragraph of two lines parts it.
            editor.buffer.place_cursor(&editor.iter(at("last") + 4));
            editor.buffer.insert_at_cursor("\nmore");
            assert!(wait(context, || editor.focus.bright_lines().is_some_and(|(first, last)| last == first + 1)));
            let two = editor.focus.bright_lines().unwrap();
            editor.buffer.place_cursor(&editor.iter(at("more")));
            editor.buffer.insert_at_cursor("\n");
            assert!(wait(context, || editor.focus.bright_lines() == Some((two.1 + 1, two.1 + 1))), "{:?}", editor.focus.bright_lines());
            settings.borrow_mut().file.set("editor.focus", daftar_core::Value::Bool(false));
            editor.settings_changed();
            assert!(!editor.focus.is_on() && editor.focus.bright_lines().is_none());

            // The slash bar: a `/` at a line's start opens it with every
            // item, the first chosen; what is typed after it filters.
            editor.buffer.place_cursor(&editor.buffer.end_iter());
            editor.buffer.insert_at_cursor("\n");
            typed("/");
            assert!(wait(context, || editor.slash.is_open()), "a `/` at a line's start opens the bar");
            let (chosen, shown) = editor.slash.shown();
            assert_eq!((chosen.as_deref(), shown.len()), (Some("h1"), 36));
            // Down, Up, Ctrl+N and Ctrl+P choose, and stop at the ends.
            assert!(key(gdk::Key::n, gdk::ModifierType::CONTROL_MASK) && key(gdk::Key::Down, none));
            assert_eq!(editor.slash.shown().0.as_deref(), Some("h3"));
            assert!(key(gdk::Key::Up, none) && key(gdk::Key::p, gdk::ModifierType::CONTROL_MASK) && key(gdk::Key::Up, none));
            assert_eq!(editor.slash.shown().0.as_deref(), Some("h1"));
            typed("tab");
            assert!(wait(context, || editor.slash.shown().0.as_deref() == Some("table")), "{:?}", editor.slash.shown());
            // Nothing matches: the list hides and still listens, Return is
            // an ordinary key, and Backspace brings the list back.
            typed("zz");
            assert!(wait(context, || !editor.slash.is_open()));
            assert!(matches!(editor.completing.get(), Completing::Slash(_)));
            assert!(!key(gdk::Key::Return, none), "with the list hidden Return is an ordinary key");
            let backspace = || {
                if !key(gdk::Key::BackSpace, none) {
                    let mut at = editor.buffer.iter_at_mark(&editor.buffer.get_insert());
                    editor.buffer.backspace(&mut at, true, true);
                }
            };
            backspace();
            backspace();
            assert!(wait(context, || editor.slash.is_open() && editor.slash.shown().0.as_deref() == Some("table")));
            // Escape closes the list and leaves the `/` and what was typed;
            // the same `/` does not open it again.
            assert!(key(gdk::Key::Escape, none));
            assert!(!editor.slash.is_open() && editor.completing.get() == Completing::Nothing);
            assert!(editor.text().ends_with("\n/tab"));
            typed("l");
            spin(100);
            assert!(!editor.slash.is_open());
            // Return inserts the chosen row in place of the `/` and what was typed, as one step.
            editor.buffer.insert_at_cursor("\n");
            typed("/task");
            assert!(wait(context, || editor.slash.is_open() && editor.slash.shown().0.as_deref() == Some("task")));
            assert!(key(gdk::Key::Return, none));
            assert!(editor.text().ends_with("\n- [ ] ") && !editor.slash.is_open(), "{}", editor.text());
            assert_eq!(editor.cursor(), editor.buffer.char_count() as usize);
            editor.buffer.undo();
            assert!(editor.text().ends_with("\n/task"), "one undo takes the insert back");
            // A space closes the list; so does the caret leaving the `/`.
            editor.buffer.place_cursor(&editor.buffer.end_iter());
            editor.buffer.insert_at_cursor("\n");
            typed("/h");
            assert!(wait(context, || editor.slash.is_open()));
            typed(" ");
            assert!(wait(context, || !editor.slash.is_open() && editor.completing.get() == Completing::Nothing));
            editor.buffer.insert_at_cursor("\n");
            typed("/he");
            assert!(wait(context, || editor.slash.is_open()));
            editor.buffer.place_cursor(&editor.iter(editor.cursor() - 3));
            assert!(wait(context, || !editor.slash.is_open() && editor.completing.get() == Completing::Nothing));
            // A `/` in a word, in inline code, and with the setting off opens nothing.
            editor.buffer.place_cursor(&editor.buffer.end_iter());
            editor.buffer.insert_at_cursor("\n");
            typed("a/");
            spin(100);
            assert!(!editor.slash.is_open() && editor.completing.get() == Completing::Nothing);
            editor.buffer.insert_at_cursor("\n`x y`");
            editor.buffer.place_cursor(&editor.iter(editor.cursor() - 1));
            typed(" /");
            spin(100);
            assert!(!editor.slash.is_open() && editor.completing.get() == Completing::Nothing, "not in inline code");
            settings.borrow_mut().file.set("editor.slash_menu", daftar_core::Value::Bool(false));
            editor.buffer.place_cursor(&editor.buffer.end_iter());
            editor.buffer.insert_at_cursor("\n");
            typed("/");
            spin(100);
            assert!(!editor.slash.is_open(), "`editor.slash_menu = false`: a `/` is only a `/`");

            // The link preview: the card of a link to a heading has the
            // note's title, the heading, the folder and the lines under the heading.
            let link = at("[[Plan#Next|the plan]]");
            editor.show_card((link, link + 22));
            assert!(editor.card.is_open());
            assert_eq!(editor.card.said(), ("Plan".into(), "› Next".into(), "Work".into(), "- [ ] do it\nthen rest".into()));
            // Any key closes it at once.
            editor.key(gdk::Key::Shift_L, 0, none);
            assert!(!editor.card.is_open());
            let missing = at("[[Nowhere]]");
            editor.show_card((missing, missing + 11));
            assert_eq!(editor.card.said(), ("Nowhere".into(), String::new(), String::new(), "No note yet.".into()));
            editor.close_card();
            // The pointer rests on the link for 350 ms before the card
            // shows, and the card stays 250 ms after the pointer left.
            let place = editor.view.iter_location(&editor.iter(at("the plan") + 2));
            let (x, y) = editor.view.buffer_to_window_coords(gtk::TextWindowType::Widget, place.x() + 2, place.y() + place.height() / 2);
            let point = Some((f64::from(x), f64::from(y)));
            let rested = std::time::Instant::now();
            editor.hover_link(point, false);
            assert!(!editor.card.is_open(), "not before the pointer has rested");
            assert!(wait(context, || editor.card.is_open()), "after 350 ms on the link");
            assert!(rested.elapsed() >= Duration::from_millis(daftar_core::LinkPreview::REST - 10), "{:?}", rested.elapsed());
            assert_eq!(editor.card.said().0, "Plan");
            let left = std::time::Instant::now();
            editor.hover_link(None, false);
            assert!(editor.card.is_open(), "it stays while the pointer goes to it");
            assert!(wait(context, || !editor.card.is_open()), "and closes after the pointer left");
            assert!(left.elapsed() >= Duration::from_millis(daftar_core::LinkPreview::LINGER - 10), "{:?}", left.elapsed());
            // Off a link there is no card; a key pressed since the pointer
            // last moved opens none; a button held down opens none.
            editor.hover_link(Some((f64::from(x), f64::from(y) + 300.0)), false);
            spin(450);
            assert!(!editor.card.is_open());
            editor.hover_link(point, false);
            editor.key(gdk::Key::Shift_L, 0, none);
            spin(450);
            assert!(!editor.card.is_open(), "typing under a still pointer opens no card");
            editor.hover_link(point, true);
            spin(450);
            assert!(!editor.card.is_open(), "not while a mouse button is down");
            settings.borrow_mut().file.set("appearance.link_preview", daftar_core::Value::Bool(false));
            editor.hover_link(Some((f64::from(x) + 1.0, f64::from(y))), false);
            spin(450);
            assert!(!editor.card.is_open(), "`appearance.link_preview = false`");
            editor.discard();
            window.destroy();
        });
    }

    /// The keys and the clicks of the drawn parts keep the person's text
    /// (docs/specs/look.md, part 2): a `|` line in a code block is no
    /// table, a click on a box is `TaskLine::toggled` and one step to undo
    /// in a Persian line, a repeating task and a callout, a Persian table
    /// is only formatted, and a read-only vault's text never changes.
    /// Needs a display: `xvfb-run -a cargo test -p daftar -- --ignored
    /// the_keys_and_the_clicks_keep_the_text`, alone.
    #[test]
    #[ignore = "needs a display"]
    fn the_keys_and_the_clicks_keep_the_text() {
        use crate::vault_checks::tests::{on_a_loop_of_its_own, wait};
        on_a_loop_of_its_own(|context| {
            gtk::init().unwrap();
            adw::init().unwrap();
            crate::style::install();
            let vault = tempfile::tempdir().unwrap();
            let text = "---\n| k | v |\n---\n\n```sh\n| a | b |\ncat x\n```\n\n- [ ] سلام دنیا 📅 2026-11-20\n- [ ] Water 🔁 every week 📅 2026-11-20\n\n> [!tip] T\n> - [ ] in callout\n\n| نام | B |\n| --- | --- |\n| یک | 2 |\n\n- [x] one ✅ 2001-01-01\n- [ ] ab [[Some Person]] and more\n\nend\n";
            fs::write(vault.path().join("Note.md"), text).unwrap();
            let index = Vault::index(vault.path(), None);
            let store = Rc::new(VaultStore::new(vault.path().to_path_buf(), index, std::sync::Arc::new(daftar_core::SearchIndex::new())));
            let settings = Rc::new(RefCell::new(AppSettings::of(vault.path().join("config.toml"), daftar_core::ConfigFile::TEMPLATE)));
            let editor = Editor::new(settings.clone(), |_| {}, |_| {});
            editor.set_store(store);
            let window = gtk::Window::builder().default_width(1100).default_height(700).child(&editor.widget).build();
            window.present();
            assert!(editor.load(vault.path(), "Note.md", Some(0)));
            let at = |needle: &str| {
                let text = editor.text();
                text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?} is in {text:?}"))].chars().count()
            };
            let tagged = |name: &str, at: usize| {
                editor.iter(at).tags().iter().any(|tag| tag.name().is_some_and(|n| n.starts_with(&format!("daftar:{name}"))))
            };
            let none = gdk::ModifierType::empty();
            let key = |key: gdk::Key, state: gdk::ModifierType| editor.key(key, 0, state) == glib::Propagation::Stop;
            let today = Day::today().to_string();
            let spin = |ms: u64| {
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(ms) {
                    while context.iteration(false) {}
                    std::thread::sleep(Duration::from_millis(5));
                }
            };

            // A `|` line in a code block and in the front matter is no table:
            // Return and Shift+Return are the view's own keys, Tab indents as
            // in any text, and no line is formatted.
            assert!(wait(context, || tagged("box", at("- [ ] Water"))), "the note is styled");
            for needle in ["| a | b |", "| k | v |"] {
                editor.buffer.place_cursor(&editor.iter(at(needle) + 4));
                assert!(!key(gdk::Key::Return, none), "{needle}");
                assert!(!key(gdk::Key::Return, gdk::ModifierType::SHIFT_MASK), "{needle}");
                assert_eq!(editor.text(), text, "{needle}");
            }
            editor.buffer.place_cursor(&editor.iter(at("| a | b |") + 4));
            assert!(key(gdk::Key::Tab, none));
            assert_eq!(editor.text(), text.replace("| a | b |", "| a     | b |"), "Tab is four spaces in code, not a table's move");
            editor.buffer.undo();
            assert_eq!(editor.text(), text);
            assert!(!editor.styler.in_grid(at("| a | b |")), "and it is no grid");

            // A click on the box of a Persian line: where the box is drawn
            // is where the click lands, and the line is the toggle's.
            let persian = at("- [ ] سلام");
            let hit = |offset: usize| {
                let place = editor.view.iter_location(&editor.iter(offset + 2));
                editor.draw.box_at(editor.view.upcast_ref(), place.x() + place.width() / 2, place.y() + place.height() / 2)
            };
            assert!(wait(context, || hit(persian) == Some(persian as i32)), "the Persian line's box takes the click: {:?}", hit(persian));
            editor.buffer.place_cursor(&editor.iter(at("دنیا") + 2));
            let caret = editor.cursor();
            editor.click_box(persian);
            assert_eq!(editor.text(), text.replace("- [ ] سلام دنیا 📅 2026-11-20", &format!("- [x] سلام دنیا 📅 2026-11-20 ✅ {today}")));
            assert_eq!(editor.cursor(), caret, "the caret stays in its word");
            editor.buffer.undo();
            assert_eq!(editor.text(), text, "one undo");

            // A repeating task leaves its next occurrence on the line above,
            // the caret stays on the task's own text, and one undo takes both lines back.
            let water = "- [ ] Water 🔁 every week 📅 2026-11-20";
            let toggled = daftar_core::TaskLine::toggled(water, &today).unwrap();
            assert_eq!(toggled.len(), 2, "{toggled:?}");
            editor.buffer.place_cursor(&editor.iter(at("Water") + 3));
            editor.click_box(at(water));
            assert_eq!(editor.text(), text.replace(water, &toggled.join("\n")));
            let done = at(&toggled[1]);
            assert_eq!(editor.cursor(), done + 6 + 3, "on the done line's `Water`");
            assert_eq!(editor.text().chars().skip(editor.cursor() - 3).take(5).collect::<String>(), "Water");
            editor.buffer.undo();
            assert_eq!(editor.text(), text, "one undo takes both lines back");
            editor.buffer.redo();
            assert_eq!(editor.text(), text.replace(water, &toggled.join("\n")));
            editor.buffer.undo();

            // A click on a box above the caret moves the caret's line and
            // sets no mark: the line under it, with a link across where
            // its start was, is styled where it is now, and its marks stay
            // hidden. The same when the click adds a line above.
            let link = "[[Some Person]]";
            editor.buffer.place_cursor(&editor.iter(at(link) + 24));
            assert!(wait(context, || tagged("hidden", at(link)) && tagged("box", at("- [x] one"))));
            editor.click_box(at("- [x] one"));
            spin(100);
            assert_eq!(editor.text(), text.replace("- [x] one ✅ 2001-01-01", "- [ ] one"));
            assert_eq!(editor.cursor(), at(link) + 24, "the caret stays on its text");
            assert!(tagged("hidden", at(link)) && tagged("hidden", at(link) + 14) && tagged("box", at("- [ ] ab")));
            editor.click_box(at(water));
            spin(100);
            assert_eq!(editor.cursor(), at(link) + 24);
            assert!(tagged("hidden", at(link)) && tagged("box", at("- [ ] ab")), "styled where it is after a line came above");
            editor.buffer.place_cursor(&editor.iter(at(link) + 4));
            assert!(wait(context, || !tagged("hidden", at(link))), "the marks show with the caret in the link");
            editor.buffer.place_cursor(&editor.iter(at(link) + 24));
            assert!(wait(context, || tagged("hidden", at(link))));
            editor.buffer.undo();
            editor.buffer.undo();
            assert_eq!(editor.text(), text);
            spin(100);

            // A task in a callout is drawn and can be clicked.
            let quoted = at("> - [ ] in callout");
            assert!(wait(context, || tagged("box", quoted + 2)), "a box in a callout");
            editor.click_box(quoted + 2);
            assert_eq!(editor.text(), text.replace("> - [ ] in callout", &format!("> - [x] in callout ✅ {today}")));
            editor.buffer.undo();
            assert_eq!(editor.text(), text);

            // A Persian table: Return on its last row formats it and adds a
            // row, with every cell's text kept, as one step to undo; Left
            // and Right are the view's own in a right-to-left row.
            let row = at("| یک |");
            editor.buffer.place_cursor(&editor.iter(row + 3));
            assert!(wait(context, || editor.styler.in_grid(row)));
            assert!(!key(gdk::Key::Right, none) && !key(gdk::Key::Left, none));
            assert!(key(gdk::Key::Return, none));
            let rows: Vec<String> = ["| نام | B |", "| --- | --- |", "| یک | 2 |", "|  |  |"].map(String::from).to_vec();
            let formatted = MarkdownTable::format(&rows).join("\n");
            assert_eq!(editor.text(), text.replace("| نام | B |\n| --- | --- |\n| یک | 2 |", &formatted));
            for cell in ["نام", "یک", "B", "2"] {
                assert!(formatted.contains(cell), "{cell}");
            }
            editor.buffer.undo();
            assert_eq!(editor.text(), text, "one undo takes the row and the formatting back");

            // With `editor.live_preview = false` nothing is drawn, and the keys still work.
            settings.borrow_mut().file.set("editor.live_preview", daftar_core::Value::Bool(false));
            editor.settings_changed();
            assert!(wait(context, || !tagged("box", at("- [ ] Water")) && !editor.styler.in_grid(row)), "raw with live preview off");
            assert_eq!(hit(persian), None, "no box to click");
            assert!(!tagged("panel", at("> [!tip]")) && !tagged("chip", at("📅 2026-11-20") + 3));
            editor.buffer.place_cursor(&editor.iter(row + 3));
            assert!(key(gdk::Key::Tab, none), "Tab still goes to the next cell");
            editor.buffer.undo();
            assert_eq!(editor.text(), text);
            settings.borrow_mut().file.set("editor.live_preview", daftar_core::Value::Bool(true));
            editor.settings_changed();
            assert!(wait(context, || tagged("box", at("- [ ] Water"))));

            // A read-only vault: everything is drawn, a click on a box
            // changes nothing, and a `/` is only a `/`.
            let git = |arguments: &[&str]| {
                assert!(std::process::Command::new("git").args(arguments).current_dir(vault.path()).output().unwrap().status.success());
            };
            git(&["init", "-q"]);
            git(&["config", "daftar.readonly", "true"]);
            assert!(daftar_core::VaultEdits::writable(vault.path()).is_err());
            editor.click_box(at(water));
            editor.click_box(persian);
            assert_eq!(editor.text(), text, "a read-only vault's text is not changed by a click");
            assert!(tagged("box", at("- [ ] Water")), "and the box is still drawn");
            editor.discard();
            window.destroy();
        });
    }

    /// The fault found on the Mac, in a real view: `/tab` and Return in a
    /// note of three paragraphs, then Tab, Shift+Tab and Return in the
    /// cells. The table stands apart by a blank line on each side, the
    /// placeholders are selected so typing replaces them, no paragraph
    /// becomes a row, and one undo takes the insert back. Needs a display:
    /// `xvfb-run -a cargo test -p daftar -- --ignored
    /// a_table_from_the_slash_bar_keeps_the_text_beside_it`, alone.
    #[test]
    #[ignore = "needs a display"]
    fn a_table_from_the_slash_bar_keeps_the_text_beside_it() {
        use crate::vault_checks::tests::{on_a_loop_of_its_own, wait};
        on_a_loop_of_its_own(|context| {
            gtk::init().unwrap();
            adw::init().unwrap();
            crate::style::install();
            let vault = tempfile::tempdir().unwrap();
            let text = "x\n\nA first paragraph.\n\nA second paragraph.\n";
            fs::write(vault.path().join("Note.md"), text).unwrap();
            let index = Vault::index(vault.path(), None);
            let store = Rc::new(VaultStore::new(vault.path().to_path_buf(), index, std::sync::Arc::new(daftar_core::SearchIndex::new())));
            let settings = Rc::new(RefCell::new(AppSettings::of(vault.path().join("config.toml"), daftar_core::ConfigFile::TEMPLATE)));
            let editor = Editor::new(settings, |_| {}, |_| {});
            editor.set_store(store);
            let window = gtk::Window::builder().default_width(1100).default_height(700).child(&editor.widget).build();
            window.present();
            assert!(editor.load(vault.path(), "Note.md", Some(0)));
            let none = gdk::ModifierType::empty();
            let key = |key: gdk::Key, state: gdk::ModifierType| editor.key(key, 0, state) == glib::Propagation::Stop;
            // Keys as the view gets them: one the editor leaves alone goes
            // into the text in place of the selection, as the view types it.
            let typed = |letters: &str| {
                for letter in letters.chars() {
                    let name = match letter {
                        '/' => "slash".to_string(),
                        ' ' => "space".to_string(),
                        letter => letter.to_string(),
                    };
                    if !key(gdk::Key::from_name(name).unwrap(), none) {
                        editor.buffer.delete_selection(true, true);
                        editor.buffer.insert_at_cursor(&letter.to_string());
                    }
                }
            };
            let selected = || editor.slice(editor.selection());
            let table = "| Column | Column |\n| ------ | ------ |\n|        |        |";
            // The `/` at the start of line 0, at its end after a space, and on the blank line under it.
            let rest = "A first paragraph.\n\nA second paragraph.\n";
            for (caret, lead, slashed, inserted) in [
                (0, "", format!("/tabx\n\n{rest}"), format!("{table}\n\nx\n\n{rest}")),
                (1, " ", format!("x /tab\n\n{rest}"), format!("x \n\n{table}\n\n{rest}")),
                (2, "", format!("x\n/tab\n{rest}"), format!("x\n\n{table}\n\n{rest}")),
            ] {
                assert_eq!(editor.text(), text);
                editor.buffer.place_cursor(&editor.iter(caret));
                typed(lead);
                typed("/tab");
                assert!(
                    wait(context, || editor.slash.is_open() && editor.slash.shown().0.as_deref() == Some("table")),
                    "the `/` at {caret}"
                );
                assert!(key(gdk::Key::Return, none));
                assert_eq!(editor.text(), inserted, "the `/` at {caret}");
                assert_eq!(selected(), "Column", "the first header is selected");
                // One undo takes the table and its blank lines back.
                editor.buffer.undo();
                assert_eq!(editor.text(), slashed, "one undo");
                editor.buffer.redo();
                assert_eq!(editor.text(), inserted);
                let start = inserted.find(table).unwrap();
                editor.buffer.select_range(&editor.iter(start + 8), &editor.iter(start + 2));
                // What is typed takes the placeholder's place, and Tab selects the next.
                typed("Name");
                assert!(key(gdk::Key::Tab, none));
                assert_eq!(selected(), "Column", "Tab selects the next placeholder");
                typed("Age");
                assert!(key(gdk::Key::Tab, none));
                assert!(editor.selection().is_empty(), "an empty cell has a caret");
                typed("Sara");
                assert!(key(gdk::Key::Tab, none));
                typed("7");
                let rows = |more: &str| inserted.replace(table, &format!("| Name | Age |\n| ---- | --- |\n| Sara | 7   |{more}"));
                // Tab past the last cell makes a row: the paragraph under the table is no row.
                assert!(key(gdk::Key::Tab, none));
                assert_eq!(editor.text(), rows("\n|      |     |"));
                assert!(key(gdk::Key::ISO_Left_Tab, gdk::ModifierType::SHIFT_MASK));
                assert_eq!(editor.text(), rows("\n|      |     |"));
                // Return with a selection in a cell goes down and keeps the text.
                let seven = editor.cursor();
                editor.buffer.select_range(&editor.iter(seven + 1), &editor.iter(seven));
                assert_eq!(selected(), "7");
                assert!(key(gdk::Key::Return, none));
                assert_eq!(editor.text(), rows("\n|      |     |"));
                // Return on the last row, which is empty, ends the table: the row goes.
                assert!(key(gdk::Key::Return, none));
                assert_eq!(editor.text(), rows("\n\n"));
                // In the paragraphs the keys are the text's own.
                let paragraph = editor.text().find("A first").unwrap();
                editor.buffer.place_cursor(&editor.iter(paragraph + 3));
                assert!(!key(gdk::Key::Return, none) && !key(gdk::Key::Return, gdk::ModifierType::SHIFT_MASK));
                assert_eq!(editor.text(), rows("\n\n"));
                while editor.buffer.can_undo() {
                    editor.buffer.undo();
                }
            }
            editor.discard();
            window.destroy();
        });
    }

    /// Neovim mode (docs/specs/look.md, part 2): the parts are drawn and
    /// focus mode follows Neovim's cursor, a click on a box is sent to
    /// Neovim as a change of its lines (one `u`), the table's keys and the
    /// slash bar are not there, and Neovim's keys change no text by
    /// themselves. Needs a display and `nvim`: `xvfb-run -a cargo test -p
    /// daftar -- --ignored in_neovim_mode`, alone.
    #[test]
    #[ignore = "needs a display and nvim"]
    fn in_neovim_mode_the_parts_are_drawn_and_a_click_is_a_change_of_lines() {
        use crate::vault_checks::tests::{on_a_loop_of_its_own, wait};
        on_a_loop_of_its_own(|context| {
            gtk::init().unwrap();
            adw::init().unwrap();
            crate::style::install();
            let vault = tempfile::tempdir().unwrap();
            let text =
                "# T\n\n- [ ] Buy 📅 2001-01-01\n- [ ] Water 🔁 every week 📅 2001-01-01\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\nend\n";
            fs::write(vault.path().join("Note.md"), text).unwrap();
            let config = daftar_core::ConfigFile::TEMPLATE
                .replace("vim_mode = false", "vim_mode = true")
                .replace("nvim_config = \"user\"", "nvim_config = \"clean\"")
                .replace("focus = false", "focus = true");
            let settings = Rc::new(RefCell::new(AppSettings::of(vault.path().join("config.toml"), &config)));
            assert!(settings.borrow().focus());
            let editor = Editor::new(settings, |_| {}, |_| {});
            let window = gtk::Window::builder().default_width(1100).default_height(700).child(&editor.widget).build();
            window.present();
            assert!(editor.load(vault.path(), "Note.md", Some(0)));
            assert!(wait(context, || editor.vim.borrow().as_ref().is_some_and(|vim| vim.mirror.is_some())), "Neovim started");
            let at = |needle: &str| {
                let text = editor.text();
                text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?} is in {text:?}"))].chars().count()
            };
            let tagged = |name: &str, at: usize| {
                editor.iter(at).tags().iter().any(|tag| tag.name().is_some_and(|n| n.starts_with(&format!("daftar:{name}"))))
            };
            let input = |keys: &str| editor.vim.borrow().as_ref().unwrap().session.input(keys);
            let none = gdk::ModifierType::empty();
            let spin = |ms: u64| {
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(ms) {
                    while context.iteration(false) {}
                    std::thread::sleep(Duration::from_millis(5));
                }
            };
            let on_disk = || fs::read_to_string(vault.path().join("Note.md")).unwrap();
            let today = Day::today().to_string();

            // The box, the chip and the grid are drawn, and focus mode has
            // the paragraph of Neovim's cursor.
            assert!(wait(context, || tagged("box", 5) && tagged("chip", at("📅 2001") + 3) && editor.styler.in_grid(at("| 1 |"))));
            input("6G");
            assert!(wait(context, || editor.focus.bright_lines() == Some((5, 7))), "{:?}", editor.focus.bright_lines());

            // Return and Tab on a table's row are Neovim's keys: in normal
            // mode they move its cursor and no line is formatted.
            assert_eq!(editor.key(gdk::Key::Return, 0, none), glib::Propagation::Stop);
            assert_eq!(editor.key(gdk::Key::Tab, 0, none), glib::Propagation::Stop);
            spin(300);
            assert_eq!(editor.text(), text);

            // A click on a box is a change of that line in Neovim, which
            // writes the file; one `u` takes it back.
            editor.click_box(5);
            let done = text.replace("- [ ] Buy 📅 2001-01-01", &format!("- [x] Buy 📅 2001-01-01 ✅ {today}"));
            assert!(wait(context, || editor.text() == done), "{}", editor.text());
            assert!(wait(context, || on_disk() == done), "Neovim wrote the file");
            input("u");
            assert!(wait(context, || editor.text() == text), "one u takes the click back");
            // A repeating task: two lines for one, and one `u`.
            let water = "- [ ] Water 🔁 every week 📅 2001-01-01";
            let toggled = daftar_core::TaskLine::toggled(water, &today).unwrap();
            editor.click_box(at(water));
            assert!(wait(context, || editor.text() == text.replace(water, &toggled.join("\n"))), "{}", editor.text());
            input("u");
            assert!(wait(context, || editor.text() == text), "one u takes both lines back");

            // A `/` typed in insert mode is only a `/`: no slash bar.
            input("Go/");
            assert!(wait(context, || editor.text().ends_with("\n/\n")));
            spin(300);
            assert!(!editor.slash.is_open() && editor.completing.get() == Completing::Nothing);
            input("<Esc>u");
            assert!(wait(context, || editor.text() == text));
            editor.discard();
            window.destroy();
        });
    }

    #[test]
    fn the_schemes_are_valid_xml_with_the_palette() {
        let text = scheme("daftar-naz", "Daftar naz", &NAZ);
        assert!(text.contains(r##"<style name="cursor" foreground="#ffb040"/>"##));
        assert_eq!(text.matches("<style ").count(), 4);
    }

    #[test]
    fn a_pasted_picture_goes_to_obsidians_attachment_folder() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(attachment_folder(root.path()), "");
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::write(root.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath": "uploads"}"#).unwrap();
        assert_eq!(attachment_folder(root.path()), "uploads");
        // "./" is next to the note, which a paste with no note cannot know.
        fs::write(root.path().join(".obsidian/app.json"), r#"{"attachmentFolderPath": "./"}"#).unwrap();
        assert_eq!(attachment_folder(root.path()), "");
    }
}
