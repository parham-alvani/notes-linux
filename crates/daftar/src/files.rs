//! New notes and folders, renames, moves, the Trash and extracting a
//! selection to a new note, as on the Mac
//! (`FileActionSheet`, `VaultStore`): each in a dialog that says what will
//! happen to the links before anything is written. A move goes through
//! `VaultMoves`, so every link to what moved follows it, and the bookmarks,
//! the Iconic icons and the reading places follow too.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gio, glib};

use daftar_core::text::slugs;
use daftar_core::{EditorCommands, Iconic, MoveFailure, Vault, VaultIndex, VaultMoves};

use crate::store::{VaultStore, file_in};

/// What a file command does, and to what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    NewNote {
        folder: String,
    },
    NewFolder {
        parent: String,
    },
    Rename {
        path: String,
    },
    Move {
        path: String,
    },
    Trash {
        path: String,
    },
    /// The editor's selection, [text], to a new note in [folder].
    Extract {
        folder: String,
        text: String,
    },
}

/// What a command changed: the paths to read again, where each file went,
/// and the note to open after it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Done {
    pub changed: BTreeSet<String>,
    pub moves: BTreeMap<String, String>,
    pub open: Option<String>,
    /// After an extract: the link that takes the selection's place.
    pub link: Option<String>,
}

// The work, with no window

/// A new note named [name] in [folder], holding [text]; its vault path.
pub fn create_note(root: &Path, index: &VaultIndex, name: &str, folder: &str, text: &str) -> Result<String, MoveFailure> {
    if !VaultMoves::is_valid_name(name) {
        return Err(MoveFailure::BadName(name.into()));
    }
    let path = join(folder, &VaultMoves::note_name(name));
    if index.files.iter().any(|f| slugs::fold(f) == slugs::fold(&path)) {
        return Err(MoveFailure::Exists(path));
    }
    let file = file_in(&path, root).ok_or_else(|| MoveFailure::BadName(name.into()))?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    // `create_new`: never over a file another app made meanwhile.
    let mut new = fs::OpenOptions::new().write(true).create_new(true).open(&file).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => MoveFailure::Exists(path.clone()),
        _ => e.into(),
    })?;
    std::io::Write::write_all(&mut new, text.as_bytes())?;
    Ok(path)
}

/// The link an extracted note gets where the selection was: its bare name
/// when no other note in [files] has it, else its path, as on the Mac.
pub fn extract_link(path: &str, files: &BTreeSet<String>) -> String {
    let title = Vault::title(path);
    let taken = files.iter().any(|f| f != path && Vault::is_note(f) && Vault::title(f).to_lowercase() == title.to_lowercase());
    format!("[[{}]]", if taken { path.strip_suffix(".md").unwrap_or(path) } else { &title })
}

/// A new folder named [name] in [parent]; its vault path.
pub fn create_folder(root: &Path, name: &str, parent: &str) -> Result<String, MoveFailure> {
    if !VaultMoves::is_valid_name(name) {
        return Err(MoveFailure::BadName(name.into()));
    }
    let path = join(parent, name.trim());
    let folder = file_in(&path, root).ok_or_else(|| MoveFailure::BadName(name.into()))?;
    if folder.exists() {
        return Err(MoveFailure::Exists(path));
    }
    fs::create_dir_all(&folder)?;
    Ok(path)
}

/// Where a rename to [name] takes [path]: a note keeps `.md`.
pub fn renamed(path: &str, name: &str, is_file: bool) -> String {
    let parent = parent(path);
    let mut name = name.trim().to_string();
    if is_file && Vault::is_note(path) {
        name = VaultMoves::note_name(&name);
    }
    join(parent, &name)
}

/// Every folder [path] can move to, the vault itself ("") first, by [query].
pub fn folders(index: &VaultIndex, path: &str, query: &str) -> Vec<String> {
    let mut all: BTreeSet<String> = BTreeSet::from([String::new()]);
    all.extend(index.empty_folders.iter().cloned());
    for file in &index.files {
        let parts: Vec<&str> = file.split('/').collect();
        for end in 1..parts.len() {
            all.insert(parts[..end].join("/"));
        }
    }
    let needle = slugs::fold(query.trim());
    let from = parent(path);
    let inside = format!("{path}/");
    let mut out: Vec<String> = all
        .into_iter()
        .filter(|c| c != from && c != path && !c.starts_with(&inside) && (needle.is_empty() || slugs::fold(c).contains(&needle)))
        .collect();
    out.sort_by(|a, b| (!a.is_empty()).cmp(&!b.is_empty()).then_with(|| daftar_core::text::standard_compare(a, b)));
    out
}

/// How many notes link to [path] or into it, from outside it.
pub fn linking_notes(index: &VaultIndex, path: &str) -> usize {
    let inside = format!("{path}/");
    let within = |p: &str| p == path || p.starts_with(&inside);
    let mut sources: BTreeSet<&str> = BTreeSet::new();
    for file in index.files.iter().filter(|f| within(f)) {
        for link in index.backlinks(file) {
            if !within(&link.source) {
                sources.insert(&link.source);
            }
        }
    }
    sources.len()
}

/// Renames or moves [from] to [to] with every link to it, and what the vault
/// keeps beside the files follows: bookmarks, icons, reading places.
pub fn move_path(store: &VaultStore, from: &str, to: &str) -> Result<Done, MoveFailure> {
    if file_in(from, &store.root).is_none() || file_in(to, &store.root).is_none() {
        return Err(MoveFailure::BadName(to.into()));
    }
    let (moves, changed) = {
        let index = store.index();
        let moves = VaultMoves::plan(from, to, &index.files)?;
        (moves.clone(), VaultMoves::perform(from, to, &index, &store.root)?)
    };
    let is_file = moves.len() == 1 && moves.contains_key(from);
    let folder = (!is_file).then_some((from, to));
    {
        let mut bookmarks = store.bookmarks.borrow_mut();
        if bookmarks.follow(&moves, folder) {
            let _ = bookmarks.save(&store.root);
        }
    }
    let iconic = Iconic::file(&store.root);
    if let Ok(data) = fs::read_to_string(&iconic)
        && let Ok(Some(moved)) = Iconic::moving(&moves, folder, &data)
        && write(&iconic, &moved).is_ok()
    {
        *store.iconic.borrow_mut() = Iconic::parse(&moved);
    }
    store.places.moved(&moves);
    let open = moves.get(from).cloned();
    Ok(Done { changed, moves, open, link: None })
}

/// Moves [path] to the Trash, where it can be taken back.
pub fn trash(store: &VaultStore, path: &str) -> Result<Done, String> {
    let file = file_in(path, &store.root).ok_or_else(|| format!("{path} is not in the vault"))?;
    gio::File::for_path(&file).trash(gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    {
        let mut bookmarks = store.bookmarks.borrow_mut();
        let before = bookmarks.entries();
        bookmarks.remove(path);
        if bookmarks.entries() != before {
            let _ = bookmarks.save(&store.root);
        }
    }
    store.places.forget(path);
    Ok(Done { changed: BTreeSet::from([path.to_string()]), ..Done::default() })
}

/// Bookmarks [path], or takes its bookmark away; false when it was not saved.
pub fn toggle_bookmark(store: &VaultStore, path: &str) -> Result<bool, String> {
    let mut bookmarks = store.bookmarks.borrow_mut();
    bookmarks.toggle(path, SystemTime::now());
    bookmarks.save(&store.root).map_err(|e| e.to_string())?;
    Ok(bookmarks.contains(path))
}

fn write(file: &Path, text: &str) -> std::io::Result<()> {
    let temporary = file.with_extension("json.daftar-tmp");
    fs::write(&temporary, text)?;
    fs::rename(&temporary, file)
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// A file's name as the person reads it: a note without `.md`.
fn short_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

// The dialogs

/// Asks for what [action] needs, does it, and tells [done] what changed, or
/// [failed] why nothing did.
pub fn present(
    parent_widget: &impl IsA<gtk::Widget>,
    action: Action,
    store: Rc<VaultStore>,
    done: impl Fn(Done) + 'static,
    failed: impl Fn(String) + 'static,
) {
    let vault = store.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let place = |folder: &str| if folder.is_empty() { vault.clone() } else { folder.replace('/', " › ") };
    let (heading, body, primary, appearance) = match &action {
        Action::NewNote { folder } => {
            ("New Note".to_string(), format!("In {}", place(folder)), "Create", adw::ResponseAppearance::Suggested)
        }
        Action::NewFolder { parent } => {
            ("New Folder".into(), format!("In {}", place(parent)), "Create", adw::ResponseAppearance::Suggested)
        }
        Action::Rename { path } => {
            let kind = if store.index().files.contains(path) { "Rename File" } else { "Rename Folder" };
            (kind.into(), path.clone(), "Rename", adw::ResponseAppearance::Suggested)
        }
        Action::Move { path } => (format!("Move “{}”", short_name(path)), path.clone(), "Move", adw::ResponseAppearance::Suggested),
        Action::Extract { folder, .. } => {
            ("Extract to a New Note".into(), format!("In {}", place(folder)), "Extract", adw::ResponseAppearance::Suggested)
        }
        Action::Trash { path } => (
            format!("Move “{}” to the Trash?", short_name(path)),
            "The Trash keeps it, so it can come back, and git keeps every version that was committed.".into(),
            "Move to Trash",
            adw::ResponseAppearance::Destructive,
        ),
    };
    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("go", primary);
    dialog.set_response_appearance("go", appearance);
    dialog.set_default_response(Some("go"));
    dialog.set_close_response("cancel");

    let extra = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
    // What follows the change, for a rename, a move and the Trash.
    let target = match &action {
        Action::Rename { path } | Action::Move { path } | Action::Trash { path } => Some(path.clone()),
        _ => None,
    };
    let links = target.as_deref().map(|path| linking_notes(&store.index(), path)).unwrap_or(0);
    let link_note = (links > 0).then(|| {
        let notes = if links == 1 { "1 note links here".to_string() } else { format!("{links} notes link here") };
        if matches!(action, Action::Trash { .. }) {
            format!("{notes}; those links will show as missing.")
        } else {
            format!("{notes}; the links follow the change.")
        }
    });

    // The name, for everything but a move and the Trash.
    let entry = gtk::Entry::builder().activates_default(true).build();
    let chosen: Rc<std::cell::RefCell<Option<String>>> = Rc::new(std::cell::RefCell::new(None));
    match &action {
        Action::NewNote { folder } => {
            let free = VaultMoves::free_name("Untitled", "md", folder, &store.index().files);
            entry.set_text(&short_name(&free));
            extra.append(&entry);
        }
        Action::Extract { folder, text } => {
            let name = EditorCommands::extract_name(text);
            let free = VaultMoves::free_name(&name, "md", folder, &store.index().files);
            entry.set_text(&short_name(&free));
            extra.append(&entry);
        }
        Action::NewFolder { parent } => {
            let index = store.index();
            let mut taken = index.files.clone();
            taken.extend(index.empty_folders.iter().cloned());
            entry.set_text(&short_name(&VaultMoves::free_name("New Folder", "", parent, &taken)));
            extra.append(&entry);
        }
        Action::Rename { path } => {
            entry.set_text(&short_name(path));
            extra.append(&entry);
        }
        Action::Move { path } => {
            dialog.set_response_enabled("go", false);
            extra.append(&folder_picker(&store, path, &dialog, chosen.clone(), &place));
        }
        Action::Trash { .. } => {}
    }
    if let Some(note) = link_note {
        extra.append(&gtk::Label::builder().label(&note).xalign(0.0).wrap(true).css_classes(["dim-label"]).build());
    }
    if extra.first_child().is_some() {
        dialog.set_extra_child(Some(&extra));
    }
    if entry.parent().is_some() {
        let check = {
            let dialog = dialog.clone();
            move |entry: &gtk::Entry| dialog.set_response_enabled("go", VaultMoves::is_valid_name(&entry.text()))
        };
        check(&entry);
        entry.connect_changed(check);
        let focus = entry.clone();
        glib::idle_add_local_once(move || {
            focus.grab_focus();
            focus.select_region(0, -1);
        });
    }

    dialog.connect_response(None, move |_, response| {
        if response != "go" {
            return;
        }
        let name = entry.text().to_string();
        let result: Result<Done, String> = match &action {
            Action::NewNote { folder } => create_note(&store.root, &store.index(), &name, folder, "")
                .map(|path| Done { changed: BTreeSet::from([path.clone()]), open: Some(path), ..Done::default() })
                .map_err(|e| e.to_string()),
            Action::Extract { folder, text } => {
                let created = create_note(&store.root, &store.index(), &name, folder, &EditorCommands::extract_body(text));
                created
                    .map(|path| {
                        let mut files = store.index().files.clone();
                        files.insert(path.clone());
                        let link = extract_link(&path, &files);
                        Done { changed: BTreeSet::from([path]), link: Some(link), ..Done::default() }
                    })
                    .map_err(|e| e.to_string())
            }
            Action::NewFolder { parent } => create_folder(&store.root, &name, parent)
                .map(|path| Done { changed: BTreeSet::from([path]), ..Done::default() })
                .map_err(|e| e.to_string()),
            Action::Rename { path } => {
                let is_file = store.index().files.contains(path);
                move_path(&store, path, &renamed(path, &name, is_file)).map_err(|e| e.to_string())
            }
            Action::Move { path } => match chosen.borrow().clone() {
                Some(folder) => {
                    let name = path.rsplit('/').next().unwrap_or(path);
                    move_path(&store, path, &join(&folder, name)).map_err(|e| e.to_string())
                }
                None => return,
            },
            Action::Trash { path } => trash(&store, path),
        };
        match result {
            Ok(result) => {
                store.wrote();
                done(result);
            }
            Err(error) => failed(error),
        }
    });
    dialog.present(Some(parent_widget));
}

/// A field to find a folder by name, and the folders under it.
fn folder_picker(
    store: &VaultStore,
    path: &str,
    dialog: &adw::AlertDialog,
    chosen: Rc<std::cell::RefCell<Option<String>>>,
    place: &dyn Fn(&str) -> String,
) -> gtk::Box {
    let all = folders(&store.index(), path, "");
    let names: Vec<(String, String)> = all.iter().map(|f| (f.clone(), place(f))).collect();
    let search = gtk::SearchEntry::builder().placeholder_text("Find a folder").build();
    // A click picks a folder; a double click or Enter moves there.
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(false)
        .css_classes(["boxed-list"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(240)
        .max_content_height(240)
        .child(&list)
        .build();
    let fill = {
        let list = list.clone();
        move |query: &str| {
            list.remove_all();
            let needle = slugs::fold(query.trim());
            for (folder, label) in names.iter().filter(|(f, _)| needle.is_empty() || slugs::fold(f).contains(&needle)) {
                let row = gtk::Box::builder().spacing(8).margin_top(6).margin_bottom(6).margin_start(8).margin_end(8).build();
                row.append(&gtk::Image::from_icon_name(if folder.is_empty() { "user-home-symbolic" } else { "folder-symbolic" }));
                row.append(&gtk::Label::builder().label(label).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
                let item = gtk::ListBoxRow::builder().child(&row).build();
                item.set_widget_name(&format!("f:{folder}"));
                list.append(&item);
            }
            // While the person types, the best match is the one Enter takes.
            if !needle.is_empty() {
                list.select_row(list.row_at_index(0).as_ref());
            }
        }
    };
    fill("");
    search.connect_search_changed(move |search| fill(&search.text()));
    let commit = {
        let dialog = dialog.clone();
        move || {
            if dialog.is_response_enabled("go") {
                dialog.emit_by_name::<()>("response", &[&"go"]);
                dialog.force_close();
            }
        }
    };
    {
        let commit = commit.clone();
        search.connect_activate(move |_| commit());
    }
    {
        let dialog = dialog.clone();
        list.connect_row_selected(move |_, row| {
            let folder = row.map(|r| r.widget_name().trim_start_matches("f:").to_string());
            dialog.set_response_enabled("go", folder.is_some());
            *chosen.borrow_mut() = folder;
        });
    }
    list.connect_row_activated(move |_, _| commit());
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    content.append(&search);
    content.append(&scroller);
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use daftar_core::{Bookmarks, SearchIndex};

    fn vault() -> (tempfile::TempDir, VaultIndex) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::create_dir_all(root.path().join("Projects/Alpha")).unwrap();
        fs::create_dir_all(root.path().join("Empty")).unwrap();
        fs::write(root.path().join("Projects/Alpha/Plan.md"), "# Plan\n").unwrap();
        fs::write(root.path().join("Inbox.md"), "See [[Plan]] and [[Projects/Alpha/Plan|the plan]].\n").unwrap();
        let index = Vault::index(root.path(), None);
        (root, index)
    }

    #[test]
    fn a_new_note_never_takes_a_taken_name() {
        let (root, index) = vault();
        assert_eq!(create_note(root.path(), &index, "Idea", "Projects", "").unwrap(), "Projects/Idea.md");
        assert!(root.path().join("Projects/Idea.md").exists());
        assert_eq!(create_note(root.path(), &index, "inbox", "", ""), Err(MoveFailure::Exists("inbox.md".into())));
        assert!(matches!(create_note(root.path(), &index, "a/b", "", ""), Err(MoveFailure::BadName(_))));
        assert_eq!(create_folder(root.path(), "Notes", "Projects").unwrap(), "Projects/Notes");
        assert_eq!(create_folder(root.path(), "Notes", "Projects"), Err(MoveFailure::Exists("Projects/Notes".into())));
    }

    #[test]
    fn an_extract_writes_the_selection_and_links_by_the_shortest_name() {
        let (root, index) = vault();
        let text = "## Rollout\n\nShip it on Monday.";
        let name = EditorCommands::extract_name(text);
        assert_eq!(name, "Rollout");
        let path = create_note(root.path(), &index, &name, "Projects", &EditorCommands::extract_body(text)).unwrap();
        assert_eq!(fs::read_to_string(root.path().join(&path)).unwrap(), "Ship it on Monday.");
        let mut files = index.files.clone();
        files.insert(path.clone());
        assert_eq!(extract_link(&path, &files), "[[Rollout]]");
        // Another note has the name: the link says the path.
        files.insert("Old/rollout.md".into());
        assert_eq!(extract_link(&path, &files), "[[Projects/Rollout]]");
        // A name another note has gets a free one.
        assert_eq!(VaultMoves::free_name("Plan", "md", "Projects/Alpha", &files), "Projects/Alpha/Plan 1.md");
    }

    #[test]
    fn a_rename_keeps_the_note_a_note() {
        assert_eq!(renamed("Projects/Alpha/Plan.md", "Roadmap", true), "Projects/Alpha/Roadmap.md");
        assert_eq!(renamed("Projects/Alpha", "Beta", false), "Projects/Beta");
        assert_eq!(renamed("uploads/a.png", "b.png", true), "uploads/b.png");
    }

    #[test]
    fn folders_to_move_to_leave_out_where_it_is_and_itself() {
        let (_root, index) = vault();
        assert_eq!(folders(&index, "Projects/Alpha/Plan.md", ""), ["", "Empty", "Projects"]);
        assert_eq!(folders(&index, "Projects", ""), ["Empty"]);
        assert_eq!(folders(&index, "Inbox.md", "alp"), ["Projects/Alpha"]);
        assert_eq!(linking_notes(&index, "Projects"), 1);
        assert_eq!(linking_notes(&index, "Inbox.md"), 0);
    }

    #[test]
    fn a_move_takes_its_links_bookmark_and_place() {
        let (root, index) = vault();
        let store = VaultStore::new(root.path().to_path_buf(), index, Arc::new(SearchIndex::new()));
        toggle_bookmark(&store, "Projects/Alpha/Plan.md").unwrap();
        let done = move_path(&store, "Projects/Alpha", "Projects/Beta").unwrap();
        assert_eq!(done.moves.get("Projects/Alpha/Plan.md").map(String::as_str), Some("Projects/Beta/Plan.md"));
        let inbox = fs::read_to_string(root.path().join("Inbox.md")).unwrap();
        assert!(inbox.contains("[[Projects/Beta/Plan|the plan]]"), "{inbox}");
        assert!(store.bookmarks.borrow().contains("Projects/Beta/Plan.md"));
        assert!(Bookmarks::load(root.path()).contains("Projects/Beta/Plan.md"));
    }
}
