//! Ctrl+2: the vault's tasks on a board, as on the Mac. Columns by status,
//! by day or by file; a card dragged to a status or a day column changes its
//! line in the note as the Tasks plugin would write it, and only while the
//! line still holds the task the card showed. A click opens the task's note
//! at its line; a right click edits it or moves it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use daftar_core::text::{direction, slugs};
use daftar_core::{BoardDrop, Column, Day, Grouping, TaskBoard, Vault, VaultTask};

use crate::store::VaultStore;
use crate::today::{plain, priority};

/// How many days back the Done and Cancelled columns reach, unless all closed
/// tasks are asked for.
const CLOSED_WITHIN: i64 = 14;

/// Opens or edits the task at a line of a note.
pub type AtLine = Rc<dyn Fn(&str, usize)>;
/// Ticks the task at a line of a note, which had this status.
pub type Toggle = Rc<dyn Fn(&str, usize, char)>;
/// Writes a task dropped on a column.
pub type Move = Rc<dyn Fn(&VaultTask, &BoardDrop)>;

/// What the window does for the board.
pub struct Handlers {
    pub open: AtLine,
    pub toggle: Toggle,
    pub edit: AtLine,
    pub moved: Move,
}

pub struct Board {
    pub widget: gtk::Box,
    count: gtk::Label,
    columns: gtk::Box,
    empty: adw::StatusPage,
    stack: gtk::Stack,
    files: gtk::DropDown,
    all_closed: gtk::ToggleButton,
    grouping: Cell<Grouping>,
    /// The notes in the file filter, after "All Notes".
    file_paths: RefCell<Vec<String>>,
    query: RefCell<String>,
    store: RefCell<Option<Rc<VaultStore>>>,
    handlers: Handlers,
}

impl Board {
    pub fn new(handlers: Handlers) -> Rc<Board> {
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["daftar-page"]).build();
        let head = gtk::Box::builder().spacing(10).css_classes(["daftar-board-head"]).build();
        head.append(&gtk::Image::builder().icon_name("view-dual-symbolic").css_classes(["daftar-board-icon"]).build());
        let titles = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
        titles.append(&gtk::Label::builder().label("Board").xalign(0.0).css_classes(["heading"]).build());
        let count = gtk::Label::builder().xalign(0.0).css_classes(["caption", "dim-label"]).build();
        titles.append(&count);
        head.append(&titles);
        head.append(&gtk::Box::builder().hexpand(true).build());

        let groupings = gtk::Box::builder().css_classes(["linked"]).build();
        let mut first: Option<gtk::ToggleButton> = None;
        let mut buttons = Vec::new();
        for (grouping, label) in [(Grouping::Status, "Status"), (Grouping::Day, "Day"), (Grouping::File, "File")] {
            let button = gtk::ToggleButton::builder().label(label).active(grouping == Grouping::Status).build();
            button.set_group(first.as_ref());
            first.get_or_insert(button.clone());
            groupings.append(&button);
            buttons.push((grouping, button));
        }
        head.append(&groupings);
        let files = gtk::DropDown::from_strings(&["All Notes"]);
        files.set_tooltip_text(Some("Only the tasks of one note"));
        head.append(&files);
        let all_closed = gtk::ToggleButton::builder()
            .icon_name("folder-documents-symbolic")
            .tooltip_text("Every done and cancelled task, not only the last two weeks'")
            .build();
        head.append(&all_closed);
        let filter = gtk::SearchEntry::builder().placeholder_text("Filter").width_chars(16).build();
        head.append(&filter);
        widget.append(&head);

        let columns = gtk::Box::builder().spacing(14).margin_start(16).margin_end(16).margin_bottom(16).vexpand(true).build();
        let scroller = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&columns)
            .css_classes(["daftar-page"])
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("checkbox-checked-symbolic")
            .title("No tasks")
            .description("A task is a line like <tt>- [ ] Call Sara 📅 2026-11-02</tt> in any note.")
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scroller, Some("columns"));
        stack.add_named(&empty, Some("empty"));
        widget.append(&stack);

        let board = Rc::new(Board {
            widget,
            count,
            columns,
            empty,
            stack,
            files,
            all_closed,
            grouping: Cell::new(Grouping::Status),
            file_paths: RefCell::new(Vec::new()),
            query: RefCell::new(String::new()),
            store: RefCell::new(None),
            handlers,
        });
        for (grouping, button) in buttons {
            let this = Rc::downgrade(&board);
            button.connect_toggled(move |button| {
                if let Some(this) = this.upgrade()
                    && button.is_active()
                {
                    this.grouping.set(grouping);
                    this.all_closed.set_visible(grouping == Grouping::Status);
                    this.draw();
                }
            });
        }
        {
            let this = Rc::downgrade(&board);
            board.files.connect_selected_notify(move |_| {
                if let Some(this) = this.upgrade() {
                    this.draw();
                }
            });
        }
        {
            let this = Rc::downgrade(&board);
            board.all_closed.connect_toggled(move |_| {
                if let Some(this) = this.upgrade() {
                    this.draw();
                }
            });
        }
        {
            let this = Rc::downgrade(&board);
            filter.connect_search_changed(move |entry| {
                if let Some(this) = this.upgrade() {
                    *this.query.borrow_mut() = entry.text().to_string();
                    this.draw();
                }
            });
        }
        board
    }

    /// Draws the board again from [store]'s index, keeping the filters.
    pub fn show(&self, store: Rc<VaultStore>) {
        *self.store.borrow_mut() = Some(store.clone());
        // The notes that hold tasks, by title, for the file filter.
        let mut paths: Vec<String> = store.index().tasks().into_iter().map(|t| t.path).collect();
        paths.sort_by(|a, b| daftar_core::text::standard_compare(&Vault::title(a), &Vault::title(b)).then(a.cmp(b)));
        paths.dedup();
        if *self.file_paths.borrow() != paths {
            let chosen = self.chosen_file();
            let mut names = vec!["All Notes".to_string()];
            names.extend(paths.iter().map(|p| Vault::title(p)));
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let at = chosen.and_then(|c| paths.iter().position(|p| *p == c)).map_or(0, |i| i as u32 + 1);
            *self.file_paths.borrow_mut() = paths;
            self.files.set_model(Some(&gtk::StringList::new(&names)));
            self.files.set_selected(at);
        }
        self.draw();
    }

    fn chosen_file(&self) -> Option<String> {
        let at = self.files.selected();
        if at == 0 || at == gtk::INVALID_LIST_POSITION {
            return None;
        }
        self.file_paths.borrow().get(at as usize - 1).cloned()
    }

    fn draw(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tasks = shown(store.index().tasks(), self.chosen_file().as_deref(), &self.query.borrow());
        let open = tasks.iter().filter(|t| t.task.is_open()).count();
        self.count.set_label(&format!("{open} open {}", if open == 1 { "task" } else { "tasks" }));
        let within = (!self.all_closed.is_active()).then_some(CLOSED_WITHIN);
        let columns = TaskBoard::columns(&tasks, self.grouping.get(), Day::today(), within);
        while let Some(child) = self.columns.first_child() {
            self.columns.remove(&child);
        }
        if columns.iter().all(|c| c.tasks.is_empty()) {
            let query = self.query.borrow();
            self.empty.set_title(&if query.trim().is_empty() {
                "No tasks".to_string()
            } else {
                format!("No task matches “{}”", query.trim())
            });
            self.stack.set_visible_child_name("empty");
            return;
        }
        self.stack.set_visible_child_name("columns");
        let targets: Vec<(String, BoardDrop)> =
            TaskBoard::columns(&[], self.grouping.get(), Day::today(), None).into_iter().filter_map(|c| Some((c.title, c.drop?))).collect();
        for column in &columns {
            self.columns.append(&self.column(column, &tasks, &targets));
        }
    }

    fn column(&self, column: &Column, tasks: &[VaultTask], targets: &[(String, BoardDrop)]) -> gtk::Box {
        let tint = tint(column);
        let outer = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .width_request(300)
            .css_classes(["daftar-column", &tint])
            .build();
        let head = gtk::Box::builder().spacing(8).css_classes(["column-head"]).build();
        head.append(&gtk::Box::builder().valign(gtk::Align::Center).css_classes(["column-dot"]).build());
        head.append(
            &gtk::Label::builder()
                .label(&column.title)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build(),
        );
        head.append(&gtk::Label::builder().label(column.tasks.len().to_string()).css_classes(["daftar-count", &tint, "numeric"]).build());
        outer.append(&head);
        let cards = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        for task in &column.tasks {
            cards.append(&self.card(task, &tint, targets));
        }
        outer.append(&gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&cards).build());

        // A card dropped here: its id is `path:line`, found again in [tasks].
        if let Some(drop) = column.drop.clone() {
            let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
            let tasks = tasks.to_vec();
            let moved = self.moved();
            target.connect_drop(move |_, value, _, _| {
                let Ok(id) = value.get::<String>() else { return false };
                let Some(task) = tasks.iter().find(|t| t.id() == id) else { return false };
                moved(task, &drop);
                true
            });
            let lit = outer.clone();
            target.connect_enter(move |_, _, _| {
                lit.add_css_class("drop-target");
                gdk::DragAction::MOVE
            });
            let lit = outer.clone();
            target.connect_leave(move |_| lit.remove_css_class("drop-target"));
            outer.add_controller(target);
        }
        outer
    }

    fn card(&self, task: &VaultTask, tint: &str, targets: &[(String, BoardDrop)]) -> gtk::Box {
        let t = &task.task;
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["board-card"])
            .tooltip_text(&task.path)
            .build();
        if !t.is_open() {
            card.add_css_class("is-closed");
        }
        let top = gtk::Box::builder().spacing(8).build();
        let check = gtk::CheckButton::builder().valign(gtk::Align::Start).active(!t.is_open()).build();
        check.set_inconsistent(t.status == '/');
        check.set_tooltip_text(Some(if t.is_open() { "Complete" } else { "Open again" }));
        {
            let toggle = self.toggle();
            let (path, line, status) = (task.path.clone(), t.line, t.status);
            check.connect_toggled(move |_| toggle(&path, line, status));
        }
        top.append(&check);
        let text = plain(&t.text);
        let rtl = direction::starts_rtl(&text);
        top.append(
            &gtk::Label::builder()
                .label(&text)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(4)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .hexpand(true)
                .xalign(if rtl { 1.0 } else { 0.0 })
                .css_classes(["board-text"])
                .build(),
        );
        card.append(&top);
        let meta = gtk::Box::builder().spacing(5).css_classes(["caption"]).build();
        meta.append(
            &gtk::Label::builder()
                .label(Vault::title(&task.path))
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["daftar-note"])
                .build(),
        );
        if !t.section.is_empty() {
            meta.append(
                &gtk::Label::builder()
                    .label(format!("› {}", t.section))
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["daftar-muted"])
                    .build(),
            );
        }
        meta.append(&gtk::Box::builder().hexpand(true).build());
        if t.recurring.is_some() {
            meta.append(&gtk::Label::builder().label("🔁").tooltip_text(t.recurring.as_deref().unwrap_or("")).build());
        }
        if let Some((mark, tint)) = t.priority.as_deref().and_then(priority) {
            meta.append(&gtk::Label::builder().label(mark).css_classes([tint, "tint"]).build());
        }
        if let Some(day) = t.day() {
            let mark = if t.due.is_some() { "📅" } else { "⏳" };
            meta.append(
                &gtk::Label::builder().label(format!("{mark} {day}")).css_classes(["daftar-day", day_tint(day, t.is_open())]).build(),
            );
        }
        card.append(&meta);
        let wrapper = gtk::Box::builder().css_classes([tint]).build();
        card.set_hexpand(true);
        wrapper.append(&card);

        // A click opens the note at the task; a drag carries the task's id.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        {
            let open = self.open();
            let (path, line) = (task.path.clone(), t.line);
            click.connect_released(move |gesture, presses, _, _| {
                if presses == 1 && gesture.widget().is_some() {
                    open(&path, line);
                }
            });
        }
        card.add_controller(click);
        let drag = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        {
            let id = task.id();
            drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
        }
        {
            let card = card.clone();
            drag.connect_drag_begin(move |source, _| {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&card)).current_image()), 20, 12);
            });
        }
        card.add_controller(drag);

        // A right click: open, edit, or move without dragging.
        let menu = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        let popover = gtk::Popover::builder().child(&menu).has_arrow(false).build();
        popover.set_parent(&card);
        let item = |label: &str, run: Box<dyn Fn()>| {
            let button =
                gtk::Button::builder().child(&gtk::Label::builder().label(label).xalign(0.0).build()).css_classes(["flat"]).build();
            let popover = popover.clone();
            button.connect_clicked(move |_| {
                popover.popdown();
                run();
            });
            menu.append(&button);
        };
        {
            let open = self.open();
            let (path, line) = (task.path.clone(), t.line);
            item("Open Note", Box::new(move || open(&path, line)));
        }
        {
            let edit = self.edit();
            let (path, line) = (task.path.clone(), t.line);
            item("Edit Task…", Box::new(move || edit(&path, line)));
        }
        for (title, drop) in targets {
            let moved = self.moved();
            let (task, drop) = (task.clone(), drop.clone());
            item(&format!("Move to {title}"), Box::new(move || moved(&task, &drop)));
        }
        let right = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        right.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
        card.add_controller(right);
        wrapper
    }

    fn open(&self) -> AtLine {
        self.handlers.open.clone()
    }

    fn toggle(&self) -> Toggle {
        self.handlers.toggle.clone()
    }

    fn edit(&self) -> AtLine {
        self.handlers.edit.clone()
    }

    fn moved(&self) -> Move {
        self.handlers.moved.clone()
    }
}

/// The tasks the filters let through: one note, and words anywhere in the
/// text, the heading or the path.
pub fn shown(tasks: Vec<VaultTask>, file: Option<&str>, query: &str) -> Vec<VaultTask> {
    let needle = slugs::fold(query.trim());
    tasks
        .into_iter()
        .filter(|t| file.is_none_or(|f| t.path == f))
        .filter(|t| needle.is_empty() || slugs::fold(&format!("{} {} {}", t.task.text, t.task.section, t.path)).contains(&needle))
        .collect()
}

/// A column's color, as the Mac gives it.
pub fn tint(column: &Column) -> String {
    let name = match column.id.as_str() {
        "todo" | "today" => "accent",
        "doing" | "week" => "blue",
        "done" => "green",
        "overdue" => "red",
        "tomorrow" => "yellow",
        "later" => "purple",
        "cancelled" | "none" => "muted",
        // A note's column: a color of its own, the same at every draw.
        path => {
            const CYCLE: [&str; 6] = ["aqua", "purple", "pink", "blue", "green", "accent"];
            CYCLE[path.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) % CYCLE.len()]
        }
    };
    format!("t-{name}")
}

/// Red when late, accent today, blue after; muted when closed.
fn day_tint(text: &str, open: bool) -> &'static str {
    let Some(day) = Day::parse(text).filter(|_| open) else { return "t-muted" };
    let today = Day::today();
    if day < today {
        "t-red"
    } else if day == today {
        "t-accent"
    } else {
        "t-blue"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::TaskLine;

    fn task(path: &str, raw: &str, line: usize) -> VaultTask {
        VaultTask { path: path.into(), task: TaskLine::parse(raw, line, "Kit").unwrap() }
    }

    #[test]
    fn filters_and_colors() {
        let tasks = vec![task("Tasks/Work.md", "- [ ] Ship the plan", 0), task("Tasks/Home.md", "- [ ] Book flights", 0)];
        assert_eq!(shown(tasks.clone(), Some("Tasks/Home.md"), "").len(), 1);
        assert_eq!(shown(tasks.clone(), None, "SHIP").len(), 1);
        assert_eq!(shown(tasks.clone(), None, "kit").len(), 2);
        assert_eq!(shown(tasks, None, "nothing").len(), 0);
        let column = |id: &str| Column { id: id.into(), title: id.into(), tasks: vec![], drop: None };
        assert_eq!(tint(&column("overdue")), "t-red");
        assert_eq!(tint(&column("Tasks/Work.md")), tint(&column("Tasks/Work.md")));
        assert!(crate::style::TINTS.contains(&tint(&column("Tasks/Home.md")).trim_start_matches("t-")));
    }
}
