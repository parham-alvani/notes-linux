//! The Problems panel (`docs/specs/problems-and-format.md` §3): under the
//! note, what `just audit` and `just check` report, by note, with Fix where
//! a tool mends it. `VaultChecks` finds; this lists.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use daftar_core::{Problem, ProblemCounts, ProblemFilter, ProblemLevel, Vault};
use gtk::{gdk, glib};

use crate::vault_checks::{State, VaultChecks};

/// Opens a note at a line from 0, or where it was read with `None`.
type Open = Box<dyn Fn(&str, Option<usize>)>;

/// What the panel's buttons and rows do.
pub struct Actions {
    pub open: Open,
    /// The panel closed itself (its button or Escape).
    pub closed: Box<dyn Fn()>,
}

/// What a row of the list is.
enum Row {
    Note(String),
    Finding(Problem),
}

pub struct ProblemsView {
    pub widget: gtk::Box,
    checks: Rc<VaultChecks>,
    actions: Actions,
    counts: gtk::Label,
    filters: [(ProblemFilter, gtk::ToggleButton); 3],
    fix_all: gtk::Button,
    /// The vault is read-only: nothing is fixed, and the buttons are off.
    read_only: Cell<bool>,
    check: gtk::Button,
    spinner: gtk::Spinner,
    list: gtk::ListBox,
    empty: gtk::Label,
    stack: gtk::Stack,
    rows: RefCell<Vec<Row>>,
    filter: Cell<ProblemFilter>,
}

/// "Error" or "Warning", and its icon and color.
fn level(problem: &Problem) -> (&'static str, &'static str, &'static str) {
    match problem.level {
        ProblemLevel::Error => ("Error", "dialog-error-symbolic", "error"),
        ProblemLevel::Warning => ("Warning", "dialog-warning-symbolic", "warning"),
    }
}

/// "Line 12", or "Note" for the whole file.
fn place(problem: &Problem) -> String {
    if problem.line == 0 { "Note".into() } else { format!("Line {}", problem.line) }
}

impl ProblemsView {
    pub fn new(checks: Rc<VaultChecks>, actions: Actions) -> Rc<ProblemsView> {
        let title = gtk::Label::builder().label("Problems").css_classes(["heading"]).build();
        let counts = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();
        let filter_box = gtk::Box::builder().css_classes(["linked"]).build();
        let make = |filter: ProblemFilter| {
            let button = gtk::ToggleButton::builder().css_classes(["flat", "caption"]).build();
            filter_box.append(&button);
            (filter, button)
        };
        let filters = [make(ProblemFilter::Errors), make(ProblemFilter::Warnings), make(ProblemFilter::All)];
        filters[1].1.set_group(Some(&filters[0].1));
        filters[2].1.set_group(Some(&filters[0].1));
        filters[2].1.set_active(true);
        let fix_all =
            gtk::Button::builder().label("Fix All").tooltip_text("What just fix mends, in the whole vault").css_classes(["flat"]).build();
        let check = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Check Again").css_classes(["flat"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text("Close (Escape)").css_classes(["flat"]).build();
        let spinner = gtk::Spinner::builder().spinning(true).visible(false).build();
        let header = gtk::Box::builder().spacing(8).margin_start(12).margin_end(6).margin_top(4).margin_bottom(4).build();
        header.append(&title);
        header.append(&counts);
        header.append(&spinner);
        header.append(&gtk::Box::builder().hexpand(true).build());
        header.append(&filter_box);
        header.append(&fix_all);
        header.append(&check);
        header.append(&close);
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
        let empty = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["dim-label"]).margin_top(24).build();
        let stack = gtk::Stack::new();
        stack.add_named(
            &gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build(),
            Some("list"),
        );
        stack.add_named(&empty, Some("empty"));
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .height_request(220)
            .visible(false)
            .css_classes(["daftar-page"])
            .build();
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&header);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&stack);
        let view = Rc::new(ProblemsView {
            widget,
            checks,
            actions,
            counts,
            filters,
            fix_all,
            read_only: Cell::new(false),
            check,
            spinner,
            list,
            empty,
            stack,
            rows: RefCell::default(),
            filter: Cell::new(ProblemFilter::All),
        });
        for (filter, button) in &view.filters {
            let (this, filter) = (Rc::downgrade(&view), *filter);
            button.connect_toggled(move |button| {
                if let Some(this) = this.upgrade().filter(|_| button.is_active()) {
                    this.filter.set(filter);
                    this.update();
                }
            });
        }
        {
            let this = Rc::downgrade(&view);
            view.fix_all.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.checks.fix_all(None);
                }
            });
        }
        {
            let this = Rc::downgrade(&view);
            view.check.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.checks.refresh();
                }
            });
        }
        {
            let this = Rc::downgrade(&view);
            close.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.close();
                }
            });
        }
        {
            let this = Rc::downgrade(&view);
            view.list.connect_row_activated(move |_, row| {
                let Some(this) = this.upgrade() else { return };
                let target = match this.rows.borrow().get(usize::try_from(row.index()).unwrap_or(usize::MAX)) {
                    Some(Row::Note(path)) => Some((path.clone(), None)),
                    Some(Row::Finding(problem)) => Some((problem.path.clone(), problem.source_line())),
                    None => None,
                };
                if let Some((path, line)) = target {
                    (this.actions.open)(&path, line);
                }
            });
        }
        let keys = gtk::EventControllerKey::new();
        {
            let this = Rc::downgrade(&view);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape
                    && let Some(this) = this.upgrade()
                {
                    this.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        view.widget.add_controller(keys);
        view
    }

    pub fn is_open(&self) -> bool {
        self.widget.is_visible()
    }

    /// Shown, checked at once, and with the keys.
    pub fn open(self: &Rc<Self>) {
        self.widget.set_visible(true);
        self.checks.refresh();
        self.update();
        self.list.grab_focus();
    }

    pub fn close(&self) {
        self.widget.set_visible(false);
        (self.actions.closed)();
    }

    /// A read-only vault is not fixed: Fix and Fix All are off.
    pub fn set_read_only(self: &Rc<Self>, on: bool) {
        if self.read_only.replace(on) != on {
            self.update();
        }
    }

    /// Drawn again from the checks, while it is open.
    pub fn update(self: &Rc<Self>) {
        if !self.is_open() {
            return;
        }
        let counts = self.checks.counts();
        let (state, running, fixing) = (self.checks.state(), self.checks.running(), self.checks.fixing());
        // A fix under way and a read-only vault both turn the Fix buttons off.
        let no_fix = fixing || self.read_only.get();
        self.counts.set_label(if state == State::Done || counts.total() > 0 { counts.spoken() } else { String::new() }.as_str());
        self.spinner.set_visible(running || fixing);
        self.check.set_sensitive(!running);
        let labels = [format!("Errors {}", counts.errors), format!("Warnings {}", counts.warnings), "All".to_string()];
        for ((_, button), label) in self.filters.iter().zip(labels) {
            button.set_label(&label);
        }
        let notes = self.checks.grouped(self.filter.get());
        self.fix_all.set_sensitive(!no_fix && notes.iter().any(daftar_core::ProblemNote::fixable));
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut rows = Vec::new();
        for note in &notes {
            self.list.append(&self.note_row(&note.path, note.counts, note.fixable(), no_fix));
            rows.push(Row::Note(note.path.clone()));
            for problem in &note.problems {
                self.list.append(&self.finding_row(problem, no_fix));
                rows.push(Row::Finding(problem.clone()));
            }
        }
        *self.rows.borrow_mut() = rows;
        if notes.is_empty() {
            self.empty.set_label(&self.why_empty(&state, running, counts));
            self.stack.set_visible_child_name("empty");
        } else {
            self.stack.set_visible_child_name("list");
        }
    }

    /// With nothing to list, why.
    fn why_empty(&self, state: &State, running: bool, counts: ProblemCounts) -> String {
        if let Some(why) = VaultChecks::explanation(state) {
            return why;
        }
        if running || *state == State::Idle {
            return "Checking…".into();
        }
        match self.filter.get() {
            ProblemFilter::Errors if counts.total() > 0 => "No errors".into(),
            ProblemFilter::Warnings if counts.total() > 0 => "No warnings".into(),
            _ => "No problems".into(),
        }
    }

    fn note_row(self: &Rc<Self>, path: &str, counts: ProblemCounts, fixable: bool, fixing: bool) -> gtk::ListBoxRow {
        let line = gtk::Box::builder().spacing(8).margin_top(6).build();
        line.append(
            &gtk::Label::builder().label(Vault::title(path)).css_classes(["heading"]).ellipsize(gtk::pango::EllipsizeMode::End).build(),
        );
        let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
        line.append(
            &gtk::Label::builder().label(folder).css_classes(["dim-label", "caption"]).ellipsize(gtk::pango::EllipsizeMode::Middle).build(),
        );
        line.append(
            &gtk::Label::builder()
                .label(counts.spoken())
                .hexpand(true)
                .xalign(0.0)
                .css_classes(["dim-label", "caption", "numeric"])
                .build(),
        );
        if fixable {
            let fix = gtk::Button::builder().label("Fix All").sensitive(!fixing).css_classes(["flat", "caption"]).build();
            fix.update_property(&[gtk::accessible::Property::Label(&format!("Fix all in {}", Vault::title(path)))]);
            let (this, path) = (Rc::downgrade(self), path.to_string());
            fix.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.checks.fix_all(Some(&path));
                }
            });
            line.append(&fix);
        }
        let row = gtk::ListBoxRow::builder().child(&line).build();
        row.update_property(&[gtk::accessible::Property::Label(&format!("{}, {}", Vault::title(path), counts.spoken()))]);
        row
    }

    fn finding_row(self: &Rc<Self>, problem: &Problem, fixing: bool) -> gtk::ListBoxRow {
        let (word, icon, color) = level(problem);
        let line = gtk::Box::builder().spacing(8).margin_start(12).build();
        line.append(&gtk::Image::builder().icon_name(icon).css_classes([color]).tooltip_text(word).build());
        line.append(&gtk::Label::builder().label(place(problem)).width_chars(8).xalign(0.0).css_classes(["dim-label", "numeric"]).build());
        line.append(
            &gtk::Label::builder()
                .label(&problem.message)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .tooltip_text(&problem.message)
                .build(),
        );
        line.append(&gtk::Label::builder().label(&problem.code).css_classes(["dim-label", "caption", "monospace"]).build());
        if problem.fixable {
            let fix = gtk::Button::builder().label("Fix").sensitive(!fixing).css_classes(["flat", "caption"]).build();
            let title = Vault::title(&problem.path);
            fix.update_property(&[gtk::accessible::Property::Label(&format!("Fix {} in {title}", problem.code))]);
            let (this, problem) = (Rc::downgrade(self), problem.clone());
            fix.connect_clicked(move |_| {
                if let Some(this) = this.upgrade() {
                    this.checks.fix(&problem);
                }
            });
            line.append(&fix);
        }
        let row = gtk::ListBoxRow::builder().child(&line).build();
        let spoken = format!("{word}, {}: {}. {}", place(problem).to_lowercase(), problem.message, problem.code);
        row.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        row
    }
}

/// The toolbar's count: errors in red, else warnings in yellow; hidden
/// with none. Its spoken value is the counts in words.
pub fn show_count(button: &gtk::Button, counts: ProblemCounts) {
    button.set_visible(counts.total() > 0);
    let (number, error) = if counts.errors > 0 { (counts.errors, true) } else { (counts.warnings, false) };
    button.set_label(&number.to_string());
    button.set_css_classes(&["flat", "numeric", if error { "error" } else { "warning" }]);
    let spoken = counts.spoken();
    button.set_tooltip_text(Some(&format!("{spoken} (Ctrl+Shift+M)")));
    button.update_property(&[gtk::accessible::Property::Label(&spoken)]);
}
