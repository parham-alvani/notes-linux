//! A picture of the editor seen large, as the reader's viewer shows it: the
//! wheel, a pinch, + and − zoom around the pointer, a drag pans, a double
//! click goes between fitting the window and the actual size (0 and 1 too),
//! ← and → go to the note's other pictures, and Escape closes it. An SVG is
//! drawn again at each zoom, so it stays sharp; a GIF keeps playing.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::editor::media::{Loaded, play};

/// The largest an SVG is drawn, in pixels across.
const SHARPEST: f64 = 8192.0;

struct Viewer {
    files: Vec<PathBuf>,
    at: Cell<usize>,
    loaded: RefCell<Option<Loaded>>,
    /// The zoom, and the zoom that fits the window.
    scale: Cell<f64>,
    fit: Cell<f64>,
    picture: gtk::Picture,
    scroller: gtk::ScrolledWindow,
    title: adw::WindowTitle,
    label: gtk::Label,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    /// Where the pointer was, in the window and in the picture as a share of
    /// it, for the scroll to put that point back under it once laid out.
    anchor: Cell<Option<(f64, f64, f64, f64)>>,
    redraw: RefCell<Option<glib::SourceId>>,
}

/// Opens [files] large, at the one numbered [at].
pub fn show(parent: &impl IsA<gtk::Widget>, files: Vec<PathBuf>, at: usize) {
    if files.is_empty() {
        return;
    }
    let picture = gtk::Picture::builder().can_shrink(true).build();
    let scroller = gtk::ScrolledWindow::builder().child(&picture).hexpand(true).vexpand(true).build();
    let title = adw::WindowTitle::new("", "");
    let label = gtk::Label::builder().width_chars(5).css_classes(["dim-label", "numeric"]).build();
    let viewer = Rc::new(Viewer {
        files,
        at: Cell::new(at),
        loaded: RefCell::new(None),
        scale: Cell::new(1.0),
        fit: Cell::new(1.0),
        picture,
        scroller,
        title,
        label,
        tick: RefCell::new(None),
        anchor: Cell::new(None),
        redraw: RefCell::new(None),
    });

    let header = adw::HeaderBar::builder().title_widget(&viewer.title).build();
    let button = |icon: &str, tip: &str| gtk::Button::builder().icon_name(icon).tooltip_text(tip).build();
    let (previous, next) = (button("go-previous-symbolic", "Previous picture (←)"), button("go-next-symbolic", "Next picture (→)"));
    let (out, zoom_in) = (button("zoom-out-symbolic", "Zoom out (−)"), button("zoom-in-symbolic", "Zoom in (+)"));
    let (fit, actual) = (button("zoom-fit-best-symbolic", "Fit the window (0)"), button("zoom-original-symbolic", "Actual size (1)"));
    let open = button("document-open-symbolic", "Open in the default app");
    if viewer.files.len() > 1 {
        header.pack_start(&previous);
        header.pack_start(&next);
    }
    header.pack_end(&open);
    header.pack_end(&actual);
    header.pack_end(&fit);
    header.pack_end(&zoom_in);
    header.pack_end(&viewer.label);
    header.pack_end(&out);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&viewer.scroller));
    let dialog = adw::Dialog::builder().content_width(1000).content_height(720).child(&view).build();

    let v = viewer.clone();
    previous.connect_clicked(move |_| v.step(-1));
    let v = viewer.clone();
    next.connect_clicked(move |_| v.step(1));
    let v = viewer.clone();
    out.connect_clicked(move |_| v.zoom_to(v.scale.get() * 0.8, None));
    let v = viewer.clone();
    zoom_in.connect_clicked(move |_| v.zoom_to(v.scale.get() * 1.25, None));
    let v = viewer.clone();
    fit.connect_clicked(move |_| v.zoom_to(v.fit.get(), None));
    let v = viewer.clone();
    actual.connect_clicked(move |_| v.zoom_to(1.0, None));
    let v = viewer.clone();
    open.connect_clicked(move |_| {
        let file = gio::File::for_path(&v.files[v.at.get()]);
        let _ = gio::AppInfo::launch_default_for_uri(&file.uri(), gio::AppLaunchContext::NONE);
    });

    let keys = gtk::EventControllerKey::new();
    let (v, d) = (viewer.clone(), dialog.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        match key {
            gdk::Key::Escape => {
                d.close();
            }
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => v.zoom_to(v.scale.get() * 1.25, None),
            gdk::Key::minus | gdk::Key::KP_Subtract => v.zoom_to(v.scale.get() * 0.8, None),
            gdk::Key::_0 | gdk::Key::KP_0 => v.zoom_to(v.fit.get(), None),
            gdk::Key::_1 | gdk::Key::KP_1 => v.zoom_to(1.0, None),
            gdk::Key::Left | gdk::Key::Page_Up => v.step(-1),
            gdk::Key::Right | gdk::Key::Page_Down => v.step(1),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    dialog.add_controller(keys);

    // The wheel zooms, before the scroller scrolls with it.
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = viewer.clone();
    wheel.connect_scroll(move |wheel, _, dy| {
        let pointer = wheel.current_event().and_then(|e| e.position()).and_then(|(x, y)| {
            let root = v.scroller.root()?;
            let point = root.compute_point(&v.scroller, &gtk::graphene::Point::new(x as f32, y as f32))?;
            Some((f64::from(point.x()), f64::from(point.y())))
        });
        v.zoom_to(v.scale.get() * 1.15f64.powf(-dy), pointer);
        glib::Propagation::Stop
    });
    viewer.scroller.add_controller(wheel);

    let pinch = gtk::GestureZoom::new();
    let start = Rc::new(Cell::new(1.0));
    let (v, s) = (viewer.clone(), start.clone());
    pinch.connect_begin(move |_, _| s.set(v.scale.get()));
    let v = viewer.clone();
    pinch.connect_scale_changed(move |gesture, delta| {
        let centre = gesture.bounding_box_center();
        v.zoom_to(start.get() * delta, centre);
    });
    viewer.scroller.add_controller(pinch);

    let drag = gtk::GestureDrag::new();
    let from = Rc::new(Cell::new((0.0, 0.0)));
    let (v, f) = (viewer.clone(), from.clone());
    drag.connect_drag_begin(move |_, _, _| f.set((v.scroller.hadjustment().value(), v.scroller.vadjustment().value())));
    let v = viewer.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let (x, y) = from.get();
        v.scroller.hadjustment().set_value(x - dx);
        v.scroller.vadjustment().set_value(y - dy);
    });
    viewer.scroller.add_controller(drag);

    let double = gtk::GestureClick::new();
    let v = viewer.clone();
    double.connect_pressed(move |_, presses, x, y| {
        if presses == 2 {
            let target = if (v.scale.get() - 1.0).abs() < 0.01 && (v.fit.get() - 1.0).abs() > 0.01 { v.fit.get() } else { 1.0 };
            v.zoom_to(target, Some((x, y)));
        }
    });
    viewer.scroller.add_controller(double);

    // Once laid out at the new size, the point under the pointer is put back.
    for horizontal in [true, false] {
        let v = viewer.clone();
        let adjustment = if horizontal { viewer.scroller.hadjustment() } else { viewer.scroller.vadjustment() };
        adjustment.connect_changed(move |adjustment| {
            let Some((px, py, fx, fy)) = v.anchor.get() else { return };
            let (pointer, share) = if horizontal { (px, fx) } else { (py, fy) };
            adjustment.set_value(share * (adjustment.upper() - adjustment.lower()) - pointer);
        });
    }
    let v = viewer.clone();
    dialog.connect_closed(move |_| {
        if let Some(tick) = v.tick.borrow_mut().take() {
            tick.remove();
        }
    });

    dialog.present(Some(parent));
    // The window's size is known once it is shown; the fit follows it, and
    // a picture that fitted keeps fitting.
    let v = viewer.clone();
    glib::idle_add_local_once(move || v.load());
    let v = viewer;
    let last = Rc::new(Cell::new((0, 0)));
    v.scroller.clone().add_tick_callback(move |scroller, _| {
        let size = (scroller.width(), scroller.height());
        if size != last.replace(size) {
            v.refit();
        }
        glib::ControlFlow::Continue
    });
}

impl Viewer {
    fn step(self: &Rc<Self>, by: isize) {
        let count = self.files.len() as isize;
        if count < 2 {
            return;
        }
        self.at.set(((self.at.get() as isize + by).rem_euclid(count)) as usize);
        self.load();
    }

    /// Shows the picture the viewer is at, fitted.
    fn load(self: &Rc<Self>) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        let file = &self.files[self.at.get()];
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.title.set_title(&name);
        let count = if self.files.len() > 1 { format!("{} / {}", self.at.get() + 1, self.files.len()) } else { String::new() };
        let loaded = Loaded::load(file);
        if let Some(loaded) = &loaded {
            self.title.set_subtitle(
                &[format!("{} × {}", loaded.width, loaded.height), count]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
            self.picture.set_paintable(Some(&loaded.texture));
            if let Some(animation) = &loaded.animation {
                self.tick.replace(Some(play(&self.picture, animation)));
            }
        } else {
            self.title.set_subtitle("Not a picture Daftar can show");
            self.picture.set_paintable(gdk::Paintable::NONE);
        }
        self.loaded.replace(loaded);
        self.fit.set(0.0);
        self.refit();
    }

    fn vector(&self) -> bool {
        self.files[self.at.get()].extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    }

    /// The zoom that fits the window: a drawing fills it, a photo is never
    /// blown up past its pixels.
    fn refit(self: &Rc<Self>) {
        let Some((width, height)) = self.loaded.borrow().as_ref().map(|l| (f64::from(l.width), f64::from(l.height))) else { return };
        let (room_w, room_h) = (f64::from(self.scroller.width()), f64::from(self.scroller.height()));
        if width <= 0.0 || height <= 0.0 || room_w <= 1.0 {
            return;
        }
        let fits = ((room_w - 24.0) / width).min((room_h - 24.0) / height);
        let fit = if self.vector() { fits } else { fits.min(1.0) };
        let fitted = (self.scale.get() - self.fit.get()).abs() < 1e-6 || self.fit.get() == 0.0;
        self.fit.set(fit);
        if fitted {
            self.zoom_to(fit, None);
        }
    }

    /// Zooms to [scale], keeping the point under [pointer] (in the scroller)
    /// where it is; with none, the middle of the window.
    fn zoom_to(self: &Rc<Self>, scale: f64, pointer: Option<(f64, f64)>) {
        let Some((width, height)) = self.loaded.borrow().as_ref().map(|l| (f64::from(l.width), f64::from(l.height))) else { return };
        let scale = scale.clamp(self.fit.get(), 32f64.max(self.fit.get()));
        let (h, v) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        let (px, py) = pointer.unwrap_or_else(|| (h.page_size() / 2.0, v.page_size() / 2.0));
        let share = |adjustment: &gtk::Adjustment, at: f64| {
            let span = adjustment.upper() - adjustment.lower();
            if span > 0.0 { (adjustment.value() + at) / span } else { 0.5 }
        };
        self.anchor.set(Some((px, py, share(&h, px), share(&v, py))));
        self.scale.set(scale);
        // The size asked for is the zoom; the scroller gives it at least the window.
        self.picture.set_size_request((width * scale).round() as i32, (height * scale).round() as i32);
        // Below the fit, a photo smaller than the window is shown as it is.
        let fitted = scale <= self.fit.get() + 1e-6;
        self.picture.set_content_fit(if fitted && !self.vector() { gtk::ContentFit::ScaleDown } else { gtk::ContentFit::Contain });
        self.label.set_label(&format!("{}%", (scale * 100.0).round()));
        // The anchor is for this layout only.
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(120), move || this.anchor.set(None));
        self.sharpen();
    }

    /// An SVG drawn again at the zoom, once the zoom stops changing.
    fn sharpen(self: &Rc<Self>) {
        if !self.vector() {
            return;
        }
        if let Some(source) = self.redraw.borrow_mut().take() {
            source.remove();
        }
        let this = self.clone();
        let source = glib::timeout_add_local_once(Duration::from_millis(150), move || {
            this.redraw.replace(None);
            let Some((width, height)) = this.loaded.borrow().as_ref().map(|l| (f64::from(l.width), f64::from(l.height))) else { return };
            let factor = f64::from(this.picture.scale_factor());
            let scale = (this.scale.get() * factor).min(SHARPEST / width.max(height));
            let file = &this.files[this.at.get()];
            if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(file, (width * scale) as i32, (height * scale) as i32, true) {
                this.picture.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf)));
            }
        });
        self.redraw.replace(Some(source));
    }
}
