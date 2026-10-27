//! A Daftar window: the tree on the side, and tabs of notes, each in its
//! own reader, as the Mac's window tabs have them.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::json;

use daftar_core::{SearchIndex, Value, Vault};

use crate::board::{self, Board};
use crate::editor::{Editor, Saved};
use crate::home::{self, HomePage};
use crate::inspector::Inspector;
use crate::notifier::{Notifier, Wanted};
use crate::palette::{self, Mode};
use crate::reader::{OpenIn, Reader, Request};
use crate::settings::AppSettings;
use crate::sidebar::Sidebar;
use crate::store::VaultStore;
use crate::sync::{GitSync, Run};
use crate::task_edit;
use crate::terminal::TerminalPane;
use crate::today::Today;
use crate::watcher::FileWatcher;

mod crumbs;
mod extras;
mod file_actions;
mod split;
mod tab;
mod vault_tools;

pub(crate) use tab::{Group, Tab};

/// How many notes the quick switcher remembers as opened lately.
const RECENT: usize = 20;

#[derive(Clone)]
pub struct Window {
    window: adw::ApplicationWindow,
    settings: Rc<RefCell<AppSettings>>,
    title: Rc<crumbs::TitleBar>,
    toasts: adw::ToastOverlay,
    split: adw::NavigationSplitView,
    tab_view: adw::TabView,
    /// Never empty: closing the last tab closes the window instead.
    tabs: Rc<RefCell<Vec<Rc<Tab>>>>,
    sidebar: Rc<Sidebar>,
    git: Rc<GitSync>,
    terminal: Rc<TerminalPane>,
    notifier: Rc<Notifier>,
    /// Backlinks and unlinked mentions, at the right of the note.
    inspector: Rc<Inspector>,
    inspector_split: adw::OverlaySplitView,
    store: Rc<RefCell<Option<Rc<VaultStore>>>>,
    watcher: Rc<RefCell<Option<FileWatcher>>>,
    /// The notes opened lately in any tab, the last first.
    recent: Rc<RefCell<Vec<String>>>,
    /// On a periodic note: the earlier and later notes of the series, and a
    /// menu of the current one and the same time in earlier years.
    periodic_nav: PeriodicNav,
}

#[derive(Clone)]
struct PeriodicNav {
    widget: gtk::Box,
    previous: gtk::Button,
    menu: gtk::MenuButton,
    next: gtk::Button,
}

impl Window {
    /// Runs what the application's shortcut for [key] with [state] runs.
    fn activate_shortcut(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        let Some(app) = self.window.application() else { return false };
        let name = gtk::accelerator_name(key.to_lower(), state & gtk::accelerator_get_default_mod_mask());
        let mut ran = false;
        for detailed in app.actions_for_accel(&name) {
            if let Ok((action, target)) = gio::Action::parse_detailed_name(&detailed) {
                ran |= WidgetExt::activate_action(&self.window, &action, target.as_ref()).is_ok();
            }
        }
        ran
    }

    /// A new window on the configured vault, with the note at [open] when given.
    pub fn present(app: &adw::Application, open: Option<String>) {
        let settings = Rc::new(RefCell::new(AppSettings::load()));
        apply_theme(&settings.borrow().theme());
        crate::style::install();

        let window = adw::ApplicationWindow::builder().title("Daftar").default_width(1200).default_height(800).build();
        let store: Rc<RefCell<Option<Rc<VaultStore>>>> = Rc::new(RefCell::new(None));
        let title = crumbs::TitleBar::new(store.clone());
        title.set("Daftar", None);
        let toasts = adw::ToastOverlay::new();
        let split = adw::NavigationSplitView::new();

        // The window, filled in once the sidebar exists.
        let opener: Rc<RefCell<Option<Window>>> = Rc::new(RefCell::new(None));
        // Neovim's Ctrl keys (Ctrl+O, Ctrl+R, Ctrl+W, Ctrl+[) reach it before
        // the app's shortcuts: this runs first because it is added before the
        // application gives the window its shortcuts.
        {
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let opener = opener.clone();
            keys.connect_key_pressed(move |_, key, keycode, state| {
                let Some(this) = opener.borrow().clone() else { return glib::Propagation::Proceed };
                // With the Persian layout on, Ctrl+ح is Ctrl+P: the key in
                // the same place on the Latin layout.
                let latin = latin_shortcut_key(key, keycode, state);
                if this.tab().editor.takes_from_window(latin.unwrap_or(key), state) {
                    return glib::Propagation::Stop;
                }
                match latin {
                    Some(latin) if this.activate_shortcut(latin, state) => glib::Propagation::Stop,
                    _ => glib::Propagation::Proceed,
                }
            });
            window.add_controller(keys);
        }
        window.set_application(Some(app));
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
        let go_home =
            gtk::Button::builder().icon_name("go-home-symbolic").tooltip_text("Home (Ctrl+Shift+D)").action_name("win.home").build();
        let terminal = {
            let settings = settings.borrow();
            TerminalPane::new(
                settings.file.string("terminal.shell").unwrap_or_default(),
                &settings.file.string("appearance.code_font").unwrap_or_default(),
                settings.file.int("terminal.font_size").unwrap_or(13),
            )
        };
        let inspector = {
            let (open, notices, tags) = (opener.clone(), toasts.clone(), opener.clone());
            Inspector::new(
                move |path, line| {
                    if let Some(window) = open.borrow().as_ref() {
                        window.open_line(path, line);
                    }
                },
                move |text| notices.add_toast(adw::Toast::new(text)),
                move |tag| {
                    if let Some(window) = tags.borrow().as_ref() {
                        window.palette(Mode::Text, &format!("#{tag}"));
                    }
                },
            )
        };
        // The inspector has its own header bar: at the window's right edge,
        // it holds the window's buttons while it is open.
        let inspector_pane = {
            let header = adw::HeaderBar::builder()
                .show_start_title_buttons(false)
                .title_widget(&gtk::Label::builder().label("Inspector").css_classes(["heading"]).build())
                .build();
            let pane = adw::ToolbarView::new();
            pane.add_top_bar(&header);
            pane.set_content(Some(&inspector.widget));
            pane
        };
        let inspector_split =
            adw::OverlaySplitView::builder().sidebar_position(gtk::PackType::End).show_sidebar(false).sidebar(&inspector_pane).build();
        let tab_view = adw::TabView::new();
        let periodic_nav = {
            let previous = gtk::Button::builder().icon_name("go-previous-symbolic").action_name("win.periodic-previous").build();
            let next = gtk::Button::builder().icon_name("go-next-symbolic").action_name("win.periodic-next").build();
            let menu = gtk::MenuButton::builder().always_show_arrow(false).build();
            let widget = gtk::Box::builder().css_classes(["linked"]).visible(false).build();
            widget.append(&previous);
            widget.append(&menu);
            widget.append(&next);
            PeriodicNav { widget, previous, menu, next }
        };
        let this = Window {
            window: window.clone(),
            settings,
            title: title.clone(),
            toasts: toasts.clone(),
            split: split.clone(),
            tab_view: tab_view.clone(),
            tabs: Rc::new(RefCell::new(Vec::new())),
            sidebar: sidebar.clone(),
            git: git.clone(),
            terminal: terminal.clone(),
            notifier: Notifier::new(),
            inspector,
            inspector_split: inspector_split.clone(),
            store: store.clone(),
            watcher: Rc::new(RefCell::new(None)),
            recent: Rc::new(RefCell::new(Vec::new())),
            periodic_nav: periodic_nav.clone(),
        };
        *opener.borrow_mut() = Some(this.clone());
        {
            let opener = opener.clone();
            let mover = opener.clone();
            this.sidebar.connect_move(move |paths, folder| {
                let window = mover.borrow().clone();
                if let Some(window) = window {
                    window.move_into(paths, folder);
                }
            });
            this.sidebar.connect_tag(move |tag| {
                let window = opener.borrow().clone();
                if let Some(window) = window {
                    window.palette(Mode::Text, &format!("#{tag}"));
                }
            });
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
            gtk::Button::builder().icon_name("x-office-calendar-symbolic").tooltip_text("Today (Ctrl+1)").action_name("win.today").build();
        side_header.pack_end(&today_button);
        let side = adw::ToolbarView::new();
        side.add_top_bar(&side_header);
        side.set_content(Some(&sidebar.widget));
        split.set_sidebar(Some(&adw::NavigationPage::builder().title("Daftar").child(&side).build()));

        // The notes, in tabs.
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title.widget));
        let history = gtk::Box::builder().css_classes(["linked"]).build();
        history.append(&go_back);
        history.append(&go_forward);
        header.pack_start(&history);
        header.pack_start(&go_home);
        header.pack_start(
            &gtk::Button::builder().icon_name("tab-new-symbolic").tooltip_text("New Tab (Ctrl+T)").action_name("win.new-tab").build(),
        );
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
        header.pack_end(&periodic_nav.widget);
        let tab_bar = adw::TabBar::builder().view(&tab_view).autohide(true).build();
        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.add_top_bar(&tab_bar);
        let panes = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&tab_view)
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
            let this = this.clone();
            tab_view.connect_selected_page_notify(move |_| this.tab_selected());
        }
        {
            let this = this.clone();
            tab_view.connect_close_page(move |_, page| this.close_page(page));
        }
        {
            let this = this.clone();
            window.connect_close_request(move |_| {
                this.save_tabs();
                let tabs = this.tabs.borrow().clone();
                for tab in &tabs {
                    tab.editor.save();
                }
                glib::Propagation::Proceed
            });
        }
        this.add_actions();
        let first = this.new_tab();
        window.present();

        match this.settings.borrow().vault() {
            Some(root) => this.load(root, open),
            None => first.stack.set_visible_child_name("choose"),
        }
    }

    /// A tab, at the end and chosen, on the home page once a vault is open.
    fn new_tab(&self) -> Rc<Tab> {
        let group = Group::new();
        let tab = self.new_pane(&group);
        group.paned.set_start_child(Some(&tab.stack));
        *group.focused.borrow_mut() = Rc::downgrade(&tab);
        // Chosen at once, so it is in the list before the view tells of it.
        let page = self.tab_view.append(&group.paned);
        page.set_title("Home");
        *group.page.borrow_mut() = Some(page.clone());
        self.tab_view.set_selected_page(&page);
        self.start_pane(&tab);
        tab
    }

    /// A pane's first page: home once a vault is open.
    fn start_pane(&self, tab: &Rc<Tab>) {
        match self.store.borrow().clone() {
            Some(_) => self.show_home(tab),
            None if self.settings.borrow().vault().is_some() => tab.stack.set_visible_child_name("loading"),
            None => tab.stack.set_visible_child_name("choose"),
        }
    }

    /// A note pane in [group], in the window's list and not yet on screen.
    fn new_pane(&self, group: &Rc<Group>) -> Rc<Tab> {
        let tab = Rc::new_cyclic(|weak: &Weak<Tab>| {
            let reader = Reader::new(self.settings.clone());
            let editor = {
                let (notices, conflicts, weak) = (self.toasts.clone(), self.toasts.clone(), weak.clone());
                Editor::new(
                    self.settings.clone(),
                    move |text| notices.add_toast(adw::Toast::new(text)),
                    move |text| {
                        let toast = adw::Toast::builder().title(text).button_label("Reload").timeout(0).build();
                        let weak = weak.clone();
                        toast.connect_button_clicked(move |_| {
                            if let Some(tab) = weak.upgrade() {
                                tab.editor.reload();
                            }
                        });
                        conflicts.add_toast(toast);
                    },
                )
            };
            let today = {
                let (open, toggle, edit) = (self.clone(), self.clone(), self.clone());
                Today::new(
                    move |path, line| open.open_line(path, line),
                    move |path, line, status| toggle.toggle_task(path, line, status),
                    move |path, line| edit.edit_task(path, line),
                )
            };
            let board = {
                let (open, toggle, edit, moved) = (self.clone(), self.clone(), self.clone(), self.clone());
                Board::new(board::Handlers {
                    open: Rc::new(move |path, line| open.open_line(path, line)),
                    toggle: Rc::new(move |path, line, status| toggle.toggle_task(path, line, status)),
                    edit: Rc::new(move |path, line| edit.edit_task(path, line)),
                    moved: Rc::new(move |task, drop| moved.move_task(task, drop)),
                })
            };
            let home = {
                let (open, search) = (self.clone(), self.clone());
                HomePage::new(home::Handlers {
                    open: Box::new(move |path| open.open(path, None)),
                    search: Box::new(move |text| search.home_search(text)),
                })
            };
            // As wide as the page on screen needs, not as the widest page
            // (the board): a split pane may be narrow.
            let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).hhomogeneous(false).build();
            stack.add_named(&home.widget, Some("empty"));
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
            stack.add_named(&self.choose_page(), Some("choose"));
            Tab {
                stack,
                reader,
                editor,
                today,
                board,
                home,
                back: RefCell::new(Vec::new()),
                forward: RefCell::new(Vec::new()),
                title: RefCell::new(("Home".into(), String::new())),
                crumb: RefCell::new(None),
                group: group.clone(),
            }
        });
        {
            let (this, weak) = (self.clone(), Rc::downgrade(&tab));
            tab.reader.connect_open(move |path, anchor, place| {
                let Some(tab) = weak.upgrade() else { return };
                match place {
                    OpenIn::NewTab => this.open_in_new_tab(path, anchor),
                    OpenIn::OtherPane => this.open_in_other_pane(&tab, path, anchor),
                    // A link opens in the pane it is in, which the click focused.
                    OpenIn::Here => this.go_in(&tab, path, anchor, None, true),
                }
            });
        }
        self.follow_focus(&tab);
        {
            // Home is greyed out while the pane already shows the home page.
            let (this, weak) = (self.clone(), Rc::downgrade(&tab));
            tab.stack.connect_visible_child_name_notify(move |_| {
                if let Some(tab) = weak.upgrade()
                    && this.is_chosen(&tab)
                {
                    this.update_history();
                }
            });
        }
        {
            let toasts = self.toasts.clone();
            tab.reader.connect_notice(move |text| toasts.add_toast(adw::Toast::new(text)));
        }
        {
            let this = self.clone();
            tab.reader.connect_edit_task(move |path, line| this.edit_task(path, line));
        }
        {
            let this = self.clone();
            tab.reader.connect_tag(move |tag| this.palette(Mode::Text, &format!("#{tag}")));
        }
        {
            let (this, weak) = (self.clone(), Rc::downgrade(&tab));
            tab.reader.connect_section(move |line| {
                if weak.upgrade().is_some_and(|tab| this.is_chosen(&tab)) {
                    this.inspector.set_section(line);
                }
            });
        }
        {
            let (this, weak) = (self.clone(), Rc::downgrade(&tab));
            tab.reader.connect_create(move |target, from| {
                let (Some(store), Some(tab)) = (this.store.borrow().clone(), weak.upgrade()) else { return };
                match store.create_note_for_link(target, from) {
                    Ok(path) => {
                        this.sidebar.set_tree(&store.tree());
                        this.sidebar.set_tags(&store.tag_tree());
                        this.go_in(&tab, &path, None, None, true);
                        if this.is_chosen(&tab) {
                            this.toggle_edit();
                        }
                    }
                    Err(error) => this.toasts.add_toast(adw::Toast::new(&format!("The note was not made: {error}"))),
                }
            });
        }
        {
            // The reading cursor is kept as the note's place, and is reading time for WakaTime.
            let (store, settings) = (self.store.clone(), self.settings.clone());
            tab.reader.connect_cursor(move |path, line| {
                let Some(store) = store.borrow().clone() else { return };
                store.places.set(path, line);
                crate::wakatime::heartbeat(&settings.borrow().file, &store.root, path, false);
            });
        }
        {
            // Neovim's mode beside "Editing", while the editor is on screen.
            let (this, weak) = (self.clone(), Rc::downgrade(&tab));
            tab.editor.connect_mode(move |mode| {
                if let Some(tab) = weak.upgrade().filter(|tab| tab.editing()) {
                    this.set_subtitle(&tab, &mode.map_or("Editing".to_string(), |mode| format!("Editing · {mode}")));
                }
            });
        }
        let store = self.store.borrow().clone();
        if let Some(store) = &store {
            tab.reader.set_store(store.clone());
            tab.editor.set_store(store.clone());
        }
        self.tabs.borrow_mut().push(tab.clone());
        tab
    }

    /// The focused pane of the chosen tab.
    fn tab(&self) -> Rc<Tab> {
        let tabs = self.tabs.borrow().clone();
        let chosen = self.tab_view.selected_page().map(|page| page.child());
        let in_chosen: Vec<&Rc<Tab>> = tabs.iter().filter(|tab| tab.group.holds(chosen.as_ref())).collect();
        in_chosen
            .iter()
            .find(|tab| tab.group.is_focused(tab))
            .or_else(|| in_chosen.first())
            .map(|tab| (*tab).clone())
            .or_else(|| tabs.first().cloned())
            .expect("a window has a tab")
    }

    /// Is [tab] the pane the header and the commands follow?
    fn is_chosen(&self, tab: &Tab) -> bool {
        let chosen = self.tab_view.selected_page().map(|page| page.child());
        tab.group.holds(chosen.as_ref()) && (tab.group.is_focused(tab) || self.panes_of(&tab.group).len() < 2)
    }

    /// The panes of [group], the left one first.
    fn panes_of(&self, group: &Rc<Group>) -> Vec<Rc<Tab>> {
        let mut panes: Vec<Rc<Tab>> = self.tabs.borrow().iter().filter(|tab| Rc::ptr_eq(&tab.group, group)).cloned().collect();
        panes.sort_by_key(|tab| !tab.is_start());
        panes
    }

    /// The tabs in the order the bar shows them, each with its panes.
    fn tabs_in_order(&self) -> Vec<Vec<Rc<Tab>>> {
        let tabs = self.tabs.borrow().clone();
        (0..self.tab_view.n_pages())
            .filter_map(|at| {
                let child = self.tab_view.nth_page(at).child();
                let group = tabs.iter().find(|tab| tab.group.holds(Some(&child)))?.group.clone();
                Some(self.panes_of(&group))
            })
            .collect()
    }

    /// Another tab is chosen: the header, the history and the inspector follow it.
    fn tab_selected(&self) {
        if self.tabs.borrow().is_empty() {
            return;
        }
        let tab = self.tab();
        let (title, subtitle) = tab.title.borrow().clone();
        self.title.set(&title, crumb_path(&tab).as_deref());
        self.title.set_subtitle(&subtitle);
        if let Some(action) = self.window.lookup_action("edit").and_downcast::<gio::SimpleAction>() {
            action.set_state(&tab.editing().to_variant());
        }
        self.update_history();
        self.refresh_inspector();
    }

    /// A tab's close button, Ctrl+W or a middle click: the editor saves
    /// first, and the last tab closes the window.
    fn close_page(&self, page: &adw::TabPage) -> glib::Propagation {
        let child = page.child();
        let Some(group) = self.tabs.borrow().iter().find(|tab| tab.group.holds(Some(&child))).map(|tab| tab.group.clone()) else {
            return glib::Propagation::Proceed;
        };
        if self.tab_view.n_pages() <= 1 {
            self.tab_view.close_page_finish(page, false);
            self.window.close();
            return glib::Propagation::Stop;
        }
        let panes = self.panes_of(&group);
        if panes.iter().any(|tab| tab.editing() && tab.editor.close() == Saved::ChangedOnDisk) {
            self.tab_view.close_page_finish(page, false);
            self.toasts.add_toast(adw::Toast::new("The note changed on disk; the tab stays open so the text is not lost."));
            return glib::Propagation::Stop;
        }
        self.tabs.borrow_mut().retain(|other| !Rc::ptr_eq(&other.group, &group));
        // The hidden side holds the tab, which holds it.
        group.spare.borrow_mut().take();
        glib::Propagation::Proceed
    }

    /// The title and the subtitle of [tab], in its tab and, while it is
    /// chosen, in the header.
    fn set_titles(&self, tab: &Tab, title: &str, subtitle: &str) {
        *tab.title.borrow_mut() = (title.to_string(), subtitle.to_string());
        // The bar names a tab after its focused pane.
        if tab.group.is_focused(tab)
            && let Some(page) = tab.group.page.borrow().as_ref()
        {
            page.set_title(title);
            page.set_tooltip(&glib::markup_escape_text(if subtitle.is_empty() { title } else { subtitle }));
        }
        if self.is_chosen(tab) {
            self.title.set(title, crumb_path(tab).as_deref());
            self.title.set_subtitle(subtitle);
        }
    }

    /// Under a note's breadcrumb while it is read: for a note of 50 words
    /// or more, its words and minutes to read at 220 a minute, as
    /// the Mac's breadcrumb has them.
    fn reading_subtitle(&self, path: &str) -> String {
        let Some(store) = self.store.borrow().clone() else { return String::new() };
        let words = Vault::is_note(path)
            .then(|| std::fs::read_to_string(store.root.join(path)).ok())
            .flatten()
            .map(|text| words_of(&text))
            .unwrap_or(0);
        if words < 50 {
            return String::new();
        }
        format!("{} words · {} min", grouped(words), (words as f64 / 220.0).round().max(1.0) as usize)
    }

    fn set_subtitle(&self, tab: &Tab, subtitle: &str) {
        let title = tab.title.borrow().0.clone();
        tab.title.borrow_mut().1 = subtitle.to_string();
        if self.is_chosen(tab) {
            self.title.set(&title, crumb_path(tab).as_deref());
            self.title.set_subtitle(subtitle);
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
        let tabs = self.tabs.borrow().clone();
        if !root.is_dir() {
            for tab in &tabs {
                tab.stack.set_visible_child_name("choose");
            }
            self.toasts.add_toast(adw::Toast::new(&format!("{} is not a folder.", root.display())));
            return;
        }
        if !tabs.iter().all(|tab| self.leave_editor_in(tab)) {
            return;
        }
        for tab in &tabs {
            tab.stack.set_visible_child_name("loading");
        }
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
            {
                let (names, show) = this.hidden_settings();
                store.set_hidden(&names, show);
            }
            let watcher = {
                let this = this.clone();
                FileWatcher::new(&folder, move |changed| this.files_changed(changed))
            };
            match watcher {
                Ok(watcher) => *this.watcher.borrow_mut() = Some(watcher),
                Err(error) => this.toasts.add_toast(adw::Toast::new(&format!("Changes on disk are not followed: {error}"))),
            }
            this.recent.borrow_mut().clear();
            this.sidebar.set_tree(&store.tree());
            this.sidebar.set_tags(&store.tag_tree());
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
            this.refresh_updated();
            this.plan_notifications();
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(page) = this.split.sidebar() {
                page.set_title(&name);
            }
            let tabs = this.tabs.borrow().clone();
            for tab in tabs {
                tab.reader.set_store(store.clone());
                tab.editor.set_store(store.clone());
                tab.back.borrow_mut().clear();
                tab.forward.borrow_mut().clear();
                this.show_home(&tab);
            }
            // A path in the vault, or a name as a link names it: `Inbox`.
            let open = open.and_then(|p| {
                let index = store.index();
                if index.files.contains(&p) { Some(p) } else { index.resolve(&p, "") }
            });
            match open {
                Some(path) => this.open(&path, None),
                // The tabs the last window had, when this is the only one.
                None if this.window.application().is_some_and(|app| app.windows().len() == 1) => this.restore_tabs(&store),
                None => {}
            }
        });
    }

    /// The home page in [tab]: the time, a field, the notes read last.
    fn show_home(&self, tab: &Rc<Tab>) {
        let Some(store) = self.store.borrow().clone() else { return };
        let image = self.settings.borrow().file.string("home.image").unwrap_or_default();
        tab.home.refresh(&store, &image, &self.recent.borrow());
        tab.stack.set_visible_child_name("empty");
        *tab.crumb.borrow_mut() = None;
        self.set_titles(tab, "Home", "");
        if self.is_chosen(tab) {
            self.update_history();
            self.refresh_inspector();
        }
    }

    /// Return in the home page's field: the best match opens here; with no
    /// text, or no match, Quick Open takes over with the text.
    fn home_search(&self, text: &str) {
        let Some(store) = self.store.borrow().clone() else { return };
        // Out of the borrow before the note opens, which adds to the list.
        let recent = self.recent.borrow().clone();
        if !text.is_empty()
            && let Some(best) = palette::best(&store, text, &recent)
        {
            self.open(&best, None);
            return;
        }
        self.palette(Mode::Files, text);
    }

    /// Opens a note or a base in the reader of the chosen tab, and any other
    /// file in the app the system gives it to.
    pub fn open(&self, path: &str, anchor: Option<&str>) {
        self.go(path, anchor, None, true);
    }

    /// Opens the note at [path] with its [line] in view, as a task's link does.
    pub fn open_line(&self, path: &str, line: usize) {
        self.go(path, None, Some(line), true);
    }

    /// A link Ctrl+clicked or middle-clicked, or "Open in New Tab".
    fn open_in_new_tab(&self, path: &str, anchor: Option<&str>) {
        if self.store.borrow().is_none() {
            return;
        }
        if !Vault::opens_in_app(path) {
            self.open(path, anchor);
            return;
        }
        let tab = self.new_tab();
        self.go_in(&tab, path, anchor, None, false);
    }

    /// Ctrl+T: a tab on the note on screen, or on the home page, as on the Mac.
    fn new_tab_here(&self) {
        let current = self.tab().current();
        let tab = self.new_tab();
        match current {
            Some(path) => self.go_in(&tab, &path, None, None, false),
            None => tab.home.focus(),
        }
    }

    /// Saves and closes the chosen tab's editor; false when the save was
    /// refused, and the editor stays so the person's text is not lost.
    fn leave_editor(&self) -> bool {
        self.leave_editor_in(&self.tab())
    }

    fn leave_editor_in(&self, tab: &Rc<Tab>) -> bool {
        if !tab.editing() {
            return true;
        }
        let path = tab.editor.path();
        if tab.editor.close() == Saved::ChangedOnDisk {
            return false;
        }
        if self.is_chosen(tab) {
            self.set_edit_state(false);
        }
        // Back to the note's folder under its title, as reading shows it.
        if let Some(path) = path.or_else(|| tab.reader.shown().map(|shown| shown.path)) {
            self.set_subtitle(tab, &self.reading_subtitle(&path));
        }
        true
    }

    /// Ctrl+E: the note on screen in the editor, at the line read last; again,
    /// back to reading it where the cursor is.
    fn toggle_edit(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tab = self.tab();
        if tab.editing() {
            let line = tab.editor.line();
            let Some(path) = tab.editor.path() else { return };
            if !self.leave_editor_in(&tab) {
                return;
            }
            tab.stack.set_visible_child_name("reader");
            tab.reader.show(Request { path, heading: None, line: Some(line) }, false);
            return;
        }
        let Some(shown) = tab.reader.shown().filter(|_| tab.showing().as_deref() == Some("reader")) else { return };
        if !Vault::is_note(&shown.path) {
            self.toasts.add_toast(adw::Toast::new("Only a note can be edited here."));
            return;
        }
        if !self.one_editor(&tab) {
            return;
        }
        if !tab.editor.load(&store.root, &shown.path, shown.line) {
            self.toasts.add_toast(adw::Toast::new(&format!("{} could not be read.", shown.path)));
            return;
        }
        tab.stack.set_visible_child_name("editor");
        self.set_edit_state(true);
    }

    /// The edit action's state, and "Editing" under the title while it is on.
    fn set_edit_state(&self, on: bool) {
        if let Some(action) = self.window.lookup_action("edit").and_downcast::<gio::SimpleAction>() {
            action.set_state(&on.to_variant());
        }
        if on {
            self.set_subtitle(&self.tab(), "Editing");
        }
    }

    /// The Today page, drawn from the index as it is now.
    fn show_today(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        let tab = self.tab();
        tab.today.show(&store);
        *tab.crumb.borrow_mut() = None;
        self.set_titles(&tab, "Today", "");
        tab.stack.set_visible_child_name("today");
        self.split.set_show_content(true);
    }

    /// Ctrl+2: the task board, drawn from the index as it is now.
    fn show_board(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor() {
            return;
        }
        let tab = self.tab();
        tab.board.show(store);
        *tab.crumb.borrow_mut() = None;
        self.set_titles(&tab, "Board", "");
        tab.stack.set_visible_child_name("board");
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
        let tab = self.tab();
        match tab.showing().as_deref() {
            Some("today") => self.show_today(),
            Some("board") => self.show_board(),
            Some("reader") => tab.reader.reload(),
            _ => {}
        }
    }

    /// Opens [path] in the chosen tab; with [record], the note it leaves is
    /// one Back away.
    fn go(&self, path: &str, anchor: Option<&str>, line: Option<usize>, record: bool) {
        self.go_in(&self.tab(), path, anchor, line, record);
    }

    fn go_in(&self, tab: &Rc<Tab>, path: &str, anchor: Option<&str>, line: Option<usize>, record: bool) {
        let Some(store) = self.store.borrow().clone() else { return };
        if !self.leave_editor_in(tab) {
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
        // The note left and the note shown are both read as they are now.
        if let Some(current) = tab.current() {
            store.places.mark_read(&current);
        }
        if Vault::is_note(path) {
            store.places.mark_read(path);
        }
        if record && let Some(current) = tab.current().filter(|current| current != path) {
            tab.back.borrow_mut().push(current);
            tab.forward.borrow_mut().clear();
        }
        *tab.crumb.borrow_mut() = Some(path.to_string());
        self.set_titles(tab, &Vault::title(path), &self.reading_subtitle(path));
        tab.stack.set_visible_child_name("reader");
        self.split.set_show_content(true);
        // A note opens where it was being read, unless a link names a place in it.
        let line = line.or_else(|| anchor.is_none().then(|| self.place(path)).flatten());
        tab.reader.show(Request { path: path.to_string(), heading: anchor.map(String::from), line }, false);
        // The voice reads the note it started on, and stops when its tab leaves it.
        crate::read_aloud::left(&tab.reader.view, Some(path));
        crate::wakatime::heartbeat(&self.settings.borrow().file, &store.root, path, false);
        if self.is_chosen(tab) {
            self.update_history();
            self.refresh_inspector();
        }
        self.refresh_updated();
    }

    /// Back when [backward], else Forward: the notes that are gone are skipped.
    fn travel(&self, backward: bool) {
        let tab = self.tab();
        let (from, to) = if backward { (&tab.back, &tab.forward) } else { (&tab.forward, &tab.back) };
        let Some(store) = self.store.borrow().clone() else { return };
        loop {
            let Some(path) = from.borrow_mut().pop() else { break };
            if !store.index().files.contains(&path) {
                continue;
            }
            if let Some(current) = tab.current() {
                to.borrow_mut().push(current);
            }
            self.go_in(&tab, &path, None, None, false);
            return;
        }
        self.update_history();
    }

    fn update_history(&self) {
        let tab = self.tab();
        if let Some(action) = self.window.lookup_action("back").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(!tab.back.borrow().is_empty());
        }
        if let Some(action) = self.window.lookup_action("forward").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(!tab.forward.borrow().is_empty());
        }
        if let Some(action) = self.window.lookup_action("home").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(tab.showing().as_deref() != Some("empty"));
        }
        self.refresh_periodic_nav(&tab);
    }

    /// The periodic controls for the note in [tab]: shown on a daily,
    /// weekly or monthly note of the vault's series, as the Mac's breadcrumb
    /// has them.
    fn refresh_periodic_nav(&self, tab: &Rc<Tab>) {
        let nav = &self.periodic_nav;
        let store = self.store.borrow().clone();
        let current = tab.current();
        let Some((store, path)) = store.zip(current).filter(|(store, path)| store.periodic.period_of(path).is_some()) else {
            nav.widget.set_visible(false);
            // Enabled away from a periodic note: the keys say why nothing opens.
            for name in ["periodic-previous", "periodic-next"] {
                if let Some(action) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                    action.set_enabled(true);
                }
            }
            return;
        };
        let periodic = &store.periodic;
        let index = store.index();
        let earlier = periodic.nearest(&index.files, &path, -1);
        let later = periodic.nearest(&index.files, &path, 1);
        let years = periodic.years_ago(&index.files, &path);
        drop(index);
        let tip = |found: &Option<String>, word: &str, keys: &str| match found {
            Some(note) => format!("{word}: {} ({keys})", Vault::title(note)),
            None => format!("No {} note", word.to_lowercase().replace("previous", "earlier").replace("next", "later")),
        };
        nav.previous.set_tooltip_text(Some(&tip(&earlier, "Previous", "Ctrl+Alt+Left")));
        nav.next.set_tooltip_text(Some(&tip(&later, "Next", "Ctrl+Alt+Right")));
        for (name, found) in [("periodic-previous", &earlier), ("periodic-next", &later)] {
            if let Some(action) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_enabled(found.is_some());
            }
        }
        let label = gtk::Box::builder().spacing(6).build();
        label.append(&gtk::Image::from_icon_name("x-office-calendar-symbolic"));
        label.append(&gtk::Label::new(Some(periodic.period.noun())));
        nav.menu.set_child(Some(&label));
        nav.menu.set_tooltip_text(Some("This period's note, and the same time in earlier years"));
        let menu = gio::Menu::new();
        menu.append(Some(periodic.period.current()), Some("win.periodic"));
        if !years.is_empty() {
            let earlier_years = gio::Menu::new();
            for (back, note) in &years {
                let when = if *back == 1 { "A year ago".to_string() } else { format!("{back} years ago") };
                let item = gio::MenuItem::new(Some(&format!("{when}: {}", Vault::title(note))), None);
                item.set_action_and_target_value(Some("win.open-note"), Some(&note.to_variant()));
                earlier_years.append_item(&item);
            }
            menu.append_section(Some("Earlier years"), &earlier_years);
        }
        nav.menu.set_menu_model(Some(&menu));
        nav.widget.set_visible(true);
    }

    /// Every tab's reader drawn again, as a new text size or font asks.
    /// The notes being edited, loaded again where they were: Neovim starts
    /// or stops as the settings now say. With Neovim, a split keeps one editor.
    fn restart_editors(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tabs = self.tabs.borrow().clone();
        for tab in &tabs {
            if !tab.editing() || !self.one_editor(tab) {
                continue;
            }
            let Some(path) = tab.editor.path() else { continue };
            tab.editor.load(&store.root, &path, Some(tab.editor.line()));
        }
    }

    /// `files.hidden` and `files.show_hidden`, as the settings file has them.
    fn hidden_settings(&self) -> (Vec<String>, bool) {
        let settings = self.settings.borrow();
        let names = settings.file.strings("files.hidden").unwrap_or_else(|| daftar_core::HIDDEN_NAMES.map(String::from).to_vec());
        (names, settings.file.bool("files.show_hidden").unwrap_or(false))
    }

    /// The hidden files read again from the settings: the tree, the lists
    /// and the pages follow.
    fn refresh_hidden(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let (names, show) = self.hidden_settings();
        store.set_hidden(&names, show);
        if let Some(action) = self.window.lookup_action("show-hidden").and_downcast::<gio::SimpleAction>() {
            action.set_state(&show.to_variant());
        }
        self.sidebar.set_tree(&store.tree());
        self.sidebar.set_tags(&store.tag_tree());
        self.refresh_updated();
        self.refresh_homes();
        self.refresh_inspector();
    }

    /// Ctrl+Shift+.: the hidden and the excluded files shown in the lists, or not.
    fn toggle_hidden(&self) {
        let show = !self.hidden_settings().1;
        let result = self.settings.borrow_mut().set("files.show_hidden", daftar_core::Value::Bool(show));
        if let Err(error) = result {
            self.toasts.add_toast(adw::Toast::new(&format!("The settings were not saved: {error}")));
        }
        self.refresh_hidden();
    }

    fn reload_readers(&self) {
        let tabs = self.tabs.borrow().clone();
        for tab in &tabs {
            tab.reader.reload();
        }
    }

    /// Every home page drawn again, as a new picture asks.
    fn refresh_homes(&self) {
        let tabs = self.tabs.borrow().clone();
        for tab in tabs {
            if tab.showing().as_deref() == Some("empty") {
                self.show_home(&tab);
            }
        }
    }

    /// The reader's text size one [step] bigger or smaller, or 14 again for
    /// 0, kept in the config as the Mac keeps it: 10 to 28.
    fn text_size(&self, step: i64) {
        let next = if step == 0 { 14 } else { (self.settings.borrow().text_size() + step).clamp(10, 28) };
        if let Err(error) = self.settings.borrow_mut().set("appearance.text_size", Value::Int(next)) {
            self.toasts.add_toast(adw::Toast::new(&format!("The settings were not saved: {error}")));
        }
        self.reload_readers();
    }

    /// Files changed on disk, by another app, `git` or this one: the index
    /// learns them, and each tab's page is drawn again when it, or a note it
    /// links to or embeds, is one of them.
    fn files_changed(&self, changed: BTreeSet<String>) {
        let Some(store) = self.store.borrow().clone() else { return };
        let (files, folders) = {
            let index = store.index();
            (index.files.clone(), index.empty_folders.clone())
        };
        store.refresh(&changed.iter().cloned().collect::<Vec<_>>());
        self.refresh_updated();
        self.git.refresh();
        self.plan_notifications();
        self.refresh_inspector();
        let moved = {
            let index = store.index();
            index.files != files || index.empty_folders != folders
        };
        if moved {
            self.sidebar.set_tree(&store.tree());
        }
        // An edit can add or drop a tag in a note that stays.
        self.sidebar.set_tags(&store.tag_tree());
        if moved {
            self.refresh_marks();
        }
        let tabs = self.tabs.borrow().clone();
        for tab in tabs {
            self.follow_disk(&tab, &store, &changed);
        }
    }

    /// [tab]'s page after [changed] files changed on disk.
    fn follow_disk(&self, tab: &Rc<Tab>, store: &Rc<VaultStore>, changed: &BTreeSet<String>) {
        match tab.showing().as_deref() {
            Some("today") => tab.today.show(store),
            Some("board") => tab.board.show(store.clone()),
            Some("empty") => self.show_home(tab),
            Some("editor") => {
                if tab.editor.path().is_some_and(|path| changed.contains(&path)) {
                    tab.editor.changed_on_disk();
                }
            }
            Some("reader") => {
                let Some(shown) = tab.reader.shown() else { return };
                let index = store.index();
                if !index.files.contains(&shown.path) {
                    drop(index);
                    self.show_home(tab);
                    return;
                }
                let touched =
                    changed.contains(&shown.path) || index.resolved_links(&shown.path).values().any(|target| changed.contains(target));
                drop(index);
                if touched {
                    tab.reader.reload();
                }
            }
            _ => {}
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
            command("Home", "Go", "win.home", "Ctrl+Shift+D"),
            command("Back", "Go", "win.back", "Ctrl+["),
            command("Forward", "Go", "win.forward", "Ctrl+]"),
            command("New Tab", "Tab", "win.new-tab", "Ctrl+T"),
            command("Close Tab", "Tab", "win.close-tab", "Ctrl+W"),
            command("Split Right or Close the Split", "View", "win.split", "Ctrl+\\"),
            command("Focus the Other Pane", "View", "win.other-pane", "Ctrl+Alt+\\"),
            command("Edit or Read the Note", "Note", "win.edit", "Ctrl+E"),
            command("Show the Graph", "View", "win.graph", "Ctrl+Alt+G"),
            command("Show or Hide the Inspector", "View", "win.inspector", "Ctrl+Alt+I"),
            command("New Note", "File", "win.new-note::", "Ctrl+N"),
            command("New Folder", "File", "win.new-folder::", "Ctrl+Shift+N"),
            command("Rename", "File", "win.rename::", "F2"),
            command("Extract Selection to New Note", "Note", "win.extract", "Ctrl+Alt+X"),
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
        commands.extend(self.vault_tool_commands());
        if self.git.button.is_visible() {
            commands.push(command("Sync Now", "Git", "win.sync", "Ctrl+Shift+S"));
            commands.push(command("Pull", "Git", "win.pull", ""));
        }
        commands.extend(self.extra_commands());
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
        let center = self.tab().note_in_view().filter(|p| Vault::is_note(p));
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
                let this = self.clone();
                move || this.reload_readers()
            }),
            home: Box::new({
                let this = self.clone();
                move || this.refresh_homes()
            }),
            git: Box::new({
                let git = self.git.clone();
                move |_, interval| git.reconfigure(interval)
            }),
            editor: Box::new({
                let this = self.clone();
                move || this.restart_editors()
            }),
            notifications: Box::new({
                let this = self.clone();
                move || this.plan_notifications()
            }),
            hidden: Box::new({
                let this = self.clone();
                move || this.refresh_hidden()
            }),
            notice: Box::new({
                let toasts = self.toasts.clone();
                move |text| toasts.add_toast(adw::Toast::new(text))
            }),
        };
        let root = self.store.borrow().as_ref().map(|store| store.root.clone());
        crate::preferences::present(&self.window, self.settings.clone(), root, Rc::new(hooks));
    }

    fn palette(&self, mode: Mode, query: &str) {
        let Some(store) = self.store.borrow().clone() else { return };
        let this = self.clone();
        // With nothing typed, the notes read last: here, then in the last
        // sessions, as the home page lists them.
        let recent = {
            let files = &store.index().files;
            daftar_core::home::recent(
                &self.recent.borrow(),
                &store.places.recently_read(40),
                |p| files.contains(p) && !store.is_hidden(p),
                40,
            )
        };
        palette::present(&self.window, mode, store, recent, query, move |path| this.open(path, None));
    }

    /// The tabs' notes, kept for the next launch: `tabs-<hash>.json` beside
    /// the reading places, the chosen one marked. `splits` has the right
    /// pane's note of a split tab (`""` on its home page), else null, and
    /// `focused` the side each tab's commands acted on.
    fn save_tabs(&self) {
        let Some(store) = self.store.borrow().clone() else { return };
        let tabs = self.tabs_in_order();
        let chosen = self.tab();
        let note = |tab: &Rc<Tab>| tab.current().unwrap_or_default();
        let paths: Vec<String> = tabs.iter().map(|panes| note(&panes[0])).collect();
        let splits: Vec<Option<String>> = tabs.iter().map(|panes| panes.get(1).map(note)).collect();
        let focused: Vec<usize> = tabs.iter().map(|panes| panes.iter().position(|tab| tab.group.is_focused(tab)).unwrap_or(0)).collect();
        let at = tabs.iter().position(|panes| Rc::ptr_eq(&panes[0].group, &chosen.group)).unwrap_or(0);
        let file = crate::places::state_file(&store.root, "tabs");
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(&file, json!({ "tabs": paths, "splits": splits, "focused": focused, "chosen": at }).to_string());
    }

    /// The tabs [save_tabs] kept, each on its note while it is still there.
    fn restore_tabs(&self, store: &Rc<VaultStore>) {
        let Ok(text) = std::fs::read_to_string(crate::places::state_file(&store.root, "tabs")) else { return };
        let Ok(saved) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        let paths: Vec<String> = saved
            .get("tabs")
            .and_then(|t| t.as_array())
            .map(|t| t.iter().filter_map(|p| p.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let chosen = saved.get("chosen").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
        let at_index = |key: &str, at: usize| saved.get(key).and_then(|list| list.get(at)).cloned();
        for (at, path) in paths.iter().enumerate().take(20) {
            let tab = if at == 0 { self.tab() } else { self.new_tab() };
            if store.index().files.contains(path) {
                self.go_in(&tab, path, None, None, false);
            }
            let Some(right) = at_index("splits", at).and_then(|split| split.as_str().map(String::from)) else { continue };
            let other = self.split_pane(&tab);
            if store.index().files.contains(&right) {
                self.go_in(&other, &right, None, None, false);
            } else {
                self.show_home(&other);
            }
            let focused = at_index("focused", at).and_then(|f| f.as_u64()).unwrap_or(0);
            self.focus_pane(if focused == 1 { &other } else { &tab }, false);
        }
        let page = self.tabs_in_order().get(chosen).and_then(|panes| panes[0].group.page.borrow().clone());
        if let Some(page) = page {
            self.tab_view.set_selected_page(&page);
        }
    }

    fn add_actions(&self) {
        self.add_file_actions();
        self.add_vault_tools();
        self.add_extra_actions();
        self.add_split_actions();
        let quick_open = gio::SimpleAction::new("quick-open", None);
        let this = self.clone();
        quick_open.connect_activate(move |_, _| this.palette(Mode::Files, ""));
        self.window.add_action(&quick_open);
        let search = gio::SimpleAction::new("search", None);
        let this = self.clone();
        search.connect_activate(move |_, _| this.palette(Mode::Text, ""));
        self.window.add_action(&search);
        for (name, action) in [("find", "open"), ("find-next", "next"), ("find-previous", "previous")] {
            let find = gio::SimpleAction::new(name, None);
            let this = self.clone();
            find.connect_activate(move |_, _| this.tab().reader.find(action));
            self.window.add_action(&find);
        }
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
        let hidden = gio::SimpleAction::new_stateful("show-hidden", None, &self.hidden_settings().1.to_variant());
        let this = self.clone();
        hidden.connect_activate(move |_, _| this.toggle_hidden());
        self.window.add_action(&hidden);
        let edit = gio::SimpleAction::new_stateful("edit", None, &false.to_variant());
        let this = self.clone();
        edit.connect_activate(move |_, _| this.toggle_edit());
        self.window.add_action(&edit);
        let save = gio::SimpleAction::new("save", None);
        let this = self.clone();
        save.connect_activate(move |_, _| {
            this.tab().editor.save();
        });
        self.window.add_action(&save);
        let board = gio::SimpleAction::new("board", None);
        let this = self.clone();
        board.connect_activate(move |_, _| {
            this.show_board();
            // Once the new cards are on screen, which a focus needs.
            let board = this.tab().board.clone();
            glib::idle_add_local_once(move || board.focus_cards());
        });
        self.window.add_action(&board);
        let today = gio::SimpleAction::new("today", None);
        let this = self.clone();
        today.connect_activate(move |_, _| this.show_today());
        self.window.add_action(&today);
        let home = gio::SimpleAction::new("home", None);
        let this = self.clone();
        // The home page in the focused pane, as on the Mac: Back returns to
        // the note that was open.
        home.connect_activate(move |_, _| {
            if this.leave_editor() {
                let tab = this.tab();
                if let Some(current) = tab.current() {
                    tab.back.borrow_mut().push(current);
                    tab.forward.borrow_mut().clear();
                }
                crate::read_aloud::left(&tab.reader.view, None);
                this.show_home(&tab);
                tab.home.focus();
            }
        });
        self.window.add_action(&home);
        let new_tab = gio::SimpleAction::new("new-tab", None);
        let this = self.clone();
        new_tab.connect_activate(move |_, _| this.new_tab_here());
        self.window.add_action(&new_tab);
        // A note by its path, here: the periodic menu's earlier years.
        let open_note = gio::SimpleAction::new("open-note", Some(glib::VariantTy::STRING));
        let this = self.clone();
        open_note.connect_activate(move |_, target| {
            if let Some(path) = target.and_then(|t| t.get::<String>()) {
                this.open(&path, None);
            }
        });
        self.window.add_action(&open_note);
        let open_tab = gio::SimpleAction::new("open-tab", Some(glib::VariantTy::STRING));
        let this = self.clone();
        open_tab.connect_activate(move |_, target| {
            if let Some(path) = target.and_then(|t| t.get::<String>()) {
                this.open_in_new_tab(&path, None);
            }
        });
        self.window.add_action(&open_tab);
        let close_tab = gio::SimpleAction::new("close-tab", None);
        let this = self.clone();
        close_tab.connect_activate(move |_, _| {
            if let Some(page) = this.tab_view.selected_page() {
                this.tab_view.close_page(&page);
            }
        });
        self.window.add_action(&close_tab);
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

/// The words of a note's text, its properties left out.
fn words_of(text: &str) -> usize {
    let body = text.strip_prefix("---\n").and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..])).unwrap_or(text);
    body.split_whitespace().filter(|word| word.chars().any(char::is_alphanumeric)).count()
}

/// 12345 as "12,345".
fn grouped(number: usize) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The folder of a vault path, as the subtitle shows it under a note's title.
/// The note the breadcrumb is the path of.
fn crumb_path(tab: &Tab) -> Option<String> {
    tab.crumb.borrow().clone()
}

/// The Latin layout's key for a shortcut typed on another layout: Ctrl,
/// Alt or Super with a key whose letter is not Latin.
fn latin_shortcut_key(key: gtk::gdk::Key, keycode: u32, state: gtk::gdk::ModifierType) -> Option<gtk::gdk::Key> {
    use gtk::gdk::ModifierType;
    if !state.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK) {
        return None;
    }
    crate::neovim_session::latin_key(key, keycode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_leave_the_properties_and_marks_out() {
        assert_eq!(words_of("---\ntags: [a, b]\n---\n# Plan\n\nShip the plan - today."), 5);
        assert_eq!(words_of("one two\nthree"), 3);
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(12345), "12,345");
        assert_eq!(grouped(1234567), "1,234,567");
    }
}
