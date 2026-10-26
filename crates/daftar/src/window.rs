//! A Daftar window: the tree on the side, the note in the reader.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{SearchIndex, Value, Vault};

use crate::board::{self, Board};
use crate::editor::{Editor, Saved};
use crate::inspector::Inspector;
use crate::notifier::{Notifier, Wanted};
use crate::palette::{self, Mode};
use crate::reader::{Reader, Request};
use crate::settings::AppSettings;
use crate::sidebar::Sidebar;
use crate::store::VaultStore;
use crate::sync::{GitSync, Run};
use crate::task_edit;
use crate::terminal::TerminalPane;
use crate::today::Today;
use crate::watcher::FileWatcher;

mod file_actions;

/// How many notes the quick switcher remembers as opened lately.
const RECENT: usize = 20;

#[derive(Clone)]
pub struct Window {
    window: adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    stack: gtk::Stack,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    split: adw::NavigationSplitView,
    reader: Reader,
    sidebar: Rc<Sidebar>,
    git: Rc<GitSync>,
    today: Rc<Today>,
    board: Rc<Board>,
    editor: Rc<Editor>,
    terminal: Rc<TerminalPane>,
    notifier: Rc<Notifier>,
    /// Backlinks and unlinked mentions, at the right of the note.
    inspector: Rc<Inspector>,
    inspector_split: adw::OverlaySplitView,
    store: Rc<RefCell<Option<Rc<VaultStore>>>>,
    watcher: Rc<RefCell<Option<FileWatcher>>>,
    /// The notes opened lately, the last first.
    recent: Rc<RefCell<Vec<String>>>,
    /// Where Back and Forward go, the nearest last.
    back: Rc<RefCell<Vec<String>>>,
    forward: Rc<RefCell<Vec<String>>>,
}

impl Window {
    /// A new window on the configured vault, with the note at [open] when given.
    pub fn present(app: &adw::Application, open: Option<String>) {
        let settings = Rc::new(RefCell::new(AppSettings::load()));
        apply_theme(&settings.borrow().theme());
        crate::style::install();

        let window = adw::ApplicationWindow::builder().application(app).title("Daftar").default_width(1200).default_height(800).build();
        let reader = Reader::new(settings.clone());
        let title = adw::WindowTitle::new("Daftar", "");
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        let toasts = adw::ToastOverlay::new();
        let split = adw::NavigationSplitView::new();

        // The open note's path, filled in once the sidebar exists.
        let opener: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
        let sidebar = {
            let opener = opener.clone();
            Rc::new(Sidebar::new(move |path| {
                if let Some(window) = opener.borrow().as_ref() {
                    window.open(path, None);
                }
            }))
        };

        let git = {
            let toasts = toasts.clone();
            GitSync::new(move |text| toasts.add_toast(adw::Toast::new(text)))
        };
        let go_back =
            gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Back (Ctrl+[)").action_name("win.back").build();
        let go_forward =
            gtk::Button::builder().icon_name("go-next-symbolic").tooltip_text("Forward (Ctrl+])").action_name("win.forward").build();
        let today = {
            let (open, toggle, edit) = (opener.clone(), opener.clone(), opener.clone());
            Today::new(
                move |path, line| {
                    if let Some(window) = open.borrow().as_ref() {
                        window.open_line(path, line);
                    }
                },
                move |path, line, status| {
                    if let Some(window) = toggle.borrow().as_ref() {
                        window.toggle_task(path, line, status);
                    }
                },
                move |path, line| {
                    if let Some(window) = edit.borrow().as_ref() {
                        window.edit_task(path, line);
                    }
                },
            )
        };
        let board = {
            let (open, toggle, edit, moved) = (opener.clone(), opener.clone(), opener.clone(), opener.clone());
            Board::new(board::Handlers {
                open: Rc::new(move |path, line| {
                    if let Some(window) = open.borrow().as_ref() {
                        window.open_line(path, line);
                    }
                }),
                toggle: Rc::new(move |path, line, status| {
                    if let Some(window) = toggle.borrow().as_ref() {
                        window.toggle_task(path, line, status);
                    }
                }),
                edit: Rc::new(move |path, line| {
                    if let Some(window) = edit.borrow().as_ref() {
                        window.edit_task(path, line);
                    }
                }),
                moved: Rc::new(move |task, drop| {
                    if let Some(window) = moved.borrow().as_ref() {
                        window.move_task(task, drop);
                    }
                }),
            })
        };
        let editor = {
            let (notices, conflicts, opener) = (toasts.clone(), toasts.clone(), opener.clone());
            Editor::new(
                settings.clone(),
                move |text| notices.add_toast(adw::Toast::new(text)),
                move |text| {
                    let toast = adw::Toast::builder().title(text).button_label("Reload").timeout(0).build();
                    let opener = opener.clone();
                    toast.connect_button_clicked(move |_| {
                        if let Some(window) = opener.borrow().as_ref() {
                            window.editor.reload();
                        }
                    });
                    conflicts.add_toast(toast);
                },
            )
        };
        let terminal = {
            let settings = settings.borrow();
            TerminalPane::new(
                settings.file.string("terminal.shell").unwrap_or_default(),
                &settings.file.string("appearance.code_font").unwrap_or_default(),
                settings.file.int("terminal.font_size").unwrap_or(13),
            )
        };
        let inspector = {
            let (open, notices) = (opener.clone(), toasts.clone());
            Inspector::new(
                move |path, line| {
                    if let Some(window) = open.borrow().as_ref() {
                        window.open_line(path, line);
                    }
                },
                move |text| notices.add_toast(adw::Toast::new(text)),
            )
        };
        let inspector_split =
            adw::OverlaySplitView::builder().sidebar_position(gtk::PackType::End).show_sidebar(false).sidebar(&inspector.widget).build();
        let this = Window {
            window: window.clone(),
            settings,
            stack: stack.clone(),
            title: title.clone(),
            toasts: toasts.clone(),
            split: split.clone(),
            reader: reader.clone(),
            sidebar: sidebar.clone(),
            git: git.clone(),
            today: today.clone(),
            board: board.clone(),
            editor: editor.clone(),
            terminal: terminal.clone(),
            notifier: Notifier::new(),
            inspector,
            inspector_split: inspector_split.clone(),
            store: Rc::new(RefCell::new(None)),
            watcher: Rc::new(RefCell::new(None)),
            recent: Rc::new(RefCell::new(Vec::new())),
            back: Rc::new(RefCell::new(Vec::new())),
            forward: Rc::new(RefCell::new(Vec::new())),
        };
        *opener.borrow_mut() = Some(this.clone());
        {
            // Neovim's mode beside "Editing", while the editor is on screen.
            let (title, stack) = (title.clone(), stack.clone());
            editor.connect_mode(move |mode| {
                if stack.visible_child_name().as_deref() == Some("editor") {
                    title.set_subtitle(&mode.map_or("Editing".to_string(), |mode| format!("Editing · {mode}")));
                }
            });
        }

        {
            let this = this.clone();
            reader.connect_open(move |path, anchor| this.open(path, anchor));
        }
        {
            let this = this.clone();
            reader.connect_notice(move |text| this.toasts.add_toast(adw::Toast::new(text)));
        }
        {
            let this = this.clone();
            reader.connect_edit_task(move |path, line| this.edit_task(path, line));
        }

        // The sidebar.
        let side_header = adw::HeaderBar::new();
        let quick_open = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Open a Note (Ctrl+P)")
            .action_name("win.quick-open")
            .build();
        side_header.pack_end(&quick_open);
        side_header.pack_start(&git.button);
        let today_button =
            gtk::Button::builder().icon_name("weather-clear-symbolic").tooltip_text("Today (Ctrl+1)").action_name("win.today").build();
        side_header.pack_end(&today_button);
        let side = adw::ToolbarView::new();
        side.add_top_bar(&side_header);
        side.set_content(Some(&sidebar.widget));
        split.set_sidebar(Some(&adw::NavigationPage::builder().title("Daftar").child(&side).build()));

        // The note.
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        let history = gtk::Box::builder().css_classes(["linked"]).build();
        history.append(&go_back);
        history.append(&go_forward);
        header.pack_start(&history);
        let edit =
            gtk::ToggleButton::builder().icon_name("document-edit-symbolic").tooltip_text("Edit (Ctrl+E)").action_name("win.edit").build();
        header.pack_end(&edit);
        header.pack_end(
            &gtk::ToggleButton::builder()
                .icon_name("sidebar-show-right-symbolic")
                .tooltip_text("Inspector (Ctrl+Alt+I)")
                .action_name("win.inspector")
                .build(),
        );
        stack.add_named(
            &adw::StatusPage::builder()
                .icon_name("text-x-generic-symbolic")
                .title("No Note Open")
                .description("Pick a note in the sidebar.")
                .build(),
            Some("empty"),
        );
        stack.add_named(
            &gtk::Spinner::builder()
                .spinning(true)
                .width_request(32)
                .height_request(32)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build(),
            Some("loading"),
        );
        stack.add_named(&reader.view, Some("reader"));
        stack.add_named(&today.widget, Some("today"));
        stack.add_named(&board.widget, Some("board"));
        stack.add_named(&editor.widget, Some("editor"));
        stack.add_named(&this.choose_page(), Some("choose"));
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        let panes = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&stack)
            .end_child(&terminal.widget)
            .resize_end_child(false)
            .shrink_end_child(false)
            .build();
        panes.set_position(560);
        content.set_content(Some(&panes));
        inspector_split.set_content(Some(&content));
        toasts.set_child(Some(&inspector_split));
        split.set_content(Some(&adw::NavigationPage::builder().title("Note").child(&toasts).build()));

        // A narrow window shows one pane at a time.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 600sp").unwrap());
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(breakpoint);
        window.set_content(Some(&split));
        {
            let editor = editor.clone();
            window.connect_close_request(move |_| {
                editor.save();
                glib::Propagation::Proceed
            });
        }
        this.add_actions();
        window.present();

        match this.settings.borrow().vault() {
            Some(root) => this.load(root, open),
            None => this.stack.set_visible_child_name("choose"),
        }
    }

    fn choose_page(&self) -> adw::StatusPage {
        let button =
            gtk::Button::builder().label("Open Vault…").halign(gtk::Align::Center).css_classes(["pill", "suggested-action"]).build();
        let this = self.clone();
        button.connect_clicked(move |_| this.choose_vault());
        adw::StatusPage::builder()
            .icon_name("folder-open-symbolic")
            .title("Open a Vault")
            .description("The folder of an Obsidian vault. Daftar keeps it in ~/.config/daftar/config.toml.")
            .child(&button)
            .build()
    }

    fn choose_vault(&self) {
        let dialog = gtk::FileDialog::builder().title("Open Vault").modal(true).build();
        let this = self.clone();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|folder| folder.path()) else { return };
            if let Err(error) = this.settings.borrow_mut().set("vault", Value::String(path.to_string_lossy().into_owned())) {
                this.toasts.add_toast(adw::Toast::new(&format!("The settings were not saved: {error}")));
            }
            this.load(path, None);
        });
    }

    /// Indexes the vault at [root] away from the window, then shows it.
    fn load(&self, root: PathBuf, open: Option<String>) {
        if !root.is_dir() {
            self.stack.set_visible_child_name("choose");
            self.toasts.add_toast(adw::Toast::new(&format!("{} is not a folder.", root.display())));
            return;
        }
        if !self.leave_editor() {
            return;
        }
        self.stack.set_visible_child_name("loading");
        let this = self.clone();
        glib::spawn_future_local(async move {
            let search = Arc::new(SearchIndex::new());
            let index = {
                let (root, search) = (root.clone(), search.clone());
                match gio::spawn_blocking(move || Vault::index(&root, Some(&search))).await {
                    Ok(index) => index,
                    Err(_) => return,
                }
            };
            let folder = root;
            let store = Rc::new(VaultStore::new(folder.clone(), index, search));
            let watcher = {
                let this = this.clone();
                FileWatcher::new(&folder, move |changed| this.files_changed(changed))
            };
            match watcher {
                Ok(watcher) => *this.watcher.borrow_mut() = Some(watcher),
                Err(error) => this.toasts.add_toast(adw::Toast::new(&format!("Changes on disk are not followed: {error}"))),
            }
            this.recent.borrow_mut().clear();
            this.back.borrow_mut().clear();
            this.forward.borrow_mut().clear();
            this.sidebar.set_tree(store.index().tree());
            this.reader.set_store(store.clone());
            this.editor.set_store(store.clone());
            {
                let git = this.git.clone();
                store.connect_wrote(move || git.edited());
            }
            let (pull, interval) = {
                let settings = this.settings.borrow();
                (
                    settings.file.bool("git.pull_on_launch").unwrap_or(true),
                    settings.file.int("git.sync_interval").unwrap_or(0).max(0) as u32,
                )
            };
            this.git.attach(folder.clone(), pull, interval);
            this.terminal.set_folder(folder.clone());
            *this.store.borrow_mut() = Some(store.clone());
            this.after_load(&store);
            this.plan_notifications();
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(page) = this.split.sidebar() {
                page.set_title(&name);
            }
            this.stack.set_visible_child_name("empty");
            if let Some(path) = open.filter(|p| this.store.borrow().as_ref().is_some_and(|s| s.index().files.contains(p))) {
                this.open(&path, None);
            }
        });
    }

    /// Opens a note or a base in the reader, and any other file in the app
    /// the system gives it to.
    pub fn open(&self, path: &str, anchor: Option<&str>) {
        self.go(path, anchor, None, true);
    }

    /// Opens the note at [path] with its [line] in view, as a task's link does.
    pub fn open_line(&self, path: &str, line: usize) {
        self.go(path, None, Some(line), true);
    }

    fn editing(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("editor")
    }

    /// Saves and closes the editor; false when the save was refused, and the
    /// editor stays so the person's text is not lost.
    fn leave_editor(&self) -> bool {
        if !self.editing() {
            return true;
        }
        let path = self.editor.path();
        if self.editor.close() == Saved::ChangedOnDisk {
            return false;
        }
        self.set_edit_state(false);
        // Back to the note's folder under its title, as reading shows it.
        if let Some(path) = path.or_else(|| self.reader.shown().map(|shown| shown.path)) {
            self.title.set_subtitle(folder_of(&path));
        }
        true
    }

    /// Ctrl+E: the note on screen in the editor, at the line read last; again,
    /// back to reading it where the cursor is.
    fn toggle_edit(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        if self.editing() {
            let line = self.editor.line();
            let Some(path) = self.editor.path() else { return };
            if !self.leave_editor() {
                return;
            }
            self.stack.set_visible_child_name("reader");
            self.reader.show(Request { path, heading: None, line: Some(line) }, false);
            return;
        }
        let Some(shown) = self.reader.shown().filter(|_| self.stack.visible_child_name().as_deref() == Some("reader")) else { return };
        if !Vault::is_note(&shown.path) {
            self.toasts.add_toast(adw::Toast::new("Only a note can be edited here."));
            return;
        }
        if !self.editor.load(&store.root, &shown.path, shown.line) {
            self.toasts.add_toast(adw::Toast::new(&format!("{} could not be read.", shown.path)));
            return;
        }
        self.stack.set_visible_child_name("editor");
        self.set_edit_state(true);
    }

    /// The edit action's state, and "Editing" under the title while it is on.
    fn set_edit_state(&self, on: bool) {
        if let Some(action) = self.window.lookup_action("edit").and_downcast::<gio::SimpleAction>() {
            action.set_state(&on.to_variant());
        }
        if on {
            self.title.set_subtitle("Editing");
        }
    }

    /// The Today page, drawn from the index as it is now.
    fn show_today(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        self.today.show(&store);
        self.title.set_title("Today");
        self.title.set_subtitle("");
        self.stack.set_visible_child_name("today");
        self.split.set_show_content(true);
    }

    /// Ctrl+2: the task board, drawn from the index as it is now.
    fn show_board(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        self.board.show(store);
        self.title.set_title("Board");
        self.title.set_subtitle("");
        self.stack.set_visible_child_name("board");
        self.split.set_show_content(true);
    }

    /// Ticks a task from a page drawn by the window, not the reader.
    fn toggle_task(&self, path: &str, line: usize, status: char) {
        let Some(store) = self.store.borrow().clone() else { return };
        if let Err(failure) = store.toggle_task(path, line, status) {
            self.toasts.add_toast(adw::Toast::new(&failure.to_string()));
        }
        self.redraw_page();
    }

    /// A card dropped on a board column, or moved from its menu.
    fn move_task(&self, task: &daftar_core::VaultTask, drop: &daftar_core::BoardDrop) {
        let Some(store) = self.store.borrow().clone() else { return };
        if store.move_task(task, drop).is_err() {
            let text = crate::today::plain(&task.task.text);
            self.toasts.add_toast(adw::Toast::new(&format!("“{text}” changed in its note before the move. Nothing was written.")));
        }
        self.redraw_page();
    }

    /// The Edit Task dialog on the task at [line] of [path], from the reader,
    /// Today or the board.
    fn edit_task(&self, path: &str, line: usize) {
        let Some(store) = self.store.borrow().clone() else { return };
        let this = self.clone();
        task_edit::present(&self.window, store, path, line, move || this.redraw_page());
    }

    /// The page on screen drawn again after the app wrote a task.
    fn redraw_page(&self) {
        match self.stack.visible_child_name().as_deref() {
            Some("today") => self.show_today(),
            Some("board") => self.show_board(),
            Some("reader") => self.reader.reload(),
            _ => {}
        }
    }

    /// Opens [path]; with [record], the note it leaves is one Back away.
    fn go(&self, path: &str, anchor: Option<&str>, line: Option<usize>, record: bool) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        if !Vault::opens_in_app(path) {
            let file = gio::File::for_path(store.root.join(path));
            gtk::FileLauncher::new(Some(&file)).launch(Some(&self.window), gio::Cancellable::NONE, |_| {});
            return;
        }
        {
            let mut recent = self.recent.borrow_mut();
            recent.retain(|p| p != path);
            recent.insert(0, path.to_string());
            recent.truncate(RECENT);
        }
        if record && let Some(shown) = self.reader.shown().filter(|shown| shown.path != path) {
            self.back.borrow_mut().push(shown.path);
            self.forward.borrow_mut().clear();
        }
        self.title.set_title(&Vault::title(path));
        self.title.set_subtitle(folder_of(path));
        self.stack.set_visible_child_name("reader");
        self.split.set_show_content(true);
        // A note opens where it was being read, unless a link names a place in it.
        let line = line.or_else(|| anchor.is_none().then(|| self.place(path)).flatten());
        self.reader.show(Request { path: path.to_string(), heading: anchor.map(String::from), line }, false);
        self.update_history();
        self.refresh_inspector();
    }

    /// Back when [backward], else Forward: the notes that are gone are skipped.
    fn travel(&self, backward: bool) {
        let (from, to) = if backward { (&self.back, &self.forward) } else { (&self.forward, &self.back) };
        let Some(store) = self.store.borrow().clone() else { return };
        loop {
            let Some(path) = from.borrow_mut().pop() else { break };
            if !store.index().files.contains(&path) {
                continue;
            }
            if let Some(shown) = self.reader.shown() {
                to.borrow_mut().push(shown.path);
            }
            self.go(&path, None, None, false);
            return;
        }
        self.update_history();
    }

    fn update_history(&self) {
        if let Some(action) = self.window.lookup_action("back").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(!self.back.borrow().is_empty());
        }
        if let Some(action) = self.window.lookup_action("forward").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(!self.forward.borrow().is_empty());
        }
    }

    /// The reader's text size one [step] bigger or smaller, or 14 again for
    /// 0, kept in the config as the Mac keeps it: 10 to 28.
    fn text_size(&self, step: i64) {
        let next = if step == 0 { 14 } else { (self.settings.borrow().text_size() + step).clamp(10, 28) };
        if let Err(error) = self.settings.borrow_mut().set("appearance.text_size", Value::Int(next)) {
            self.toasts.add_toast(adw::Toast::new(&format!("The settings were not saved: {error}")));
        }
        self.reader.reload();
    }

    /// Files changed on disk, by another app, `git` or this one: the index
    /// learns them, and the note on screen is drawn again when it, or a
    /// note it links to or embeds, is one of them.
    fn files_changed(&self, changed: BTreeSet<String>) {
        let Some(store) = self.store.borrow().clone() else { return };
        let (files, folders) = {
            let index = store.index();
            (index.files.clone(), index.empty_folders.clone())
        };
        store.refresh(&changed.iter().cloned().collect::<Vec<_>>());
        self.git.refresh();
        self.plan_notifications();
        self.refresh_inspector();
        let moved = {
            let index = store.index();
            let moved = index.files != files || index.empty_folders != folders;
            if moved {
                self.sidebar.set_tree(index.tree());
            }
            moved
        };
        if moved {
            self.refresh_marks();
        }
        let index = store.index();
        if self.stack.visible_child_name().as_deref() == Some("today") {
            drop(index);
            self.show_today();
            return;
        }
        if self.stack.visible_child_name().as_deref() == Some("board") {
            drop(index);
            self.show_board();
            return;
        }
        if self.editing() {
            if self.editor.path().is_some_and(|path| changed.contains(&path)) {
                self.editor.changed_on_disk();
            }
            return;
        }
        let Some(shown) = self.reader.shown() else { return };
        if !index.files.contains(&shown.path) {
            self.title.set_title("Daftar");
            self.title.set_subtitle("");
            self.stack.set_visible_child_name("empty");
            return;
        }
        let touched = changed.contains(&shown.path) || index.resolved_links(&shown.path).values().any(|target| changed.contains(target));
        drop(index);
        if touched {
            self.reader.reload();
        }
    }

    /// Ctrl+Shift+P: every command of the window, by name.
    fn commands(&self) {
        let command = |title, group, action, keys| palette::Command { title, group, action, keys };
        let mut commands = vec![
            command("Quick Open", "Go", "win.quick-open", "Ctrl+P"),
            command("Search the Notes", "Go", "win.search", "Ctrl+Shift+F"),
            command("Today", "Go", "win.today", "Ctrl+1"),
            command("Task Board", "Go", "win.board", "Ctrl+2"),
            command("Back", "Go", "win.back", "Ctrl+["),
            command("Forward", "Go", "win.forward", "Ctrl+]"),
            command("Edit or Read the Note", "Note", "win.edit", "Ctrl+E"),
            command("Show the Graph", "View", "win.graph", "Ctrl+Alt+G"),
            command("Show or Hide the Inspector", "View", "win.inspector", "Ctrl+Alt+I"),
            command("New Note", "File", "win.new-note::", "Ctrl+N"),
            command("New Folder", "File", "win.new-folder::", "Ctrl+Shift+N"),
            command("Rename", "File", "win.rename::", "F2"),
            command("Move to…", "File", "win.move::", ""),
            command("Move to Trash", "File", "win.trash::", ""),
            command("Bookmark or Remove the Bookmark", "File", "win.bookmark::", "Ctrl+D"),
            command("Show or Hide the Terminal", "View", "win.terminal", "Ctrl+`"),
            command("Find in the Note", "Note", "win.find", "Ctrl+F"),
            command("Bigger Text", "View", "win.bigger-text", "Ctrl+="),
            command("Smaller Text", "View", "win.smaller-text", "Ctrl+-"),
            command("Actual Size", "View", "win.actual-size", "Ctrl+0"),
            command("Open Vault…", "File", "win.open-vault", "Ctrl+Shift+O"),
        ];
        if self.git.button.is_visible() {
            commands.push(command("Sync Now", "Git", "win.sync", "Ctrl+Shift+S"));
            commands.push(command("Pull", "Git", "win.pull", ""));
        }
        commands.push(command("New Window", "App", "app.new-window", "Ctrl+Alt+N"));
        commands.push(command("Settings", "App", "win.preferences", "Ctrl+,"));
        commands.push(command("Quit", "App", "app.quit", "Ctrl+Q"));
        let window = self.window.clone();
        palette::present_commands(&self.window, commands, move |action| {
            // `win.rename::` is an action with an empty string target: the note on screen.
            match action.strip_suffix("::") {
                Some(name) => {
                    let _ = WidgetExt::activate_action(&window, name, Some(&"".to_variant()));
                }
                None => {
                    let _ = WidgetExt::activate_action(&window, action, None);
                }
            }
        });
    }

    fn graph(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let center = self.reader.shown().map(|shown| shown.path).filter(|p| Vault::is_note(p));
        let this = self.clone();
        crate::graph::present(&self.window, store, center, move |path| this.open(path, None));
    }

    /// Meeting alerts and the digest, planned again from the vault as it is now.
    fn plan_notifications(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let wanted = Wanted::from(&self.settings.borrow().file);
        self.notifier.plan(&store, &wanted);
    }

    fn preferences(&self) {
        let hooks = crate::preferences::Hooks {
            theme: Box::new(apply_theme),
            reader: Box::new({
                let reader = self.reader.clone();
                move || reader.reload()
            }),
            git: Box::new({
                let git = self.git.clone();
                move |_, interval| git.reconfigure(interval)
            }),
            notice: Box::new({
                let toasts = self.toasts.clone();
                move |text| toasts.add_toast(adw::Toast::new(text))
            }),
        };
        crate::preferences::present(&self.window, self.settings.clone(), Rc::new(hooks));
    }

    fn palette(&self, mode: Mode) {
        let Some(store) = self.store.borrow().clone() else { return };
        let this = self.clone();
        palette::present(&self.window, mode, store, self.recent.borrow().clone(), move |path| this.open(path, None));
    }

    fn add_actions(&self) {
        self.add_file_actions();
        let quick_open = gio::SimpleAction::new("quick-open", None);
        let this = self.clone();
        quick_open.connect_activate(move |_, _| this.palette(Mode::Files));
        self.window.add_action(&quick_open);
        let search = gio::SimpleAction::new("search", None);
        let this = self.clone();
        search.connect_activate(move |_, _| this.palette(Mode::Text));
        self.window.add_action(&search);
        let find = gio::SimpleAction::new("find", None);
        let reader = self.reader.clone();
        find.connect_activate(move |_, _| reader.find("open"));
        self.window.add_action(&find);
        let next = gio::SimpleAction::new("find-next", None);
        let reader = self.reader.clone();
        next.connect_activate(move |_, _| reader.find("next"));
        self.window.add_action(&next);
        let previous = gio::SimpleAction::new("find-previous", None);
        let reader = self.reader.clone();
        previous.connect_activate(move |_, _| reader.find("previous"));
        self.window.add_action(&previous);
        let open_vault = gio::SimpleAction::new("open-vault", None);
        let this = self.clone();
        open_vault.connect_activate(move |_, _| this.choose_vault());
        self.window.add_action(&open_vault);
        let terminal = gio::SimpleAction::new("terminal", None);
        let pane = self.terminal.clone();
        terminal.connect_activate(move |_, _| pane.toggle());
        self.window.add_action(&terminal);
        let graph = gio::SimpleAction::new("graph", None);
        let this = self.clone();
        graph.connect_activate(move |_, _| this.graph());
        self.window.add_action(&graph);
        let preferences = gio::SimpleAction::new("preferences", None);
        let this = self.clone();
        preferences.connect_activate(move |_, _| this.preferences());
        self.window.add_action(&preferences);
        let commands = gio::SimpleAction::new("commands", None);
        let this = self.clone();
        commands.connect_activate(move |_, _| this.commands());
        self.window.add_action(&commands);
        let edit = gio::SimpleAction::new_stateful("edit", None, &false.to_variant());
        let this = self.clone();
        edit.connect_activate(move |_, _| this.toggle_edit());
        self.window.add_action(&edit);
        let save = gio::SimpleAction::new("save", None);
        let editor = self.editor.clone();
        save.connect_activate(move |_, _| {
            editor.save();
        });
        self.window.add_action(&save);
        let board = gio::SimpleAction::new("board", None);
        let this = self.clone();
        board.connect_activate(move |_, _| {
            this.show_board();
            // Once the new cards are on screen, which a focus needs.
            let board = this.board.clone();
            glib::idle_add_local_once(move || board.focus_cards());
        });
        self.window.add_action(&board);
        let today = gio::SimpleAction::new("today", None);
        let this = self.clone();
        today.connect_activate(move |_, _| this.show_today());
        self.window.add_action(&today);
        for (name, backward) in [("back", true), ("forward", false)] {
            let action = gio::SimpleAction::new(name, None);
            action.set_enabled(false);
            let this = self.clone();
            action.connect_activate(move |_, _| this.travel(backward));
            self.window.add_action(&action);
        }
        for (name, step) in [("bigger-text", 1), ("smaller-text", -1), ("actual-size", 0)] {
            let action = gio::SimpleAction::new(name, None);
            let this = self.clone();
            action.connect_activate(move |_, _| this.text_size(step));
            self.window.add_action(&action);
        }
        for (name, kind) in [("sync", Run::Sync), ("pull", Run::Pull)] {
            let action = gio::SimpleAction::new(name, None);
            let git = self.git.clone();
            action.connect_activate(move |_, _| git.run(kind, true));
            self.window.add_action(&action);
        }
    }
}

fn apply_theme(theme: &str) {
    let scheme = match theme {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

/// The folder of a vault path, as the subtitle shows it under a note's title.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(folder, _)| folder).unwrap_or("")
}
