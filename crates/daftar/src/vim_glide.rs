//! The editor in Neovim mode moving as Neovide moves: a cursor that glides
//! to where Neovim puts it, a block in Normal mode and a bar while
//! inserting, and the page scrolling smoothly to where Neovim's window
//! starts. A turn of the wheel is told back to Neovim, so its window follows
//! the page instead of pulling it back at the next key.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

/// How fast the cursor and the page catch up: the time to cover most of
/// the way, as Neovide's animation lengths feel.
const CURSOR_PACE: f64 = 0.045;
const SCROLL_PACE: f64 = 0.07;
/// Room kept above and under the cursor when the page follows it.
const MARGIN: f64 = 48.0;
/// The wheel has stopped when it has been still this long.
const WHEEL_REST: Duration = Duration::from_millis(180);

/// The cursor's look, by Neovim's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Block,
    Bar,
    Underline,
    /// On the command line, which has its own caret.
    Hidden,
}

impl Shape {
    pub fn of(mode: &str) -> Shape {
        match mode.chars().next() {
            Some('i') | Some('t') => Shape::Bar,
            Some('c') => Shape::Hidden,
            Some('R') => Shape::Underline,
            _ => Shape::Block,
        }
    }
}

/// Hears the top line once the wheel stops.
type WheelListener = Box<dyn Fn(usize)>;

/// x, y, width and height, in the view's buffer coordinates.
type Frame = [f64; 4];

pub struct Glide {
    view: gtk::TextView,
    cursor: gtk::Box,
    on: Cell<bool>,
    shape: Cell<Shape>,
    /// Where the cursor is drawn now; `None` until it is first placed.
    drawn: Cell<Option<Frame>>,
    /// The page's top the scrolling goes to.
    scroll_to: Cell<Option<f64>>,
    ticking: RefCell<Option<gtk::TickCallbackId>>,
    last_frame: Cell<Option<i64>>,
    wheel_rest: RefCell<Option<glib::SourceId>>,
    on_wheel: RefCell<Option<WheelListener>>,
}

impl Glide {
    pub fn new(view: &impl IsA<gtk::TextView>) -> Rc<Glide> {
        let view = view.clone().upcast::<gtk::TextView>();
        let cursor = gtk::Box::builder().css_classes(["daftar-vim-cursor"]).can_target(false).visible(false).build();
        view.add_overlay(&cursor, 0, 0);
        let glide = Rc::new(Glide {
            view,
            cursor,
            on: Cell::new(false),
            shape: Cell::new(Shape::Block),
            drawn: Cell::new(None),
            scroll_to: Cell::new(None),
            ticking: RefCell::new(None),
            last_frame: Cell::new(None),
            wheel_rest: RefCell::new(None),
            on_wheel: RefCell::new(None),
        });
        glide.connect_wheel();
        // The text moved under the cursor: a change, a new width, a new font.
        let buffer = glide.view.buffer();
        let this = Rc::downgrade(&glide);
        buffer.connect_changed(move |_| {
            if let Some(this) = this.upgrade() {
                this.follow();
            }
        });
        if let Some(adjustment) = glide.view.vadjustment() {
            let this = Rc::downgrade(&glide);
            adjustment.connect_upper_notify(move |_| {
                if let Some(this) = this.upgrade() {
                    this.follow();
                }
            });
        }
        glide
    }

    /// Neovim mode began or ended: the view's own caret comes back without it.
    pub fn set_on(self: &Rc<Self>, on: bool) {
        self.on.set(on);
        self.view.set_cursor_visible(!on);
        self.cursor.set_visible(on && self.shape.get() != Shape::Hidden);
        self.drawn.set(None);
        self.scroll_to.set(None);
        if on {
            self.follow();
        } else if let Some(id) = self.ticking.take() {
            id.remove();
        }
    }

    /// [heard] gets the line (from 1) at the top of the page once the wheel stops.
    pub fn connect_wheel_stopped(&self, heard: impl Fn(usize) + 'static) {
        *self.on_wheel.borrow_mut() = Some(Box::new(heard));
    }

    pub fn set_shape(self: &Rc<Self>, shape: Shape) {
        if self.shape.replace(shape) != shape {
            self.cursor.remove_css_class("bar");
            if shape != Shape::Block {
                self.cursor.add_css_class("bar");
            }
            self.cursor.set_visible(self.on.get() && shape != Shape::Hidden);
        }
        self.follow();
    }

    /// The cursor moved: it glides there, and the page keeps it in sight.
    pub fn cursor_moved(self: &Rc<Self>) {
        if !self.on.get() {
            return;
        }
        let Some(adjustment) = self.view.vadjustment() else { return };
        let [_, y, _, height] = self.target();
        let top = self.scroll_to.get().unwrap_or(adjustment.value());
        let page = adjustment.page_size();
        let margin = MARGIN.min(page / 4.0);
        if y < top + margin {
            self.scroll(y - margin);
        } else if y + height > top + page - margin {
            self.scroll(y + height - page + margin);
        } else {
            self.follow();
        }
    }

    /// The page to start at the line at [offset] in the buffer, smoothly.
    pub fn scroll_to_line(self: &Rc<Self>, offset: usize) {
        let at = self.view.buffer().iter_at_offset(offset as i32);
        let (y, _) = self.view.line_yrange(&at);
        self.scroll(y as f64);
    }

    fn scroll(self: &Rc<Self>, top: f64) {
        let Some(adjustment) = self.view.vadjustment() else { return };
        let top = top.clamp(adjustment.lower(), (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()));
        self.scroll_to.set(Some(top));
        self.follow();
    }

    /// Where the cursor should be: the insert mark's character, as wide as
    /// it (or a space past the end of a line) for a block.
    fn target(&self) -> Frame {
        let buffer = self.view.buffer();
        let at = buffer.iter_at_mark(&buffer.get_insert());
        let place = self.view.iter_location(&at);
        let (mut x, y, height) = (place.x() as f64, place.y() as f64, place.height().max(1) as f64);
        // A right-to-left letter's box runs left from where it starts.
        if place.width() < 0 {
            x += place.width() as f64;
        }
        let wide = if place.width() != 0 && !at.ends_line() {
            place.width().abs() as f64
        } else {
            let metrics = self.view.pango_context().metrics(None, None);
            (metrics.approximate_char_width() as f64 / gtk::pango::SCALE as f64).max(4.0)
        };
        match self.shape.get() {
            Shape::Block => [x, y, wide, height],
            Shape::Bar | Shape::Hidden => [x, y, 2.0, height],
            Shape::Underline => [x, y + height - 3.0, wide, 3.0],
        }
    }

    /// Starts the frames that bring the cursor and the page where they go.
    fn follow(self: &Rc<Self>) {
        if !self.on.get() || self.ticking.borrow().is_some() {
            return;
        }
        self.last_frame.set(None);
        let this = Rc::downgrade(self);
        let id = self.view.add_tick_callback(move |_, clock| {
            let Some(this) = this.upgrade() else { return glib::ControlFlow::Break };
            if this.frame(clock.frame_time()) {
                return glib::ControlFlow::Continue;
            }
            this.ticking.take();
            glib::ControlFlow::Break
        });
        *self.ticking.borrow_mut() = Some(id);
    }

    /// One frame's step; false when both have arrived.
    fn frame(&self, now: i64) -> bool {
        if !self.on.get() {
            return false;
        }
        let seconds = self.last_frame.replace(Some(now)).map_or(0.0, |then| (now - then).max(0) as f64 / 1e6).min(0.1);
        let step = |pace: f64| if seconds == 0.0 { 0.0 } else { 1.0 - (-seconds / pace).exp() };
        let target = self.target();
        // The first placing is a jump: a glide from the top of the page is noise.
        let drawn = match self.drawn.get() {
            Some(drawn) => std::array::from_fn(|i| drawn[i] + (target[i] - drawn[i]) * step(CURSOR_PACE)),
            None => target,
        };
        let arrived = drawn.iter().zip(target).all(|(d, t)| (d - t).abs() < 0.5);
        let drawn = if arrived { target } else { drawn };
        self.drawn.set(Some(drawn));
        self.view.move_overlay(&self.cursor, drawn[0].round() as i32, drawn[1].round() as i32);
        self.cursor.set_size_request(drawn[2].round().max(1.0) as i32, drawn[3].round().max(1.0) as i32);
        let mut scrolling = false;
        if let (Some(top), Some(adjustment)) = (self.scroll_to.get(), self.view.vadjustment()) {
            let now = adjustment.value();
            if (top - now).abs() < 0.5 {
                adjustment.set_value(top);
                self.scroll_to.set(None);
            } else {
                adjustment.set_value(now + (top - now) * step(SCROLL_PACE));
                scrolling = true;
            }
        }
        !arrived || scrolling
    }

    /// A turn of the wheel stops any scrolling of ours, and once the wheel
    /// rests, the top line is told.
    fn connect_wheel(self: &Rc<Self>) {
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::KINETIC);
        let this = Rc::downgrade(self);
        let turned = move || {
            let Some(this) = this.upgrade() else { return };
            if !this.on.get() {
                return;
            }
            this.scroll_to.set(None);
            if let Some(id) = this.wheel_rest.take() {
                id.remove();
            }
            let later = Rc::downgrade(&this);
            *this.wheel_rest.borrow_mut() = Some(glib::timeout_add_local_once(WHEEL_REST, move || {
                let Some(this) = later.upgrade() else { return };
                this.wheel_rest.take();
                this.wheel_stopped();
            }));
        };
        let turned = Rc::new(turned);
        {
            let turned = turned.clone();
            wheel.connect_scroll(move |_, _, _| {
                turned();
                glib::Propagation::Proceed
            });
        }
        wheel.connect_decelerate(move |_, _, _| turned());
        self.view.add_controller(wheel);
    }

    fn wheel_stopped(&self) {
        let Some(adjustment) = self.view.vadjustment() else { return };
        let top = adjustment.value();
        let (at, line_top) = self.view.line_at_y(top as i32);
        // A line cut by the top of the page is not the first one shown.
        let row = at.line() as usize + 1 + usize::from((line_top as f64) < top - 1.0);
        if let Some(heard) = self.on_wheel.borrow().as_ref() {
            heard(row);
        }
    }
}
