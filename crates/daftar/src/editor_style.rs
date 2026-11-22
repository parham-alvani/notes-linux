//! The editor's colors: `MarkdownHighlighter`'s spans as GtkTextTags, in the
//! palette the Mac editor uses (`NSPalette`: naz on dark, Daylight on
//! light). The tags keep their names and change their colors when the
//! scheme does, so a note never has to be styled again for it.
//!
//! A change styles its own paragraph again; only a change to the lines that
//! open or close a block (a fence, the front matter, `$$`, `%%`) styles the
//! rest of the note, as the Mac's `restyle` does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{glib, pango};

use daftar_core::text::direction;
use daftar_core::{
    BlockState, CodeRole, Day, HighlightKind, HighlightSpan, LivePreview, MarkdownHighlighter, PreviewAbove, PreviewBlock, PreviewParts,
};

/// A color of the palette by name: (dark, light), as `NSPalette.pairs`.
fn pair(name: &str) -> (u32, u32) {
    match name {
        "text" => (0xF5F5F0, 0x2A2A28),
        "faint" => (0x707080, 0x9A9A92),
        "muted" => (0xA8A8A0, 0x6A6A62),
        "background" => (0x323232, 0xFAF9F5),
        "panel" => (0x282826, 0xEFEDE6),
        "heading1" => (0xFFE0A0, 0xA85200),
        "heading2" => (0xFFFF88, 0x8A6A00),
        "heading3" => (0x60D8FF, 0x0070A0),
        "heading4" => (0xE0FFD8, 0x0A7A48),
        "strong" => (0xFFB070, 0xA85200),
        "italic" => (0xB8FFD0, 0x0A7A48),
        "raw" => (0xFFD850, 0x8A6A00),
        "link" | "aqua" => (0x80E5FF, 0x00707A),
        "accent" | "orange" | "quote" => (0xFFB040, 0xA85200),
        "pink" => (0xFF69B4, 0xB02A70),
        "purple" => (0xF0A0F0, 0x8040A0),
        "chartreuse" => (0xA0FF40, 0x5A7A00),
        "blue" => (0x40D0FF, 0x0070A0),
        "green" => (0x20FFA0, 0x0A7A48),
        "vividYellow" => (0xFFE040, 0x8A6A00),
        "red" => (0xFF7088, 0xC01030),
        "yellow" => (0xF0E890, 0x8A6A00),
        "mint" => (0xB8FFD0, 0x0A7A48),
        "warmGrey" => (0xB09070, 0x7A5A3A),
        "grey" => (0xA8A8A0, 0x6A6A62),
        _ => (0xFFB040, 0xA85200),
    }
}

/// The dark colours are naz's with their strength capped (`appearance.dark_palette`).
static SOFT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn rgba(name: &str, alpha: f32, dark: bool) -> gtk::gdk::RGBA {
    rgba_in(name, alpha, dark, SOFT.load(std::sync::atomic::Ordering::Relaxed))
}

/// A colour of the palette: Daylight, naz, or naz with its strength capped.
fn rgba_in(name: &str, alpha: f32, dark: bool, soft: bool) -> gtk::gdk::RGBA {
    let (d, l) = pair(name);
    let value = match (dark, soft) {
        (true, true) => daftar_core::soft_palette::soften(d),
        (true, false) => d,
        (false, _) => l,
    };
    let channel = |shift: u32| ((value >> shift) & 0xFF) as f32 / 255.0;
    gtk::gdk::RGBA::new(channel(16), channel(8), channel(0), alpha)
}

/// How a tag looks, by palette names, so it can be drawn in either scheme.
#[derive(Clone, Default)]
struct Look {
    fg: Option<(&'static str, f32)>,
    bg: Option<(&'static str, f32)>,
    bold: bool,
    /// The weight of a heading and of bold text, 600: the reader's 650.
    semibold: bool,
    medium: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    scale: Option<f64>,
    mono: bool,
    /// A line aligned to the other side: its first letter is the syntax's
    /// (`- [x] سلام`), not its text's, and the view takes its direction
    /// from the first letter.
    flip: bool,
    /// Persian letters, in Vazirmatn as on the phone and the Mac.
    persian: bool,
    /// A mark live preview hides.
    hidden: bool,
    /// Clear, at its own size: an empty `>` line of a callout keeps its height.
    clear: bool,
    /// Room after each character, in em of the text size: the width a
    /// hidden mark keeps for what is drawn over it, and the room around a chip.
    spacing: Option<f64>,
    /// The room is the em given less the width of a `[`, which stays at its
    /// size so its line keeps its height: a callout's icon stands there.
    bracket: bool,
    /// A panel's lines: the room at the left and at the right of the text,
    /// in em, inside the measure.
    inset: Option<(f64, f64)>,
    /// Room above the line, in em: a panel's first line.
    above: Option<f64>,
    /// Room under the line beyond the line gap, in em: a panel's last line.
    below: Option<f64>,
    /// In small letters, as a code block's language is drawn.
    lower: bool,
    /// Never struck through: a date chip and a reason on a done task.
    unstruck: bool,
    /// Starts at the left whatever its letters: code.
    left: bool,
    /// Which tags win where two meet: 0 the text's styles, then a task's
    /// fields, the chips, the marks, and what is hidden over all.
    rank: u8,
}

/// The sizes a tag's look is worked out from: the text size in px, the room
/// at each side of the measure, and the width of a `[` in the note's font.
#[derive(Clone, Copy, PartialEq)]
pub struct Metrics {
    pub size: f64,
    pub room: i32,
    pub bracket: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics { size: 16.0, room: 32, bracket: 5.0 }
    }
}

impl Metrics {
    /// The space under every line: the line height 1.65 less the text's own.
    pub fn gap(&self) -> i32 {
        (self.size * 0.65).round() as i32
    }
}

/// What the view draws where a tag is, beside the text (`editor_draw.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drawn {
    /// A task's box over its hidden marks.
    TaskBox,
    /// A date chip's pill, in this colour of the palette at this strength.
    Chip(&'static str, f32),
    /// A callout's icon, and the kind's name when no title is written.
    CalloutIcon,
    /// A callout's panel behind its lines, in the kind's colour.
    CalloutPanel(&'static str),
    CodePanel,
    /// The first line of a panel: two panels with no line between them are two.
    PanelTop,
}

/// A colour by the name the reader's CSS has for it, as the palette here names it.
pub fn palette_name(reader: &str) -> &'static str {
    match reader {
        "list-marker" | "accent" => "accent",
        "done-color" | "text-faint" => "faint",
        "text-muted" => "muted",
        "blue" => "blue",
        "red" => "red",
        "yellow" => "yellow",
        "vivid-yellow" => "vividYellow",
        "warm-grey" => "warmGrey",
        "orange" => "orange",
        "spring-green" => "green",
        "purple" => "purple",
        "pink" => "pink",
        _ => "accent",
    }
}

fn fg(name: &'static str) -> Look {
    Look { fg: Some((name, 1.0)), ..Look::default() }
}

/// A tag's color, as the reader gives it (`hue` in reader.js).
pub fn tag_color(tag: &str) -> &'static str {
    const ORDER: [&str; 9] = ["orange", "aqua", "pink", "purple", "chartreuse", "blue", "green", "vividYellow", "red"];
    let mut hash: u32 = 0;
    for c in tag.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as u32);
    }
    ORDER[(hash % ORDER.len() as u32) as usize]
}

/// A callout kind's color, as the reader has it.
pub fn callout_color(kind: &str) -> &'static str {
    match kind.to_lowercase().as_str() {
        "warning" | "caution" | "attention" => "orange",
        "danger" | "error" | "bug" | "failure" | "fail" | "missing" => "red",
        "success" | "check" | "done" => "green",
        "tip" | "hint" | "important" => "chartreuse",
        "todo" => "vividYellow",
        "question" | "help" | "faq" => "yellow",
        "abstract" | "summary" | "tldr" => "mint",
        "example" => "purple",
        "quote" | "cite" => "warmGrey",
        "figure" => "pink",
        "read" => "aqua",
        _ => "blue",
    }
}

/// A code block's language color, as the reader has it.
pub fn language_color(language: &str) -> &'static str {
    match language.to_lowercase().as_str() {
        "go" | "dockerfile" => "aqua",
        "python" | "py" | "javascript" | "js" => "vividYellow",
        "bash" | "sh" | "shell" | "zsh" | "fish" => "chartreuse",
        "yaml" | "yml" | "toml" | "css" => "pink",
        "sql" | "hcl" | "terraform" | "kotlin" => "purple",
        "c" | "cpp" | "typescript" | "ts" | "powershell" | "lua" | "nix" => "blue",
        "java" | "ruby" => "red",
        "diff" => "green",
        "markdown" | "md" | "text" | "" => "grey",
        _ => "orange",
    }
}

/// A highlight's color from the circle at its start, as Obsidian has it.
pub fn highlight_color(first: Option<char>) -> &'static str {
    match first {
        Some('🔴') => "red",
        Some('🟠') => "orange",
        Some('🟡') => "vividYellow",
        Some('🟢') => "green",
        Some('🔵') => "blue",
        Some('🟣') => "purple",
        _ => "vividYellow",
    }
}

/// The tags of one buffer, made as they are first needed.
pub struct Styler {
    buffer: gtk::TextBuffer,
    tags: RefCell<HashMap<String, (gtk::TextTag, Look)>>,
    dark: std::cell::Cell<bool>,
    code_font: RefCell<String>,
    /// The block lines as they were last styled.
    structure: RefCell<Vec<String>>,
    /// The styling of what came on screen, soon.
    pending: RefCell<Option<glib::SourceId>>,
    /// The characters not styled yet, in order: a long note is styled where
    /// it is seen, as it scrolls, never all at once. Every part styled is
    /// laid out by GTK again, so the parts never seen are spared that.
    unstyled: RefCell<Vec<Range<usize>>>,
    /// The view that shows the buffer, for what is on screen.
    view: RefCell<Option<glib::WeakRef<gtk::TextView>>>,
    /// Live preview (`editor.live_preview`): away from the cursor's lines
    /// the marks hide, so the note reads as it will look.
    live: std::cell::Cell<bool>,
    /// The cursor's lines, as characters from the start of the first to
    /// past the end of the last, where every mark shows.
    revealed: std::cell::Cell<(usize, usize)>,
    /// The cursor's first and last line, and whether it selects, as last revealed.
    revealed_lines: std::cell::Cell<Option<((i32, i32), bool)>>,
    /// The tint of the cursor's line, as the Mac draws it.
    line: gtk::TextTag,
    /// The selection, as last revealed: an element's marks show while it is in the element.
    cursor: std::cell::Cell<(usize, usize)>,
    /// The elements with marks on the cursor's lines, the ones the
    /// selection was in when those lines were styled, and the start of the
    /// lines and the length of the text then (docs/specs/look.md).
    elements: RefCell<Vec<Range<usize>>>,
    open: RefCell<Vec<Range<usize>>>,
    elements_for: std::cell::Cell<Option<(usize, usize)>>,
    /// The text size and the measure's room, which some looks are worked out from.
    metrics: std::cell::Cell<Metrics>,
    /// The tags the view draws something for, in the order they were made.
    drawn: RefCell<Vec<(gtk::TextTag, Drawn)>>,
    /// The day the late chips were worked out for.
    day: std::cell::Cell<Day>,
}

/// The tint of the cursor's line: neutral, weaker than the reader's block.
fn line_tint(dark: bool) -> gtk::gdk::RGBA {
    rgba("text", 0.045, dark)
}

/// Whether [selection] is in [element]: a caret anywhere from its first
/// mark to after its last, or a selection that reaches into it.
fn is_open(element: &Range<usize>, selection: (usize, usize)) -> bool {
    if selection.0 == selection.1 {
        selection.0 >= element.start && selection.0 <= element.end
    } else {
        selection.0 < element.end && element.start < selection.1
    }
}

/// Lines a paragraph is taken to reach up or down, at most.
const PARAGRAPH_LINES: usize = 200;

/// Characters styled at once, and the margin styled above and below the screen.
const PART: usize = 12_000;

impl Styler {
    pub fn new(buffer: &gtk::TextBuffer) -> Rc<Styler> {
        let manager = adw::StyleManager::default();
        let styler = Rc::new(Styler {
            buffer: buffer.clone(),
            tags: RefCell::new(HashMap::new()),
            dark: std::cell::Cell::new(manager.is_dark()),
            code_font: RefCell::new("monospace".into()),
            structure: RefCell::new(Vec::new()),
            pending: RefCell::new(None),
            unstyled: RefCell::new(Vec::new()),
            view: RefCell::new(None),
            live: std::cell::Cell::new(false),
            revealed: std::cell::Cell::new((0, 0)),
            revealed_lines: std::cell::Cell::new(None),
            line: gtk::TextTag::new(Some("daftar:cursor-line")),
            cursor: std::cell::Cell::new((0, 0)),
            elements: RefCell::new(Vec::new()),
            open: RefCell::new(Vec::new()),
            elements_for: std::cell::Cell::new(None),
            metrics: std::cell::Cell::new(Metrics::default()),
            drawn: RefCell::new(Vec::new()),
            day: std::cell::Cell::new(Day::today()),
        });
        styler.line.set_paragraph_background_rgba(Some(&line_tint(manager.is_dark())));
        buffer.tag_table().add(&styler.line);
        // What is not styled yet moves with the text.
        let this = Rc::downgrade(&styler);
        buffer.connect_insert_text(move |_, at, text| {
            if let Some(this) = this.upgrade() {
                let (at, count) = (at.offset() as usize, text.chars().count());
                for range in this.unstyled.borrow_mut().iter_mut() {
                    if range.start > at {
                        range.start += count;
                    }
                    if range.end > at {
                        range.end += count;
                    }
                }
            }
        });
        let this = Rc::downgrade(&styler);
        buffer.connect_delete_range(move |_, from, to| {
            if let Some(this) = this.upgrade() {
                let (a, b) = (from.offset().min(to.offset()) as usize, from.offset().max(to.offset()) as usize);
                let moved = |x: usize| {
                    if x <= a {
                        x
                    } else if x >= b {
                        x - (b - a)
                    } else {
                        a
                    }
                };
                let mut unstyled = this.unstyled.borrow_mut();
                for range in unstyled.iter_mut() {
                    *range = moved(range.start)..moved(range.end);
                }
                unstyled.retain(|range| !range.is_empty());
            }
        });
        let this = Rc::downgrade(&styler);
        manager.connect_dark_notify(move |manager| {
            if let Some(this) = this.upgrade() {
                this.dark.set(manager.is_dark());
                this.dress();
                this.line.set_paragraph_background_rgba(Some(&line_tint(manager.is_dark())));
            }
        });
        styler
    }

    /// The dark palette is naz, soft or naz: every tag takes its colours again.
    pub fn set_soft(&self, soft: bool) {
        // Set for every editor; each one's tags take their colours again here.
        SOFT.store(soft, std::sync::atomic::Ordering::Relaxed);
        self.dress();
        self.line.set_paragraph_background_rgba(Some(&line_tint(self.dark.get())));
    }

    pub fn set_code_font(&self, family: &str) {
        if *self.code_font.borrow() == family {
            return;
        }
        *self.code_font.borrow_mut() = family.to_string();
        self.dress();
    }

    /// Every tag takes its look again: the scheme, the palette, a font or a size changed.
    fn dress(&self) {
        for (tag, look) in self.tags.borrow().values() {
            apply_look(tag, look, self.dark.get(), &self.code_font.borrow(), self.metrics.get());
        }
    }

    /// The text size and the measure's room changed: the looks that are
    /// worked out from them follow. Never while the view lays itself out.
    pub fn set_metrics(&self, metrics: Metrics) {
        if self.metrics.replace(metrics) != metrics {
            self.dress();
        }
    }

    pub fn metrics(&self) -> Metrics {
        self.metrics.get()
    }

    /// Whether the dark palette is in use.
    pub fn is_dark(&self) -> bool {
        self.dark.get()
    }

    /// The tags the view draws something for.
    pub fn drawn(&self) -> Vec<(gtk::TextTag, Drawn)> {
        self.drawn.borrow().clone()
    }

    /// A new day: the lines are styled again, since a due date may be late now.
    pub fn follow_day(self: &Rc<Self>) {
        let today = Day::today();
        if self.day.replace(today) != today && self.live.get() {
            self.restyle_all();
        }
    }

    fn tag(&self, name: &str, look: impl FnOnce() -> Look) -> gtk::TextTag {
        if let Some((tag, _)) = self.tags.borrow().get(name) {
            return tag.clone();
        }
        let look = look();
        let tag = gtk::TextTag::new(Some(&format!("daftar:{name}")));
        apply_look(&tag, &look, self.dark.get(), &self.code_font.borrow(), self.metrics.get());
        self.buffer.tag_table().add(&tag);
        self.tags.borrow_mut().insert(name.to_string(), (tag.clone(), look));
        // Marks always win: they are the top tags, and a hidden mark over
        // them. Each ranked tag goes to the top in the order of its rank.
        let top = self.buffer.tag_table().size() - 1;
        let tags = self.tags.borrow();
        let mut ranked: Vec<(u8, &gtk::TextTag)> =
            tags.values().filter(|(_, look)| look.rank > 0).map(|(tag, look)| (look.rank, tag)).collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        for (_, raised) in ranked {
            raised.set_priority(top);
        }
        tag
    }

    /// A tag the view draws something for.
    fn drawn_tag(&self, name: &str, drawn: Drawn, look: impl FnOnce() -> Look) -> gtk::TextTag {
        let known = self.tags.borrow().contains_key(name);
        let tag = self.tag(name, look);
        if !known {
            self.drawn.borrow_mut().push((tag.clone(), drawn));
        }
        tag
    }

    /// The view the buffer is shown in: what scrolls on screen is styled.
    pub fn attach(self: &Rc<Self>, view: &gtk::TextView) {
        *self.view.borrow_mut() = Some(view.downgrade());
        if let Some(v) = view.vadjustment() {
            let this = Rc::downgrade(self);
            v.connect_value_changed(move |_| {
                if let Some(this) = this.upgrade() {
                    this.style_seen_soon();
                }
            });
        }
    }

    /// Styles the whole note: the first part now, the rest as it is seen.
    pub fn restyle_all(self: &Rc<Self>) {
        self.cancel();
        *self.structure.borrow_mut() = structure(&self.text());
        let total = self.buffer.char_count().max(0) as usize;
        let first = self.line_end_after(PART.min(total));
        self.style(0..first, Some(BlockState::Normal));
        let mut unstyled = Vec::new();
        if first < total {
            unstyled.push(first..total);
        }
        *self.unstyled.borrow_mut() = unstyled;
        self.style_seen_soon();
    }

    /// Styles what is on screen, and a part above and below it, once the
    /// view has laid itself out and scrolled.
    fn style_seen_soon(self: &Rc<Self>) {
        if self.unstyled.borrow().is_empty() || self.pending.borrow().is_some() {
            return;
        }
        let this = Rc::downgrade(self);
        // A timeout, not an idle: GTK lays a long note out in idles above
        // any idle of ours, for seconds.
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(40), move || {
            if let Some(this) = this.upgrade() {
                this.pending.take();
                this.style_seen();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn style_seen(&self) {
        let Some(view) = self.view.borrow().as_ref().and_then(glib::WeakRef::upgrade) else { return };
        let rect = view.visible_rect();
        let offset = |y: i32| view.line_at_y(y).0.offset().max(0) as usize;
        let total = self.buffer.char_count().max(0) as usize;
        let (low, high) = (offset(rect.y()).saturating_sub(PART), (offset(rect.y() + rect.height()) + PART).min(total));
        let wanted: Vec<Range<usize>> =
            self.unstyled.borrow().iter().filter(|r| r.start < high && r.end > low).map(|r| r.start.max(low)..r.end.min(high)).collect();
        if wanted.is_empty() {
            return;
        }
        let text = self.text();
        for range in wanted {
            // Whole lines, from the block state the note leaves there.
            let mut start = self.buffer.iter_at_offset(range.start as i32);
            start.set_line_offset(0);
            let (mut at, end) = (start.offset() as usize, self.line_end_after(range.end));
            let mut state = MarkdownHighlighter::block_state(at, &text);
            while at < end {
                let next = self.line_end_after((at + PART).min(end));
                state = self.style(at..next, Some(state));
                at = next;
            }
            let (from, to) = (start.offset() as usize, end);
            let mut unstyled = self.unstyled.borrow_mut();
            *unstyled =
                unstyled.iter().flat_map(|r| [r.start..r.end.min(from), r.start.max(to)..r.end]).filter(|r| !r.is_empty()).collect();
        }
    }

    /// After an edit in characters [range]: its paragraph again, or the rest
    /// of the note when a block line changed.
    pub fn restyle_after(self: &Rc<Self>, range: Range<usize>) {
        let text = self.text();
        let now = structure(&text);
        let total = self.buffer.char_count().max(0) as usize;
        let (start, end) = self.paragraph(range.start.min(total), range.end.min(total));
        let state = MarkdownHighlighter::block_state(start, &text);
        if *self.structure.borrow() == now {
            self.style(start..end, Some(state));
        } else {
            // A fence or the front matter changed: everything after it is
            // styled again, what is seen now and the rest as it is seen.
            *self.structure.borrow_mut() = now;
            self.style(start..end, Some(state));
            if end < total {
                let mut unstyled = self.unstyled.borrow_mut();
                *unstyled = unstyled.iter().map(|r| r.start..r.end.min(start)).filter(|r| !r.is_empty()).collect();
                unstyled.push(end..total);
            }
            self.style_seen_soon();
        }
    }

    /// The end of the line that holds character [at], past its newline.
    fn line_end_after(&self, at: usize) -> usize {
        let mut iter = self.buffer.iter_at_offset(at as i32);
        if !iter.starts_line() {
            if !iter.ends_line() {
                iter.forward_to_line_end();
            }
            iter.forward_char();
        }
        iter.offset() as usize
    }

    /// The lines from [start] to [end], and the lines around them up to a
    /// blank one, at most `PARAGRAPH_LINES` each way; found by the buffer's
    /// lines, not by reading the note.
    fn paragraph(&self, start: usize, end: usize) -> (usize, usize) {
        let blank = |line: i32| -> bool {
            let Some(from) = self.buffer.iter_at_line(line) else { return true };
            let mut to = from;
            if !to.ends_line() {
                to.forward_to_line_end();
            }
            self.buffer.text(&from, &to, true).trim().is_empty()
        };
        let mut first = self.buffer.iter_at_offset(start as i32).line();
        let mut last = self.buffer.iter_at_offset(end as i32).line();
        let lines = self.buffer.line_count();
        let mut walked = 0;
        while first > 0 && walked < PARAGRAPH_LINES && !blank(first - 1) {
            first -= 1;
            walked += 1;
        }
        walked = 0;
        while last + 1 < lines && walked < PARAGRAPH_LINES && !blank(last + 1) {
            last += 1;
            walked += 1;
        }
        let from = self.buffer.iter_at_line(first).map_or(0, |i| i.offset() as usize);
        let mut to = self.buffer.iter_at_line(last).unwrap_or_else(|| self.buffer.end_iter());
        if !to.ends_line() {
            to.forward_to_line_end();
        }
        to.forward_char();
        (from, to.offset() as usize)
    }

    fn cancel(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
    }

    fn text(&self) -> String {
        self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), true).to_string()
    }

    /// The tags of the lines in characters [range], again, [state] the
    /// block state its first line begins in (read from the note when not
    /// given); the state after its last line. Only [range]'s own text is
    /// read: the spans are in its characters, and `base` puts them in the
    /// buffer. A tag's priority is the order it was made in; the marks are
    /// raised to the top each time a tag is made, so they dim over everything.
    fn style(&self, range: Range<usize>, state: Option<BlockState>) -> BlockState {
        let base = range.start;
        let iter = |offset: usize| self.buffer.iter_at_offset((base + offset) as i32);
        let state = state.unwrap_or_else(|| MarkdownHighlighter::block_state(range.start, &self.text()));
        let (from, to) = (self.buffer.iter_at_offset(range.start as i32), self.buffer.iter_at_offset(range.end as i32));
        let lines = self.buffer.text(&from, &to, true).to_string();
        // What live preview draws on these lines: the boxes, the chips and the panels.
        let parts = if self.live.get() {
            let (above, below) = self.around(&from, &to, &lines);
            LivePreview::parts(&lines, base, &state, &above, below, self.day.get())
        } else {
            PreviewParts::default()
        };
        let (mut spans, next) = MarkdownHighlighter::spans_of_lines(&lines, range.start, state);
        for span in &mut spans {
            span.range = span.range.start - base..span.range.end - base;
        }
        for (tag, _) in self.tags.borrow().values() {
            self.buffer.remove_tag(tag, &from, &to);
        }
        let chars: Vec<char> = lines.chars().collect();
        let local = 0..chars.len();
        let highlights: Vec<Range<usize>> = spans.iter().filter(|s| s.kind == HighlightKind::Highlight).map(|s| s.range.clone()).collect();
        let panels = Panels::of(&parts, base);
        for span in &spans {
            if span.range.end > chars.len() || span.range.is_empty() {
                continue;
            }
            for tag in self.tags_for(span, &chars, &highlights, &panels) {
                self.buffer.apply_tag(&tag, &iter(span.range.start), &iter(span.range.end));
            }
        }
        self.style_script(&chars, local.clone(), base);
        if self.live.get() {
            let elements = self.draw_parts(&parts, &chars, base);
            self.hide_marks(&spans, &chars, local, base, elements);
        }
        next
    }

    /// What is around the lines from [from] to [to], whose text is [lines]:
    /// the quote or the callout the first of them continues, found by the
    /// buffer's lines above it (at most `PARAGRAPH_LINES`), and whether the
    /// line after the last is a quote's.
    fn around(&self, from: &gtk::TextIter, to: &gtk::TextIter, lines: &str) -> (PreviewAbove, bool) {
        let quoted = |text: &str| text.trim_start_matches([' ', '\t']).starts_with('>');
        let line_at = |line: i32| -> Option<String> {
            let start = self.buffer.iter_at_line(line)?;
            let mut end = start;
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            Some(self.buffer.text(&start, &end, true).to_string())
        };
        let mut above = PreviewAbove::Nothing;
        if quoted(lines) {
            let mut line = from.line() - 1;
            let mut walked = 0;
            while line >= 0 && walked < PARAGRAPH_LINES {
                let Some(text) = line_at(line).filter(|text| quoted(text)) else { break };
                above = LivePreview::callout_head(&text).map_or(PreviewAbove::Quote, |head| PreviewAbove::Callout(head.kind));
                line -= 1;
                walked += 1;
            }
        }
        let below = !to.is_end() && to.starts_line() && line_at(to.line()).is_some_and(|text| quoted(&text));
        (above, below)
    }

    /// The tags of what live preview draws on the lines styled, whose
    /// characters are [chars] from [base]: a task's box and a callout's icon
    /// over their hidden marks, the chips, and the lines of each panel. A
    /// box and a chip are raw while the cursor is in them, and a callout's
    /// and a fence's marks show on the cursor's lines. The boxes and chips
    /// of the cursor's lines are returned, as `reveal` compares them with
    /// the cursor.
    fn draw_parts(&self, parts: &PreviewParts, chars: &[char], base: usize) -> Vec<Range<usize>> {
        let total = self.buffer.char_count().max(0) as usize;
        let iter = |offset: usize| self.buffer.iter_at_offset(offset.min(total) as i32);
        let apply = |tag: &gtk::TextTag, range: Range<usize>| {
            if range.start < range.end {
                self.buffer.apply_tag(tag, &iter(range.start), &iter(range.end));
            }
        };
        let (shown_from, shown_to) = self.revealed.get();
        let on_cursor_lines = |range: &Range<usize>| !(range.end <= shown_from || range.start >= shown_to);
        let cursor = self.cursor.get();
        let mut elements = Vec::new();
        // True when the part is drawn: the cursor is not in it.
        let mut drawn = |range: &Range<usize>| {
            if on_cursor_lines(range) {
                elements.push(range.start - base..range.end - base);
                return !is_open(range, cursor);
            }
            true
        };
        for task in &parts.boxes {
            if drawn(&task.range) {
                // The marks take the box's width, 1.1 em, and no more.
                let count = task.range.len();
                let tag = self.drawn_tag(&format!("box:{count}"), Drawn::TaskBox, || Look {
                    hidden: true,
                    spacing: Some(1.1 / count as f64),
                    unstruck: true,
                    rank: 5,
                    ..Look::default()
                });
                apply(&tag, task.range.clone());
            }
        }
        let chips: Vec<_> = parts.chips.iter().filter(|chip| drawn(&chip.range)).collect();
        for (at, chip) in chips.iter().enumerate() {
            let color = palette_name(chip.kind.color(chip.late));
            let strength: f32 = if chip.closed { 0.75 } else { 1.0 };
            let name = format!("chip:{color}{}{}", if chip.late { ":late" } else { "" }, if chip.closed { ":closed" } else { "" });
            let tag = self.drawn_tag(&name, Drawn::Chip(color, 0.14 * strength), || Look {
                fg: Some((color, strength)),
                scale: Some(0.92),
                semibold: chip.late,
                unstruck: true,
                rank: 2,
                ..Look::default()
            });
            apply(&tag, chip.range.clone());
            // The room around the pill is in the spaces beside it: 0.5 em
            // inside the pill and 0.3 em to the text, or to the next chip's pill.
            let space = |at: usize| at >= base && chars.get(at - base) == Some(&' ');
            if chip.range.start > 0 && space(chip.range.start - 1) {
                let follows = at > 0 && chips[at - 1].range.end + 1 == chip.range.start;
                let (name, room) = if follows { ("chip-gap:between", 1.05) } else { ("chip-gap", 0.55) };
                let gap = self.tag(name, || Look { spacing: Some(room), unstruck: true, rank: 4, ..Look::default() });
                apply(&gap, chip.range.start - 1..chip.range.start);
            }
            let next_follows = chips.get(at + 1).is_some_and(|next| next.range.start == chip.range.end + 1);
            if space(chip.range.end) && !next_follows {
                let gap = self.tag("chip-gap", || Look { spacing: Some(0.55), unstruck: true, rank: 4, ..Look::default() });
                apply(&gap, chip.range.end..chip.range.end + 1);
            }
        }
        let hidden = || self.tag("hidden", || Look { hidden: true, rank: 5, ..Look::default() });
        let top = || self.drawn_tag("panel-top", Drawn::PanelTop, || Look { above: Some(0.6), ..Look::default() });
        for line in &parts.lines {
            // The panel's tag is on the line's ending too, so its lines are one run.
            let whole = line.range.start..line.range.end + 1;
            let raw = on_cursor_lines(&line.range);
            match &line.block {
                PreviewBlock::Callout { kind, head, quote, last } => {
                    let color = callout_color(kind);
                    let panel = self.drawn_tag(&format!("panel:callout:{color}"), Drawn::CalloutPanel(color), || Look {
                        inset: Some((1.0, 1.0)),
                        ..Look::default()
                    });
                    apply(&panel, whole);
                    if head.is_some() {
                        apply(&top(), line.range.start..line.range.end.max(line.range.start + 1));
                    }
                    if *last {
                        let bottom = self.tag("panel-bottom", || Look { below: Some(0.6), ..Look::default() });
                        apply(&bottom, line.range.start..line.range.end.max(line.range.start + 1));
                    }
                    if raw {
                        continue;
                    }
                    // `> ` at no width; alone on its line it is clear at its
                    // own size, so an empty line of the callout keeps its height.
                    if quote.end >= line.range.end && head.is_none() {
                        apply(&self.tag("clear", || Look { clear: true, rank: 5, ..Look::default() }), quote.clone());
                    } else {
                        apply(&hidden(), quote.clone());
                    }
                    if let Some(head) = head {
                        // The icon stands where `[` is, 1.1 em and 0.4 em to the title.
                        let icon = self.drawn_tag("callout-icon", Drawn::CalloutIcon, || Look {
                            clear: true,
                            spacing: Some(1.5),
                            bracket: true,
                            rank: 5,
                            ..Look::default()
                        });
                        apply(&icon, head.mark.start..head.mark.start + 1);
                        apply(&hidden(), head.mark.start + 1..head.mark.end);
                    }
                }
                PreviewBlock::Code { role, fence } => {
                    let panel =
                        self.drawn_tag("panel:code", Drawn::CodePanel, || Look { inset: Some((1.1, 0.6)), left: true, ..Look::default() });
                    apply(&panel, whole);
                    match role {
                        CodeRole::Open => apply(&top(), line.range.start..line.range.end.max(line.range.start + 1)),
                        CodeRole::Close => {
                            // The closing fence's line is 0.4 em high.
                            let bottom = self.tag("panel-bottom:code", || Look { below: Some(0.4), ..Look::default() });
                            apply(&bottom, line.range.start..line.range.end.max(line.range.start + 1));
                        }
                        CodeRole::Body => {}
                    }
                    let Some(fence) = fence.as_ref().filter(|_| !raw) else { continue };
                    apply(&hidden(), fence.marks.clone());
                    if let Some(word) = &fence.word {
                        let language: String = chars[word.start - base..word.end - base].iter().collect();
                        let color = language_color(&language);
                        let tag = self.tag(&format!("language:{color}"), || Look {
                            mono: true,
                            scale: Some(0.72),
                            semibold: true,
                            lower: true,
                            rank: 2,
                            ..fg(color)
                        });
                        apply(&tag, word.clone());
                    }
                }
            }
        }
        elements
    }

    /// Live preview on or off; the note is styled again for it.
    pub fn set_live(self: &Rc<Self>, on: bool) {
        if self.live.replace(on) != on {
            self.restyle_all();
        }
    }

    /// The cursor is on characters [selection]: its line is tinted (with no
    /// selection), and with live preview the lines it left hide their marks
    /// again and the ones it is on show them. Run from an idle, never while
    /// the view lays out.
    pub fn reveal(self: &Rc<Self>, selection: (usize, usize)) {
        // The lines by the buffer's own index, not by reading the text: a
        // note of 20,000 lines would be copied at every key.
        let (first, last) = (self.buffer.iter_at_offset(selection.0 as i32), self.buffer.iter_at_offset(selection.1 as i32));
        let lines = (first.line(), last.line());
        let start_iter = self.buffer.iter_at_line(lines.0).unwrap_or(first);
        let mut end_iter = self.buffer.iter_at_line(lines.1).unwrap_or(last);
        if !end_iter.ends_line() {
            end_iter.forward_to_line_end();
        }
        end_iter.forward_char();
        let (start, end) = (start_iter.offset() as usize, end_iter.offset() as usize);
        let selecting = selection.0 != selection.1;
        self.cursor.set(selection);
        // Typing on the line keeps it: only where it ends moves.
        if self.revealed_lines.replace(Some((lines, selecting))) == Some((lines, selecting)) {
            self.revealed.set((start, end));
            // The same lines, but the cursor may be in another element of
            // them: its marks show and the one it left hides its own.
            if self.live.get() {
                let total = self.buffer.char_count().max(0) as usize;
                let open: Vec<Range<usize>> =
                    self.elements.borrow().iter().filter(|e| is_open(&(e.start + start..e.end + start), selection)).cloned().collect();
                let kept = self.elements_for.get() == Some((start, total));
                if !kept || open != *self.open.borrow() {
                    let text = self.text();
                    self.style(start..end.min(total), Some(MarkdownHighlighter::block_state(start, &text)));
                }
            }
            return;
        }
        let (all_start, all_end) = self.buffer.bounds();
        self.buffer.remove_tag(&self.line, &all_start, &all_end);
        if !selecting {
            self.buffer.apply_tag(&self.line, &start_iter, &end_iter);
        }
        let old = self.revealed.replace((start, end));
        if !self.live.get() {
            return;
        }
        let text = self.text();
        let total = self.buffer.char_count() as usize;
        for (a, b) in [old, (start, end)] {
            let (a, b) = (a.min(total), b.min(total));
            if a < b {
                // Whole lines: the old place may have moved with an edit.
                let a = self.buffer.iter_at_offset(a as i32).line();
                let a = self.buffer.iter_at_line(a).map_or(0, |i| i.offset() as usize);
                let b = self.line_end_after(b);
                self.style(a..b, Some(MarkdownHighlighter::block_state(a, &text)));
            }
        }
    }

    /// Away from the cursor's lines the marks go -- `**`, `==`, `[[` and a
    /// link's target, a heading's `#` -- as the Mac's `hideMarks`. A quote's
    /// `>` and a callout's type stay: they say what the line is.
    fn hide_marks(&self, spans: &[HighlightSpan], chars: &[char], range: Range<usize>, base: usize, mut elements: Vec<Range<usize>>) {
        let (shown_from, shown_to) = self.revealed.get();
        let (shown_from, shown_to) = (shown_from.saturating_sub(base), shown_to.saturating_sub(base));
        let hidden = self.tag("hidden", || Look { hidden: true, rank: 5, ..Look::default() });
        let iter = |offset: usize| self.buffer.iter_at_offset((base + offset) as i32);
        let hide = |from: usize, to: usize| {
            if from < to && to <= chars.len() && (to <= shown_from || from >= shown_to) && from >= range.start && to <= range.end {
                self.buffer.apply_tag(&hidden, &iter(from), &iter(to));
            }
        };
        // On the cursor's lines a line's marks show, and an element's marks
        // show only while the cursor is in the element.
        let cursor = self.cursor.get();
        let cursor = (cursor.0.saturating_sub(base), cursor.1.saturating_sub(base));
        let on_cursor_lines = |from: usize, to: usize| !(to <= shown_from || from >= shown_to);
        let mut hide_in = |marks: &[(usize, usize)], element: Range<usize>| {
            let open = is_open(&element, cursor);
            elements.push(element);
            if open {
                return;
            }
            for &(from, to) in marks {
                if from < to && to <= chars.len() && from >= range.start && to <= range.end {
                    self.buffer.apply_tag(&hidden, &iter(from), &iter(to));
                }
            }
        };
        for (at, span) in spans.iter().enumerate() {
            let (from, to) = (span.range.start, span.range.end);
            if to > chars.len() || from >= to {
                continue;
            }
            match span.kind {
                HighlightKind::Markup => {
                    if chars[from] == '>' || chars[from..].starts_with(&['[', '!']) {
                        continue;
                    }
                    if on_cursor_lines(from, to) {
                        if let Some(owner) = &span.owner {
                            hide_in(&[(from, to)], owner.clone());
                        }
                        continue;
                    }
                    // A heading's `#` goes with the space after it.
                    let heading = chars[from..to].iter().all(|&c| c == '#') && chars.get(to) == Some(&' ');
                    hide(from, to + usize::from(heading));
                }
                // `==🔴 text==`: the color's emoji goes too, as in the reader.
                HighlightKind::Highlight if to - from > 4 && "🔴🟠🟡🟢🔵🟣".contains(chars[from + 2]) => {
                    let swatch = (from + 2, from + 3 + usize::from(chars.get(from + 3) == Some(&' ')));
                    if on_cursor_lines(from, to) {
                        hide_in(&[swatch], from..to);
                    } else {
                        hide(swatch.0, swatch.1);
                    }
                }
                // `[label](target)`: the brackets and the target go, the label reads.
                HighlightKind::Link => {
                    let Some(next) = spans.get(at + 1).filter(|next| next.kind == HighlightKind::Url && next.range.start == to) else {
                        continue;
                    };
                    if to - from >= 2 {
                        let marks = [(from, from + 1), (to - 1, to), (next.range.start, next.range.end)];
                        if on_cursor_lines(from, next.range.end) {
                            hide_in(&marks, from..next.range.end);
                        } else {
                            for (a, b) in marks {
                                hide(a, b);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // What `reveal` compares the cursor with, when these were the cursor's lines.
        let (lines_from, lines_to) = self.revealed.get();
        if lines_from < base + range.end && base + range.start < lines_to.max(lines_from + 1) {
            let total = self.buffer.char_count().max(0) as usize;
            let cursor = self.cursor.get();
            let absolute: Vec<Range<usize>> = elements.iter().map(|e| base + e.start..base + e.end).collect();
            *self.open.borrow_mut() =
                absolute.iter().filter(|e| is_open(e, cursor)).map(|e| e.start - lines_from..e.end - lines_from).collect();
            *self.elements.borrow_mut() = absolute.iter().map(|e| e.start - lines_from..e.end - lines_from).collect();
            self.elements_for.set(Some((lines_from, total)));
        }
    }

    /// Each line on the side its text reads from, whatever its syntax starts
    /// with (`- [x] سلام` is right to left), Persian in its face, and a
    /// table's rows in the code font.
    fn style_script(&self, chars: &[char], range: Range<usize>, base: usize) {
        let iter = |offset: usize| self.buffer.iter_at_offset((base + offset) as i32);
        let mut start = range.start;
        while start < range.end.min(chars.len()) {
            let end = chars[start..].iter().position(|&c| c == '\n').map_or(chars.len(), |n| start + n);
            let line: String = chars[start..end].iter().collect();
            // A table's rows in the code font, so the columns its
            // formatting lines up stand in line.
            if daftar_core::MarkdownTable::is_row(&line) && line.trim_end().ends_with('|') {
                let table = self.tag("table", || Look { mono: true, ..Look::default() });
                self.buffer.apply_tag(&table, &iter(start), &iter(end));
            }
            if direction::contains_rtl(&line) {
                if direction::of_line(&line).is_some_and(|rtl| direction::first_strong(&line) != Some(rtl)) {
                    let flip = self.tag("flip", || Look { flip: true, ..Look::default() });
                    self.buffer.apply_tag(&flip, &iter(start), &iter(end));
                }
                let persian = self.tag("persian", || Look { persian: true, ..Look::default() });
                let mut at = start;
                while at < end {
                    if !direction::is_rtl(chars[at]) {
                        at += 1;
                        continue;
                    }
                    // A run goes on over the spaces and marks between words.
                    let mut to = at + 1;
                    while to < end && (direction::is_rtl(chars[to]) || chars[to] == ' ' || chars[to] == '\u{200c}') {
                        to += 1;
                    }
                    self.buffer.apply_tag(&persian, &iter(at), &iter(to));
                    at = to;
                }
            }
            start = end + 1;
        }
    }

    fn tags_for(&self, span: &HighlightSpan, chars: &[char], highlights: &[Range<usize>], panels: &Panels) -> Vec<gtk::TextTag> {
        let piece = || chars[span.range.clone()].iter().collect::<String>();
        let one = |name: &str, look: Look| vec![self.tag(name, || look)];
        match span.kind {
            HighlightKind::Heading(level) => {
                // The reader's sizes, in one weight (docs/specs/look.md).
                const SCALE: [f64; 6] = [1.6, 1.35, 1.12, 1.0, 1.0, 0.875];
                const COLOR: [&str; 6] = ["heading1", "heading2", "heading3", "heading4", "pink", "purple"];
                let at = (level.clamp(1, 6) - 1) as usize;
                one(&format!("h{}", at + 1), Look { scale: Some(SCALE[at]), semibold: true, ..fg(COLOR[at]) })
            }
            HighlightKind::Markup | HighlightKind::BlockId => {
                // On a highlight the marks dim in the highlight's own text color.
                if highlights.iter().any(|h| h.start <= span.range.start && span.range.end <= h.end) {
                    one("highlight-mark", Look { fg: Some(("text", 0.4)), rank: 3, ..Look::default() })
                } else {
                    one("markup", Look { rank: 3, ..fg("faint") })
                }
            }
            HighlightKind::Bold => one("bold", Look { semibold: true, ..fg("strong") }),
            HighlightKind::Italic => one("italic", Look { italic: true, ..Look::default() }),
            HighlightKind::Strike => one("strike", Look { strike: true, ..Look::default() }),
            HighlightKind::Highlight => {
                let color = highlight_color(piece().chars().nth(2));
                one(&format!("highlight:{color}"), Look { bg: Some((color, 0.32)), ..Look::default() })
            }
            HighlightKind::Code => one("code", Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..fg("raw") }),
            // In a drawn panel the panel is the background, and the body is 0.85 em.
            HighlightKind::CodeBlock if panels.code(span.range.start) => {
                one("code-body", Look { mono: true, scale: Some(0.85), ..Look::default() })
            }
            HighlightKind::CodeBlock => {
                one("code-block", Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..Look::default() })
            }
            HighlightKind::Fence => {
                let text = piece();
                let language = text.trim_matches(|c: char| "`~> \t\n".contains(c)).split(' ').next().unwrap_or("").to_string();
                let color = language_color(&language);
                if panels.code(span.range.start) {
                    one(&format!("fence-line:{color}"), Look { mono: true, scale: Some(0.85), ..fg(color) })
                } else {
                    one(&format!("fence:{color}"), Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..fg(color) })
                }
            }
            HighlightKind::FrontMatter => one("front-matter", Look { mono: true, scale: Some(0.86), ..fg("muted") }),
            HighlightKind::Link => one("link", Look { underline: true, ..fg("link") }),
            HighlightKind::Embed => one("embed", Look { underline: true, ..fg("pink") }),
            HighlightKind::Url => one("url", Look { fg: Some(("link", 0.7)), ..Look::default() }),
            HighlightKind::Tag => {
                let text = piece();
                let color = tag_color(text.trim_start_matches('#'));
                one(&format!("tag:{color}"), Look { bg: Some((color, 0.16)), ..fg(color) })
            }
            HighlightKind::TaskOpen | HighlightKind::TaskOther => one("task-open", Look { mono: true, ..fg("accent") }),
            HighlightKind::TaskDone | HighlightKind::TaskCancelled => one("task-done", Look { strike: true, ..fg("faint") }),
            HighlightKind::ListMarker => {
                // One marker colour at every level, as in the reader.
                one("list", Look { semibold: true, ..fg("accent") })
            }
            // The text keeps its own colour, as in the reader; the tint stands for the bar.
            // A callout that is drawn has its panel behind it: no tint on its lines.
            HighlightKind::Quote if panels.callout(span.range.start) => Vec::new(),
            HighlightKind::Quote => one("quote", Look { bg: Some(("orange", 0.07)), ..Look::default() }),
            HighlightKind::CalloutTitle => {
                let text = piece();
                let kind = text.split_once("[!").and_then(|(_, rest)| rest.split_once(']')).map_or("note", |(kind, _)| kind);
                let color = callout_color(kind);
                if panels.callout(span.range.start) {
                    one(&format!("callout-title:{color}"), Look { semibold: true, ..fg(color) })
                } else {
                    one(&format!("callout:{color}"), Look { semibold: true, bg: Some((color, 0.1)), ..fg(color) })
                }
            }
            HighlightKind::Comment | HighlightKind::Rule => one("faint", fg("faint")),
            HighlightKind::Math => one("math", Look { mono: true, bg: Some(("purple", 0.08)), ..fg("purple") }),
            HighlightKind::Metadata => one("metadata", Look { scale: Some(0.85), unstruck: true, rank: 1, ..fg("muted") }),
        }
    }
}

/// The lines of the panels drawn in a part being styled, in its own
/// characters: a span on one of them takes the panel's look.
#[derive(Default)]
struct Panels {
    code: Vec<Range<usize>>,
    callouts: Vec<Range<usize>>,
}

impl Panels {
    fn of(parts: &PreviewParts, base: usize) -> Panels {
        let mut panels = Panels::default();
        for line in &parts.lines {
            let range = line.range.start - base..line.range.end - base;
            match line.block {
                PreviewBlock::Code { .. } => panels.code.push(range),
                PreviewBlock::Callout { .. } => panels.callouts.push(range),
            }
        }
        panels
    }

    /// Whether the character [at] is on one of [lines], which are in order.
    fn holds(lines: &[Range<usize>], at: usize) -> bool {
        let next = lines.partition_point(|line| line.start <= at);
        next > 0 && at <= lines[next - 1].end
    }

    fn code(&self, at: usize) -> bool {
        Self::holds(&self.code, at)
    }

    fn callout(&self, at: usize) -> bool {
        Self::holds(&self.callouts, at)
    }
}

fn apply_look(tag: &gtk::TextTag, look: &Look, dark: bool, code_font: &str, metrics: Metrics) {
    if let Some((name, alpha)) = look.fg {
        tag.set_foreground_rgba(Some(&rgba(name, alpha, dark)));
    }
    if let Some((name, alpha)) = look.bg {
        tag.set_background_rgba(Some(&rgba(name, alpha, dark)));
    }
    if look.bold {
        tag.set_weight(glib::translate::IntoGlib::into_glib(pango::Weight::Bold));
    } else if look.semibold {
        tag.set_weight(glib::translate::IntoGlib::into_glib(pango::Weight::Semibold));
    } else if look.medium {
        tag.set_weight(glib::translate::IntoGlib::into_glib(pango::Weight::Medium));
    }
    if look.italic {
        tag.set_style(pango::Style::Italic);
    }
    if look.strike {
        tag.set_strikethrough(true);
    }
    if look.underline {
        tag.set_underline(pango::Underline::Single);
    }
    if let Some(scale) = look.scale {
        tag.set_scale(scale);
    }
    if look.mono {
        tag.set_family(Some(code_font));
    }
    if look.flip {
        tag.set_justification(gtk::Justification::Right);
    }
    if look.persian {
        tag.set_family(Some(PERSIAN_FONT));
    }
    if look.hidden {
        // Next to nothing and clear, as the Mac hides them. GtkTextView's
        // own `invisible` aborts the app ("byte index off the end of the
        // line") when it lays out a long note whose lines end in hidden
        // marks, so the marks stay text, too small to see.
        tag.set_scale(0.01);
        tag.set_foreground_rgba(Some(&CLEAR));
        tag.set_letter_spacing(0);
    }
    if look.clear {
        tag.set_foreground_rgba(Some(&CLEAR));
    }
    if let Some(em) = look.spacing {
        // Pango's units: 1024 to a px.
        let room = em * metrics.size - if look.bracket { metrics.bracket } else { 0.0 };
        tag.set_letter_spacing((room.max(0.0) * f64::from(pango::SCALE)).round() as i32);
    }
    if let Some((left, right)) = look.inset {
        tag.set_left_margin(metrics.room + (left * metrics.size).round() as i32);
        tag.set_right_margin(metrics.room + (right * metrics.size).round() as i32);
    }
    if let Some(above) = look.above {
        tag.set_pixels_above_lines((above * metrics.size).round() as i32);
    }
    if let Some(below) = look.below {
        tag.set_pixels_below_lines(metrics.gap() + (below * metrics.size).round() as i32);
    }
    if look.lower {
        tag.set_text_transform(pango::TextTransform::Lowercase);
    }
    if look.unstruck {
        tag.set_strikethrough(false);
    }
    if look.left {
        tag.set_justification(gtk::Justification::Left);
    }
}

/// Next to clear: GTK draws a colour with no alpha at all as a solid one.
const CLEAR: gtk::gdk::RGBA = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.002);

/// The face Persian is drawn in wherever it appears; the reader's
/// `Persian Fallback` is the same.
pub const PERSIAN_FONT: &str = "Vazirmatn";

/// The lines that open or close a block, as written, in order: where they
/// are does not count, so a new line above a fence keeps the structure.
/// The Mac's `EditorController.structure(of:)`.
pub fn structure(text: &str) -> Vec<String> {
    let mut marks: Vec<(usize, String)> = Vec::new();
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let contents = line.trim_end_matches(['\n', '\r']);
        if contents.contains("$$") || contents.contains("%%") {
            marks.push((start, line.to_string()));
        }
        let rest = contents.trim_start_matches([' ', '\t', '>']);
        if rest.starts_with("```") || rest.starts_with("~~~") {
            marks.push((start, rest.to_string()));
        } else if rest.starts_with("---") && rest.chars().all(|c| c == '-' || c == ' ') {
            marks.push((start, "---".into()));
        }
        start += line.len();
    }
    marks.sort_by_key(|(at, _)| *at);
    marks.into_iter().map(|(_, line)| line).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structure_is_the_block_lines_in_order() {
        let text = "---\ntags: [a]\n---\n# T\n> ```swift\ncode\n```\n$$ x $$\na %% b\n| --- |\n- [ ] task\n";
        assert_eq!(structure(text), ["---", "---", "```swift", "```", "$$ x $$\n", "a %% b\n"]);
        // A new line above a fence changes where it is, not the structure.
        assert_eq!(structure(&format!("\n{text}")), structure(text));
    }

    /// A caret is in an element from its first mark to after its last; a
    /// selection, when it reaches into it (docs/specs/look.md).
    #[test]
    fn the_cursor_is_in_an_element_at_its_ends_too() {
        let bold = 5..13;
        assert!(is_open(&bold, (5, 5)) && is_open(&bold, (9, 9)) && is_open(&bold, (13, 13)));
        assert!(!is_open(&bold, (4, 4)) && !is_open(&bold, (14, 14)));
        assert!(is_open(&bold, (0, 6)) && is_open(&bold, (12, 20)));
        assert!(!is_open(&bold, (0, 5)) && !is_open(&bold, (13, 20)));
    }

    #[test]
    fn naz_soft_caps_the_dark_colours_only() {
        let (dark, light) = (rgba_in("accent", 1.0, true, true), rgba_in("accent", 1.0, false, true));
        let byte = |c: f32| (c * 255.0).round() as u32;
        assert_eq!((byte(dark.red()), byte(dark.green()), byte(dark.blue())), (0xE9, 0xB8, 0x7D));
        assert_eq!((byte(light.red()), byte(light.green()), byte(light.blue())), (0xA8, 0x52, 0x00));
    }

    /// A span on a line of a drawn panel takes the panel's look; the lines are in order.
    #[test]
    fn a_span_knows_the_panel_its_line_is_in() {
        let text = "a\n> [!tip]\n> b\n\n```\nc\n```\nd\n";
        let parts = LivePreview::parts(text, 100, &BlockState::Normal, &PreviewAbove::Nothing, false, Day::today());
        let panels = Panels::of(&parts, 100);
        assert_eq!(panels.callouts, [2..10, 11..14]);
        assert_eq!(panels.code, [16..19, 20..21, 22..25]);
        assert!(panels.callout(2) && panels.callout(10) && panels.callout(12) && !panels.callout(0) && !panels.callout(15));
        assert!(panels.code(16) && panels.code(20) && panels.code(25) && !panels.code(26) && !panels.code(12));
    }

    #[test]
    fn the_looks_follow_the_text_size() {
        let metrics = Metrics { size: 20.0, room: 100, bracket: 6.0 };
        assert_eq!(metrics.gap(), 13);
        assert_eq!(Metrics::default().gap(), 10);
        // The reader's names for a status's and a chip's colour, in this palette.
        assert_eq!(palette_name("list-marker"), "accent");
        assert_eq!(palette_name("done-color"), "faint");
        assert_eq!(palette_name("spring-green"), "green");
        assert_eq!(palette_name("text-muted"), "muted");
        assert_eq!(palette_name("vivid-yellow"), "vividYellow");
    }

    #[test]
    fn colors_match_the_reader() {
        assert_eq!(callout_color("TODO"), "vividYellow");
        assert_eq!(language_color("rust"), "orange");
        assert_eq!(highlight_color(Some('🔵')), "blue");
        // The reader's hash: the same tag, the same color, every time.
        assert_eq!(tag_color("work"), tag_color("work"));
        assert_eq!(rgba("accent", 1.0, true), gtk::gdk::RGBA::new(1.0, 176.0 / 255.0, 64.0 / 255.0, 1.0));
    }
}
