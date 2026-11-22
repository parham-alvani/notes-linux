//! What the editor draws beside the text, part 2 of `docs/specs/look.md`: a
//! callout's and a code block's panel, a table's grid and a date chip's pill
//! under the text, a task's box and a callout's icon over it. Nothing here is a character of
//! the buffer: each part is found by the tag `editor_style.rs` put on its
//! text, only among the lines on screen, and drawn in the buffer's
//! coordinates from `EditorView`'s layers. A part whose tag is gone (the
//! cursor is in it, live preview is off) is not drawn.
//!
//! The icons are the reader's: the Lucide drawings `just web` writes into
//! `tasks.css` and `callouts.css`, used as a mask so each has one colour.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, pango};

use daftar_core::{LivePreview, MarkdownTable, TaskStatuses};

use crate::editor_style::{Drawn, Metrics, Styler, callout_color, palette_name, rgba};

/// x, y, width and height in the buffer's coordinates.
pub type Frame = (f32, f32, f32, f32);

/// A panel's and a chip's corner, the box's size and how much it grows under the pointer.
const PANEL_CORNER: f32 = 12.0;
const BOX_EM: f32 = 1.1;
const BOX_GROWN: f32 = 1.12;

/// The Lucide drawings in the reader's generated CSS, by name: each
/// `url("data:image/svg+xml,…")` holds one, with its name in its class.
pub fn icons_in(css: &str) -> HashMap<String, String> {
    let mut icons = HashMap::new();
    for part in css.split("url(\"data:image/svg+xml,").skip(1) {
        let Some(end) = part.find("\")") else { continue };
        let svg = percent_decoded(&part[..end]);
        let Some(name) = svg.split("lucide-").nth(1).and_then(|rest| rest.split(['"', ' ']).next()) else { continue };
        icons.entry(name.to_string()).or_insert(svg);
    }
    icons
}

/// [text] with each `%XX` as the byte it stands for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = (bytes[at] == b'%').then(|| text.get(at + 1..at + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())).flatten();
        if let Some(byte) = byte {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where a task's box is drawn: a square of [size] in the middle of the
/// room its hidden marks keep.
pub fn box_frame(marks: Frame, size: f32) -> Frame {
    let (x, y, width, height) = marks;
    (x + (width - size) / 2.0, y + (height - size) / 2.0, size, size)
}

/// A chip's pill behind its text on one row: half an em of room at each side.
pub fn pill_frame(text: Frame, em: f32) -> Frame {
    let (x, y, width, height) = text;
    (x - em / 2.0, y, width + em, height)
}

/// A panel from the top of its first line to the bottom of its last, as
/// wide as the measure: [room] at each side of a pane [pane] wide.
pub fn panel_frame(top: f32, bottom: f32, room: f32, pane: f32) -> Frame {
    (room, top, (pane - 2.0 * room).max(0.0), (bottom - top).max(0.0))
}

/// One row of a grid on screen: its top, its bottom, and where its pipes are.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    pub top: f32,
    pub bottom: f32,
    pub pipes: Vec<f32>,
}

/// The shapes of a table drawn as a grid, in the buffer's coordinates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GridShape {
    /// The box around the table: from its first pipe to its last.
    pub frame: Frame,
    /// The header row's fill.
    pub head: Option<Frame>,
    /// The hairlines under the rows and between the columns, each 1 px.
    pub lines: Vec<Frame>,
}

/// The grid of a table whose box goes from [top] to [bottom]: [header] is
/// its first line's pipes (the box is as wide as they reach, with the rows
/// on screen), [head] the header row's top and bottom when it is on screen,
/// and [rows] the rows on screen, each with whether a hairline goes under
/// it. A pipe is the hairline between two columns; the first and the last
/// are the box's own sides.
pub fn grid_shape(top: f32, bottom: f32, header: &[f32], head: Option<(f32, f32)>, rows: &[(GridRow, bool)]) -> Option<GridShape> {
    let xs = || header.iter().chain(rows.iter().flat_map(|(row, _)| row.pipes.iter())).copied();
    let (left, right) = (xs().reduce(f32::min)?, xs().reduce(f32::max)?);
    if right - left < 1.0 || bottom <= top {
        return None;
    }
    let mut shape = GridShape { frame: (left, top, right - left, bottom - top), ..GridShape::default() };
    shape.head = head.map(|(y, end)| (left, y, right - left, end - y));
    for (row, ruled) in rows {
        // A row's own first and last pipe are the box's sides, also in a
        // row that is typed in and not lined up with the others yet.
        let inner = row.pipes.get(1..row.pipes.len().saturating_sub(1)).unwrap_or(&[]);
        for x in inner.iter().filter(|x| **x - left >= 1.0 && right - **x >= 1.0) {
            shape.lines.push((x.round() - 0.5, row.top, 1.0, row.bottom - row.top));
        }
        if *ruled {
            shape.lines.push((left, row.bottom - 1.0, right - left, 1.0));
        }
    }
    Some(shape)
}

fn rect(frame: Frame) -> graphene::Rect {
    graphene::Rect::new(frame.0, frame.1, frame.2, frame.3)
}

/// The runs of [tag] that reach between [from] and [to].
fn runs(tag: &gtk::TextTag, from: &gtk::TextIter, to: &gtk::TextIter) -> Vec<(gtk::TextIter, gtk::TextIter)> {
    let mut out = Vec::new();
    let mut at = *from;
    if at.has_tag(tag) {
        if !at.starts_tag(Some(tag)) {
            at.backward_to_tag_toggle(Some(tag));
        }
    } else if !at.forward_to_tag_toggle(Some(tag)) {
        return out;
    }
    while at.offset() < to.offset() {
        let mut end = at;
        end.forward_to_tag_toggle(Some(tag));
        out.push((at, end));
        at = end;
        if !at.forward_to_tag_toggle(Some(tag)) {
            break;
        }
    }
    out
}

/// The characters from [start] to [end] as one frame for each row they are on.
fn rows(view: &gtk::TextView, start: &gtk::TextIter, end: &gtk::TextIter) -> Vec<Frame> {
    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut at = *start;
    while at.offset() < end.offset() {
        let place = view.iter_location(&at);
        // A right-to-left letter's box runs left from where it starts.
        let (left, right) = (place.x().min(place.x() + place.width()), place.x().max(place.x() + place.width()));
        match out.iter_mut().find(|row| row.1 == place.y()) {
            Some(row) => {
                let far = (row.0 + row.2).max(right);
                row.0 = row.0.min(left);
                row.2 = far - row.0;
            }
            None => out.push((left, place.y(), right - left, place.height())),
        }
        if !at.forward_char() {
            break;
        }
    }
    out.into_iter().map(|(x, y, width, height)| (x as f32, y as f32, width as f32, height as f32)).collect()
}

pub struct Draw {
    styler: Rc<Styler>,
    svgs: HashMap<String, String>,
    /// Each icon as a picture, drawn once at a size.
    textures: RefCell<HashMap<(String, i32), Option<gdk::Texture>>>,
    /// The box under the pointer, by where its marks start.
    hover: Cell<Option<i32>>,
}

impl Draw {
    pub fn new(styler: &Rc<Styler>) -> Rc<Draw> {
        let mut svgs = HashMap::new();
        for file in ["tasks.css", "callouts.css"] {
            svgs.extend(crate::reader::asset(file).map_or_default(icons_in));
        }
        Rc::new(Draw { styler: styler.clone(), svgs, textures: RefCell::new(HashMap::new()), hover: Cell::new(None) })
    }

    /// One layer of [view]: the panels and the pills under the text, the
    /// boxes and the icons over it. Only what is on screen is looked for.
    pub fn paint(&self, view: &gtk::TextView, layer: gtk::TextViewLayer, snapshot: &gtk::Snapshot) {
        let drawn = self.styler.drawn();
        if drawn.is_empty() {
            return;
        }
        let (metrics, dark) = (self.styler.metrics(), self.styler.is_dark());
        let seen = view.visible_rect();
        let (from, _) = view.line_at_y(seen.y());
        let (mut to, _) = view.line_at_y(seen.y() + seen.height());
        to.forward_line();
        let top = drawn.iter().find(|(_, what)| *what == Drawn::PanelTop).map(|(tag, _)| tag.clone());
        let grid_last = drawn.iter().find(|(_, what)| *what == Drawn::GridLast).map(|(tag, _)| tag.clone());
        let under = layer == gtk::TextViewLayer::BelowText;
        for (tag, what) in &drawn {
            match what {
                Drawn::CalloutPanel(color) if under => {
                    let (fill, line) = (rgba(color, 0.10, dark), rgba(color, 0.26, dark));
                    self.panels(view, snapshot, tag, top.as_ref(), (&from, &to), &fill, Some(&line), metrics);
                }
                Drawn::CodePanel if under => {
                    self.panels(view, snapshot, tag, top.as_ref(), (&from, &to), &rgba("panel", 1.0, dark), None, metrics);
                }
                Drawn::Grid if under => {
                    for (start, end) in runs(tag, &from, &to) {
                        self.grid(view, snapshot, (&start, &end), (&from, &to), grid_last.as_ref(), metrics, dark);
                    }
                }
                Drawn::Chip(color, strength) if under => {
                    let fill = rgba(color, *strength, dark);
                    for (start, end) in runs(tag, &from, &to) {
                        for row in rows(view, &start, &end) {
                            let pill = rect(pill_frame(row, metrics.size as f32));
                            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(pill, pill.height() / 2.0));
                            snapshot.append_color(&fill, &pill);
                            snapshot.pop();
                        }
                    }
                }
                Drawn::TaskBox if !under => {
                    for (start, end) in runs(tag, &from, &to) {
                        let Some(marks) = box_marks(view, &start, &end) else { continue };
                        let look = LivePreview::box_look(status_in(&end), &TaskStatuses::current());
                        let grown = if self.hover.get() == Some(start.offset()) { BOX_GROWN } else { 1.0 };
                        let frame = box_frame(marks, BOX_EM * metrics.size as f32 * grown);
                        self.icon(snapshot, look.icon, &rgba(palette_name(look.color), 1.0, dark), frame, metrics);
                    }
                }
                Drawn::CalloutIcon if !under => {
                    for (start, end) in runs(tag, &from, &to) {
                        self.callout_title(view, snapshot, &start, &end, metrics, dark);
                    }
                }
                _ => {}
            }
        }
    }

    /// The panels of one tag: each run of it, cut where a first line is.
    #[expect(clippy::too_many_arguments, reason = "one drawing call, with what it draws and where")]
    fn panels(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        tag: &gtk::TextTag,
        top: Option<&gtk::TextTag>,
        (from, to): (&gtk::TextIter, &gtk::TextIter),
        fill: &gdk::RGBA,
        line: Option<&gdk::RGBA>,
        metrics: Metrics,
    ) {
        let seen = view.visible_rect();
        for (start, end) in runs(tag, from, to) {
            let mut starts = vec![start];
            if let Some(top) = top {
                let mut at = start;
                while at.forward_to_tag_toggle(Some(top)) && at.offset() < end.offset() {
                    if at.starts_tag(Some(top)) {
                        starts.push(at);
                    }
                }
            }
            for (index, first) in starts.iter().enumerate() {
                let mut last = starts.get(index + 1).copied().unwrap_or(end);
                if last.offset() > first.offset() {
                    last.backward_char();
                }
                let (above, _) = view.line_yrange(first);
                let (y, height) = view.line_yrange(&last);
                // The gap under the last line is the space to what follows, outside the panel.
                let frame = panel_frame(above as f32, (y + height - metrics.gap()) as f32, metrics.room as f32, seen.width() as f32);
                if frame.1 > (seen.y() + seen.height()) as f32 || frame.1 + frame.3 < seen.y() as f32 {
                    continue;
                }
                let shape = gsk::RoundedRect::from_rect(rect(frame), PANEL_CORNER);
                snapshot.push_rounded_clip(&shape);
                snapshot.append_color(fill, &rect(frame));
                snapshot.pop();
                if let Some(line) = line {
                    snapshot.append_border(&shape, &[1.0; 4], &[*line; 4]);
                }
            }
        }
    }

    /// One table as a grid, the reader's table: a hairline box with a 12 px
    /// corner around its lines from [start] to [end], the header row on the
    /// panel colour, a hairline under each row but the last and one where
    /// each pipe stands. Only the rows between [from] and [to] are looked at.
    #[expect(clippy::too_many_arguments, reason = "one drawing call, with what it draws and where")]
    fn grid(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        (start, end): (&gtk::TextIter, &gtk::TextIter),
        (from, to): (&gtk::TextIter, &gtk::TextIter),
        last_tag: Option<&gtk::TextTag>,
        metrics: Metrics,
        dark: bool,
    ) {
        let buffer = view.buffer();
        let mut last = *end;
        if last.offset() > start.offset() {
            last.backward_char();
        }
        let (first_line, last_line) = (start.line(), last.line());
        // Where the pipes of a line stand, and the line's top and bottom.
        let row = |line: i32| -> Option<GridRow> {
            let at = buffer.iter_at_line(line)?;
            let mut line_end = at;
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            let pipes = MarkdownTable::pipes(&buffer.text(&at, &line_end, true))
                .into_iter()
                .map(|pipe| {
                    let mut place = at;
                    place.set_line_offset(pipe as i32);
                    let spot = view.iter_location(&place);
                    spot.x() as f32 + spot.width() as f32 / 2.0
                })
                .collect();
            let (y, height) = view.line_yrange(&at);
            Some(GridRow { top: y as f32, bottom: (y + height) as f32, pipes })
        };
        let Some(header) = row(first_line) else { return };
        let (y, height) = view.line_yrange(&last);
        let mut line_start = last;
        line_start.set_line_offset(0);
        // The gap under the last row is the space to what follows, outside the box.
        let gap = if last_tag.is_some_and(|tag| line_start.has_tag(tag)) { metrics.gap() } else { 0 };
        let bottom = (y + height - gap) as f32;
        let mut rows = Vec::new();
        for line in first_line.max(from.line())..=last_line.min(to.line()) {
            // The delimiter row is not drawn: it is the hairline under the header.
            if line == first_line + 1 {
                continue;
            }
            let Some(mut row) = row(line) else { continue };
            if line == last_line {
                row.bottom = bottom;
            } else if line == first_line && first_line + 2 <= last_line {
                // The header's row goes down to the first row under the delimiter.
                if let Some(next) = buffer.iter_at_line(first_line + 2) {
                    row.bottom = view.line_yrange(&next).0 as f32;
                }
            }
            rows.push((row, line != last_line && !(line == first_line && first_line + 1 == last_line)));
        }
        let head = rows.first().filter(|_| first_line >= from.line()).map(|(row, _)| (row.top, row.bottom));
        let Some(shape) = grid_shape(header.top, bottom, &header.pipes, head, &rows) else { return };
        let frame = rect(shape.frame);
        let corner = PANEL_CORNER.min(shape.frame.3 / 2.0);
        let outline = gsk::RoundedRect::from_rect(frame, corner);
        let hairline = rgba("hairline", 1.0, dark);
        snapshot.push_rounded_clip(&outline);
        if let Some(head) = shape.head {
            snapshot.append_color(&rgba("panel", 1.0, dark), &rect(head));
        }
        for line in &shape.lines {
            snapshot.append_color(&hairline, &rect(*line));
        }
        snapshot.pop();
        snapshot.append_border(&outline, &[1.0; 4], &[hairline; 4]);
    }

    /// A callout's icon where its `[` stands, and the kind's name after it
    /// when the line has no title: drawn, never written.
    fn callout_title(
        &self,
        view: &gtk::TextView,
        snapshot: &gtk::Snapshot,
        start: &gtk::TextIter,
        end: &gtk::TextIter,
        metrics: Metrics,
        dark: bool,
    ) {
        let Some(place) = rows(view, start, end).into_iter().next() else { return };
        let buffer = view.buffer();
        let mut line_start = *start;
        line_start.set_line_offset(0);
        let mut line_end = *start;
        if !line_end.ends_line() {
            line_end.forward_to_line_end();
        }
        let line = buffer.text(&line_start, &line_end, true);
        let Some(head) = LivePreview::callout_head(&line) else { return };
        let color = rgba(callout_color(&head.kind), 1.0, dark);
        let size = BOX_EM * metrics.size as f32;
        let rtl = daftar_core::text::direction::first_strong(&line) == Some(true);
        let x = if rtl { place.0 + place.2 - size } else { place.0 };
        self.icon(snapshot, LivePreview::callout_icon(&head.kind), &color, (x, place.1 + (place.3 - size) / 2.0, size, size), metrics);
        if head.written {
            return;
        }
        let layout = view.create_pango_layout(Some(&head.title));
        let attributes = pango::AttrList::new();
        attributes.insert(pango::AttrInt::new_weight(pango::Weight::Semibold));
        layout.set_attributes(Some(&attributes));
        let (_, height) = layout.pixel_size();
        snapshot.save();
        snapshot.translate(&graphene::Point::new(place.0 + place.2, place.1 + (place.3 - height as f32) / 2.0));
        snapshot.append_layout(&layout, &color);
        snapshot.restore();
    }

    /// The Lucide icon [name] in one colour, in [frame]; a plain square when
    /// the drawing is not there (no SVG loader on this computer).
    fn icon(&self, snapshot: &gtk::Snapshot, name: &str, color: &gdk::RGBA, frame: Frame, metrics: Metrics) {
        // Drawn once, at three times the size, sharp on any screen and when grown.
        let px = (BOX_EM * metrics.size as f32 * 3.0).ceil() as i32;
        let texture = self.textures.borrow_mut().entry((name.to_string(), px)).or_insert_with(|| self.render(name, px)).clone();
        let bounds = rect(frame);
        if let Some(texture) = texture {
            snapshot.push_mask(gsk::MaskMode::Alpha);
            snapshot.append_texture(&texture, &bounds);
            snapshot.pop();
            snapshot.append_color(color, &bounds);
            snapshot.pop();
        } else {
            let inset = bounds.inset_r(frame.2 * 0.12, frame.3 * 0.12);
            snapshot.append_border(&gsk::RoundedRect::from_rect(inset, 3.0), &[1.5; 4], &[*color; 4]);
        }
    }

    fn render(&self, name: &str, px: i32) -> Option<gdk::Texture> {
        let svg = self.svgs.get(name)?;
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(svg.as_bytes()));
        let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, px, px, true, gio::Cancellable::NONE).ok()?;
        Some(gdk::Texture::for_pixbuf(&pixbuf))
    }

    /// The box at a point of the buffer's coordinates: where its marks start.
    pub fn box_at(&self, view: &gtk::TextView, x: i32, y: i32) -> Option<i32> {
        let at = view.iter_at_location(x, y)?;
        let mut before = at;
        before.backward_char();
        let size = BOX_EM * self.styler.metrics().size as f32;
        for (tag, _) in self.styler.drawn().iter().filter(|(_, what)| *what == Drawn::TaskBox) {
            for near in [at, before] {
                if !near.has_tag(tag) {
                    continue;
                }
                let mut start = near;
                if !start.starts_tag(Some(tag)) {
                    start.backward_to_tag_toggle(Some(tag));
                }
                let mut end = start;
                end.forward_to_tag_toggle(Some(tag));
                let Some(marks) = box_marks(view, &start, &end) else { continue };
                let frame = box_frame(marks, size);
                let (left, right) = (frame.0.min(marks.0) - 1.0, (frame.0 + frame.2).max(marks.0 + marks.2) + 1.0);
                if (x as f32) >= left && (x as f32) <= right && (y as f32) >= marks.1 && (y as f32) <= marks.1 + marks.3 {
                    return Some(start.offset());
                }
            }
        }
        None
    }

    /// The pointer is over the box whose marks start at [offset], or over
    /// none: it grows a little, unless motion is reduced. True when it changed.
    pub fn set_hover(&self, offset: Option<i32>) -> bool {
        let grows = gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        self.hover.replace(offset.filter(|_| grows)) != offset.filter(|_| grows)
    }
}

/// The room a box's hidden marks keep, from [start] to [end], as high as
/// its row: the marks are next to nothing in height, so the height is the
/// space's after them, which is of the text's size.
fn box_marks(view: &gtk::TextView, start: &gtk::TextIter, end: &gtk::TextIter) -> Option<Frame> {
    let (x, _, width, _) = rows(view, start, end).into_iter().next()?;
    let row = view.iter_location(end);
    Some((x, row.y() as f32, width, row.height() as f32))
}

/// The status in the box whose marks end at [end]: the character before `]`.
fn status_in(end: &gtk::TextIter) -> char {
    let mut at = *end;
    at.backward_chars(2);
    at.char()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icons_are_the_readers_by_name() {
        let css = concat!(
            "input.task-list-item-checkbox { --task-icon: url(\"data:image/svg+xml,%3Csvg%20class%3D%22lucide%20lucide-square-dashed%22%20viewBox%3D%220%200%2024%2024%22%3E%3Cpath%20d%3D%22M5%203%22%20%2F%3E%3C%2Fsvg%3E\"); --task-color: var(--list-marker); }\n",
            "input[data-status='x'] { --task-icon: url(\"data:image/svg+xml,%3Csvg%20class%3D%22lucide%20lucide-square-check-big%22%3E%3C%2Fsvg%3E\"); }\n",
        );
        let icons = icons_in(css);
        assert_eq!(icons.len(), 2);
        assert_eq!(icons["square-dashed"], r#"<svg class="lucide lucide-square-dashed" viewBox="0 0 24 24"><path d="M5 3" /></svg>"#);
        assert!(icons.contains_key("square-check-big"));
        assert_eq!(percent_decoded("100%25 %zz"), "100% %zz");
    }

    /// Every icon a status or a callout kind is drawn with is in the reader's CSS.
    #[test]
    fn every_drawing_has_its_icon() {
        let mut icons = HashMap::new();
        for file in ["tasks.css", "callouts.css"] {
            icons.extend(icons_in(crate::reader::asset(file).unwrap()));
        }
        let statuses = TaskStatuses::default();
        for symbol in " xX-/><!?*\"ibIlSfkwupdc~".chars() {
            let look = LivePreview::box_look(symbol, &statuses);
            assert!(icons.contains_key(look.icon), "{symbol:?} is drawn with {}", look.icon);
        }
        for kind in [
            "note", "info", "summary", "todo", "tip", "done", "faq", "warning", "fail", "danger", "bug", "example", "quote", "figure",
            "read",
        ] {
            assert!(icons.contains_key(LivePreview::callout_icon(kind)), "{kind}");
        }
    }

    /// A table's grid: the box from the first pipe to the last, the header's
    /// fill, a hairline under each row but the last and one at each inner pipe.
    #[test]
    fn a_grid_has_its_box_its_header_and_its_hairlines() {
        let pipes = vec![100.0, 160.0, 220.0];
        let row = |top: f32, bottom: f32| GridRow { top, bottom, pipes: pipes.clone() };
        let rows = [(row(10.0, 41.0), true), (row(41.0, 71.0), true), (row(71.0, 101.0), false)];
        let shape = grid_shape(10.0, 101.0, &pipes, Some((10.0, 41.0)), &rows).unwrap();
        assert_eq!(shape.frame, (100.0, 10.0, 120.0, 91.0));
        assert_eq!(shape.head, Some((100.0, 10.0, 120.0, 31.0)));
        // One inner pipe in each of three rows, and a line under the first two rows.
        assert_eq!(shape.lines.len(), 5);
        assert_eq!(shape.lines[0], (159.5, 10.0, 1.0, 31.0));
        assert_eq!(shape.lines[1], (100.0, 40.0, 120.0, 1.0));
        assert!(!shape.lines.iter().any(|line| line.1 == 100.0 && line.3 == 1.0), "no line under the last row");
        // A row not lined up yet has its hairlines where its own pipes are, and widens the box;
        // the last pipe of a shorter row is the box's side, not a line in it.
        let ragged = [
            (GridRow { top: 41.0, bottom: 71.0, pipes: vec![100.0, 180.0, 260.0] }, true),
            (GridRow { top: 71.0, bottom: 101.0, pipes: pipes.clone() }, false),
        ];
        let shape = grid_shape(10.0, 101.0, &pipes, None, &ragged).unwrap();
        assert_eq!((shape.frame, shape.head), ((100.0, 10.0, 160.0, 91.0), None));
        assert_eq!(shape.lines, [(179.5, 41.0, 1.0, 30.0), (100.0, 70.0, 160.0, 1.0), (159.5, 71.0, 1.0, 30.0)]);
        // A line with one pipe is no box.
        assert_eq!(grid_shape(10.0, 40.0, &[100.0], None, &[]), None);
    }

    #[test]
    fn a_box_is_in_the_middle_of_its_marks_and_a_pill_has_room() {
        // 16 px text: the marks keep 17.6 px, on a row 22 px high.
        assert_eq!(box_frame((32.0, 100.0, 18.0, 22.0), 17.0), (32.5, 102.5, 17.0, 17.0));
        // Under the pointer the box grows about its middle.
        assert_eq!(box_frame((32.0, 100.0, 18.0, 22.0), 20.0), (31.0, 101.0, 20.0, 20.0));
        assert_eq!(pill_frame((100.0, 40.0, 90.0, 20.0), 16.0), (92.0, 40.0, 106.0, 20.0));
        // A panel is as wide as the measure: 560 px in a pane of 1000.
        assert_eq!(panel_frame(10.0, 110.0, 220.0, 1000.0), (220.0, 10.0, 560.0, 100.0));
    }
}
