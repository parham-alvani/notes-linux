//! The slash bar (`docs/specs/look.md`, part 2): a `/` at the start of a
//! line or after a space shows a small list of things to insert, filtered
//! as the person types. The list never has the keyboard: every key goes to
//! the text first, and the editor sends the list Up, Down, Return, Tab and
//! Escape while it is open. The items, the filter and where a `/` opens it
//! are the core's (`EditorCommands::slash_commands`, `slash_matches`,
//! `slash_bar_opens`). Not in Neovim mode, where the view does not own its text.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use daftar_core::{EditorCommands, LivePreview, MarkdownHighlighter, Snippet};

use super::{Completing, Editor, slash_snippets};
use crate::editor_draw::icons_in;
use crate::editor_style::rgba;

/// The list's width, a row's height, and how many rows show before it scrolls.
const WIDTH: i32 = 280;
const ROW: i32 = 32;
const ROWS: i32 = 8;
/// The room between the `/` and the list.
const GAP: i32 = 4;

/// What stands for an item at the start of its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashIcon {
    /// A Lucide drawing of the reader's bundle, by name.
    Lucide(&'static str),
    /// A few letters, where the bundle has no drawing.
    Glyph(&'static str),
}

/// The icon of the item [id].
pub fn slash_icon(id: &str) -> SlashIcon {
    use SlashIcon::{Glyph, Lucide};
    match id {
        "h1" => Glyph("H1"),
        "h2" => Glyph("H2"),
        "h3" => Glyph("H3"),
        "task" => Lucide("square-check-big"),
        "task-today" | "date" => Lucide("calendar-clock"),
        "bullet" => Lucide("list"),
        "number" => Glyph("1."),
        "quote" => Lucide("quote"),
        "callout" => Lucide(LivePreview::callout_icon("note")),
        "code" => Glyph("</>"),
        "mermaid" => Glyph("◇"),
        "math" => Glyph("∑"),
        "tasks" => Lucide("clipboard-list"),
        "table" => Glyph("▦"),
        "divider" => Glyph("—"),
        "link" => Glyph("[[]]"),
        "embed" => Lucide("image"),
        "time" => Glyph("◷"),
        "red" | "orange" | "yellow" | "green" | "blue" | "purple" => Glyph("●"),
        // The callout kinds, each with the reader's icon for it.
        kind => Lucide(LivePreview::callout_icon(kind)),
    }
}

/// What runs with the item picked.
type Picked = Box<dyn Fn(&Snippet)>;

pub struct SlashBar {
    popover: gtk::Popover,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    /// The items shown, in order, each with its row.
    items: RefCell<Vec<(Snippet, gtk::ListBoxRow)>>,
    chosen: Cell<usize>,
    picked: RefCell<Option<Picked>>,
    open: Cell<bool>,
    svgs: HashMap<String, String>,
}

impl SlashBar {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<SlashBar> {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(ROW * ROWS)
            .width_request(WIDTH)
            .child(&list)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroller)
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .css_classes(["daftar-slash-bar"])
            .build();
        popover.set_offset(0, GAP);
        popover.set_parent(parent);
        let mut svgs = HashMap::new();
        for file in ["tasks.css", "callouts.css"] {
            svgs.extend(crate::reader::asset(file).map_or_default(icons_in));
        }
        let bar = Rc::new(SlashBar {
            popover,
            list,
            scroller,
            items: RefCell::new(Vec::new()),
            chosen: Cell::new(0),
            picked: RefCell::new(None),
            open: Cell::new(false),
            svgs,
        });
        // A click on a row inserts it; the rows take no focus, so it stays in the text.
        let this = Rc::downgrade(&bar);
        bar.list.connect_row_activated(move |_, row| {
            let Some(this) = this.upgrade() else { return };
            let at = this.items.borrow().iter().position(|(_, item)| item == row);
            if let Some(at) = at {
                this.chosen.set(at);
                this.pick();
            }
        });
        bar
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Shows [items] at [at], the `/`'s rectangle in the parent: under it,
    /// or over it when [below] is false. With [grouped], each group has its
    /// header. The first row is chosen. [picked] runs with the row picked.
    pub fn show(&self, at: gdk::Rectangle, below: bool, items: &[Snippet], grouped: bool, dark: bool, picked: impl Fn(&Snippet) + 'static) {
        *self.picked.borrow_mut() = Some(Box::new(picked));
        self.list.remove_all();
        let mut rows = Vec::new();
        let mut group = "";
        for item in items {
            if grouped && item.group != group {
                group = &item.group;
                let header =
                    gtk::ListBoxRow::builder().activatable(false).selectable(false).can_focus(false).css_classes(["group"]).build();
                header.set_child(Some(&gtk::Label::builder().label(group).xalign(0.0).build()));
                self.list.append(&header);
            }
            let row = gtk::ListBoxRow::builder().can_focus(false).css_classes(["item"]).child(&self.row(item, dark)).build();
            self.list.append(&row);
            rows.push((item.clone(), row));
        }
        *self.items.borrow_mut() = rows;
        self.choose(0);
        self.scroller.vadjustment().set_value(0.0);
        self.popover.set_position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top });
        self.popover.set_offset(0, if below { GAP } else { -GAP });
        self.popover.set_pointing_to(Some(&at));
        if self.open.replace(true) {
            // GTK sizes a popover when it is shown, not when its list changes.
            self.popover.present();
        } else {
            self.popover.popup();
        }
    }

    /// One row: the item's icon in its colour, its title, and its group at the end.
    fn row(&self, item: &Snippet, dark: bool) -> gtk::Box {
        let line = gtk::Box::builder().spacing(8).height_request(ROW).build();
        let color = rgba(&item.color, 1.0, dark);
        let icon: gtk::Widget = match slash_icon(&item.id) {
            SlashIcon::Lucide(name) => match self.texture(name, &color) {
                Some(texture) => gtk::Image::builder().paintable(&texture).pixel_size(16).build().upcast(),
                None => gtk::Box::new(gtk::Orientation::Horizontal, 0).upcast(),
            },
            SlashIcon::Glyph(letters) => {
                let label = gtk::Label::builder().use_markup(true).css_classes(["glyph"]).build();
                label.set_markup(&format!("<span foreground=\"{}\">{}</span>", hex(&color), glib::markup_escape_text(letters)));
                label.upcast()
            }
        };
        icon.set_size_request(24, 16);
        icon.set_valign(gtk::Align::Center);
        line.append(&icon);
        line.append(&gtk::Label::builder().label(&item.title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        line.append(&gtk::Label::builder().label(&item.group).css_classes(["group-name"]).build());
        line
    }

    /// The Lucide drawing [name] in [color], sharp at twice its size.
    fn texture(&self, name: &str, color: &gdk::RGBA) -> Option<gdk::Texture> {
        let svg = self.svgs.get(name)?.replace("currentColor", &hex(color));
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(svg.as_bytes()));
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, 32, 32, true, gio::Cancellable::NONE).ok()?;
        Some(gdk::Texture::for_pixbuf(&pixbuf))
    }

    /// The row at [at] is the chosen one, and in sight.
    fn choose(&self, at: usize) {
        let items = self.items.borrow();
        for (index, (_, row)) in items.iter().enumerate() {
            if index == at {
                row.add_css_class("chosen");
            } else {
                row.remove_css_class("chosen");
            }
        }
        self.chosen.set(at);
        let Some((_, row)) = items.get(at) else { return };
        // The first item comes with its group's header.
        let Some(bounds) = row.compute_bounds(&self.list) else { return };
        let adjustment = self.scroller.vadjustment();
        let (top, bottom) = (if at == 0 { 0.0 } else { f64::from(bounds.y()) }, f64::from(bounds.y() + bounds.height()));
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    /// Down and Up: the next row or the one before. They stop at the ends.
    pub fn step(&self, down: bool) {
        let count = self.items.borrow().len();
        self.choose(next_row(self.chosen.get(), count, down));
    }

    /// The chosen row is inserted, and the list closes.
    pub fn pick(&self) {
        let item = self.items.borrow().get(self.chosen.get()).map(|(item, _)| item.clone());
        self.close();
        let picked = self.picked.borrow_mut().take();
        if let (Some(item), Some(picked)) = (item, picked) {
            picked(&item);
        }
    }

    /// The id of the chosen row, and of every row shown.
    #[cfg(test)]
    pub fn shown(&self) -> (Option<String>, Vec<String>) {
        let items = self.items.borrow();
        (items.get(self.chosen.get()).map(|(item, _)| item.id.clone()), items.iter().map(|(item, _)| item.id.clone()).collect())
    }

    pub fn close(&self) {
        if self.open.replace(false) {
            self.popover.popdown();
        }
    }
}

impl Drop for SlashBar {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

/// The row after [chosen] of [count], or the one before: never past an end.
pub fn next_row(chosen: usize, count: usize, down: bool) -> usize {
    if down { (chosen + 1).min(count.saturating_sub(1)) } else { chosen.saturating_sub(1) }
}

/// Whether a list [height] px high goes under a line at [y], [line] px
/// high, in a view [view] px high: under it when there is room, and also
/// when there is none over it either.
pub fn fits_below(y: i32, line: i32, height: i32, view: i32) -> bool {
    y + line + GAP + height <= view || y - GAP - height < 0
}

/// A colour as CSS and Pango markup write it.
fn hex(color: &gdk::RGBA) -> String {
    let byte = |channel: f32| (channel * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(color.red()), byte(color.green()), byte(color.blue()))
}

impl Editor {
    /// A `/` was typed at character [slash]: the bar opens when it stands
    /// at the start of a line or after a space, in Markdown, with
    /// `editor.slash_menu` on and in a vault that can be written. Run once
    /// the `/` is in the text and styled, with what was typed after it
    /// meanwhile as the query. The block the line is in is read from the
    /// note, on a `/` only, never on another key.
    pub(super) fn maybe_slash_bar(self: &Rc<Self>, slash: usize) {
        let line = self.current_line();
        if slash < line.start || slash >= line.end || !self.selection().is_empty() || self.slice(slash..slash + 1) != "/" {
            return;
        }
        let read_only = self.store.borrow().as_ref().is_some_and(|store| daftar_core::VaultEdits::writable(&store.root).is_err());
        let enabled = self.settings.borrow().slash_menu() && !read_only;
        let text = self.slice(line.clone());
        if !enabled || !EditorCommands::opens_slash_menu(&text, slash + 1 - line.start) {
            return;
        }
        let state = MarkdownHighlighter::block_state(line.start, &self.text());
        if EditorCommands::slash_bar_opens(&text, line.start, slash + 1, &state, self.is_neovim(), enabled) {
            self.popup.close();
            self.completing.set(Completing::Slash(slash));
            self.follow_slash(slash);
        }
    }

    /// The list for what is typed after the `/` at [slash] now. A space,
    /// or the caret before the `/`, closes it; when nothing matches it
    /// hides and still listens, so one Backspace can bring it back.
    pub(super) fn follow_slash(self: &Rc<Self>, slash: usize) {
        let at = self.cursor();
        if at <= slash || !self.selection().is_empty() || self.slice(slash..slash + 1) != "/" {
            return self.stop_completion();
        }
        let typed = self.slice(slash + 1..at);
        if typed.chars().any(char::is_whitespace) {
            return self.stop_completion();
        }
        let items = EditorCommands::slash_matches(&typed, &slash_snippets());
        if items.is_empty() {
            self.slash.close();
            return;
        }
        let place = self.view.iter_location(&self.iter(slash));
        let (x, y) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, place.x(), place.y());
        // Under the `/`; over it when there is no room below and there is above.
        let height = ROW * (items.len() as i32).min(ROWS) + 16;
        let below = fits_below(y, place.height(), height, self.view.height());
        let this = Rc::downgrade(self);
        let dark = self.styler.is_dark();
        self.slash.show(
            gdk::Rectangle::new(x, y, place.width().max(1), place.height()),
            below,
            &items,
            typed.is_empty(),
            dark,
            move |item| {
                if let Some(this) = this.upgrade() {
                    this.completing.set(Completing::Nothing);
                    this.apply_slash(item, slash);
                }
            },
        );
    }

    /// The caret moved with no change of the text: the list closes when it
    /// left the place between the `/` and the end of what was typed after it.
    pub(super) fn slash_follows_caret(&self) {
        let Completing::Slash(slash) = self.completing.get() else { return };
        let at = self.cursor();
        let line = self.current_line();
        let inside = self.selection().is_empty() && at > slash && slash >= line.start && {
            let typed = self.slice(slash + 1..line.end.max(slash + 1));
            at <= slash + 1 + typed.chars().take_while(|c| !c.is_whitespace()).count()
        };
        if !inside {
            self.stop_completion();
        }
    }

    /// A key while the `/` listens: the list's when it is one of its own.
    /// Down, Up, Ctrl+N and Ctrl+P choose a row; Return and Tab insert the
    /// chosen one; Escape closes, and the `/` stays in the text. With the
    /// list hidden, each of them is an ordinary key.
    pub(super) fn slash_key(&self, key: gdk::Key, control: bool, alt: bool, shift: bool) -> bool {
        if !matches!(self.completing.get(), Completing::Slash(_)) || alt {
            return false;
        }
        let open = self.slash.is_open();
        if key == gdk::Key::Escape && !control {
            // The `/` does not open the list again until a new one is typed.
            self.stop_completion();
            return open;
        }
        if !open {
            return false;
        }
        match key.to_lower() {
            gdk::Key::Down if !control => self.slash.step(true),
            gdk::Key::Up if !control => self.slash.step(false),
            gdk::Key::n if control && !shift => self.slash.step(true),
            gdk::Key::p if control && !shift => self.slash.step(false),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab if !control && !shift => self.slash.pick(),
            _ => return false,
        }
        true
    }

    /// The item picked for the `/` at [slash]: its text in place of the
    /// `/` and of what was typed after it, as one step to undo.
    pub(super) fn apply_slash(self: &Rc<Self>, snippet: &Snippet, slash: usize) {
        let line = self.current_line();
        let at = self.cursor();
        if slash < line.start || at <= slash || at > line.end || self.slice(slash..slash + 1) != "/" {
            return;
        }
        let edit = EditorCommands::apply_over(snippet, slash - line.start..at - line.start, &self.slice(line.clone()));
        let shift = |range: std::ops::Range<usize>| line.start + range.start..line.start + range.end;
        self.replace(shift(edit.range), &edit.text, Some(shift(edit.selection)));
        self.view.grab_focus();
        if snippet.id == "link" || snippet.id == "embed" {
            self.complete_links();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every item has an icon, and every drawing named is in the reader's bundle.
    #[test]
    fn every_item_has_its_icon() {
        let mut icons = HashMap::new();
        for file in ["tasks.css", "callouts.css"] {
            icons.extend(icons_in(crate::reader::asset(file).unwrap()));
        }
        for item in slash_snippets() {
            match slash_icon(&item.id) {
                SlashIcon::Lucide(name) => assert!(icons.contains_key(name), "{} is drawn with {name}", item.id),
                SlashIcon::Glyph(letters) => assert!(!letters.is_empty() && letters.chars().count() <= 4, "{}", item.id),
            }
        }
        assert_eq!(slash_icon("warning"), SlashIcon::Lucide("triangle-alert"));
        assert_eq!(slash_icon("h2"), SlashIcon::Glyph("H2"));
    }

    /// Down and Up stop at the ends of the list.
    #[test]
    fn the_chosen_row_stops_at_the_ends() {
        assert_eq!(next_row(0, 3, true), 1);
        assert_eq!(next_row(2, 3, true), 2);
        assert_eq!(next_row(0, 3, false), 0);
        assert_eq!(next_row(2, 3, false), 1);
        assert_eq!(next_row(0, 0, true), 0);
    }

    /// The list goes under the `/`, and over it when there is no room below.
    #[test]
    fn the_list_goes_over_the_slash_when_there_is_no_room_below() {
        assert!(fits_below(100, 22, 272, 700));
        assert!(!fits_below(600, 22, 272, 700));
        // No room over it either: under it.
        assert!(fits_below(200, 22, 272, 300));
    }
}
