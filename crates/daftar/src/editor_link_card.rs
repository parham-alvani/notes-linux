//! The link preview in the editor (`docs/specs/look.md`, part 2): resting
//! the pointer on a `[[link]]` for 350 ms shows a small card with the
//! note's title and its first lines. What the card says is the core's
//! `LinkPreview`; its body is drawn with the editor's own styles, marks
//! hidden, in a small view that takes no key and no click. The card never
//! has the keyboard focus. It works in Neovim mode too: it reads the text
//! and changes nothing.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use sourceview5::prelude::*;

use daftar_core::{LinkCard, LinkPreview, PreviewBody};

use super::Editor;
use crate::editor_draw::Draw;
use crate::editor_style::{Metrics, Styler};
use crate::editor_view::EditorView;

/// The card's width and height at most, the room to the link, the room it
/// keeps to the view's edges, and how long it takes to show, in ms.
const WIDTH: i32 = 360;
const HEIGHT: i32 = 240;
const GAP: i32 = 6;
const EDGE: i32 = 8;
const FADE: u32 = 140;
/// The room the head takes of the card's height.
const HEAD: i32 = 40;
/// The body's text, in em of the note's.
const BODY: f64 = 0.9;
/// The room at each side of the body's text.
const ROOM: i32 = 12;

/// How wide the card is in a view [view] px wide: 360 px, or the view's
/// width less 16 px when that is less.
pub fn card_width(view: i32) -> i32 {
    WIDTH.min(view - 2 * EDGE).max(120)
}

/// What the pointer does about a link, as the place it is over changes.
#[derive(Default)]
pub(super) struct Hover {
    /// The link under the pointer, as its characters in the buffer.
    link: Cell<Option<(usize, usize)>>,
    /// The link whose card shows.
    shown: Cell<Option<(usize, usize)>>,
    /// The wait before the card shows, and before it closes.
    rest: RefCell<Option<glib::SourceId>>,
    closing: RefCell<Option<glib::SourceId>>,
    /// A key was pressed since the pointer last moved: typing under a
    /// still pointer never opens a card.
    typed: Cell<bool>,
    /// The window's focus is watched, once.
    watched: Cell<bool>,
}

impl Hover {
    fn stop(timer: &RefCell<Option<glib::SourceId>>) {
        if let Some(id) = timer.take() {
            id.remove();
        }
    }
}

pub struct Card {
    popover: gtk::Popover,
    content: gtk::Box,
    head: gtk::Box,
    title: gtk::Label,
    heading: gtk::Label,
    folder: gtk::Label,
    words: gtk::Label,
    scroller: gtk::ScrolledWindow,
    view: EditorView,
    buffer: sourceview5::Buffer,
    styler: Rc<Styler>,
    font_css: gtk::CssProvider,
    /// The note the head opens.
    path: RefCell<Option<String>>,
    open: Cell<bool>,
    /// The pointer is on the card.
    inside: Cell<bool>,
    fade: RefCell<Option<adw::TimedAnimation>>,
}

impl Card {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Card> {
        let title = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["card-title"]).build();
        let heading = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["card-heading"])
            .visible(false)
            .build();
        let folder = gtk::Label::builder()
            .xalign(1.0)
            .hexpand(true)
            .halign(gtk::Align::End)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["card-folder"])
            .build();
        let head = gtk::Box::builder().spacing(6).css_classes(["card-head"]).build();
        head.append(&title);
        head.append(&heading);
        head.append(&folder);
        // The body: the editor's own view, colours and drawn parts, with no caret and no pointer.
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(false);
        // The editor's own text colour, in either scheme.
        super::install_schemes(&buffer);
        let view = EditorView::new(&buffer);
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_can_focus(false);
        view.set_can_target(false);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(8);
        view.set_bottom_margin(10);
        view.set_left_margin(ROOM);
        view.set_right_margin(ROOM);
        view.add_css_class("daftar-editor");
        let font_css = gtk::CssProvider::new();
        #[expect(deprecated, reason = "GTK has no other way to give one view its own font")]
        view.style_context().add_provider(&font_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let styler = Styler::new(buffer.upcast_ref());
        let draw = Draw::new(&styler);
        view.set_painter(move |view, layer, snapshot| draw.paint(view, layer, snapshot));
        let scroller =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).child(&view).build();
        styler.attach(view.upcast_ref());
        // The view's extent grows as it lays its lines out: the card follows.
        let sized = view.clone();
        scroller.vadjustment().connect_upper_notify(move |_| fit(&sized));
        let words = gtk::Label::builder().xalign(0.0).css_classes(["card-words"]).visible(false).build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        content.append(&head);
        content.append(&scroller);
        content.append(&words);
        let popover = gtk::Popover::builder()
            .child(&content)
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .css_classes(["daftar-link-card"])
            .build();
        popover.set_offset(0, GAP);
        popover.set_parent(parent);
        Rc::new(Card {
            popover,
            content,
            head,
            title,
            heading,
            folder,
            words,
            scroller,
            view,
            buffer,
            styler,
            font_css,
            path: RefCell::new(None),
            open: Cell::new(false),
            inside: Cell::new(false),
            fade: RefCell::new(None),
        })
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// What the card says now: the title, the heading and the folder of
    /// its head, and its body's text or words.
    #[cfg(test)]
    pub fn said(&self) -> (String, String, String, String) {
        let body = if self.words.is_visible() {
            self.words.label().to_string()
        } else {
            self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), true).to_string()
        };
        let heading = if self.heading.is_visible() { self.heading.label().to_string() } else { String::new() };
        (self.title.label().to_string(), heading, self.folder.label().to_string(), body)
    }

    /// Shows [card] at [at], the link's rectangle in the parent, which is
    /// [width] px wide and [height] high. [path] is the note its head
    /// opens. The body is in [font] at [size] px, the note's at 0.9 em,
    /// with the code font [code].
    pub fn show(
        &self,
        at: gdk::Rectangle,
        (width, height): (i32, i32),
        card: &LinkCard,
        path: Option<String>,
        (font, code, size): (&str, &str, f64),
        soft: bool,
    ) {
        let wide = card_width(width);
        self.title.set_label(&card.title);
        if card.body == PreviewBody::Missing {
            self.title.add_css_class("missing");
        } else {
            self.title.remove_css_class("missing");
        }
        self.heading.set_visible(card.heading.is_some());
        self.heading.set_label(&card.heading.as_ref().map_or_default(|heading| format!("› {heading}")));
        self.folder.set_label(&card.folder);
        self.content.set_size_request(wide, -1);
        match &card.body {
            PreviewBody::Lines(lines) => {
                let size = size * BODY;
                self.font_css.load_from_string(&format!("textview.daftar-editor {{ font-family: {font}; font-size: {size}px; }}"));
                let gap = (size * 0.65).round() as i32;
                self.view.set_pixels_below_lines(gap);
                self.view.set_pixels_inside_wrap(gap / 2);
                self.styler.set_code_font(code);
                self.styler.set_soft(soft);
                let bracket = f64::from(self.view.create_pango_layout(Some("[")).pixel_size().0);
                self.styler.set_metrics(Metrics { size, room: ROOM, bracket, ..Metrics::default() });
                self.styler.set_live(true);
                self.buffer.set_text(lines);
                self.styler.restyle_all();
                self.view.set_natural_height(None);
                self.scroller.set_max_content_height(HEIGHT - HEAD);
                self.scroller.vadjustment().set_value(0.0);
                self.scroller.set_visible(true);
                self.words.set_visible(false);
                self.fit_soon();
            }
            other => {
                self.words.set_label(other.words().unwrap_or_default());
                self.words.set_visible(true);
                self.scroller.set_visible(false);
            }
        }
        *self.path.borrow_mut() = path;
        // Under the link; over it when there is no room below and there is above.
        let below = at.y() + at.height() + GAP + HEIGHT <= height || at.y() - GAP - HEIGHT < 0;
        self.popover.set_position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top });
        self.popover.set_offset(0, if below { GAP } else { -GAP });
        // Inside the view by 8 px: a link near the end edge has its card moved back.
        let x = at.x().min(width - EDGE - wide).max(EDGE);
        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(x, at.y(), 1, at.height())));
        if !self.open.replace(true) {
            self.popover.popup();
            // It opens with a fade of 140 ms, at once when motion is reduced (libadwaita follows the setting).
            let target = adw::PropertyAnimationTarget::new(&self.content, "opacity");
            let fade = adw::TimedAnimation::new(&self.content, 0.0, 1.0, FADE, target);
            fade.play();
            *self.fade.borrow_mut() = Some(fade);
        }
    }

    /// The card is as high as its lines, up to its most, and a longer body
    /// scrolls. A text view knows its lines' height only once it has laid
    /// them out, so the height is said again shortly after the text is
    /// set, as it is whenever the view's own extent changes. Only the
    /// height the view would like changes: GTK closes a popover whose
    /// least size grows while it is on screen.
    fn fit_soon(&self) {
        let view = self.view.clone();
        glib::timeout_add_local_once(Duration::from_millis(60), move || fit(&view));
    }

    pub fn close(&self) {
        if self.open.replace(false) {
            self.popover.popdown();
        }
        self.inside.set(false);
    }
}

/// [view] would like to be as high as its lines are, with its margins,
/// and the popover that holds it takes its size again: GTK sizes a popover
/// when it is shown and when its parent is, not when what it holds grows.
fn fit(view: &EditorView) {
    let (y, height) = view.line_yrange(&view.buffer().end_iter());
    let wanted = Some(y + height + view.top_margin() + view.bottom_margin());
    if view.natural_height() != wanted {
        view.set_natural_height(wanted);
        if let Some(popover) = view.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
            popover.present();
        }
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

impl Editor {
    /// The card's own pointer: it may go onto the card, where the wheel
    /// scrolls the body; a click on the head opens the note, as a click on
    /// the link does, and a click in the body does nothing.
    pub(super) fn connect_card(self: &Rc<Self>) {
        let pointer = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        pointer.connect_enter(move |_, _, _| {
            if let Some(this) = this.upgrade() {
                this.card.inside.set(true);
                Hover::stop(&this.hover.closing);
            }
        });
        let this = Rc::downgrade(self);
        pointer.connect_leave(move |_| {
            if let Some(this) = this.upgrade() {
                this.card.inside.set(false);
                this.close_card_soon();
            }
        });
        self.card.popover.add_controller(pointer);
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        click.connect_released(move |_, _, _, _| {
            let Some(this) = this.upgrade() else { return };
            let path = this.card.path.borrow().clone();
            this.close_card();
            if let Some(path) = path {
                // After the click is done with: opening a note changes what this editor shows.
                let view = this.view.clone();
                glib::idle_add_local_once(move || {
                    view.activate_action("win.open-note", Some(&path.to_variant())).ok();
                });
            }
        });
        self.card.head.add_controller(click);
        // A scroll of the note closes the card at once.
        if let Some(v) = self.view.vadjustment() {
            let this = Rc::downgrade(self);
            v.connect_value_changed(move |_| {
                if let Some(this) = this.upgrade() {
                    this.close_card();
                }
            });
        }
    }

    /// The `[[link]]` at a point of the view, as its characters in the
    /// buffer: the pointer is on its text, drawn or raw, and the text is a
    /// link in Markdown (not in code).
    fn link_under(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (bx, by) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let at = self.view.iter_at_location(bx, by)?;
        // The nearest character may be far from the point: at a line's end, under the last line.
        let spot = self.view.iter_location(&at);
        let (left, right) = (spot.x().min(spot.x() + spot.width()), spot.x().max(spot.x() + spot.width()));
        if bx < left - 2 || bx > right + 2 || by < spot.y() || by > spot.y() + spot.height() {
            return None;
        }
        let mut start = at;
        start.set_line_offset(0);
        let mut end = at;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let line = self.buffer.text(&start, &end, true);
        let (range, _) = LinkPreview::link_at(&line, at.line_offset() as usize)?;
        let base = start.offset() as usize;
        // Its name has the link's colour: in code the same letters are no link.
        self.styler.is_link(base + range.start + 2).then_some((base + range.start, base + range.end))
    }

    /// The pointer moved to [place] in the view, or left it; [pressed]
    /// says a mouse button is down. Resting 350 ms on a link shows its
    /// card; off the link and the card for 250 ms closes it.
    pub(super) fn hover_link(self: &Rc<Self>, place: Option<(f64, f64)>, pressed: bool) {
        // The pointer moved: what was typed under it before no longer counts.
        if place.is_some() {
            self.hover.typed.set(false);
        }
        let wanted = !pressed && self.settings.borrow().link_preview();
        let link = place.filter(|_| wanted).and_then(|(x, y)| self.link_under(x, y));
        if pressed {
            self.close_card();
        }
        if link == self.hover.link.get() {
            return;
        }
        self.hover.link.set(link);
        Hover::stop(&self.hover.rest);
        match link {
            Some(link) if self.card.is_open() && self.hover.shown.get() == Some(link) => Hover::stop(&self.hover.closing),
            Some(link) => {
                self.close_card_soon();
                let this = Rc::downgrade(self);
                let id = glib::timeout_add_local_once(Duration::from_millis(LinkPreview::REST), move || {
                    let Some(this) = this.upgrade() else { return };
                    this.hover.rest.take();
                    if this.hover.link.get() == Some(link) && !this.hover.typed.get() {
                        this.show_card(link);
                    }
                });
                *self.hover.rest.borrow_mut() = Some(id);
            }
            None => self.close_card_soon(),
        }
    }

    /// The card closes 250 ms after the pointer has left both the link and the card.
    fn close_card_soon(self: &Rc<Self>) {
        if !self.card.is_open() || self.hover.closing.borrow().is_some() {
            return;
        }
        let this = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(LinkPreview::LINGER), move || {
            let Some(this) = this.upgrade() else { return };
            this.hover.closing.take();
            let on_link = this.hover.link.get().is_some() && this.hover.link.get() == this.hover.shown.get();
            if this.card.inside.get() || on_link {
                return;
            }
            // The card's own surface may take the pointer from the view while
            // the pointer is still on the link (its shadow lies over the
            // link): the view hears that the pointer left. Where the
            // pointer is says it, and the card is looked at again later.
            if this.pointer_on_shown_link() {
                this.close_card_soon();
            } else {
                // The wait on another link goes on: its card comes next.
                this.hover.shown.set(None);
                this.card.close();
            }
        });
        *self.hover.closing.borrow_mut() = Some(id);
    }

    /// Whether the pointer is over the link whose card shows, by where the
    /// pointer is in the window, not by the last event the view heard.
    fn pointer_on_shown_link(&self) -> bool {
        let place = || -> Option<(f64, f64)> {
            let native = self.view.native()?;
            let pointer = self.view.display().default_seat()?.pointer()?;
            let (x, y, _) = native.surface()?.device_position(&pointer)?;
            let (left, top) = native.surface_transform();
            let point = gtk::graphene::Point::new((x - left) as f32, (y - top) as f32);
            let point = native.compute_point(&self.view, &point)?;
            Some((f64::from(point.x()), f64::from(point.y())))
        };
        let shown = self.hover.shown.get();
        shown.is_some() && place().and_then(|(x, y)| self.link_under(x, y)) == shown
    }

    /// The card closes at once: a key, a scroll of the note, a click
    /// outside it, another note, or the window lost the focus.
    pub(super) fn close_card(&self) {
        Hover::stop(&self.hover.rest);
        Hover::stop(&self.hover.closing);
        self.hover.shown.set(None);
        self.card.close();
    }

    /// A key was pressed: no card while the pointer stays where it is.
    pub(super) fn typed_under_pointer(&self) {
        self.hover.typed.set(true);
        self.hover.link.set(None);
        self.close_card();
    }

    /// The card of the link at characters [link] of the buffer: a note of
    /// the vault with its first lines, or the words for a name with no
    /// note. A link to a file that is no note has no card.
    pub(super) fn show_card(self: &Rc<Self>, link: (usize, usize)) {
        let Some(store) = self.store.borrow().clone() else { return };
        let Some(source) = self.path() else { return };
        let total = self.buffer.char_count().max(0) as usize;
        if link.1 > total {
            return;
        }
        let Some(target) = LinkPreview::target(&self.slice(link.0..link.1)) else { return };
        let path = if target.name.is_empty() { Some(source.clone()) } else { store.index().resolve(&target.name, &source) };
        if path.as_ref().is_some_and(|path| !path.to_lowercase().ends_with(".md")) {
            return;
        }
        // The note open here is read from the editor, which may hold more than the file.
        let text = path.as_ref().and_then(|path| if *path == source { Some(self.text()) } else { store.text(path) });
        let card = LinkPreview::card(&target, path.as_deref().filter(|_| text.is_some()), text.as_deref());
        let place = self.view.iter_location(&self.iter(link.0));
        let (x, y) = self.view.buffer_to_window_coords(gtk::TextWindowType::Widget, place.x(), place.y());
        let (font, code, size, soft) = {
            let settings = self.settings.borrow();
            let code = settings.file.string("appearance.code_font").filter(|f| !f.is_empty()).unwrap_or_else(|| "monospace".into());
            (settings.font_css().0, code, settings.text_size() as f64, settings.soft_palette())
        };
        let opens = path.filter(|_| card.body != PreviewBody::Missing);
        self.card.show(
            gdk::Rectangle::new(x, y, place.width().max(1), place.height()),
            (self.view.width(), self.view.height()),
            &card,
            opens,
            (&font, &code, size),
            soft,
        );
        self.hover.shown.set(Some(link));
        Hover::stop(&self.hover.closing);
        // The window lost the focus: the card closes.
        if !self.hover.watched.replace(true)
            && let Some(window) = self.view.root().and_downcast::<gtk::Window>()
        {
            let this = Rc::downgrade(self);
            window.connect_is_active_notify(move |window| {
                if let Some(this) = this.upgrade().filter(|_| !window.is_active()) {
                    this.close_card();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_is_360_px_or_the_views_width_less_16() {
        assert_eq!(card_width(1000), 360);
        assert_eq!(card_width(376), 360);
        assert_eq!(card_width(300), 284);
        assert_eq!(card_width(60), 120);
    }
}
