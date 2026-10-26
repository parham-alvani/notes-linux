//! The folder tree, as Obsidian's file explorer shows it: folders first, a
//! folder note opens with its folder.

use std::collections::HashSet;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::{TreeNode, Vault};

pub struct Sidebar {
    pub widget: gtk::ScrolledWindow,
    root: gio::ListStore,
    model: gtk::TreeListModel,
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
    } else if Vault::opens_in_app(&node.path) {
        "text-x-generic-symbolic"
    } else {
        "image-x-generic-symbolic"
    }
}

impl Sidebar {
    /// [on_open] gets the path of a note, a base or another file to open.
    pub fn new(on_open: impl Fn(&str) + 'static) -> Sidebar {
        let root = gio::ListStore::new::<glib::BoxedAnyObject>();
        let model = gtk::TreeListModel::new(root.clone(), false, false, |item| {
            let children = node_of(item)?.children?;
            Some(store_of(&children).upcast())
        });
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&gtk::Image::new());
            row.append(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let expander = gtk::TreeExpander::new();
            expander.set_child(Some(&row));
            item.set_child(Some(&expander));
        });
        factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
            let Some(expander) = item.child().and_downcast::<gtk::TreeExpander>() else { return };
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            expander.set_list_row(Some(&row));
            let Some(node) = row.item().as_ref().and_then(node_of) else { return };
            let Some(content) = expander.child().and_downcast::<gtk::Box>() else { return };
            if let Some(image) = content.first_child().and_downcast::<gtk::Image>() {
                image.set_icon_name(Some(icon(&node)));
            }
            if let Some(label) = content.last_child().and_downcast::<gtk::Label>() {
                label.set_text(&node.name);
            }
        });

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_single_click_activate(true);
        list.add_css_class("navigation-sidebar");
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

        let widget = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
        Sidebar { widget, root, model }
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
}
