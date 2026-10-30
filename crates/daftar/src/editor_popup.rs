//! The list the editor shows at the cursor: notes for `[[`, snippets for
//! `/`. The text keeps the keyboard; the editor sends it Up, Down, Return,
//! Tab and Escape while it is open, and the query as it is typed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

/// One row: a title (Pango markup), a line under it, and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub title: String,
    pub subtitle: String,
    pub value: String,
}

/// What runs with the row picked.
type Chosen = Box<dyn Fn(&Choice)>;

pub struct Popup {
    popover: gtk::Popover,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    choices: RefCell<Vec<Choice>>,
    chosen: RefCell<Option<Chosen>>,
    open: Cell<bool>,
}

impl Popup {
    pub fn new(parent: &impl IsA<gtk::Widget>) -> Rc<Popup> {
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Browse).css_classes(["navigation-sidebar"]).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(320)
            .width_request(360)
            .child(&list)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroller)
            .autohide(false)
            .has_arrow(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .build();
        popover.set_parent(parent);
        let popup = Rc::new(Popup {
            popover,
            list,
            scroller,
            choices: RefCell::new(Vec::new()),
            chosen: RefCell::new(None),
            open: Cell::new(false),
        });
        let this = Rc::downgrade(&popup);
        popup.list.connect_row_activated(move |_, row| {
            if let Some(this) = this.upgrade() {
                this.pick(row.index());
            }
        });
        popup
    }

    /// Shows [choices] at [at] (the cursor's rectangle in the parent), under
    /// it when [below], else over it; [chosen] runs with the one picked. No
    /// choices: it closes.
    pub fn show(&self, at: gtk::gdk::Rectangle, below: bool, choices: Vec<Choice>, chosen: impl Fn(&Choice) + 'static) {
        *self.chosen.borrow_mut() = Some(Box::new(chosen));
        self.update(choices);
        if self.choices.borrow().is_empty() {
            return;
        }
        self.popover.set_position(if below { gtk::PositionType::Bottom } else { gtk::PositionType::Top });
        self.popover.set_pointing_to(Some(&at));
        if !self.open.replace(true) {
            self.popover.popup();
        }
    }

    /// New choices for what is typed now; the first is selected.
    pub fn update(&self, choices: Vec<Choice>) {
        self.list.remove_all();
        for choice in &choices {
            let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(3).margin_bottom(3).build();
            lines.append(
                &gtk::Label::builder().use_markup(true).label(&choice.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build(),
            );
            if !choice.subtitle.is_empty() {
                lines.append(
                    &gtk::Label::builder()
                        .label(&choice.subtitle)
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .css_classes(["dim-label", "caption"])
                        .build(),
                );
            }
            self.list.append(&lines);
        }
        self.list.select_row(self.list.row_at_index(0).as_ref());
        let empty = choices.is_empty();
        *self.choices.borrow_mut() = choices;
        if empty {
            self.close();
        }
    }

    /// The row at [at] selected and in sight, or none, as Neovim's menu says.
    pub fn select(&self, at: Option<usize>) {
        match at.and_then(|at| self.list.row_at_index(at as i32)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                self.reveal(&row);
            }
            None => self.list.unselect_all(),
        }
    }

    pub fn close(&self) {
        if self.open.replace(false) {
            self.popover.popdown();
        }
    }

    /// A key while the list is open: true when it was the list's.
    pub fn key(&self, key: gtk::gdk::Key) -> bool {
        if !self.open.get() {
            return false;
        }
        match key {
            gtk::gdk::Key::Up | gtk::gdk::Key::Down => {
                let step = if key == gtk::gdk::Key::Down { 1 } else { -1 };
                let at = self.list.selected_row().map_or(0, |row| row.index()) + step;
                let count = self.choices.borrow().len() as i32;
                if let Some(row) = self.list.row_at_index(at.rem_euclid(count.max(1))) {
                    self.list.select_row(Some(&row));
                    self.reveal(&row);
                }
                true
            }
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Tab => {
                let at = self.list.selected_row().map_or(0, |row| row.index());
                self.pick(at);
                true
            }
            gtk::gdk::Key::Escape => {
                self.close();
                true
            }
            _ => false,
        }
    }

    fn reveal(&self, row: &gtk::ListBoxRow) {
        let Some(bounds) = row.compute_bounds(&self.list) else { return };
        let adjustment = self.scroller.vadjustment();
        let (top, bottom) = (f64::from(bounds.y()), f64::from(bounds.y() + bounds.height()));
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    fn pick(&self, at: i32) {
        let Some(choice) = self.choices.borrow().get(at as usize).cloned() else { return };
        self.close();
        // After the key that picked it is done with, so the edit is its own.
        let chosen = self.chosen.borrow_mut().take();
        if let Some(chosen) = chosen {
            glib::idle_add_local_once(move || chosen(&choice));
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}
