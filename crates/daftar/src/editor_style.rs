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

use daftar_core::{HighlightKind, HighlightSpan, MarkdownHighlighter};

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

fn rgba(name: &str, alpha: f32, dark: bool) -> gtk::gdk::RGBA {
    let (d, l) = pair(name);
    let value = if dark { d } else { l };
    let channel = |shift: u32| ((value >> shift) & 0xFF) as f32 / 255.0;
    gtk::gdk::RGBA::new(channel(16), channel(8), channel(0), alpha)
}

/// How a tag looks, by palette names, so it can be drawn in either scheme.
#[derive(Clone, Default)]
struct Look {
    fg: Option<(&'static str, f32)>,
    bg: Option<(&'static str, f32)>,
    bold: bool,
    medium: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    scale: Option<f64>,
    mono: bool,
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
    /// The rest of a long note, styled a part at a time.
    pending: RefCell<Option<glib::SourceId>>,
}

/// Characters styled at once; the rest of a long note follows between frames.
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
        });
        let this = Rc::downgrade(&styler);
        manager.connect_dark_notify(move |manager| {
            if let Some(this) = this.upgrade() {
                this.dark.set(manager.is_dark());
                for (tag, look) in this.tags.borrow().values() {
                    apply_look(tag, look, manager.is_dark(), &this.code_font.borrow());
                }
            }
        });
        styler
    }

    pub fn set_code_font(&self, family: &str) {
        *self.code_font.borrow_mut() = family.to_string();
        for (tag, look) in self.tags.borrow().values() {
            apply_look(tag, look, self.dark.get(), family);
        }
    }

    fn tag(&self, name: &str, look: impl FnOnce() -> Look) -> gtk::TextTag {
        if let Some((tag, _)) = self.tags.borrow().get(name) {
            return tag.clone();
        }
        let look = look();
        let tag = gtk::TextTag::new(Some(&format!("daftar:{name}")));
        apply_look(&tag, &look, self.dark.get(), &self.code_font.borrow());
        self.buffer.tag_table().add(&tag);
        self.tags.borrow_mut().insert(name.to_string(), (tag.clone(), look));
        // Marks always win: they are the top tags.
        let top = self.buffer.tag_table().size() - 1;
        for raised in ["highlight-mark", "markup"] {
            if let Some((tag, _)) = self.tags.borrow().get(raised) {
                tag.set_priority(top);
            }
        }
        tag
    }

    /// Styles the whole note: the first part now, the rest between frames.
    pub fn restyle_all(self: &Rc<Self>) {
        self.cancel();
        let text = self.text();
        *self.structure.borrow_mut() = structure(&text);
        let total = text.chars().count();
        let first = paragraph_end(&text, PART.min(total));
        self.style(&text, 0..first);
        if first >= total {
            return;
        }
        let this = Rc::downgrade(self);
        let done = std::cell::Cell::new(first);
        let id = glib::idle_add_local(move || {
            let Some(this) = this.upgrade() else { return glib::ControlFlow::Break };
            let text = this.text();
            let total = text.chars().count();
            let start = done.get();
            if start >= total {
                this.pending.take();
                return glib::ControlFlow::Break;
            }
            let end = paragraph_end(&text, (start + PART).min(total));
            this.style(&text, start..end);
            done.set(end);
            if end >= total {
                this.pending.take();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    /// After an edit in characters [range]: its paragraph again, or the rest
    /// of the note when a block line changed.
    pub fn restyle_after(self: &Rc<Self>, range: Range<usize>) {
        let text = self.text();
        let now = structure(&text);
        let total = text.chars().count();
        let (start, end) = paragraph(&text, range.start.min(total), range.end.min(total));
        if *self.structure.borrow() != now {
            *self.structure.borrow_mut() = now;
            self.cancel();
            self.style(&text, start..total);
        } else {
            self.style(&text, start..end);
        }
    }

    fn cancel(&self) {
        if let Some(id) = self.pending.take() {
            id.remove();
        }
    }

    fn text(&self) -> String {
        self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), true).to_string()
    }

    /// The tags of the lines in characters [range] of [text], again. A tag's
    /// priority is the order it was made in; the marks are raised to the top
    /// each time a tag is made, so they dim over everything.
    fn style(&self, text: &str, range: Range<usize>) {
        let iter = |offset: usize| self.buffer.iter_at_offset(offset as i32);
        // [range] is whole lines: a paragraph, or a part that ends with one.
        let spans = MarkdownHighlighter::spans_in(text, range.clone());
        let (from, to) = (iter(range.start), iter(range.end));
        for (tag, _) in self.tags.borrow().values() {
            self.buffer.remove_tag(tag, &from, &to);
        }
        let chars: Vec<char> = text.chars().collect();
        let highlights: Vec<Range<usize>> = spans.iter().filter(|s| s.kind == HighlightKind::Highlight).map(|s| s.range.clone()).collect();
        for span in &spans {
            if span.range.end > chars.len() || span.range.is_empty() {
                continue;
            }
            for tag in self.tags_for(span, &chars, &highlights) {
                self.buffer.apply_tag(&tag, &iter(span.range.start), &iter(span.range.end));
            }
        }
    }

    fn tags_for(&self, span: &HighlightSpan, chars: &[char], highlights: &[Range<usize>]) -> Vec<gtk::TextTag> {
        let piece = || chars[span.range.clone()].iter().collect::<String>();
        let one = |name: &str, look: Look| vec![self.tag(name, || look)];
        match span.kind {
            HighlightKind::Heading(level) => {
                const SCALE: [f64; 6] = [1.75, 1.5, 1.25, 1.0, 1.0, 0.875];
                const COLOR: [&str; 6] = ["heading1", "heading2", "heading3", "heading4", "pink", "purple"];
                let at = (level.clamp(1, 6) - 1) as usize;
                one(&format!("h{}", at + 1), Look { scale: Some(SCALE[at]), medium: at >= 3, ..fg(COLOR[at]) })
            }
            HighlightKind::Markup | HighlightKind::BlockId => {
                // On a highlight the marks dim in the highlight's own text color.
                if highlights.iter().any(|h| h.start <= span.range.start && span.range.end <= h.end) {
                    one("highlight-mark", Look { fg: Some(("text", 0.4)), ..Look::default() })
                } else {
                    one("markup", fg("faint"))
                }
            }
            HighlightKind::Bold => one("bold", Look { bold: true, ..fg("strong") }),
            HighlightKind::Italic => one("italic", Look { italic: true, ..fg("italic") }),
            HighlightKind::Strike => one("strike", Look { strike: true, ..Look::default() }),
            HighlightKind::Highlight => {
                let color = highlight_color(piece().chars().nth(2));
                one(&format!("highlight:{color}"), Look { fg: Some(("text", 1.0)), bg: Some((color, 0.45)), ..Look::default() })
            }
            HighlightKind::Code => one("code", Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..fg("raw") }),
            HighlightKind::CodeBlock => {
                one("code-block", Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..Look::default() })
            }
            HighlightKind::Fence => {
                let text = piece();
                let language = text.trim_matches(|c: char| "`~> \t\n".contains(c)).split(' ').next().unwrap_or("").to_string();
                let color = language_color(&language);
                one(&format!("fence:{color}"), Look { mono: true, bg: Some(("panel", 1.0)), scale: Some(0.86), ..fg(color) })
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
                // Each level of a list has its own color, as in the reader.
                let line_start = chars[..span.range.start].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
                let indent: usize = chars[line_start..span.range.end]
                    .iter()
                    .take_while(|c| **c == ' ' || **c == '\t')
                    .map(|c| if *c == '\t' { 4 } else { 1 })
                    .sum();
                const COLORS: [&str; 4] = ["orange", "aqua", "pink", "chartreuse"];
                let color = COLORS[(indent / 4) % COLORS.len()];
                one(&format!("list:{color}"), Look { bold: true, ..fg(color) })
            }
            HighlightKind::Quote => one("quote", Look { bg: Some(("orange", 0.07)), ..fg("quote") }),
            HighlightKind::CalloutTitle => {
                let text = piece();
                let kind = text.split_once("[!").and_then(|(_, rest)| rest.split_once(']')).map_or("note", |(kind, _)| kind);
                let color = callout_color(kind);
                one(&format!("callout:{color}"), Look { bold: true, bg: Some((color, 0.12)), ..fg(color) })
            }
            HighlightKind::Comment | HighlightKind::Rule => one("faint", fg("faint")),
            HighlightKind::Math => one("math", Look { mono: true, bg: Some(("purple", 0.08)), ..fg("purple") }),
            HighlightKind::Metadata => one("metadata", Look { scale: Some(0.85), ..fg("muted") }),
        }
    }
}

fn apply_look(tag: &gtk::TextTag, look: &Look, dark: bool, code_font: &str) {
    if let Some((name, alpha)) = look.fg {
        tag.set_foreground_rgba(Some(&rgba(name, alpha, dark)));
    }
    if let Some((name, alpha)) = look.bg {
        tag.set_background_rgba(Some(&rgba(name, alpha, dark)));
    }
    if look.bold {
        tag.set_weight(glib::translate::IntoGlib::into_glib(pango::Weight::Bold));
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
}

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

/// The paragraph around characters [start, end): from the blank line before
/// to the blank line after, in characters.
fn paragraph(text: &str, start: usize, end: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let blank_before = |at: usize| -> usize {
        let mut i = at.min(chars.len());
        // Back to the start of this line, then up while lines have text.
        while i > 0 && chars[i - 1] != '\n' {
            i -= 1;
        }
        while i > 0 {
            let mut j = i - 1;
            while j > 0 && chars[j - 1] != '\n' {
                j -= 1;
            }
            if chars[j..i - 1].iter().all(|c| c.is_whitespace()) {
                break;
            }
            i = j;
        }
        i
    };
    let blank_after = |at: usize| -> usize {
        let mut i = at.min(chars.len());
        while i < chars.len() {
            let mut j = i;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            let blank = chars[i..j].iter().all(|c| c.is_whitespace());
            i = (j + 1).min(chars.len());
            if blank && j > at {
                break;
            }
        }
        i
    };
    (blank_before(start), blank_after(end))
}

/// The end of the paragraph that holds character [at].
fn paragraph_end(text: &str, at: usize) -> usize {
    paragraph(text, at, at).1
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

    #[test]
    fn a_paragraph_runs_between_blank_lines() {
        let text = "one\n\ntwo\nthree\n\nfour\n";
        // "three" is at 9..14: its paragraph is "two\nthree\n\n".
        assert_eq!(paragraph(text, 10, 10), (5, 16));
        assert_eq!(paragraph(text, 0, 0), (0, 5));
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
