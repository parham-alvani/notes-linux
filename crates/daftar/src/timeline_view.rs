//! Today's timeline (`docs/specs/today-timeline.md`): the hours down the
//! left, the meetings and the timed tasks in their lanes, the free time behind
//! them and a line at now. `daftar_core::Timeline` places everything; this
//! only turns its minutes into points, in a layout of its own.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;

use daftar_core::text::direction;
use daftar_core::{Timeline, TimelineEntry, TimelineItem};

/// Points a minute: an hour is 60.
const SCALE: f64 = 1.0;
/// The hours' column.
const GUTTER: i32 = 52;
/// Room above the first hour and below the last, for their labels.
const PAD: i32 = 8;
/// The space between two boxes side by side, and under one.
const GAP: i32 = 2;
/// A box this tall has room for its time on a line of its own.
const TWO_LINES: i32 = 40;
/// A free band this tall has room for its label.
const LABELLED_FREE: u32 = 24;

/// Where a child of the timeline goes.
#[derive(Clone, Copy)]
enum Place {
    /// A line across at a minute.
    Rule(u32),
    /// An hour's label, centred on its line.
    Hour(u32),
    /// A band across the lanes.
    Band(u32, u32),
    /// A box in a lane: start, end, lane, lanes.
    Item(u32, u32, usize, usize),
    /// The line at now, and its dot.
    Now(u32),
    Dot(u32),
}

type Open = Rc<dyn Fn(&str, usize)>;
type Toggle = Rc<dyn Fn(&str, usize, char)>;

/// What a press on a box does.
pub struct Actions {
    pub open: Open,
    pub toggle: Toggle,
}

/// The timeline as a widget, with the day's all-day events above it.
pub fn view(timeline: &Timeline, actions: &Actions) -> gtk::Box {
    let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    if !timeline.all_day.is_empty() {
        let chips = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(6).column_spacing(6).build();
        for meeting in &timeline.all_day {
            let chip = gtk::Box::builder().spacing(6).halign(gtk::Align::Start).css_classes(["daftar-chip", "t-blue"]).build();
            chip.append(&gtk::Label::new(Some(&meeting.title)));
            chips.insert(&chip, -1);
        }
        outer.append(&chips);
    }

    let (top, bottom) = timeline.axis;
    let y = move |minute: u32| PAD + (f64::from(minute.saturating_sub(top)) * SCALE).round() as i32;
    let height = y(bottom) + PAD;
    let children: Rc<RefCell<Vec<(gtk::Widget, Place)>>> = Rc::default();
    let layout = {
        let allocated = children.clone();
        Layout::new((GUTTER + 160, height), move |width| {
            let lanes_width = (width - GUTTER).max(1);
            for (child, place) in allocated.borrow().iter() {
                // GTK wants every child measured before it is placed.
                child.measure(gtk::Orientation::Horizontal, -1);
                let natural_height = |w: i32| child.measure(gtk::Orientation::Vertical, w).1;
                let (x, top, w, h) = match *place {
                    Place::Rule(minute) => (GUTTER, y(minute), lanes_width, 1),
                    Place::Hour(minute) => {
                        let h = natural_height(GUTTER - 8);
                        (0, (y(minute) - h / 2).max(0), GUTTER - 8, h)
                    }
                    Place::Band(start, end) => (GUTTER, y(start), lanes_width, (y(end) - y(start)).max(1)),
                    Place::Item(start, end, lane, lanes) => {
                        let column = lanes_width / lanes.max(1) as i32;
                        let w = (column - GAP).max(1);
                        let h = (y(end) - y(start) - GAP).max(child.measure(gtk::Orientation::Vertical, w).0);
                        (GUTTER + column * lane as i32, y(start), w, h)
                    }
                    Place::Now(minute) => (GUTTER, y(minute) - 1, lanes_width, 2),
                    Place::Dot(minute) => (GUTTER - 4, y(minute) - 4, 8, 8),
                };
                child.size_allocate(&gtk::Allocation::new(x, top, w, h), -1);
            }
        })
    };
    let area = gtk::Box::builder().overflow(gtk::Overflow::Hidden).css_classes(["daftar-timeline"]).build();
    area.set_layout_manager(Some(layout));
    let add = |widget: gtk::Widget, place: Place| {
        widget.set_parent(&area);
        children.borrow_mut().push((widget, place));
    };

    for (start, end) in &timeline.free {
        let band = gtk::Box::builder().css_classes(["daftar-free"]).build();
        if end - start >= LABELLED_FREE {
            band.append(
                &gtk::Label::builder()
                    .label(format!("free {}", Timeline::duration(end - start)))
                    .hexpand(true)
                    .halign(gtk::Align::End)
                    .valign(gtk::Align::Start)
                    .css_classes(["caption", "daftar-free-label"])
                    .build(),
            );
        }
        add(band.upcast(), Place::Band(*start, *end));
    }
    for hour in (top..=bottom).step_by(60) {
        add(gtk::Box::builder().css_classes(["daftar-hour-rule"]).build().upcast(), Place::Rule(hour));
        let label = gtk::Label::builder()
            .label(format!("{:02}:00", hour / 60 % 24))
            .xalign(1.0)
            .css_classes(["caption", "daftar-muted", "numeric"])
            .build();
        add(label.upcast(), Place::Hour(hour));
    }
    for item in &timeline.items {
        add(slot(item, actions), Place::Item(item.start, item.end, item.lane, item.lanes));
    }
    if let Some(now) = timeline.now.filter(|now| (top..=bottom).contains(now)) {
        add(gtk::Box::builder().css_classes(["daftar-now"]).build().upcast(), Place::Now(now));
        add(gtk::Box::builder().css_classes(["daftar-now", "dot"]).build().upcast(), Place::Dot(now));
    }
    outer.append(&area);
    outer
}

/// `09:30`, from minutes.
fn clock(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// One box: a meeting, or a task with its box to tick.
fn slot(item: &TimelineItem, actions: &Actions) -> gtk::Widget {
    let tall = f64::from(item.end - item.start) * SCALE >= f64::from(TWO_LINES);
    // A meeting says its own times, also on the day it is cut at midnight.
    let time = match &item.entry {
        TimelineEntry::Meeting(meeting) => crate::today::time_range(meeting),
        TimelineEntry::Task(..) => format!("{}–{}", clock(item.start), clock(item.end)),
    };
    let words = |title: &str| -> gtk::Box {
        let rtl = direction::starts_rtl(title);
        let lines = gtk::Box::builder()
            .orientation(if tall { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal })
            .spacing(if tall { 0 } else { 6 })
            .hexpand(true)
            .valign(gtk::Align::Start)
            .build();
        lines.append(&gtk::Label::builder().label(&time).xalign(0.0).css_classes(["caption", "numeric", "tint"]).build());
        lines.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(if rtl { 1.0 } else { 0.0 })
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        lines
    };
    let row = gtk::Box::builder().spacing(4).css_classes(["daftar-slot"]).build();
    if item.past {
        row.add_css_class("is-past");
    }
    if item.current {
        row.add_css_class("is-current");
    }
    match &item.entry {
        TimelineEntry::Meeting(meeting) => {
            row.add_css_class(if meeting.busy_only { "t-muted" } else { "t-blue" });
            if meeting.tentative {
                row.add_css_class("tentative");
            }
            row.append(&words(&meeting.title));
            let mut tip = vec![format!("{time} {}", meeting.title)];
            if !meeting.calendars.is_empty() {
                tip.push(meeting.calendars.join(", "));
            }
            if let Some(overlaps) = &meeting.overlaps {
                tip.push(format!("Overlaps {overlaps}"));
            }
            if meeting.tentative {
                tip.push("Maybe".into());
            }
            if let Some(join) = meeting.join.clone().filter(|url| url.starts_with("https://") || url.starts_with("http://")) {
                tip.push(format!("Join: {join}"));
                row.set_cursor_from_name(Some("pointer"));
                let click = gtk::GestureClick::new();
                click.connect_released(move |gesture, _, _, _| {
                    let window = gesture.widget().and_then(|w| w.root()).and_downcast::<gtk::Window>();
                    gtk::UriLauncher::new(&join).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
                });
                row.add_controller(click);
            }
            row.set_tooltip_text(Some(&tip.join("\n")));
        }
        TimelineEntry::Task(task, text) => {
            row.add_css_class("t-accent");
            let check = gtk::CheckButton::builder().valign(gtk::Align::Start).tooltip_text("Complete").build();
            check.set_inconsistent(daftar_core::TaskState::new(task.task.status) == daftar_core::TaskType::InProgress);
            {
                let (toggle, path, line, status) = (actions.toggle.clone(), task.path.clone(), task.task.line, task.task.status);
                check.connect_toggled(move |_| toggle(&path, line, status));
            }
            row.append(&check);
            row.append(&words(&crate::today::plain(text)));
            row.set_tooltip_text(Some(&format!("{}: {}", daftar_core::Vault::title(&task.path), crate::today::plain(text))));
            row.set_cursor_from_name(Some("pointer"));
            let click = gtk::GestureClick::new();
            let (open, path, line) = (actions.open.clone(), task.path.clone(), task.task.line);
            click.connect_released(move |gesture, _, x, y| {
                // The box ticks; the rest opens the note.
                if let Some(widget) = gesture.widget()
                    && let Some(picked) = widget.pick(x, y, gtk::PickFlags::DEFAULT)
                    && picked.ancestor(gtk::CheckButton::static_type()).is_some()
                {
                    return;
                }
                open(&path, line);
            });
            row.add_controller(click);
        }
    }
    row.upcast()
}

type Allocate = Box<dyn Fn(i32)>;

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// A fixed size, and a function that places the children in a width.
    #[derive(Default)]
    pub struct Layout {
        pub size: Cell<(i32, i32)>,
        pub allocate: RefCell<Option<super::Allocate>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Layout {
        const NAME: &'static str = "DaftarTimelineLayout";
        type Type = super::Layout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for Layout {}

    impl LayoutManagerImpl for Layout {
        fn request_mode(&self, _: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, _: &gtk::Widget, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.size.get();
            let size = if orientation == gtk::Orientation::Vertical { height } else { width };
            (size, size, -1, -1)
        }

        fn allocate(&self, _: &gtk::Widget, width: i32, _: i32, _: i32) {
            if let Some(allocate) = self.allocate.borrow().as_ref() {
                allocate(width);
            }
        }
    }
}

glib::wrapper! {
    /// The timeline's layout: gtk4-rs has no binding of `GtkCustomLayout`.
    pub struct Layout(ObjectSubclass<imp::Layout>) @extends gtk::LayoutManager;
}

impl Layout {
    fn new(size: (i32, i32), allocate: impl Fn(i32) + 'static) -> Layout {
        let layout: Layout = glib::Object::new();
        layout.imp().size.set(size);
        layout.imp().allocate.replace(Some(Box::new(allocate)));
        layout
    }
}
