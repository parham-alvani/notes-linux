//! The editor's caret (`docs/specs/look.md`, "The caret in the accent").
//! GtkTextView draws its own caret in the text's colour and blinks it on and
//! off, so the editor hides that one and draws this: a bar two px wide in
//! the accent, over the text, that glides to where the cursor goes, stays
//! solid while keys come and fades slowly once they stop.
//!
//! It is drawn over the text when the view draws itself, at the place the
//! view gives for the cursor then: the lines are laid out by that time, so
//! the caret is never where the cursor was before a scroll, and a key reads
//! no more of the note for it. A frame is asked for only while it glides or
//! its strength changes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

/// The caret moves between two places in this long.
const GLIDE_MS: f64 = 70.0;
/// With no key for this long, the caret starts to fade.
const REST_MS: f64 = 650.0;
/// One fade, from full to faint and back.
const FADE_MS: f64 = 1200.0;
/// How strong the caret is at the faintest of its fade.
const FAINT: f64 = 0.25;
const WIDTH: f32 = 2.0;
/// A change of strength smaller than this is not drawn again.
const STEP: f64 = 0.03;

/// x, y and height, in the view's buffer coordinates.
type Place = [f64; 3];

/// How strong the caret is drawn [idle] ms after the last key: solid while
/// keys come, then fading to a quarter and back once in 1200 ms. With
/// [fades] false (GTK's cursor blink is off, or motion is reduced) it is solid.
pub fn strength(idle: f64, fades: bool) -> f64 {
    if !fades || idle <= REST_MS {
        return 1.0;
    }
    let turn = ((idle - REST_MS) % FADE_MS) / FADE_MS * std::f64::consts::TAU;
    FAINT + (1.0 - FAINT) * (0.5 + 0.5 * turn.cos())
}

/// Where the caret is [elapsed] ms after it left [from] for [to]: there
/// after 70 ms, slowing as it arrives.
pub fn glided(from: Place, to: Place, elapsed: f64) -> Place {
    let part = (elapsed / GLIDE_MS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - part) * (1.0 - part);
    std::array::from_fn(|at| from[at] + (to[at] - from[at]) * eased)
}

/// Whether a move from [from] to [to] glides: not one of more than the
/// view's height, and none when motion is reduced.
pub fn glides(from: Place, to: Place, view_height: f64, motion: bool) -> bool {
    motion && (to[1] - from[1]).abs() <= view_height
}

pub struct Caret {
    view: gtk::TextView,
    /// Off in Neovim's normal mode, which has its own block.
    on: Cell<bool>,
    focused: Cell<bool>,
    /// The cursor moved or a key came since the caret was last drawn.
    moved: Cell<bool>,
    /// Where the cursor was when the caret was last drawn, and where the caret was drawn.
    target: Cell<Option<Place>>,
    drawn: Cell<Option<Place>>,
    /// The glide under way: where it left from, and when (the frame clock's µs).
    glide: Cell<Option<(Place, i64)>>,
    /// The last key or move, in the frame clock's µs.
    active: Cell<i64>,
    /// How strong the caret was last drawn.
    shown: Cell<f64>,
    ticking: RefCell<Option<gtk::TickCallbackId>>,
}

/// Whether the caret fades and glides: GTK's own settings say.
fn motion() -> (bool, bool) {
    let settings = gtk::Settings::default();
    let moves = settings.as_ref().is_none_or(gtk::Settings::is_gtk_enable_animations);
    (moves && settings.as_ref().is_none_or(gtk::Settings::is_gtk_cursor_blink), moves)
}

impl Caret {
    pub fn new(view: &impl IsA<gtk::TextView>) -> Rc<Caret> {
        let view = view.clone().upcast::<gtk::TextView>();
        // The view's own caret is clear (`style.rs`): this one is drawn in its
        // place. It stays "visible" to GTK, since a view with no cursor
        // scrolls on the arrow keys instead of moving it.
        let caret = Rc::new(Caret {
            view,
            on: Cell::new(true),
            focused: Cell::new(false),
            moved: Cell::new(false),
            target: Cell::new(None),
            drawn: Cell::new(None),
            glide: Cell::new(None),
            active: Cell::new(0),
            shown: Cell::new(1.0),
            ticking: RefCell::new(None),
        });
        let buffer = caret.view.buffer();
        let this = Rc::downgrade(&caret);
        buffer.connect_mark_set(move |_, _, mark| {
            if let Some(this) = this.upgrade().filter(|_| mark.name().as_deref() == Some("insert")) {
                this.moved();
            }
        });
        let this = Rc::downgrade(&caret);
        buffer.connect_changed(move |_| {
            if let Some(this) = this.upgrade() {
                this.moved();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let this = Rc::downgrade(&caret);
        focus.connect_enter(move |_| {
            if let Some(this) = this.upgrade() {
                this.focused.set(true);
                this.moved();
            }
        });
        let this = Rc::downgrade(&caret);
        focus.connect_leave(move |_| {
            if let Some(this) = this.upgrade() {
                this.focused.set(false);
                this.view.queue_draw();
            }
        });
        caret.view.add_controller(focus);
        caret
    }

    /// Neovim's normal mode has its block instead: the caret is off there.
    pub fn set_on(self: &Rc<Self>, on: bool) {
        if self.on.replace(on) != on {
            // It comes back where the cursor is, with no glide from where it was.
            self.drawn.set(None);
            self.moved();
        }
    }

    /// The cursor moved or a key came: the caret is solid, and glides there
    /// when it is next drawn.
    fn moved(self: &Rc<Self>) {
        self.active.set(glib::monotonic_time());
        self.moved.set(true);
        self.view.queue_draw();
        if self.ticking.borrow().is_some() || !self.seen() {
            return;
        }
        // Frames for as long as it glides or fades; each asks for a drawing
        // only when the caret would look different.
        let this = Rc::downgrade(self);
        let id = self.view.add_tick_callback(move |view, clock| {
            let Some(this) = this.upgrade() else { return glib::ControlFlow::Break };
            let (fades, _) = motion();
            let gliding = this.glide.get().is_some() || this.moved.get();
            if !this.seen() || !(fades || gliding) {
                this.ticking.take();
                return glib::ControlFlow::Break;
            }
            let idle = (clock.frame_time() - this.active.get()).max(0) as f64 / 1000.0;
            if gliding || (strength(idle, fades) - this.shown.get()).abs() >= STEP {
                view.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        *self.ticking.borrow_mut() = Some(id);
    }

    fn seen(&self) -> bool {
        self.on.get() && self.focused.get()
    }

    /// Where the view puts the cursor: the selection's moving end.
    fn place(&self) -> Place {
        let (strong, _) = self.view.cursor_locations(None);
        [f64::from(strong.x()), f64::from(strong.y()), f64::from(strong.height().max(1))]
    }

    /// The caret over the text, in [color], in the buffer's coordinates.
    /// Not drawn with no focus.
    pub fn paint(&self, snapshot: &gtk::Snapshot, color: &gtk::gdk::RGBA) {
        if !self.seen() {
            self.glide.set(None);
            self.drawn.set(None);
            return;
        }
        let now = self.view.frame_clock().map_or_else(glib::monotonic_time, |clock| clock.frame_time());
        let (fades, moves) = motion();
        let target = self.place();
        // A new place after a key or a move is glided to; the lines moving
        // under the cursor (a new width, a mark shown) take the caret with them.
        if self.target.replace(Some(target)) != Some(target) {
            let from = self.drawn.get().filter(|from| self.moved.get() && glides(*from, target, f64::from(self.view.height()), moves));
            self.glide.set(from.map(|from| (from, now)));
        }
        self.moved.set(false);
        let place = match self.glide.get() {
            Some((from, since)) if ((now - since) as f64) < GLIDE_MS * 1000.0 => glided(from, target, (now - since).max(0) as f64 / 1000.0),
            _ => {
                self.glide.set(None);
                target
            }
        };
        self.drawn.set(Some(place));
        let strength = strength((now - self.active.get()).max(0) as f64 / 1000.0, fades);
        self.shown.set(strength);
        let bar = gtk::graphene::Rect::new(place[0] as f32, place[1] as f32, WIDTH, place[2] as f32);
        snapshot.append_color(&color.with_alpha(color.alpha() * strength as f32), &bar);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caret_is_solid_while_keys_come_and_fades_after() {
        assert_eq!(strength(0.0, true), 1.0);
        assert_eq!(strength(650.0, true), 1.0);
        // Half a fade later it is at its faintest, a quarter; a whole one later, full again.
        assert!((strength(650.0 + 600.0, true) - 0.25).abs() < 1e-9);
        assert!((strength(650.0 + 1200.0, true) - 1.0).abs() < 1e-9);
        assert!((strength(650.0 + 300.0, true) - 0.625).abs() < 1e-9);
        // GTK's cursor blink off, or motion reduced: solid for good.
        assert_eq!(strength(5000.0, false), 1.0);
    }

    #[test]
    fn the_caret_glides_in_seventy_ms() {
        let (from, to) = ([10.0, 100.0, 20.0], [110.0, 140.0, 30.0]);
        assert_eq!(glided(from, to, 0.0), from);
        assert_eq!(glided(from, to, 70.0), to);
        assert_eq!(glided(from, to, 500.0), to);
        // Half way in time it is three quarters of the way: it slows as it arrives.
        assert_eq!(glided(from, to, 35.0), [85.0, 130.0, 27.5]);
    }

    #[test]
    fn a_move_of_more_than_the_views_height_does_not_glide() {
        let from = [10.0, 100.0, 20.0];
        assert!(glides(from, [300.0, 500.0, 20.0], 600.0, true));
        assert!(!glides(from, [10.0, 900.0, 20.0], 600.0, true));
        assert!(!glides([10.0, 900.0, 20.0], from, 600.0, true));
        // Reduce Motion: no glide.
        assert!(!glides(from, [300.0, 500.0, 20.0], 600.0, false));
    }
}
