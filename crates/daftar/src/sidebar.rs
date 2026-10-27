//! The folder tree, as Obsidian's file explorer shows it: folders first, a
//! folder note opens with its folder. Above it, the vault's bookmarks; on
//! each row, the Iconic plugin's icon and color, and a menu of the file
//! commands on a right click.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use daftar_core::{BookmarkEntry, Canvas, Icon, Iconic, TagNode, TreeNode, Vault};

pub struct Sidebar {
    pub widget: gtk::Box,
    root: gio::ListStore,
    model: gtk::TreeListModel,
    marks: gtk::ListBox,
    marks_section: gtk::Box,
    /// The notes changed on disk since they were read, as the Mac lists them.
    updated: gtk::ListBox,
    updated_title: gtk::Label,
    read_all: gtk::Button,
    updated_section: gtk::Box,
    /// The vault's tags as a tree, under the files, as on the Mac.
    tags: gio::ListStore,
    tags_title: gtk::Label,
    tags_section: gtk::Box,
    shared: Rc<Shared>,
}

/// Hears the tag a click picks.
type TagHandler = Rc<dyn Fn(&str)>;
/// Hears the vault paths dropped on a folder, and the folder.
type MoveHandler = Rc<dyn Fn(Vec<String>, &str)>;

/// What the rows read when they are drawn.
#[derive(Default)]
struct Shared {
    iconic: RefCell<Iconic>,
    bookmarked: RefCell<HashSet<String>>,
    /// The vault's folder, for the file a dragged row carries.
    root: RefCell<Option<std::path::PathBuf>>,
    /// What a click on a tag does: the window lists its notes.
    on_tag: RefCell<Option<TagHandler>>,
    /// What a drop on a folder does: the window moves the files into it.
    on_move: RefCell<Option<MoveHandler>>,
}

fn node_of(item: &glib::Object) -> Option<TreeNode> {
    item.downcast_ref::<glib::BoxedAnyObject>().map(|boxed| boxed.borrow::<TreeNode>().clone())
}

fn store_of(nodes: &[TreeNode]) -> gio::ListStore {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for node in nodes {
        store.append(&glib::BoxedAnyObject::new(node.clone()));
    }
    store
}

fn icon(node: &TreeNode) -> &'static str {
    if node.is_folder() {
        "folder-symbolic"
    } else if Vault::is_base(&node.path) {
        "x-office-spreadsheet-symbolic"
    } else if Canvas::is_canvas(&node.path) {
        "view-grid-symbolic"
    } else if Vault::opens_in_app(&node.path) {
        "text-x-generic-symbolic"
    } else {
        "image-x-generic-symbolic"
    }
}

/// A logo as read: when its file changed, and its picture if it read.
type Read = (Option<SystemTime>, Option<gdk::Texture>);

thread_local! {
    /// Folder logos, read once and kept until the file changes.
    static LOGOS: RefCell<HashMap<PathBuf, Read>> = RefCell::new(HashMap::new());
}

/// The side a logo is drawn at, in pixels: twice the row's icon, so an SVG
/// stays sharp on a doubled screen.
const LOGO_SIZE: i32 = 32;

/// The picture of the logo at [file], or `None` when it does not read.
fn logo(file: &Path) -> Option<gdk::Texture> {
    let modified = std::fs::metadata(file).and_then(|m| m.modified()).ok();
    if let Some((at, texture)) = LOGOS.with(|logos| logos.borrow().get(file).cloned())
        && at == modified
    {
        return texture;
    }
    // A drawing is drawn at the size it is shown at; a picture as it is.
    #[allow(deprecated)]
    let texture = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(file, LOGO_SIZE, LOGO_SIZE, true)
        .ok()
        .map(|pixbuf| gdk::Texture::for_pixbuf(&pixbuf))
        .or_else(|| gdk::Texture::from_filename(file).ok());
    LOGOS.with(|logos| logos.borrow_mut().insert(file.to_path_buf(), (modified, texture.clone())));
    texture
}

/// Draws [icon] on a row's image and emoji label, or the default [fallback].
/// A folder's [logo] wins over both.
fn draw_icon(image: &gtk::Image, emoji: &gtk::Label, icon: Option<&Icon>, fallback: &str, logo_file: Option<PathBuf>) {
    for class in image.css_classes() {
        if class.starts_with("iconic-") || class == "folder-logo" {
            image.remove_css_class(&class);
        }
    }
    if let Some(texture) = logo_file.as_deref().and_then(logo) {
        emoji.set_visible(false);
        image.set_visible(true);
        image.set_paintable(Some(&texture));
        image.add_css_class("folder-logo");
        return;
    }
    match icon.and_then(|i| i.name.as_deref()).filter(|_| icon.is_some_and(Icon::is_emoji)) {
        Some(emoji_name) => {
            emoji.set_label(emoji_name);
            emoji.set_visible(true);
            image.set_visible(false);
        }
        None => {
            emoji.set_visible(false);
            image.set_visible(true);
            let name = icon.and_then(|i| i.name.as_deref()).and_then(lucide).unwrap_or(fallback);
            image.set_icon_name(Some(name));
            if let Some(class) = icon.and_then(|i| i.color.as_deref()).and_then(color_class) {
                image.add_css_class(&class);
            }
        }
    }
}

impl Sidebar {
    /// [on_open] gets the path of a note, a base or another file to open.
    pub fn new(on_open: impl Fn(&str) + 'static) -> Sidebar {
        install_colors();
        let on_open: Rc<dyn Fn(&str)> = Rc::new(on_open);
        let shared = Rc::new(Shared::default());
        let root = gio::ListStore::new::<glib::BoxedAnyObject>();
        let model = gtk::TreeListModel::new(root.clone(), false, false, |item| {
            let children = node_of(item)?.children?;
            Some(store_of(&children).upcast())
        });
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let factory = gtk::SignalListItemFactory::new();
        {
            let shared = shared.clone();
            factory.connect_setup(move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                // Rounded corners clip a logo, as the Mac draws it.
                row.append(&gtk::Image::builder().overflow(gtk::Overflow::Hidden).build());
                row.append(&gtk::Label::builder().visible(false).width_chars(2).build());
                row.append(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
                let expander = gtk::TreeExpander::new();
                expander.set_child(Some(&row));
                item.set_child(Some(&expander));
                // A right click: the file commands for this row.
                let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
                let (weak, marks) = (item.downgrade(), shared.clone());
                click.connect_pressed(move |gesture, _, x, y| {
                    let Some(item) = weak.upgrade() else { return };
                    let Some(node) = item.item().and_downcast::<gtk::TreeListRow>().and_then(|row| row.item()).as_ref().and_then(node_of)
                    else {
                        return;
                    };
                    let Some(widget) = gesture.widget() else { return };
                    let bookmarked = marks.bookmarked.borrow().contains(&node.path);
                    popup(&widget, &menu(&node.path, node.is_folder(), bookmarked), x, y);
                });
                expander.add_controller(click);
                // A drag: the row's file, which the editor takes as a link
                // and another app as the file.
                let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
                let (weak, dragged) = (item.downgrade(), shared.clone());
                drag.connect_prepare(move |_, _, _| {
                    let item = weak.upgrade()?;
                    let node = item.item().and_downcast::<gtk::TreeListRow>().and_then(|row| row.item()).as_ref().and_then(node_of)?;
                    let file = gio::File::for_path(dragged.root.borrow().as_ref()?.join(&node.path));
                    Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&[file]).to_value()))
                });
                expander.add_controller(drag);
                // A drop of the vault's files on a folder moves them into it.
                let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
                let (weak, shared) = (item.downgrade(), shared.clone());
                let folder_under = move || {
                    let item = weak.upgrade()?;
                    let node = item.item().and_downcast::<gtk::TreeListRow>().and_then(|row| row.item()).as_ref().and_then(node_of)?;
                    node.is_folder().then_some(node.path)
                };
                {
                    let folder_under = folder_under.clone();
                    drop.connect_enter(move |target, _, _| {
                        if folder_under().is_none() {
                            return gdk::DragAction::empty();
                        }
                        if let Some(widget) = target.widget() {
                            widget.add_css_class("daftar-drop");
                        }
                        gdk::DragAction::COPY
                    });
                }
                drop.connect_leave(|target| {
                    if let Some(widget) = target.widget() {
                        widget.remove_css_class("daftar-drop");
                    }
                });
                drop.connect_drop(move |target, value, _, _| {
                    if let Some(widget) = target.widget() {
                        widget.remove_css_class("daftar-drop");
                    }
                    let (Some(folder), Ok(files), Some(root)) =
                        (folder_under(), value.get::<gdk::FileList>(), shared.root.borrow().clone())
                    else {
                        return false;
                    };
                    let paths: Vec<String> = files
                        .files()
                        .iter()
                        .filter_map(|file| Some(file.path()?.strip_prefix(&root).ok()?.to_string_lossy().to_string()))
                        .collect();
                    let handler = shared.on_move.borrow().clone();
                    match handler {
                        Some(handler) if !paths.is_empty() => {
                            handler(paths, &folder);
                            true
                        }
                        _ => false,
                    }
                });
                expander.add_controller(drop);
            });
        }
        {
            let shared = shared.clone();
            factory.connect_bind(move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
                let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else { return };
                let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
                expander.set_list_row(Some(&row));
                let Some(node) = row.item().as_ref().and_then(node_of) else { return };
                let Some(content) = expander.child().and_downcast::<gtk::Box>() else { return };
                let (Some(image), Some(emoji), Some(label)) = (
                    content.first_child().and_downcast::<gtk::Image>(),
                    content.first_child().and_then(|w| w.next_sibling()).and_downcast::<gtk::Label>(),
                    content.last_child().and_downcast::<gtk::Label>(),
                ) else {
                    return;
                };
                let iconic = shared.iconic.borrow();
                let logo = node.logo.as_ref().and_then(|file| Some(shared.root.borrow().as_ref()?.join(file)));
                draw_icon(&image, &emoji, iconic.icon(&node.path, node.is_folder()), icon(&node), logo);
                label.set_text(&node.name);
            });
        }

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_single_click_activate(true);
        list.add_css_class("navigation-sidebar");
        {
            let on_open = on_open.clone();
            list.connect_activate(move |list, position| {
                let Some(row) = list.model().and_then(|m| m.item(position)).and_downcast::<gtk::TreeListRow>() else { return };
                let Some(node) = row.item().as_ref().and_then(node_of) else { return };
                if node.is_folder() {
                    row.set_expanded(!row.is_expanded());
                    if let Some(note) = &node.folder_note {
                        on_open(note);
                    }
                } else {
                    on_open(&node.path);
                }
            });
        }
        let tree = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();

        // The bookmarks, as the Mac and Obsidian show them: above the tree.
        // Above them, the notes a pull or another app changed since they were read.
        let updated = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
        {
            let on_open = on_open.clone();
            updated.connect_row_activated(move |_, row| on_open(row.widget_name().trim_start_matches("u:")));
        }
        let updated_title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .margin_start(14)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption-heading", "dim-label"])
            .build();
        // Under the list, as on the Mac, and only for more than one note: in
        // the heading it cut the heading short.
        let read_all = gtk::Button::builder()
            .label("Mark All as Read")
            .halign(gtk::Align::End)
            .margin_end(6)
            .css_classes(["flat", "caption"])
            .action_name("win.mark-read")
            .action_target(&"".to_variant())
            .build();
        let updated_head = gtk::Box::builder().margin_top(10).margin_bottom(2).margin_end(6).build();
        updated_head.append(&updated_title);
        let updated_section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
        updated_section.append(&updated_head);
        updated_section.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(180)
                .child(&updated)
                .build(),
        );
        updated_section.append(&read_all);
        updated_section.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let marks = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
        marks.connect_row_activated(move |_, row| on_open(row.widget_name().trim_start_matches("b:")));
        let marks_section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
        marks_section.append(
            &gtk::Label::builder()
                .label("Bookmarks")
                .xalign(0.0)
                .margin_start(14)
                .margin_top(6)
                .css_classes(["caption-heading", "dim-label"])
                .build(),
        );
        let marks_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(220)
            .child(&marks)
            .build();
        marks_section.append(&marks_scroll);
        marks_section.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let (tags, tags_title, tags_section) = tag_section(&shared);
        let widget = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        widget.append(&updated_section);
        widget.append(&marks_section);
        widget.append(&tree);
        widget.append(&tags_section);
        Sidebar {
            widget,
            root,
            model,
            marks,
            marks_section,
            updated,
            updated_title,
            read_all,
            updated_section,
            tags,
            tags_title,
            tags_section,
            shared,
        }
    }

    /// [handler] gets the vault paths dropped on a folder row, and the folder.
    pub fn connect_move(&self, handler: impl Fn(Vec<String>, &str) + 'static) {
        *self.shared.on_move.borrow_mut() = Some(Rc::new(handler));
    }

    /// [handler] gets the tag (`a/b`, without `#`) a click on a tag row picks.
    pub fn connect_tag(&self, handler: impl Fn(&str) + 'static) {
        *self.shared.on_tag.borrow_mut() = Some(Rc::new(handler));
    }

    /// The tags, as `TagNode::tree` makes them; the section hides with none.
    pub fn set_tags(&self, tags: &[TagNode]) {
        self.tags.remove_all();
        for node in tags {
            self.tags.append(&glib::BoxedAnyObject::new(node.clone()));
        }
        let all: usize = count_tags(tags);
        self.tags_title.set_label(&format!("Tags · {all}"));
        self.tags_section.set_visible(!tags.is_empty());
    }

    /// The notes changed on disk since they were read, newest change first,
    /// with when they changed, seconds since 1970; at most 12 show.
    pub fn set_updated(&self, notes: &[(String, i64)]) {
        self.updated.remove_all();
        let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(0);
        for (path, modified) in notes.iter().take(12) {
            let row = gtk::Box::builder().spacing(8).build();
            row.append(&gtk::Image::builder().icon_name("starred-symbolic").css_classes(["accent"]).build());
            row.append(
                &gtk::Label::builder()
                    .label(Vault::title(path))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            row.append(&gtk::Label::builder().label(ago(now - modified)).css_classes(["caption", "dim-label"]).build());
            let item = gtk::ListBoxRow::builder().child(&row).tooltip_text(format!("{path} changed since you last read it")).build();
            item.set_widget_name(&format!("u:{path}"));
            let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let path = path.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                let Some(widget) = gesture.widget() else { return };
                let menu = gio::Menu::new();
                menu.append_item(&item_for("Mark as Read", "win.mark-read", &path));
                menu.append_item(&item_for("Open in New Tab", "win.open-tab", &path));
                menu.append_item(&item_for("Open in Split", "win.open-split", &path));
                popup(&widget, &menu, x, y);
            });
            item.add_controller(click);
            self.updated.append(&item);
        }
        self.updated_title.set_label(&format!("Updated since you read · {}", notes.len()));
        self.read_all.set_visible(notes.len() > 1);
        self.updated_section.set_visible(!notes.is_empty());
    }

    /// Shows [tree], with the folders that were open still open.
    pub fn set_tree(&self, tree: &[TreeNode]) {
        let open: HashSet<String> = (0..self.model.n_items())
            .filter_map(|at| self.model.row(at))
            .filter(|row| row.is_expanded())
            .filter_map(|row| row.item().as_ref().and_then(node_of))
            .map(|node| node.path)
            .collect();
        let nodes: Vec<glib::BoxedAnyObject> = tree.iter().map(|node| glib::BoxedAnyObject::new(node.clone())).collect();
        self.root.splice(0, self.root.n_items(), &nodes);
        // Opening a folder puts its rows after it, so the walk sees them too.
        let mut at = 0;
        while at < self.model.n_items() {
            if let Some(row) = self.model.row(at)
                && row.item().as_ref().and_then(node_of).is_some_and(|node| open.contains(&node.path))
            {
                row.set_expanded(true);
            }
            at += 1;
        }
    }

    /// The vault's folder, for dragging a row out.
    pub fn set_root(&self, root: std::path::PathBuf) {
        *self.shared.root.borrow_mut() = Some(root);
    }

    /// The Iconic icons, for the rows drawn from now on; [set_tree] redraws.
    pub fn set_iconic(&self, iconic: Iconic) {
        *self.shared.iconic.borrow_mut() = iconic;
    }

    /// The bookmarks that still name something: files in [files], and folders.
    pub fn set_bookmarks(&self, entries: &[BookmarkEntry], files: &std::collections::BTreeSet<String>) {
        self.marks.remove_all();
        let shown: Vec<&BookmarkEntry> = entries.iter().filter(|e| e.folder || files.contains(&e.path)).collect();
        *self.shared.bookmarked.borrow_mut() = entries.iter().filter(|e| !e.folder).map(|e| e.path.clone()).collect();
        let iconic = self.shared.iconic.borrow();
        for entry in &shown {
            // A bookmarked folder opens its folder note.
            let open = if entry.folder {
                let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
                format!("{}/{name}.md", entry.path)
            } else {
                entry.path.clone()
            };
            let title = entry.title.clone().unwrap_or_else(|| Vault::title(&entry.path));
            let row = gtk::Box::builder().spacing(8).build();
            let (image, emoji) = (
                gtk::Image::builder().overflow(gtk::Overflow::Hidden).build(),
                gtk::Label::builder().visible(false).width_chars(2).build(),
            );
            let fallback = if entry.folder { "folder-symbolic" } else { "user-bookmarks-symbolic" };
            let logo = entry
                .folder
                .then(|| Vault::logo(&entry.path, files))
                .flatten()
                .and_then(|file| Some(self.shared.root.borrow().as_ref()?.join(file)));
            draw_icon(&image, &emoji, iconic.icon(&entry.path, entry.folder), fallback, logo);
            row.append(&image);
            row.append(&emoji);
            row.append(&gtk::Label::builder().label(&title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let item = gtk::ListBoxRow::builder().child(&row).tooltip_text(&entry.path).build();
            item.set_widget_name(&format!("b:{open}"));
            let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let path = entry.path.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                let Some(widget) = gesture.widget() else { return };
                let menu = gio::Menu::new();
                menu.append_item(&item_for("Remove Bookmark", "win.bookmark", &path));
                popup(&widget, &menu, x, y);
            });
            item.add_controller(click);
            self.marks.append(&item);
        }
        self.marks_section.set_visible(!shown.is_empty());
    }
}

/// [seconds] ago, short: `now`, `5m`, `3h`, `2d`.
fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// Every tag in [nodes] and under them.
fn count_tags(nodes: &[TagNode]) -> usize {
    nodes.iter().map(|node| 1 + node.children.as_deref().map_or(0, count_tags)).sum()
}

fn tag_of(item: &glib::Object) -> Option<TagNode> {
    item.downcast_ref::<glib::BoxedAnyObject>().map(|boxed| boxed.borrow::<TagNode>().clone())
}

/// The tag tree under the files: a heading that folds it, and a row per tag
/// in the tag's own color, with the notes it has; a click lists the notes, a
/// right click renames or copies the tag. The tint of a tag is the reader's.
fn tag_section(shared: &Rc<Shared>) -> (gio::ListStore, gtk::Label, gtk::Box) {
    let tags = gio::ListStore::new::<glib::BoxedAnyObject>();
    let model = gtk::TreeListModel::new(tags.clone(), false, false, |item| {
        let children = tag_of(item)?.children?;
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        for node in &children {
            store.append(&glib::BoxedAnyObject::new(node.clone()));
        }
        Some(store.upcast())
    });
    let selection = gtk::NoSelection::new(Some(model));
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&gtk::Label::builder().label("#").css_classes(["tint", "heading"]).build());
        row.append(
            &gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["tint"]).build(),
        );
        row.append(&gtk::Label::builder().valign(gtk::Align::Center).css_classes(["daftar-count"]).build());
        let expander = gtk::TreeExpander::new();
        expander.set_child(Some(&row));
        item.set_child(Some(&expander));
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = item.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(item) = weak.upgrade() else { return };
            let Some(node) = item.item().and_downcast::<gtk::TreeListRow>().and_then(|row| row.item()).as_ref().and_then(tag_of) else {
                return;
            };
            let Some(widget) = gesture.widget() else { return };
            let menu = gio::Menu::new();
            menu.append_item(&item_for("Rename Tag…", "win.rename-tag", &node.tag));
            menu.append_item(&item_for("Copy Tag", "win.copy-text", &format!("#{}", node.tag)));
            popup(&widget, &menu, x, y);
        });
        expander.add_controller(click);
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else { return };
        let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
        expander.set_list_row(Some(&row));
        let Some(node) = row.item().as_ref().and_then(tag_of) else { return };
        let Some(content) = expander.child().and_downcast::<gtk::Box>() else { return };
        let tint = format!("t-{}", tag_tint(&node.tag));
        let (Some(mark), Some(name), Some(count)) = (
            content.first_child().and_downcast::<gtk::Label>(),
            content.first_child().and_then(|w| w.next_sibling()).and_downcast::<gtk::Label>(),
            content.last_child().and_downcast::<gtk::Label>(),
        ) else {
            return;
        };
        mark.set_css_classes(&["tint", "heading", tint.as_str()]);
        name.set_css_classes(&["tint", tint.as_str()]);
        name.set_text(&node.name);
        count.set_text(&node.count.to_string());
        count.set_css_classes(&["daftar-count", tint.as_str()]);
        content.update_property(&[gtk::accessible::Property::Label(&format!("#{}, {} notes", node.tag, node.count))]);
    });
    let list = gtk::ListView::new(Some(selection), Some(factory));
    list.set_single_click_activate(true);
    list.add_css_class("navigation-sidebar");
    {
        let shared = shared.clone();
        list.connect_activate(move |list, position| {
            let Some(row) = list.model().and_then(|m| m.item(position)).and_downcast::<gtk::TreeListRow>() else { return };
            let Some(node) = row.item().as_ref().and_then(tag_of) else { return };
            let handler = shared.on_tag.borrow().clone();
            if let Some(handler) = handler {
                handler(&node.tag);
            }
        });
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(260)
        .child(&list)
        .build();
    let title = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["caption-heading", "dim-label"]).build();
    let arrow = gtk::Image::from_icon_name("pan-down-symbolic");
    let head_box = gtk::Box::builder().spacing(4).margin_start(8).build();
    head_box.append(&title);
    head_box.append(&arrow);
    let head = gtk::Button::builder().child(&head_box).css_classes(["flat"]).tooltip_text("Show or hide the tags").build();
    {
        let (scroll, arrow) = (scroll.clone(), arrow.clone());
        head.connect_clicked(move |_| {
            let open = !scroll.is_visible();
            scroll.set_visible(open);
            arrow.set_icon_name(Some(if open { "pan-down-symbolic" } else { "pan-end-symbolic" }));
        });
    }
    let section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();
    section.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    section.append(&head);
    section.append(&scroll);
    (tags, title, section)
}

/// The tint a tag's color is, from the reader's colors by tag
/// (`editor_style::tag_color`), as the sidebar's palette names them.
fn tag_tint(tag: &str) -> &'static str {
    match crate::editor_style::tag_color(tag) {
        "orange" => "accent",
        "chartreuse" | "green" => "green",
        "vividYellow" => "yellow",
        other => ["aqua", "pink", "purple", "blue", "red"].into_iter().find(|t| *t == other).unwrap_or("accent"),
    }
}

fn item_for(label: &str, action: &str, target: &str) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
    item
}

/// The file commands for a row, as the Mac's context menu has them.
fn menu(path: &str, folder: bool, bookmarked: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    if !folder {
        let open = gio::Menu::new();
        open.append_item(&item_for("Open in New Tab", "win.open-tab", path));
        open.append_item(&item_for("Open in Split", "win.open-split", path));
        menu.append_section(None, &open);
    }
    if folder {
        let create = gio::Menu::new();
        create.append_item(&item_for("New Note", "win.new-note", path));
        create.append_item(&item_for("New Folder…", "win.new-folder", path));
        menu.append_section(None, &create);
        let find = gio::Menu::new();
        find.append_item(&item_for("Replace in Folder…", "win.replace-in-folder", path));
        menu.append_section(None, &find);
    }
    let change = gio::Menu::new();
    change.append_item(&item_for("Rename…", "win.rename", path));
    change.append_item(&item_for("Move to…", "win.move", path));
    if !folder {
        change.append_item(&item_for(if bookmarked { "Remove Bookmark" } else { "Bookmark" }, "win.bookmark", path));
    }
    menu.append_section(None, &change);
    let gone = gio::Menu::new();
    gone.append_item(&item_for("Move to Trash…", "win.trash", path));
    menu.append_section(None, &gone);
    menu
}

fn popup(widget: &gtk::Widget, menu: &gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    popover.set_parent(widget);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|popover| {
        // After the item's action: unparenting at once would drop it.
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// Iconic's color names in naz (dark) and Daylight (light), as the phone's
/// `IconColors.kt` and the Mac draw them.
const ICONIC_COLORS: [(&str, &str, &str); 9] = [
    ("red", "#ff5070", "#c01030"),
    ("orange", "#ffb040", "#a85200"),
    ("yellow", "#ffd850", "#8a6a00"),
    ("green", "#20ffa0", "#0a7a48"),
    ("cyan", "#80e5ff", "#00707a"),
    ("blue", "#40d0ff", "#0070a0"),
    ("purple", "#f0a0f0", "#8040a0"),
    ("pink", "#ff69b4", "#b02a70"),
    ("gray", "#a8a8a0", "#6a6a62"),
];

/// The class that colors an icon [color]: one of Iconic's names, or a
/// `#rrggbb` (or `#rgb`) of the person's own, which looks the same in both
/// schemes, as Iconic draws it. `None` for anything else.
fn color_class(color: &str) -> Option<String> {
    if Iconic::COLORS.contains(&color) {
        return Some(format!("iconic-{color}"));
    }
    let hex = hex_color(color)?;
    let class = format!("iconic-hex-{hex}");
    let added = HEX_COLORS.with(|colors| colors.borrow_mut().insert(hex));
    if added {
        reload_colors();
    }
    Some(class)
}

/// `#rrggbb` or `#rgb` as six lowercase digits.
fn hex_color(color: &str) -> Option<String> {
    let digits = color.strip_prefix('#')?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => Some(digits.to_ascii_lowercase()),
        3 => Some(digits.chars().flat_map(|c| [c, c]).collect::<String>().to_ascii_lowercase()),
        _ => None,
    }
}

fn iconic_css(dark: bool, hexes: &BTreeSet<String>) -> String {
    let named =
        ICONIC_COLORS.iter().map(|(name, naz, daylight)| format!(".iconic-{name} {{ color: {}; }}\n", if dark { naz } else { daylight }));
    let own = hexes.iter().map(|hex| format!(".iconic-hex-{hex} {{ color: #{hex}; }}\n"));
    named.chain(own).collect()
}

thread_local! {
    /// The colors' stylesheet, once a display has one, and the `#rrggbb`s
    /// the vault's icons asked for so far.
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static HEX_COLORS: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
}

fn reload_colors() {
    let dark = adw::StyleManager::default().is_dark();
    PROVIDER.with(|provider| {
        if let Some(provider) = provider.borrow().as_ref() {
            provider.load_from_string(&HEX_COLORS.with(|hexes| iconic_css(dark, &hexes.borrow())));
        }
    });
}

/// The colors, for the scheme now and whenever it changes.
fn install_colors() {
    let Some(display) = gdk::Display::default() else { return };
    if PROVIDER.with(|provider| provider.borrow().is_some()) {
        return;
    }
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    PROVIDER.with(|slot| *slot.borrow_mut() = Some(provider));
    reload_colors();
    adw::StyleManager::default().connect_dark_notify(|_| reload_colors());
}

/// A Lucide name, as Iconic keeps it, to the nearest icon GTK's theme has;
/// `None` draws the default. The theme has far fewer icons than Lucide, so
/// some are only near.
pub fn lucide(name: &str) -> Option<&'static str> {
    Some(match name.strip_prefix("lucide-")? {
        "building" | "building-2" | "landmark" | "school" | "hospital" => "x-office-address-book-symbolic",
        "house" | "house-plus" | "home" => "user-home-symbolic",
        "laptop" | "computer" | "monitor" => "computer-symbolic",
        "tv" => "tv-symbolic",
        "smartphone" | "phone" => "phone-symbolic",
        "brain" | "flask-conical" | "sigma" => "applications-science-symbolic",
        "wrench" | "hammer" | "settings" => "applications-engineering-symbolic",
        "utensils" | "coffee" | "cake" | "apple" => "emoji-food-symbolic",
        "train" | "truck" | "car" | "plane" | "ship" | "bike" | "route" | "map" | "baggage-claim" => "emoji-travel-symbolic",
        "paw-print" | "leaf" | "sprout" | "mountain" | "tent" | "waves" => "emoji-nature-symbolic",
        "lock-keyhole" | "lock" | "key" => "changes-prevent-symbolic",
        "shield-check" | "shield" => "security-high-symbolic",
        "sun" => "weather-clear-symbolic",
        "moon" => "weather-clear-night-symbolic",
        "cloud" | "umbrella" => "weather-overcast-symbolic",
        "star" | "sparkles" | "award" | "trophy" => "starred-symbolic",
        "heart" => "emote-love-symbolic",
        "smile-plus" | "drama" => "face-smile-symbolic",
        "skull" => "dialog-warning-symbolic",
        "square-check-big" | "check-square" | "list-todo" | "check" => "checkbox-checked-symbolic",
        "calendar" | "clock" => "x-office-calendar-symbolic",
        "chart-no-axes-column-increasing" | "trending-up" | "calculator" => "x-office-spreadsheet-symbolic",
        "piggy-bank" | "dollar-sign" | "banknote" | "euro" | "receipt-euro" | "coins" | "wallet" | "credit-card" | "shopping-cart" => {
            "emoji-objects-symbolic"
        }
        "book" | "book-open" | "book-open-text" | "notebook" | "newspaper" | "graduation-cap" => "accessories-dictionary-symbolic",
        "file" | "file-text" => "text-x-generic-symbolic",
        "folder" | "folder-open" | "archive" | "inbox" => "folder-symbolic",
        "mail" | "message-circle" => "mail-unread-symbolic",
        "globe" | "languages" | "compass" => "web-browser-symbolic",
        "map-pin" | "pin" => "find-location-symbolic",
        "music" | "headphones" | "mic" => "audio-x-generic-symbolic",
        "image" | "camera" | "film" | "clapperboard" | "palette" => "image-x-generic-symbolic",
        "code" | "terminal" | "git-branch" | "bug" | "cpu" | "bot" => "utilities-terminal-symbolic",
        "database" | "server" | "network" => "network-server-symbolic",
        "user" | "id-card" | "baby" => "avatar-default-symbolic",
        "users" => "system-users-symbolic",
        "box" | "package" | "gift" => "package-x-generic-symbolic",
        "gamepad-2" | "dumbbell" | "biceps-flexed" | "swords" | "target" | "flame" | "zap" | "rocket" => "emoji-activities-symbolic",
        "link" | "paperclip" => "insert-link-symbolic",
        "list" => "view-list-bullet-symbolic",
        "bookmark" => "user-bookmarks-symbolic",
        "bell" | "megaphone" | "flag" => "emblem-important-symbolic",
        "search" | "glasses" => "edit-find-symbolic",
        "trash-2" => "user-trash-symbolic",
        "pen" | "clipboard" => "document-edit-symbolic",
        "lightbulb" | "puzzle" | "pill" | "stethoscope" | "briefcase-medical" | "briefcase" | "scale" | "shirt" | "wifi" => {
            "emoji-objects-symbolic"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lucide_names_find_a_theme_icon_or_none() {
        assert_eq!(lucide("lucide-house"), Some("user-home-symbolic"));
        assert_eq!(lucide("lucide-building"), Some("x-office-address-book-symbolic"));
        assert_eq!(lucide("lucide-no-such-icon"), None);
        assert_eq!(lucide("😋"), None);
    }

    #[test]
    fn own_colors_are_six_lowercase_digits() {
        assert_eq!(hex_color("#FF8800").as_deref(), Some("ff8800"));
        assert_eq!(hex_color("#f80").as_deref(), Some("ff8800"));
        assert_eq!(hex_color("ff8800"), None);
        assert_eq!(hex_color("#ff88001"), None);
        assert_eq!(hex_color("#gg0000"), None);
        let css = iconic_css(true, &BTreeSet::from(["ff8800".to_string()]));
        assert!(css.contains(".iconic-orange { color: #ffb040; }"));
        assert!(css.ends_with(".iconic-hex-ff8800 { color: #ff8800; }\n"));
    }
}
