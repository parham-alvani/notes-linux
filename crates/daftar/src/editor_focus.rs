//! Focus mode (`editor.focus`, `docs/specs/look.md`, part 2): every
//! paragraph but the caret's is dimmed. Nothing of the text changes and no
//! tag is laid: the editor draws a veil in the page's colour over the lines
//! that are not the paragraph, at 55%, so each colour under it keeps its hue
//! and 45% of its strength, and so does every tint, panel, box, chip and
//! grid. The paragraph is kept as two marks of the buffer, which move with
//! the text: a key that leaves the caret in its paragraph does nothing here,
//! and a move changes two ranges.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib, graphene};

use daftar_core::FocusParagraph;

/// How strong the dimmed text stays, and how long a change takes, in µs.
pub const DIMMED: f32 = 0.45;
const CHANGE: i64 = 120_000;

/// The veil's strength over one band of the page: none over the bright
/// paragraph, full over the rest, and in between while a change runs.
/// [progress] is 0 when the caret has just moved and 1 when the change is done.
pub fn veil(in_bright: bool, in_left: bool, progress: f32) -> f32 {
    let full = 1.0 - DIMMED;
    if in_bright {
        full * (1.0 - progress)
    } else if in_left {
        full * progress
    } else {
        full
    }
}

/// The bands of the page from [top] to [bottom], cut where the bright
/// paragraph and the one the caret left begin and end: each with its veil.
pub fn bands(top: f32, bottom: f32, bright: Range<f32>, left: Option<Range<f32>>, progress: f32) -> Vec<(f32, f32, f32)> {
    let mut cuts = vec![top, bottom, bright.start, bright.end];
    if let Some(left) = &left {
        cuts.extend([left.start, left.end]);
    }
    cuts.retain(|y| *y >= top && *y <= bottom);
    cuts.sort_by(f32::total_cmp);
    cuts.dedup();
    let inside = |range: &Range<f32>, y: f32| y >= range.start && y < range.end;
    cuts.windows(2)
        .map(|pair| {
            let middle = f32::midpoint(pair[0], pair[1]);
            (pair[0], pair[1], veil(inside(&bright, middle), left.as_ref().is_some_and(|left| inside(left, middle)), progress))
        })
        .filter(|band| band.2 > 0.001)
        .collect()
}

pub struct Focus {
    view: glib::WeakRef<gtk::TextView>,
    buffer: gtk::TextBuffer,
    on: Cell<bool>,
    /// The bright paragraph: the start of its first line and the end of its last.
    bright: RefCell<Option<(gtk::TextMark, gtk::TextMark)>>,
    /// The paragraph the caret left, and when, while it dims.
    left: RefCell<Option<(gtk::TextMark, gtk::TextMark, i64)>>,
    ticking: Cell<bool>,
}

impl Focus {
    pub fn new(view: &gtk::TextView) -> Rc<Focus> {
        Rc::new(Focus {
            view: view.downgrade(),
            buffer: view.buffer(),
            on: Cell::new(false),
            bright: RefCell::new(None),
            left: RefCell::new(None),
            ticking: Cell::new(false),
        })
    }

    pub fn is_on(&self) -> bool {
        self.on.get()
    }

    /// Focus mode on or off; the page is drawn again for it.
    pub fn set_on(self: &Rc<Self>, on: bool) {
        if self.on.replace(on) == on {
            return;
        }
        self.forget();
        if on {
            self.follow(None);
        }
        self.redraw();
    }

    /// Another note's text is in the buffer: the paragraph is found anew.
    pub fn forget(&self) {
        for (start, end) in self.bright.take().into_iter().chain(self.left.take().map(|(start, end, _)| (start, end))) {
            self.buffer.delete_mark(&start);
            self.buffer.delete_mark(&end);
        }
    }

    fn redraw(&self) {
        if let Some(view) = self.view.upgrade() {
            view.queue_draw();
        }
    }

    fn blank(&self, line: i32) -> bool {
        let Some(start) = self.buffer.iter_at_line(line) else { return true };
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        self.buffer.text(&start, &end, true).trim().is_empty()
    }

    /// The first and the last line of the bright paragraph.
    pub fn bright_lines(&self) -> Option<(i32, i32)> {
        let bright = self.bright.borrow();
        let (start, end) = bright.as_ref()?;
        Some((self.buffer.iter_at_mark(start).line(), self.buffer.iter_at_mark(end).line()))
    }

    /// Whether the paragraph kept is still one, after a change to the
    /// lines [changed]: a blank line alone and still blank, or lines with
    /// no blank one among those changed, between two blank lines. Only the
    /// lines changed and the two beside the paragraph are read.
    fn still_whole(&self, (first, last): (i32, i32), changed: &Range<i32>) -> bool {
        if first == last && self.blank(first) {
            return true;
        }
        let lines = self.buffer.line_count();
        (first.max(changed.start)..=last.min(changed.end)).all(|line| !self.blank(line))
            && !self.blank(first)
            && !self.blank(last)
            && (first == 0 || self.blank(first - 1))
            && (last + 1 >= lines || self.blank(last + 1))
    }

    /// The caret moved, or the lines [changed] of the text did: the bright
    /// paragraph follows the selection. With the caret still in its
    /// paragraph and no change, nothing is read and nothing is drawn.
    pub fn follow(self: &Rc<Self>, changed: Option<Range<i32>>) {
        if !self.on.get() {
            return;
        }
        let (a, b) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (at, at)
        });
        let (from, to) = (a.line().min(b.line()), a.line().max(b.line()));
        let kept = self.bright_lines();
        if let Some((first, last)) = kept
            && from >= first
            && to <= last
            && changed.as_ref().is_none_or(|changed| self.still_whole((first, last), changed))
        {
            return;
        }
        let count = self.buffer.line_count().max(1) as usize;
        let (first, last) = FocusParagraph::lines(from as usize, to as usize, count, |line| self.blank(line as i32));
        let (first, last) = (first as i32, last as i32);
        if kept == Some((first, last)) {
            return;
        }
        let start = self.buffer.iter_at_line(first).unwrap_or_else(|| self.buffer.start_iter());
        let mut end = self.buffer.iter_at_line(last).unwrap_or_else(|| self.buffer.end_iter());
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        // The start stays before what is typed at it, and the end goes on after what is typed at it.
        let marks = (self.buffer.create_mark(None, &start, true), self.buffer.create_mark(None, &end, false));
        let old = self.bright.replace(Some(marks));
        if let Some((start, end, _)) = self.left.take() {
            self.buffer.delete_mark(&start);
            self.buffer.delete_mark(&end);
        }
        // The paragraph left dims over 120 ms, unless it only grew or motion is reduced.
        let moves = gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        let apart = kept.is_some_and(|(was_first, was_last)| was_last < first || was_first > last);
        match old {
            Some((start, end)) if moves && apart => {
                *self.left.borrow_mut() = Some((start, end, glib::monotonic_time()));
                self.tick();
            }
            Some((start, end)) => {
                self.buffer.delete_mark(&start);
                self.buffer.delete_mark(&end);
            }
            None => {}
        }
        self.redraw();
    }

    /// Draws the page again at each frame while a change runs.
    fn tick(self: &Rc<Self>) {
        let Some(view) = self.view.upgrade() else { return };
        if self.ticking.replace(true) {
            return;
        }
        let this = Rc::downgrade(self);
        view.add_tick_callback(move |view, clock| {
            let Some(this) = this.upgrade() else { return glib::ControlFlow::Break };
            let done = this.left.borrow().as_ref().is_none_or(|(_, _, since)| clock.frame_time() - since >= CHANGE);
            if done {
                if let Some((start, end, _)) = this.left.take() {
                    this.buffer.delete_mark(&start);
                    this.buffer.delete_mark(&end);
                }
                this.ticking.set(false);
            }
            view.queue_draw();
            if done { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
    }

    /// The veil over what is on screen, in the buffer's coordinates, in the
    /// page's colour [page]. Drawn over the text and under the caret.
    pub fn paint(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot, page: &gdk::RGBA) {
        if !self.on.get() {
            return;
        }
        let bright = self.bright.borrow();
        let Some((start, end)) = bright.as_ref() else { return };
        let span = |start: &gtk::TextMark, end: &gtk::TextMark| {
            let (top, _) = view.line_yrange(&self.buffer.iter_at_mark(start));
            let (y, height) = view.line_yrange(&self.buffer.iter_at_mark(end));
            top as f32..(y + height) as f32
        };
        let seen = view.visible_rect();
        let left = self.left.borrow();
        let progress = left.as_ref().map_or(1.0, |(_, _, since)| ((glib::monotonic_time() - since) as f32 / CHANGE as f32).clamp(0.0, 1.0));
        let left = left.as_ref().map(|(start, end, _)| span(start, end));
        let (top, bottom) = (seen.y() as f32, (seen.y() + seen.height()) as f32);
        for (from, to, strength) in bands(top, bottom, span(start, end), left, progress) {
            let color = gdk::RGBA::new(page.red(), page.green(), page.blue(), strength);
            snapshot.append_color(&color, &graphene::Rect::new(seen.x() as f32, from, seen.width() as f32, to - from));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dimmed text keeps 45% of its strength: the veil is the page's colour at 55%.
    #[test]
    fn the_veil_leaves_the_bright_paragraph_and_dims_the_rest() {
        assert!((veil(false, false, 1.0) - 0.55).abs() < 1e-6);
        assert!(veil(true, false, 1.0).abs() < 1e-6);
        // While the caret's move runs, the new paragraph brightens and the old one dims.
        assert!((veil(true, false, 0.0) - 0.55).abs() < 1e-6);
        assert!(veil(false, true, 0.0).abs() < 1e-6);
        assert!((veil(false, true, 0.5) - 0.275).abs() < 1e-6);
        // The page from 0 to 500 with the paragraph from 200 to 260: two bands, above and under it.
        let still = bands(0.0, 500.0, 200.0..260.0, None, 1.0);
        assert_eq!(still.iter().map(|band| (band.0, band.1)).collect::<Vec<_>>(), [(0.0, 200.0), (260.0, 500.0)]);
        // Half way through a move from the paragraph at 40 to 80: both are half dimmed.
        let moving = bands(0.0, 500.0, 200.0..260.0, Some(40.0..80.0), 0.5);
        let strength = |y: f32| moving.iter().find(|band| band.0 <= y && y < band.1).map_or(0.0, |band| band.2);
        assert!((strength(10.0) - 0.55).abs() < 1e-6 && (strength(60.0) - 0.275).abs() < 1e-6);
        assert!((strength(230.0) - 0.275).abs() < 1e-6 && (strength(400.0) - 0.55).abs() < 1e-6);
        // A paragraph off screen: the whole page is one band.
        assert_eq!(bands(0.0, 500.0, 900.0..960.0, None, 1.0).len(), 1);
    }
}
