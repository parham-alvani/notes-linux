//! The quick switcher (Ctrl+P), the vault's search (Ctrl+Shift+F) and the
//! command palette (Ctrl+Shift+P): a field and a list under it, as on the
//! Mac. Up and Down move in the list, Enter opens or runs, Escape closes.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use daftar_core::{Candidate, Fuzzy, QuickSwitcher, Vault};

use crate::store::VaultStore;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Files by name.
    Files,
    /// Notes by their text.
    Text,
}

/// One row: what it opens or runs, and how it shows, as markup.
struct Row {
    path: String,
    title: String,
    subtitle: String,
}

/// A command of the palette: the window action it runs, as `win.sync`.
pub struct Command {
    pub title: &'static str,
    pub group: &'static str,
    pub action: &'static str,
    pub keys: &'static str,
}

const LIMIT: usize = 50;

/// Every file of the vault as the quick switcher ranks it.
fn candidates(store: &VaultStore) -> Vec<Candidate> {
    let index = store.index();
    index
        .files
        .iter()
        .map(|path| {
            let aliases = index.notes.get(path).map(|n| n.parsed.aliases.clone()).unwrap_or_default();
            Candidate::new(path, &Vault::title(path), &aliases)
        })
        .collect()
}

/// The file the quick switcher ranks first for [query], as the home page's
/// field opens it.
pub fn best(store: &VaultStore, query: &str, recent: &[String]) -> Option<String> {
    QuickSwitcher::rank(query, &candidates(store), recent, 1).into_iter().next()
}

/// Shows the palette over [parent], with [query] in its field; [on_open]
/// gets the path picked.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    mode: Mode,
    store: Rc<VaultStore>,
    recent: Vec<String>,
    query: &str,
    on_open: impl Fn(&str) + 'static,
) {
    let candidates: Vec<Candidate> = match mode {
        Mode::Files => candidates(&store),
        Mode::Text => vec![],
    };
    let placeholder = match mode {
        Mode::Files => "Open a note or a file",
        Mode::Text => "Search the notes",
    };
    let find = move |query: &str| -> Vec<Row> {
        match mode {
            Mode::Files => QuickSwitcher::rank(query, &candidates, &recent, LIMIT)
                .into_iter()
                .map(|path| {
                    let folder = QuickSwitcher::folder(&path);
                    let title = Vault::title(&path);
                    let (in_title, in_folder) = QuickSwitcher::highlights(query, &title, &folder);
                    Row { title: marked(&title, &in_title), subtitle: marked(&folder, &in_folder), path }
                })
                .collect(),
            Mode::Text => store
                .search
                .search(query, LIMIT)
                .into_iter()
                .map(|hit| Row {
                    title: glib::markup_escape_text(&Vault::title(&hit.path)).into(),
                    subtitle: snippet(&hit.snippet),
                    path: hit.path,
                })
                .collect(),
        }
    };
    show(parent, placeholder, query, find, on_open);
}

/// The command palette: [commands] ranked by how well their names match.
pub fn present_commands(parent: &impl IsA<gtk::Widget>, commands: Vec<Command>, on_run: impl Fn(&str) + 'static) {
    let find = move |query: &str| -> Vec<Row> {
        let mut found: Vec<(i64, usize, Row)> = commands
            .iter()
            .enumerate()
            .filter_map(|(at, command)| {
                let hit = Fuzzy::matches(query, command.title)?;
                let positions = hit.positions.iter().copied().collect();
                let subtitle =
                    if command.keys.is_empty() { command.group.to_string() } else { format!("{} · {}", command.group, command.keys) };
                let row = Row {
                    title: marked(command.title, &positions),
                    subtitle: glib::markup_escape_text(&subtitle).into(),
                    path: command.action.to_string(),
                };
                Some((hit.score, at, row))
            })
            .collect();
        // The best match first; with no query, the palette's own order.
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found.into_iter().map(|(_, _, row)| row).collect()
    };
    show(parent, "Run a command", "", find, on_run);
}

/// The dialog: a field, and the rows [find] gives for what is typed in it.
fn show(
    parent: &impl IsA<gtk::Widget>,
    placeholder: &str,
    query: &str,
    find: impl Fn(&str) -> Vec<Row> + 'static,
    on_open: impl Fn(&str) + 'static,
) {
    let entry = gtk::SearchEntry::builder().placeholder_text(placeholder).hexpand(true).build();
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Browse).css_classes(["navigation-sidebar"]).build();
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&entry);
    content.append(&scroller);
    let dialog = adw::Dialog::builder().content_width(640).content_height(460).child(&content).title(placeholder).build();

    let rows: Rc<RefCell<Vec<Row>>> = Rc::new(RefCell::new(Vec::new()));
    let fill = {
        let (list, rows) = (list.clone(), rows.clone());
        move |query: &str| {
            let found = find(query);
            list.remove_all();
            for row in &found {
                let title =
                    gtk::Label::builder().use_markup(true).label(&row.title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
                let subtitle = gtk::Label::builder()
                    .use_markup(true)
                    .label(&row.subtitle)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["dim-label", "caption"])
                    .build();
                let lines = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(4).margin_bottom(4).build();
                lines.append(&title);
                if !row.subtitle.is_empty() {
                    lines.append(&subtitle);
                }
                list.append(&lines);
            }
            list.select_row(list.row_at_index(0).as_ref());
            *rows.borrow_mut() = found;
        }
    };
    entry.set_text(query);
    entry.set_position(-1);
    fill(query);

    // The note opens once the dialog is gone: WebKit draws no frame for a
    // page that changes while the closing dialog still covers it.
    let chosen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    {
        let chosen = chosen.clone();
        dialog.connect_closed(move |_| {
            if let Some(path) = chosen.take() {
                on_open(&path);
            }
        });
    }
    let pick = {
        let (dialog, rows) = (dialog.clone(), rows.clone());
        move |at: i32| {
            let path = rows.borrow().get(at as usize).map(|row| row.path.clone());
            if path.is_some() {
                *chosen.borrow_mut() = path;
                dialog.close();
            }
        }
    };
    {
        let pick = pick.clone();
        list.connect_row_activated(move |_, row| pick(row.index()));
    }
    {
        let list = list.clone();
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                pick(row.index());
            }
        });
    }
    entry.connect_search_changed(move |entry| fill(&entry.text()));
    {
        let dialog = dialog.clone();
        entry.connect_stop_search(move |_| {
            dialog.close();
        });
    }
    // Up and Down move in the list while the field keeps the keyboard.
    let keys = gtk::EventControllerKey::new();
    {
        let (list, scroller) = (list.clone(), scroller.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            let step = match key {
                gtk::gdk::Key::Down => 1,
                gtk::gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(-1, |row| row.index()) + step;
            if let Some(row) = list.row_at_index(at.max(0)) {
                list.select_row(Some(&row));
                // A list box does not scroll to its selection by itself.
                if let Some(bounds) = row.compute_bounds(&list) {
                    let adjustment = scroller.vadjustment();
                    let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
                    if top < adjustment.value() {
                        adjustment.set_value(top);
                    } else if bottom > adjustment.value() + adjustment.page_size() {
                        adjustment.set_value(bottom - adjustment.page_size());
                    }
                }
            }
            glib::Propagation::Stop
        });
    }
    entry.add_controller(keys);
    dialog.present(Some(parent));
    entry.grab_focus();
}

/// [text] as Pango markup, with the characters at [marks] in bold.
fn marked(text: &str, marks: &BTreeSet<usize>) -> String {
    let mut out = String::new();
    for (at, character) in text.chars().enumerate() {
        let escaped = glib::markup_escape_text(character.encode_utf8(&mut [0; 4]));
        if marks.contains(&at) {
            out.push_str(&format!("<b>{escaped}</b>"));
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

/// A search snippet, its matches between `\u{1}` and `\u{2}`, as markup.
fn snippet(text: &str) -> String {
    let mut out = String::new();
    for (at, piece) in text.replace('\n', " ").split(['\u{1}', '\u{2}']).enumerate() {
        let escaped = glib::markup_escape_text(piece);
        if at % 2 == 1 {
            out.push_str(&format!("<b>{escaped}</b>"));
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_bold_and_the_rest_escaped() {
        assert_eq!(marked("A&B", &[0, 2].into()), "<b>A</b>&amp;<b>B</b>");
        assert_eq!(snippet("a \u{1}plan\u{2} <x>"), "a <b>plan</b> &lt;x&gt;");
    }
}
