//! A Daftar window: the tree on the side, the note in the reader.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{SearchIndex, Value, Vault};

use crate::palette::{self, Mode};
use crate::reader::{Reader, Request};
use crate::settings::AppSettings;
use crate::sidebar::Sidebar;
use crate::store::VaultStore;
use crate::sync::{GitSync, Run};
use crate::watcher::FileWatcher;

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
            store: Rc::new(RefCell::new(None)),
            watcher: Rc::new(RefCell::new(None)),
            recent: Rc::new(RefCell::new(Vec::new())),
            back: Rc::new(RefCell::new(Vec::new())),
            forward: Rc::new(RefCell::new(Vec::new())),
        };
        *opener.borrow_mut() = Some(this.clone());

        {
            let this = this.clone();
            reader.connect_open(move |path, anchor| this.open(path, anchor));
        }
        {
            let this = this.clone();
            reader.connect_notice(move |text| this.toasts.add_toast(adw::Toast::new(text)));
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
        stack.add_named(&this.choose_page(), Some("choose"));
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.set_content(Some(&stack));
        toasts.set_child(Some(&content));
        split.set_content(Some(&adw::NavigationPage::builder().title("Note").child(&toasts).build()));

        // A narrow window shows one pane at a time.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 600sp").unwrap());
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(breakpoint);
        window.set_content(Some(&split));
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
            *this.store.borrow_mut() = Some(store);
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
        self.go(path, anchor, true);
    }

    /// Opens [path]; with [record], the note it leaves is one Back away.
    fn go(&self, path: &str, anchor: Option<&str>, record: bool) {
        let Some(store) = self.store.borrow().clone() else { return };
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
        let folder = path.rsplit_once('/').map(|(folder, _)| folder).unwrap_or("");
        self.title.set_title(&Vault::title(path));
        self.title.set_subtitle(folder);
        self.stack.set_visible_child_name("reader");
        self.split.set_show_content(true);
        self.reader.show(Request { path: path.to_string(), heading: anchor.map(String::from), line: None }, false);
        self.update_history();
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
            self.go(&path, None, false);
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
        let index = store.index();
        if index.files != files || index.empty_folders != folders {
            self.sidebar.set_tree(index.tree());
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

    fn palette(&self, mode: Mode) {
        let Some(store) = self.store.borrow().clone() else { return };
        let this = self.clone();
        palette::present(&self.window, mode, store, self.recent.borrow().clone(), move |path| this.open(path, None));
    }

    fn add_actions(&self) {
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
