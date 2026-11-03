//! The day picker of a task's dates and a date property: a month of days in
//! the Gregorian calendar, or in the Jalali one when the Persian date is on
//! (`docs/specs/calendar-events.md`), with today, the picked day, the
//! weekend and the holidays and feasts of `calendar.place` marked. It
//! stands in for `gtk::Calendar`, which knows one calendar and no holidays.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

use daftar_core::jalali::Jalali;
use daftar_core::{Calendar, Day, Month};

pub struct DayPicker {
    pub widget: gtk::Box,
    head: gtk::Box,
    back: gtk::Button,
    on: gtk::Button,
    title: gtk::Label,
    grid: gtk::Grid,
    month: Cell<Month>,
    selected: Cell<Option<Day>>,
    picked: Box<dyn Fn(Day)>,
}

impl DayPicker {
    /// [picked] gets the day clicked.
    pub fn new(picked: impl Fn(Day) + 'static) -> Rc<DayPicker> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["daftar-picker"]).build();
        let head = gtk::Box::builder().spacing(4).build();
        let back = gtk::Button::builder().icon_name("go-previous-symbolic").css_classes(["flat"]).tooltip_text("Month before").build();
        let title = gtk::Label::builder().hexpand(true).css_classes(["heading"]).build();
        let today = gtk::Button::builder().label("Today").css_classes(["flat", "caption"]).build();
        let on = gtk::Button::builder().icon_name("go-next-symbolic").css_classes(["flat"]).tooltip_text("Month after").build();
        head.append(&back);
        head.append(&title);
        head.append(&today);
        head.append(&on);
        widget.append(&head);
        let grid = gtk::Grid::builder().row_spacing(2).column_spacing(2).column_homogeneous(true).build();
        widget.append(&grid);
        let picker = Rc::new(DayPicker {
            widget,
            head,
            back: back.clone(),
            on: on.clone(),
            title,
            grid,
            month: Cell::new(Month::of(crate::settings::calendar(), Day::today())),
            selected: Cell::new(None),
            picked: Box::new(picked),
        });
        for (button, step) in [(&back, -1), (&on, 1)] {
            let picker = Rc::downgrade(&picker);
            button.connect_clicked(move |_| {
                if let Some(picker) = picker.upgrade() {
                    picker.month.set(picker.month.get().adding(step));
                    picker.draw();
                }
            });
        }
        {
            let picker = Rc::downgrade(&picker);
            today.connect_clicked(move |_| {
                if let Some(picker) = picker.upgrade() {
                    picker.month.set(Month::of(picker.month.get().calendar, Day::today()));
                    picker.draw();
                }
            });
        }
        picker.draw();
        picker
    }

    /// Marks [day] as the one picked and shows its month, or today's, in the
    /// calendar the settings want now.
    pub fn select(self: &Rc<Self>, day: Option<Day>) {
        self.selected.set(day);
        self.month.set(Month::of(crate::settings::calendar(), day.unwrap_or_else(Day::today)));
        self.draw();
    }

    fn draw(self: &Rc<Self>) {
        let month = self.month.get();
        let jalali = month.calendar == Calendar::Jalali;
        // A Jalali month reads from the right, its weeks starting on Saturday.
        // A child takes the default direction, not its parent's, so each is set.
        let direction = if jalali { gtk::TextDirection::Rtl } else { gtk::TextDirection::Ltr };
        for widget in [self.widget.upcast_ref::<gtk::Widget>(), self.head.upcast_ref(), self.grid.upcast_ref()] {
            widget.set_direction(direction);
        }
        self.back.set_icon_name(if jalali { "go-next-symbolic" } else { "go-previous-symbolic" });
        self.on.set_icon_name(if jalali { "go-previous-symbolic" } else { "go-next-symbolic" });
        self.title.set_label(&month.title());
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        for (column, name) in month.weekdays().iter().enumerate() {
            let label = gtk::Label::builder().label(*name).css_classes(["caption", "daftar-muted"]).build();
            self.grid.attach(&label, column as i32, 0, 1, 1);
        }
        let cells = month.cells();
        let (first, last) = (cells.iter().flatten().next().copied(), cells.iter().flatten().last().copied());
        let events = match (first, last) {
            (Some(first), Some(last)) => crate::settings::events(first, last),
            _ => Vec::new(),
        };
        let today = Day::today();
        for (index, cell) in cells.iter().enumerate() {
            let Some(day) = *cell else { continue };
            let on: Vec<_> = events.iter().filter(|h| h.day == day).map(|h| h.event).collect();
            let face = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            face.append(&gtk::Label::new(Some(&month.number(day))));
            if jalali {
                face.append(&gtk::Label::builder().label(day.day().to_string()).css_classes(["daftar-picker-other"]).build());
            }
            let dot =
                gtk::Label::builder().label("•").css_classes(["daftar-picker-dot"]).opacity(if on.is_empty() { 0.0 } else { 1.0 }).build();
            face.append(&dot);
            let button = gtk::Button::builder().child(&face).css_classes(["flat", "daftar-picker-day"]).build();
            let weekend = if jalali { index % 7 == 6 } else { index % 7 >= 5 };
            if weekend {
                button.add_css_class("weekend");
            }
            if on.iter().any(|e| e.holiday) {
                button.add_css_class("holiday");
            } else if !on.is_empty() {
                button.add_css_class("feast");
            }
            if day == today {
                button.add_css_class("today");
            }
            if Some(day) == self.selected.get() {
                button.add_css_class("selected");
            }
            let mut tip = vec![day.date().format("%A %-d %B %Y").to_string()];
            if jalali {
                tip.push(Jalali::long(day));
            }
            for event in &on {
                let kind = if event.holiday { " · holiday" } else { "" };
                tip.push(if event.local == event.title {
                    format!("{}{kind}", event.title)
                } else {
                    format!("{} ({}){kind}", event.title, event.local)
                });
            }
            button.set_tooltip_text(Some(&tip.join("\n")));
            let picker = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(picker) = picker.upgrade() {
                    picker.selected.set(Some(day));
                    (picker.picked)(day);
                }
            });
            self.grid.attach(&button, (index % 7) as i32, (index / 7 + 1) as i32, 1, 1);
        }
    }
}
