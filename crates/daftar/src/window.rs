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
    store: Rc<RefCell<Option<Rc<VaultStore>>>>,
    watcher: Rc<RefCell<Option<FileWatcher>>>,
    /// The notes opened lately, the last first.
    recent: Rc<RefCell<Vec<String>>>,
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

        let this = Window {
            window: window.clone(),
            settings,
            stack: stack.clone(),
            title: title.clone(),
            toasts: toasts.clone(),
            split: split.clone(),
            reader: reader.clone(),
            sidebar: sidebar.clone(),
            store: Rc::new(RefCell::new(None)),
            watcher: Rc::new(RefCell::new(None)),
            recent: Rc::new(RefCell::new(Vec::new())),
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
        let side = adw::ToolbarView::new();
        side.add_top_bar(&side_header);
        side.set_content(Some(&sidebar.widget));
        split.set_sidebar(Some(&adw::NavigationPage::builder().title("Daftar").child(&side).build()));

        // The note.
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
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
            this.sidebar.set_tree(store.index().tree());
            this.reader.set_store(store.clone());
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
        let folder = path.rsplit_once('/').map(|(folder, _)| folder).unwrap_or("");
        self.title.set_title(&Vault::title(path));
        self.title.set_subtitle(folder);
        self.stack.set_visible_child_name("reader");
        self.split.set_show_content(true);
        self.reader.show(Request { path: path.to_string(), heading: anchor.map(String::from), line: None }, false);
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
