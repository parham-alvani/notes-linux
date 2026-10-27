//! Editing a note: the markdown in a GtkSourceView, colored by the core's
//! `MarkdownHighlighter` in Daftar's palette, with the Mac editor's commands
//! on its keys: Return continues a list, Tab and Shift+Tab indent an item or
//! go to the next cell of a table, Ctrl+B, Ctrl+I and Ctrl+Shift+H mark the
//! selection, Ctrl+K links it, Ctrl+Return ticks the task on the line,
//! Alt+Up and Alt+Down move lines, `[[` offers notes and `/` snippets, and a
//! pasted picture is saved in the vault's attachment folder. The pictures a
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
    BufferMirror, Candidate, Day, EditorCommands, Fuzzy, HtmlMarkdown, MarkdownTable, QuickSwitcher, Return, Snippet, TaskSorter, Vault,
    VaultMoves,
};

#[path = "editor_media.rs"]
mod media;

use crate::editor_popup::{Choice, Popup};
use crate::editor_style::Styler;
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

/// The note open in the editor: its path, and the text the file had when it
/// was loaded or last saved.
struct Open {
    path: String,
    file: PathBuf,
    on_disk: String,
}

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
}

impl Editor {
    /// [notice] says a line to the person; [conflict] says a save was refused
    /// because the file changed on disk, and may offer `reload`.
    pub fn new(settings: Rc<RefCell<AppSettings>>, notice: impl Fn(&str) + 'static, conflict: impl Fn(&str) + 'static) -> Rc<Editor> {
        let buffer = sourceview5::Buffer::new(None);
        // The colors are the core highlighter's, as on the Mac.
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(false);
        let view = sourceview5::View::builder()
            .buffer(&buffer)
            .wrap_mode(gtk::WrapMode::WordChar)
            .auto_indent(true)
            .indent_width(4)
            .tab_width(4)
            .insert_spaces_instead_of_tabs(true)
            .smart_backspace(true)
            .top_margin(24)
            .bottom_margin(240)
            .left_margin(32)
            .right_margin(32)
            .css_classes(["daftar-editor"])
            .build();
        // A clamp that scrolls its child, so the view can keep the cursor in sight.
        let clamp = adw::ClampScrollable::builder().maximum_size(860).tightening_threshold(860).child(&view).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
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
        #[allow(deprecated)]
        view.style_context().add_provider(&font_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let editor = Rc::new(Editor {
            widget,
            styler: Styler::new(buffer.upcast_ref()),
            popup: Popup::new(&view),
            glide: Glide::new(&view),
            stage,
            floats: RefCell::new(Vec::new()),
            float_css: {
                let provider = gtk::CssProvider::new();
                if let Some(display) = gtk::gdk::Display::default() {
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
        });
        install_schemes(&editor.buffer);
        editor.connect_buffer();
        editor.connect_keys();
        editor.connect_clicks();
        editor.connect_drop();
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
        self.popup.close();
        self.completing.set(Completing::Nothing);
        self.auto_closed.set(None);
        self.apply_fonts();
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.set_modified(false);
        self.dirty.set(None);
        self.styler.restyle_all();
        *self.open.borrow_mut() = Some(Open { path: path.to_string(), file, on_disk: text });
        let line = line.unwrap_or(0);
        self.go_to_line(line);
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

    /// Saves, then lets go of the note.
    pub fn close(&self) -> Saved {
        let saved = self.save();
        if saved != Saved::ChangedOnDisk {
            self.stop_vim();
            self.popup.close();
            self.clear_pictures();
            *self.open.borrow_mut() = None;
        }
        saved
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
            open.on_disk = text;
        }
        if let Some(at) = self.buffer.iter_at_line(line as i32) {
            self.buffer.place_cursor(&at);
        }
    }

    // The buffer

    fn text(&self) -> String {
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
        // Line height 1.6: the space between lines and between wrapped rows.
        let gap = (size as f64 * 0.6).round() as i32;
        self.view.set_pixels_below_lines(gap);
        self.view.set_pixels_inside_wrap(gap / 2);
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
            this.update_counts();
        });
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
            }
            this.update_counts();
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
        if self.vim.borrow().is_some() {
            return self.vim_key(key, keycode, control, alt, super_, shift);
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

    /// Return continues a list, and on an empty item ends it; at the end of
    /// a table row it makes a new row, and on an empty last row ends the table.
    fn on_return(&self) -> bool {
        let line = self.current_line();
        let cursor = self.selection();
        if !cursor.is_empty() {
            return false;
        }
        let text = self.slice(line.clone());
        // In a table row Return goes to a new row from any cell, as Tab
        // leaves the cursor inside one; elsewhere only at the line's end.
        let table = self.table_block();
        if table.is_none() && cursor.start != line.end {
            return false;
        }
        // Return on the table's empty last row ends the table: the row goes,
        // and a blank line follows it, or the next line would be a row too.
        if let Some((_, lines, row)) = &table
            && MarkdownTable::cells(&text).iter().all(|cell| cell.trim().is_empty())
            && *row + 1 == lines.len()
            && *row > 0
        {
            self.replace(line.clone(), "\n", Some(line.start + 1..line.start + 1));
            return true;
        }
        if let Some((range, lines, row)) = table
            && !MarkdownTable::cells(&text).iter().all(String::is_empty)
        {
            let formatted = MarkdownTable::new_row(&lines[..=row]);
            let replaced = range.start..line.end;
            let joined = formatted.join("\n");
            let last = formatted.last().unwrap();
            let start = replaced.start + joined.chars().count() - last.chars().count();
            let cell = start + MarkdownTable::cell_start(last, 0);
            self.replace(replaced, &joined, Some(cell..cell));
            return true;
        }
        match EditorCommands::on_return(&text) {
            Some(Return::Insert(insert)) => {
                let at = cursor.start + insert.chars().count();
                self.replace(cursor.clone(), &insert, Some(at..at));
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
        if self.selection().is_empty() && self.table_tab(back) {
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

    /// The rows of the table the cursor is in: their characters, their
    /// lines, and which of them holds the cursor.
    fn table_block(&self) -> Option<(Range<usize>, Vec<String>, usize)> {
        let line = self.current_line();
        if !MarkdownTable::is_row(&self.slice(line.clone())) {
            return None;
        }
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert()).line();
        let row_text = |line: i32| -> Option<String> {
            let start = self.buffer.iter_at_line(line)?;
            let mut end = start;
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            Some(self.buffer.text(&start, &end, true).to_string())
        };
        let mut first = at;
        while first > 0 && row_text(first - 1).is_some_and(|t| MarkdownTable::is_row(&t)) {
            first -= 1;
        }
        let mut last = at;
        while row_text(last + 1).is_some_and(|t| MarkdownTable::is_row(&t)) {
            last += 1;
        }
        let lines: Vec<String> = (first..=last).filter_map(row_text).collect();
        let start = self.buffer.iter_at_line(first)?.offset() as usize;
        let mut end = self.buffer.iter_at_line(last)?;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        Some((start..end.offset() as usize, lines, (at - first) as usize))
    }

    fn table_tab(&self, back: bool) -> bool {
        let Some((range, lines, row)) = self.table_block() else { return false };
        let line = self.current_line();
        let column = MarkdownTable::column(self.cursor() - line.start, &self.slice(line));
        let moved = MarkdownTable::tab(&lines, row, column, back);
        let start = range.start + moved.lines[..moved.row].iter().map(|l| l.chars().count() + 1).sum::<usize>();
        let cell = start + MarkdownTable::cell_start(&moved.lines[moved.row], moved.column);
        self.replace(range, &moved.lines.join("\n"), Some(cell..cell));
        true
    }

    fn move_lines(&self, up: bool) {
        match EditorCommands::move_lines(&self.text(), self.selection(), up) {
            Some(edit) => self.replace(edit.range, &edit.text, Some(edit.selection)),
            None => self.view.error_bell(),
        }
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
        let chars: Vec<char> = self.text().chars().collect();
        if !selection.is_empty()
            && let Some(close) = Self::wrapping(typed)
            && !chars[selection.clone()].contains(&'\n')
        {
            let inner: String = chars[selection.clone()].iter().collect();
            self.replace(selection.clone(), &format!("{typed}{inner}{close}"), Some(selection.start + 1..selection.end + 1));
            return true;
        }
        if !selection.is_empty() {
            return false;
        }
        let at = selection.start;
        if self.auto_closed.get() == Some(at) && chars.get(at) == Some(&typed) && [')', ']', '}'].contains(&typed) {
            self.auto_closed.set(None);
            self.buffer.place_cursor(&self.iter(at + 1));
            return true;
        }
        // `[[`: the link's own closing, and the note completion.
        if typed == '[' && at >= 1 && chars.get(at - 1) == Some(&'[') && chars.get(at) == Some(&']') && self.auto_closed.get() == Some(at) {
            self.replace(at..at + 1, "[]]", Some(at + 1..at + 1));
            self.auto_closed.set(None);
            self.complete_links();
            return true;
        }
        if typed == '[' && at >= 1 && chars.get(at - 1) == Some(&'[') {
            self.replace(at..at, "[]]", Some(at + 1..at + 1));
            self.complete_links();
            return true;
        }
        if typed == '/' {
            // After the slash is in, so it is on screen under the list.
            let this = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(this) = this.upgrade() {
                    this.maybe_slash_menu();
                }
            });
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
        let next = chars.get(at).copied().unwrap_or('\n');
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
                    let aliases = index.notes.get(path).map(|n| n.parsed.aliases.clone()).unwrap_or_default();
                    Candidate::new(path, &Vault::title(path), &aliases)
                })
                .collect();
        }
        self.completing.set(Completing::Link);
        self.follow_completion();
    }

    fn maybe_slash_menu(self: &Rc<Self>) {
        let at = self.cursor();
        if EditorCommands::opens_slash_menu(&self.text(), at) {
            self.completing.set(Completing::Slash(at - 1));
            self.follow_completion();
        }
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
            Completing::Slash(slash) => {
                let at = self.cursor();
                if at <= slash || self.slice(slash..slash + 1) != "/" {
                    return self.stop_completion();
                }
                let typed = self.slice(slash + 1..at);
                if typed.chars().any(char::is_whitespace) {
                    return self.stop_completion();
                }
                let mut found: Vec<(i64, usize, Choice)> = slash_snippets()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(order, snippet)| {
                        let hit = Fuzzy::matches(&typed, &snippet.title)?;
                        let choice = Choice {
                            title: glib::markup_escape_text(&snippet.title).into(),
                            subtitle: snippet.group.clone(),
                            value: snippet.id.clone(),
                        };
                        Some((hit.score, order, choice))
                    })
                    .collect();
                found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                found.into_iter().map(|(_, _, choice)| choice).collect()
            }
        };
        // Nothing matches what is typed yet: no list, but the `[[` or the `/`
        // still listens, as more is typed.
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
                match kind {
                    Completing::Link => this.insert_completion(&choice.value),
                    Completing::Slash(slash) => this.apply_slash(&choice.value, slash),
                    Completing::Nothing => {}
                }
            }
        });
    }

    fn stop_completion(&self) {
        self.completing.set(Completing::Nothing);
        self.popup.close();
    }

    /// The note picked for `[[`: its name, or its path when another note has
    /// the same name, and the cursor past the `]]`.
    fn insert_completion(self: &Rc<Self>, path: &str) {
        let Some(query) = EditorCommands::link_query(&self.text(), self.cursor()) else { return };
        let files: BTreeSet<String> = self.store.borrow().as_ref().map(|s| s.index().files.clone()).unwrap_or_default();
        let link = EditorCommands::link(path, &files);
        let target = link.trim_start_matches('!').trim_start_matches("[[").trim_end_matches("]]").to_string();
        let end = query.start + target.chars().count();
        self.replace(query, &target, Some(end..end));
        if self.slice(end..end + 2) == "]]" {
            self.buffer.place_cursor(&self.iter(end + 2));
        }
        self.view.grab_focus();
    }

    /// The snippet picked for the `/` at [slash], in its place, as one step.
    fn apply_slash(self: &Rc<Self>, id: &str, slash: usize) {
        let Some(snippet) = slash_snippets().into_iter().find(|s| s.id == id) else { return };
        if self.slice(slash..slash + 1) != "/" {
            return;
        }
        // What was typed after the slash to find it goes with the slash.
        let typed_end = self.cursor().max(slash + 1);
        self.buffer.begin_user_action();
        let (mut start, mut end) = (self.iter(slash + 1), self.iter(typed_end));
        self.buffer.delete(&mut start, &mut end);
        let edit = EditorCommands::apply(&snippet, slash, &self.text());
        self.replace(edit.range, &edit.text, Some(edit.selection));
        self.buffer.end_user_action();
        self.view.grab_focus();
        if snippet.id == "link" || snippet.id == "embed" {
            self.complete_links();
        }
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
                let markdown = match html {
                    true => read_html(&clipboard).await.and_then(|html| HtmlMarkdown::convert(&html)),
                    false => None,
                };
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

    fn paste_picture(&self, texture: &gdk::Texture) {
        let Some(store) = self.store.borrow().clone() else { return };
        let folder = attachment_folder(&store.root);
        let name = EditorCommands::pasted_image_name(chrono::Local::now().naive_local(), "png");
        let base = name.trim_end_matches(".png");
        let path = VaultMoves::free_name(base, "png", &folder, &store.index().files);
        let Some(file) = file_in(&path, &store.root) else { return };
        let written = file.parent().map_or(Ok(()), fs::create_dir_all).and_then(|_| fs::write(&file, texture.save_to_png_bytes()));
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
        session.set_persian(&crate::neovim_session::keyboard_langmap());
        *self.vim.borrow_mut() =
            Some(Vim { session, mirror: None, mode: "n".into(), start_line: line, save_soon: None, wheel_top: None, command: false });
        self.glide.set_on(true);
        self.glide.set_shape(Shape::Block);
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
            .map(|p| (p.x() as f64, p.y() as f64))
            .unwrap_or((0.0, 0.0));
        let (cursor_x, cursor_y) = self.glide.cursor_point();
        let (bound_w, bound_h) = (self.stage.width() as f64, self.stage.height() as f64);
        for view in &floats.views {
            let markup = format!("<span font=\"{} {}px\">{}</span>", glib::markup_escape_text(&family), size, view.rows.join("\n"));
            let label = gtk::Label::builder().use_markup(true).label(&markup).xalign(0.0).yalign(0.0).build();
            let (width, height) = label.layout().pixel_size();
            let (cell_w, cell_h) = (width as f64 / view.width.max(1) as f64, height as f64 / view.height.max(1) as f64);
            let (mut x, mut y) = if view.on_editor {
                (origin.0 + self.view.left_margin() as f64 + view.column * cell_w, origin.1 + view.row * cell_h)
            } else {
                let (row, column) = floats.window_cursor.unwrap_or((0, 0));
                (origin.0 + cursor_x + (view.column - column as f64) * cell_w, origin.1 + cursor_y + (view.row - row as f64) * cell_h)
            };
            if view.corner.ends_with('E') {
                x -= width as f64;
            }
            if view.corner.starts_with('S') {
                y -= height as f64;
            }
            let x = x.clamp(0.0, (bound_w - width as f64).max(0.0));
            let y = y.clamp(0.0, (bound_h - height as f64).max(0.0));
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
        let scale = gtk::pango::SCALE as f64;
        let line = (metrics.ascent() + metrics.descent()) as f64 / scale + self.view.pixels_below_lines() as f64;
        let width = metrics.approximate_char_width() as f64 / scale;
        let (height, across) = (self.view.height() as f64, self.view.width() as f64 - 64.0);
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
                self.buffer.select_range(&self.iter(cursor), &self.iter(if cursor == one.start { one.end } else { one.start }))
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
        // A block cursor in normal mode, a bar while inserting, gliding there.
        self.glide.set_shape(Shape::of(&mode));
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
        let name = key.name().map(|n| n.to_string()).unwrap_or_default();
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

    /// A click moves Neovim's cursor where it landed.
    fn connect_clicks(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_released(move |_, _, _, _| {
            let Some(this) = this.upgrade() else { return };
            let this = Rc::downgrade(&this);
            glib::idle_add_local_once(move || {
                let Some(this) = this.upgrade() else { return };
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
