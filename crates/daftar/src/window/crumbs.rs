//! The header's title as the Mac's breadcrumb: the vault, each folder and
//! the note, with the subtitle under them. A folder is a menu of what it
//! holds, to go sideways; the note's own crumb has the file commands. A
//! page that is not a note (home, Today, the board) has a plain title.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use daftar_core::{TreeNode, Vault};

use crate::store::VaultStore;

pub(crate) struct TitleBar {
    pub widget: gtk::Box,
    crumbs: gtk::Box,
    subtitle: gtk::Label,
    store: Rc<RefCell<Option<Rc<VaultStore>>>>,
    /// The title and the path the crumbs were last made for.
    shown: RefCell<(String, Option<String>)>,
}

impl TitleBar {
    pub fn new(store: Rc<RefCell<Option<Rc<VaultStore>>>>) -> Rc<TitleBar> {
        let crumbs = gtk::Box::builder().spacing(0).halign(gtk::Align::Center).build();
        let subtitle = gtk::Label::builder().css_classes(["subtitle"]).ellipsize(gtk::pango::EllipsizeMode::End).visible(false).build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .css_classes(["title", "daftar-crumbs"])
            .build();
        widget.append(&crumbs);
        widget.append(&subtitle);
        Rc::new(TitleBar { widget, crumbs, subtitle, store, shown: RefCell::new((String::new(), None)) })
    }

    pub fn set_subtitle(&self, text: &str) {
        self.subtitle.set_text(text);
        self.subtitle.set_visible(!text.is_empty());
    }

    /// [title] alone, or as the last crumb of [path].
    pub fn set(&self, title: &str, path: Option<&str>) {
        // A path waits for the vault: until it is read, the title alone.
        let path = path.filter(|_| self.store.borrow().is_some());
        let now = (title.to_string(), path.map(String::from));
        if *self.shown.borrow() == now {
            return;
        }
        *self.shown.borrow_mut() = now;
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        let store = self.store.borrow().clone();
        let (Some(path), Some(store)) = (path, store) else {
            self.crumbs
                .append(&gtk::Label::builder().label(title).css_classes(["title"]).ellipsize(gtk::pango::EllipsizeMode::End).build());
            return;
        };
        let parts: Vec<&str> = path.split('/').collect();
        let vault = store.root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.crumbs.append(&self.folder_crumb(&store, "", &vault, "user-bookmarks-symbolic"));
        for at in 0..parts.len().saturating_sub(1) {
            self.crumbs.append(&separator());
            let folder = parts[..=at].join("/");
            self.crumbs.append(&self.folder_crumb(&store, &folder, parts[at], "folder-symbolic"));
        }
        self.crumbs.append(&separator());
        self.crumbs.append(&note_crumb(&store, path, title));
    }

    /// A folder: its name, and a menu of what is in it, made when it opens.
    fn folder_crumb(&self, store: &Rc<VaultStore>, folder: &str, name: &str, icon: &str) -> gtk::MenuButton {
        let content = gtk::Box::builder().spacing(4).build();
        content.append(&gtk::Image::from_icon_name(icon));
        content.append(
            &gtk::Label::builder()
                .label(name)
                .css_classes(["dim-label"])
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .max_width_chars(18)
                .build(),
        );
        let button = gtk::MenuButton::builder()
            .child(&content)
            .css_classes(["flat", "daftar-crumb"])
            .tooltip_text(if folder.is_empty() { format!("What is at the top of {name}") } else { format!("What is in {name}") })
            .build();
        let (store, folder, name) = (store.clone(), folder.to_string(), name.to_string());
        button.set_create_popup_func(move |button| {
            let tree = store.tree();
            let menu = gio::Menu::new();
            let note = format!("{folder}/{name}.md");
            if !folder.is_empty() && store.index().files.contains(&note) {
                let open = gio::Menu::new();
                open.append_item(&item(&format!("Open {name}"), "win.open-note", &note));
                menu.append_section(None, &open);
            }
            let overview = gio::Menu::new();
            overview.append_item(&item("Overview", "win.open-note", &format!("{}/", folder)));
            menu.append_section(None, &overview);
            let entries = gio::Menu::new();
            for node in Vault::folder(&folder, &tree).unwrap_or(&[]) {
                entry(&entries, node);
            }
            menu.append_section(None, &entries);
            button.set_menu_model(Some(&menu));
        });
        button
    }
}

/// One row of a folder's menu: a note opens, a folder is a submenu.
fn entry(menu: &gio::Menu, node: &TreeNode) {
    if node.is_folder() {
        let inner = gio::Menu::new();
        if let Some(note) = &node.folder_note {
            let open = gio::Menu::new();
            open.append_item(&item(&format!("Open {}", node.name), "win.open-note", note));
            inner.append_section(None, &open);
        }
        let rest = gio::Menu::new();
        for child in node.children.as_deref().unwrap_or(&[]) {
            entry(&rest, child);
        }
        inner.append_section(None, &rest);
        menu.append_submenu(Some(&node.name), &inner);
    } else {
        menu.append_item(&item(&node.name, "win.open-note", &node.path));
    }
}

/// The note: its name, bold, and the commands on the file.
fn note_crumb(store: &Rc<VaultStore>, path: &str, title: &str) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let change = gio::Menu::new();
    change.append_item(&item("Rename…", "win.rename", path));
    change.append_item(&item("Move to…", "win.move", path));
    let bookmarked = store.bookmarks.borrow().contains(path);
    change.append_item(&item(if bookmarked { "Remove Bookmark" } else { "Bookmark" }, "win.bookmark", path));
    menu.append_section(None, &change);
    let copy = gio::Menu::new();
    copy.append_item(&item("Copy Link", "win.copy-text", &format!("[[{}]]", Vault::title(path))));
    copy.append_item(&item("Copy Path", "win.copy-text", path));
    copy.append_item(&item("Show in Files", "win.reveal", path));
    menu.append_section(None, &copy);
    let label = gtk::Label::builder().label(title).css_classes(["heading"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    gtk::MenuButton::builder()
        .child(&label)
        .menu_model(&menu)
        .css_classes(["flat", "daftar-crumb", "daftar-crumb-note"])
        .tooltip_text("The commands on this note")
        .build()
}

fn separator() -> gtk::Image {
    let image = gtk::Image::from_icon_name("go-next-symbolic");
    image.add_css_class("dim-label");
    image.set_pixel_size(10);
    image
}

fn item(label: &str, action: &str, target: &str) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
    item
}
