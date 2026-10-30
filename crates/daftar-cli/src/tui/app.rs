//! What the TUI shows and what a key does to it, with no terminal: the
//! tree, the open note, the lists over it, and the edits. Every rule is
//! `daftar-core`'s; this file only keeps the state.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use daftar_core::{
    Beautitab, Bookmarks, Candidate, Day, ExcludedFiles, Git, GitStatus, NoteParser, QueryTask, QuickSwitcher, Quote, ReadingPlaces,
    SearchIndex, Sections, SyncFailure, TagNode, TaskLine, TasksQuery, TreeNode, Vault, VaultEdits, VaultIndex, VaultTask, home,
    reading_places,
};

use super::markdown::{self, Block, Row};
use super::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Note,
}

/// A row of the sidebar: a folder or a file, at its depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub folder: bool,
    pub open: bool,
    pub folder_note: Option<String>,
}

pub struct OpenNote {
    pub path: String,
    pub text: String,
    pub blocks: Vec<Block>,
    /// The blocks wrapped to [width].
    pub rows: Vec<Row>,
    pub width: u16,
    /// The row the cursor is on, and the first row on screen.
    pub cursor: usize,
    pub scroll: usize,
    /// The line of the file to put the cursor on once the rows are known.
    pub wanted: Option<usize>,
}

impl OpenNote {
    /// The block under the cursor.
    pub fn block(&self) -> Option<&Block> {
        self.rows.get(self.cursor).and_then(|row| self.blocks.get(row.block))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Switcher,
    Search,
    Tasks,
    Backlinks,
    Tags,
    TagNotes,
    Links,
    Outline,
}

/// A row of a list. One with no path, task or tag is a heading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Item {
    pub line: Line<'static>,
    /// A second, dimmer line: a folder, a snippet.
    pub detail: Option<Line<'static>>,
    pub path: Option<String>,
    /// The line of the file, from 0.
    pub at: Option<usize>,
    pub task: Option<char>,
    pub tag: Option<String>,
    pub link: Option<String>,
    /// In the outline, the heading's place among the note's headings.
    pub heading: Option<usize>,
}

impl Item {
    pub fn selectable(&self) -> bool {
        self.path.is_some() || self.tag.is_some() || self.link.is_some()
    }
}

pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    /// What is typed, for a list that filters.
    pub query: Option<String>,
    pub items: Vec<Item>,
    pub selected: usize,
    pub scroll: usize,
}

pub enum Overlay {
    Picker(Picker),
    Help,
}

/// The note pane with no note, as the app's home page in text: the time, a
/// greeting, the notes read last, the bookmarks and a quote, as the vault's
/// Beautitab settings say.
#[derive(Debug, Clone, Default)]
pub struct Home {
    pub settings: Beautitab,
    /// The notes listed, under their headings: Recent, then Bookmarks.
    pub lists: Vec<(&'static str, Vec<String>)>,
    /// Of every note listed, in order.
    pub selected: usize,
    pub quote: Option<Quote>,
}

impl Home {
    /// Every note listed, in order: the same note may be in both lists.
    pub fn notes(&self) -> Vec<&String> {
        self.lists.iter().flat_map(|(_, notes)| notes).collect()
    }
}

/// Work the loop does with the terminal: an editor to hand the screen to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Edit(String, usize),
}

/// What a worker thread hands back: the vault read again, after its work.
pub struct Loaded {
    pub index: VaultIndex,
    pub search: SearchIndex,
    pub message: Option<(String, bool)>,
}

/// Where the last draw put each pane, for the mouse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Areas {
    pub tree: Rect,
    pub note: Rect,
    /// The open list's frame, its rows, and the item each row shows.
    pub picker: Rect,
    pub list: Rect,
    pub list_items: Vec<usize>,
    /// The home page's note on each row of the note pane, by its place in
    /// `Home::notes`.
    pub home_rows: Vec<Option<usize>>,
}

pub struct App {
    pub root: PathBuf,
    pub theme: Theme,
    pub index: VaultIndex,
    pub search: SearchIndex,
    pub git: Option<Git>,
    pub git_status: Option<GitStatus>,
    pub expanded: HashSet<String>,
    /// What the lists leave out, as in the app: Obsidian's excluded files
    /// and `files.hidden`, by these names; nothing while [show_hidden].
    pub hidden: ExcludedFiles,
    pub hidden_names: Vec<String>,
    pub show_hidden: bool,
    /// The tree without the hidden files, made again when the files change.
    nodes: Vec<TreeNode>,
    pub tree: Vec<TreeRow>,
    pub tree_selected: usize,
    pub tree_scroll: usize,
    pub note: Option<OpenNote>,
    /// What the note pane shows with no note.
    pub home: Home,
    /// The notes before this one, and where the cursor was in each.
    pub history: Vec<(String, usize)>,
    pub recent: Vec<String>,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    /// A line for the status bar, and whether it is an error.
    pub message: Option<(String, bool)>,
    pub job: Option<Job>,
    /// The headings closed in each note's outline, as `level:text`: the
    /// app's, read from its reading places, and this session's changes.
    closed: HashMap<String, Vec<String>>,
    /// What a worker thread does now, and since when, for the spinner.
    pub busy: Option<(String, Instant)>,
    worker: Option<Receiver<Loaded>>,
    /// Files that changed while the worker read the vault: read again after it.
    touched: BTreeSet<String>,
    pub quit: bool,
    /// The note pane's height, from the last draw.
    pub page: usize,
    pub areas: Areas,
}

const LIMIT: usize = 200;

impl App {
    /// The app on [root], read before it returns: for the tests.
    #[cfg(test)]
    pub fn new(root: PathBuf, theme: Theme) -> App {
        let search = SearchIndex::new();
        let index = Vault::index(&root, Some(&search));
        App::with(root, theme, index, search)
    }

    /// The app on [root] at once, the vault read on a worker thread.
    pub fn loading(root: PathBuf, theme: Theme) -> App {
        let mut app = App::with(root, theme, VaultIndex::default(), SearchIndex::new());
        app.spawn("Reading the vault", |_| None);
        app
    }

    fn with(root: PathBuf, theme: Theme, index: VaultIndex, search: SearchIndex) -> App {
        let git = Some(Git::new(&root)).filter(Git::is_repository);
        let hidden_names: Vec<String> = daftar_core::HIDDEN_NAMES.map(String::from).to_vec();
        let hidden = ExcludedFiles::load_with_names(&root, &hidden_names);
        let mut app = App {
            root,
            theme,
            index,
            search,
            git,
            git_status: None,
            expanded: HashSet::new(),
            hidden,
            hidden_names,
            show_hidden: false,
            nodes: Vec::new(),
            tree: Vec::new(),
            tree_selected: 0,
            tree_scroll: 0,
            note: None,
            home: Home::default(),
            history: Vec::new(),
            recent: Vec::new(),
            focus: Focus::Sidebar,
            overlay: None,
            message: None,
            job: None,
            closed: HashMap::new(),
            busy: None,
            worker: None,
            touched: BTreeSet::new(),
            quit: false,
            page: 20,
            areas: Areas::default(),
        };
        app.refresh_git();
        app.refresh_nodes();
        app.rebuild_tree();
        app.refresh_home();
        app
    }

    /// The hidden files are the ones named [names] and Obsidian's excluded
    /// files, and are listed anyway while [show]; the lists again.
    pub fn set_hidden(&mut self, names: Vec<String>, show: bool) {
        self.hidden = if show { ExcludedFiles::default() } else { ExcludedFiles::load_with_names(&self.root, &names) };
        self.hidden_names = names;
        self.show_hidden = show;
        self.refresh_nodes();
        self.rebuild_tree();
        self.reread();
    }

    /// `.`: the hidden files shown, or hidden again, as Ctrl+Shift+. in the app.
    pub fn toggle_hidden(&mut self) {
        self.set_hidden(self.hidden_names.clone(), !self.show_hidden);
        let what = if self.show_hidden { "Hidden files shown" } else { "Hidden files hidden" };
        self.message = Some((what.into(), false));
    }

    /// Whether the lists leave [path] out. A link to it still opens it.
    pub fn is_hidden(&self, path: &str) -> bool {
        self.hidden.contains(path)
    }

    fn refresh_nodes(&mut self) {
        self.nodes = if self.hidden.is_empty() {
            self.index.tree().to_vec()
        } else {
            self.hidden.tree(&self.index.files, &self.index.empty_folders)
        };
    }

    pub fn vault_name(&self) -> String {
        self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    pub fn refresh_git(&mut self) {
        self.git_status = self.git.as_ref().and_then(|git| git.status().ok());
    }

    // The tree

    pub fn rebuild_tree(&mut self) {
        let selected = self.tree.get(self.tree_selected).map(|row| row.path.clone());
        let mut rows = Vec::new();
        flatten(&self.nodes, 0, &self.expanded, &mut rows);
        self.tree = rows;
        self.tree_selected = selected.and_then(|path| self.tree.iter().position(|row| row.path == path)).unwrap_or(0);
    }

    pub fn tree_move(&mut self, by: isize) {
        let last = self.tree.len().saturating_sub(1) as isize;
        self.tree_selected = (self.tree_selected as isize + by).clamp(0, last.max(0)) as usize;
    }

    /// Enter on the tree: a folder opens or closes, and shows its folder note.
    pub fn tree_activate(&mut self) {
        let Some(row) = self.tree.get(self.tree_selected).cloned() else { return };
        if row.folder {
            if !self.expanded.remove(&row.path) {
                self.expanded.insert(row.path.clone());
            }
            self.rebuild_tree();
            if let Some(note) = row.folder_note {
                self.open(&note, None, true);
                self.focus = Focus::Sidebar;
            }
        } else {
            self.open(&row.path, None, true);
        }
    }

    /// Left on the tree: closes the folder, or goes to the one it is in.
    pub fn tree_collapse(&mut self) {
        let Some(row) = self.tree.get(self.tree_selected).cloned() else { return };
        if row.folder && row.open {
            self.expanded.remove(&row.path);
            self.rebuild_tree();
        } else if let Some(parent) = row.path.rsplit_once('/').map(|(p, _)| p.to_string())
            && let Some(at) = self.tree.iter().position(|r| r.path == parent)
        {
            self.tree_selected = at;
        }
    }

    /// Opens the folders above [path] and puts the tree's selection on it.
    fn reveal(&mut self, path: &str) {
        let mut folder = String::new();
        for part in path.split('/').collect::<Vec<_>>().split_last().map(|(_, dirs)| dirs.to_vec()).unwrap_or_default() {
            folder = if folder.is_empty() { part.to_string() } else { format!("{folder}/{part}") };
            self.expanded.insert(folder.clone());
        }
        self.rebuild_tree();
        if let Some(at) = self.tree.iter().position(|row| row.path == path || row.folder_note.as_deref() == Some(path)) {
            self.tree_selected = at;
        }
    }

    // The note

    /// Shows [path], with the cursor on [line] of the file when given.
    pub fn open(&mut self, path: &str, line: Option<usize>, remember: bool) {
        if !Vault::is_note(path) {
            let what = if Vault::is_base(path) { "A base opens in the app" } else { "This file opens in the app" };
            self.message = Some((format!("{what}: {path}"), false));
            return;
        }
        let Ok(text) = fs::read_to_string(self.root.join(path)) else {
            self.message = Some((format!("{path} could not be read"), true));
            return;
        };
        if remember
            && let Some(note) = &self.note
            && note.path != path
        {
            self.history.push((note.path.clone(), note.cursor));
        }
        let parsed = NoteParser::parse(&text);
        let blocks = markdown::blocks(&text, &parsed.properties, &self.theme, &Day::today().to_string());
        self.note = Some(OpenNote { path: path.to_string(), text, blocks, rows: Vec::new(), width: 0, cursor: 0, scroll: 0, wanted: line });
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_string());
        self.focus = Focus::Note;
        self.message = None;
        self.reveal(path);
    }

    /// Wraps the note to [width] when it changed, and keeps the cursor on screen.
    pub fn layout(&mut self, width: u16, height: usize) {
        self.page = height.max(1);
        let theme = self.theme;
        let Some(note) = &mut self.note else { return };
        if note.width != width {
            let block = note.rows.get(note.cursor).map(|r| r.block);
            note.rows = markdown::wrap(&note.blocks, width, &theme);
            note.width = width;
            if let Some(block) = block {
                note.cursor = note.rows.iter().position(|r| r.block == block).unwrap_or(0);
            }
        }
        if let Some(line) = note.wanted.take() {
            note.cursor = note
                .rows
                .iter()
                .position(|r| note.blocks[r.block].source.is_some_and(|s| s >= line))
                .unwrap_or_else(|| note.rows.len().saturating_sub(1));
            note.scroll = note.cursor.saturating_sub(height / 3);
        }
        note.cursor = note.cursor.min(note.rows.len().saturating_sub(1));
        if note.cursor < note.scroll {
            note.scroll = note.cursor;
        } else if note.cursor >= note.scroll + height {
            note.scroll = note.cursor + 1 - height;
        }
    }

    pub fn note_move(&mut self, by: isize) {
        if let Some(note) = &mut self.note {
            let last = note.rows.len().saturating_sub(1) as isize;
            note.cursor = (note.cursor as isize + by).clamp(0, last.max(0)) as usize;
        }
    }

    /// The wheel on the tree: the rows move, the selection stays on screen.
    pub fn tree_scroll_by(&mut self, by: isize) {
        let height = (self.areas.tree.height as usize).max(1);
        let last = self.tree.len().saturating_sub(height);
        self.tree_scroll = (self.tree_scroll as isize + by).clamp(0, last as isize) as usize;
        let bottom = (self.tree_scroll + height - 1).min(self.tree.len().saturating_sub(1));
        self.tree_selected = self.tree_selected.clamp(self.tree_scroll.min(bottom), bottom);
    }

    /// The wheel on the note: the rows move, the cursor stays on screen.
    pub fn note_scroll_by(&mut self, by: isize) {
        let height = self.page.max(1);
        let Some(note) = &mut self.note else { return };
        let last = note.rows.len().saturating_sub(height);
        note.scroll = (note.scroll as isize + by).clamp(0, last as isize) as usize;
        let bottom = (note.scroll + height - 1).min(note.rows.len().saturating_sub(1));
        note.cursor = note.cursor.clamp(note.scroll.min(bottom), bottom);
    }

    /// The cursor to the next or the previous row with a link or a task.
    pub fn note_jump(&mut self, forward: bool) {
        let Some(note) = &mut self.note else { return };
        let interesting = |row: &Row| {
            let block = &note.blocks[row.block];
            !block.links.is_empty() || block.task.is_some()
        };
        let current = note.rows.get(note.cursor).map(|r| r.block);
        let found = if forward {
            (note.cursor + 1..note.rows.len()).find(|&i| Some(note.rows[i].block) != current && interesting(&note.rows[i]))
        } else {
            (0..note.cursor).rev().find(|&i| Some(note.rows[i].block) != current && interesting(&note.rows[i]))
        };
        match found {
            Some(at) => note.cursor = at,
            None => self.message = Some(("No more links or tasks".into(), false)),
        }
    }

    /// H: the home page in the note pane, as the app's Home button; Back
    /// returns to the note that was open.
    pub fn go_home(&mut self) {
        if let Some(note) = self.note.take() {
            self.history.push((note.path, note.cursor));
        }
        self.home.quote = None;
        self.home.selected = 0;
        self.refresh_home();
        self.focus = if self.home.notes().is_empty() { Focus::Sidebar } else { Focus::Note };
    }

    /// The home page's lists and settings read again: the notes opened here
    /// first, then those the app read last, while they are in the vault.
    pub fn refresh_home(&mut self) {
        let settings = Beautitab::read(&self.root);
        let selected = self.home.notes().get(self.home.selected).map(|p| (*p).clone());
        let files = &self.index.files;
        let mut lists = Vec::new();
        if settings.show_recent_files {
            let read = fs::read_to_string(reading_places::state_file(&self.root, "places"))
                .map(|data| ReadingPlaces::from_data(&data).recently_read(40))
                .unwrap_or_default();
            let recent = home::recent(&self.recent, &read, |p| files.contains(p) && !self.hidden.contains(p), 8);
            if !recent.is_empty() {
                lists.push(("Recent", recent));
            }
        }
        if settings.show_bookmarks {
            let marked: Vec<String> = Bookmarks::load(&self.root)
                .entries()
                .into_iter()
                .filter(|e| !e.folder && files.contains(&e.path))
                .map(|e| e.path)
                .take(8)
                .collect();
            if !marked.is_empty() {
                lists.push(("Bookmarks", marked));
            }
        }
        // One quote for each visit, kept while it is still one of them.
        let quote = self.home.quote.take().filter(|q| settings.show_quote && settings.quotes.contains(q)).or_else(|| {
            let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize);
            settings.show_quote.then(|| settings.quotes.get(at % settings.quotes.len().max(1)).cloned()).flatten()
        });
        self.home = Home { settings, lists, selected: 0, quote };
        let notes = self.home.notes();
        self.home.selected = selected.and_then(|path| notes.iter().position(|p| **p == path)).unwrap_or(0);
    }

    pub fn home_move(&mut self, by: isize) {
        let last = self.home.notes().len().saturating_sub(1) as isize;
        self.home.selected = (self.home.selected as isize + by).clamp(0, last.max(0)) as usize;
    }

    /// Enter on the home page: the note chosen opens.
    pub fn home_activate(&mut self) {
        if let Some(path) = self.home.notes().get(self.home.selected).map(|p| (*p).clone()) {
            self.open(&path, None, true);
        }
    }

    pub fn back(&mut self) {
        match self.history.pop() {
            Some((path, cursor)) => {
                self.open(&path, None, false);
                if let Some(note) = &mut self.note {
                    note.cursor = cursor;
                    note.scroll = cursor.saturating_sub(self.page / 3);
                }
            }
            None => self.message = Some(("Nothing to go back to".into(), false)),
        }
    }

    /// Enter on the note: follows the link on the line, or asks which one.
    pub fn follow(&mut self) {
        let Some(links) = self.note.as_ref().and_then(|n| n.block()).map(|b| b.links.clone()) else { return };
        match links.len() {
            0 => self.message = Some(("No link on this line".into(), false)),
            1 => self.follow_link(&links[0]),
            _ => {
                let items = links
                    .iter()
                    .map(|link| Item {
                        line: Line::from(Span::styled(link.clone(), Style::new().fg(self.theme.link))),
                        link: Some(link.clone()),
                        ..Item::default()
                    })
                    .collect();
                self.show_picker(PickerKind::Links, "Links on this line", None, items);
            }
        }
    }

    pub fn follow_link(&mut self, target: &str) {
        let Some(from) = self.note.as_ref().map(|n| n.path.clone()) else { return };
        if target.contains("://") || target.starts_with("mailto:") {
            self.message = Some((format!("A web link: {target}"), false));
            return;
        }
        let decoded = target.replace("%20", " ");
        let (file, heading) = match decoded.split_once('#') {
            Some((file, heading)) => (file.to_string(), Some(heading.to_string())),
            None => (decoded.clone(), None),
        };
        let Some(path) = self.index.resolve(&file, &from) else {
            self.message = Some((format!("No note “{file}” yet"), true));
            return;
        };
        let line = heading.and_then(|heading| {
            let wanted = daftar_core::text::slugs::heading(&heading);
            self.index
                .notes
                .get(&path)?
                .parsed
                .headings
                .iter()
                .find(|h| daftar_core::text::slugs::heading(&h.text) == wanted)
                .map(|h| h.line)
        });
        self.open(&path, line, true);
    }

    /// Space on the note: ticks the task on the line, or opens it again.
    pub fn toggle_here(&mut self) {
        let Some(note) = &self.note else { return };
        let Some(block) = note.block() else { return };
        let (Some(status), Some(line)) = (block.task, block.source) else {
            self.message = Some(("No task on this line".into(), false));
            return;
        };
        let path = note.path.clone();
        self.toggle(&path, line, status);
    }

    /// Writes the tick through `VaultEdits`, which refuses when the line is
    /// no longer the task that was shown.
    pub fn toggle(&mut self, path: &str, line: usize, status: char) {
        let result = VaultEdits::apply(path, &self.root, |text| VaultEdits::toggle_task(text, line, status, Day::today()));
        match result {
            Ok(()) => {
                let done = if TaskLine::is_closed(status) { "Opened again" } else { "Done" };
                self.message = Some((format!("{done}: {path}:{}", line + 1), false));
                self.changed(&[path.to_string()]);
            }
            Err(error) => self.message = Some((format!("Not changed: {error}"), true)),
        }
    }

    /// [paths] changed on disk: the index, the tree, the note and the list again.
    pub fn changed(&mut self, paths: &[String]) {
        let changed: BTreeSet<String> = paths.iter().cloned().collect();
        self.index = Vault::update(&self.index, &changed, &self.root, Some(&self.search));
        self.refresh_nodes();
        self.rebuild_tree();
        self.reread();
        self.refresh_git();
    }

    /// The open note from disk, the cursor where it was, and the tree's
    /// selection where it was; and the list over them again.
    fn reread(&mut self) {
        let tree = (self.tree.get(self.tree_selected).map(|r| r.path.clone()), self.tree_scroll);
        if let Some((path, cursor, scroll)) = self.note.as_ref().map(|n| (n.path.clone(), n.cursor, n.scroll)) {
            if self.root.join(&path).is_file() {
                let message = self.message.clone();
                let focus = self.focus;
                self.open(&path, None, false);
                self.message = message;
                self.focus = focus;
                if let Some(note) = &mut self.note {
                    note.cursor = cursor;
                    note.scroll = scroll;
                }
            } else {
                self.note = None;
                self.focus = Focus::Sidebar;
                self.message = Some((format!("{path} is gone"), false));
            }
        }
        self.refresh_home();
        if let Some(at) = tree.0.and_then(|path| self.tree.iter().position(|row| row.path == path)) {
            self.tree_selected = at;
            self.tree_scroll = tree.1;
        }
        let Some(Overlay::Picker(picker)) = &self.overlay else { return };
        let (kind, selected) = (picker.kind, picker.selected);
        let chosen = picker.items.get(selected).and_then(|i| Some((i.path.clone()?, i.at)));
        match kind {
            PickerKind::Tasks => self.open_tasks(),
            PickerKind::Switcher | PickerKind::Search => self.refilter(),
            _ => return,
        }
        if let Some(picker) = self.picker() {
            // The same file when it is still listed, else the same row.
            picker.selected = chosen
                .and_then(|(path, at)| picker.items.iter().position(|i| i.path.as_deref() == Some(path.as_str()) && i.at == at))
                .unwrap_or_else(|| selected.min(picker.items.len().saturating_sub(1)));
        }
    }

    /// The watcher says [paths] changed: the same as an edit made here.
    pub fn files_changed(&mut self, paths: BTreeSet<String>) {
        if self.busy.is_some() {
            self.touched.extend(paths.iter().cloned());
        }
        let paths: Vec<String> = paths.into_iter().collect();
        self.changed(&paths);
    }

    pub fn edit(&mut self) {
        match &self.note {
            Some(note) => {
                let line = note.block().and_then(|b| b.source).unwrap_or(0);
                self.job = Some(Job::Edit(note.path.clone(), line));
            }
            None => self.message = Some(("Open a note to edit it".into(), false)),
        }
    }

    /// After the editor: the note as it was saved.
    pub fn edited(&mut self, path: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.message = None;
                self.changed(&[path.to_string()]);
            }
            Err(error) => self.message = Some((error, true)),
        }
    }

    pub fn start_sync(&mut self) {
        if self.git.is_none() {
            self.message = Some(("The vault is not a git repository".into(), true));
            return;
        }
        // Commit, pull with a rebase, push, as `daftar-cli sync`; then every
        // file again, since a pull may change any of them.
        self.spawn("Syncing", |root| {
            let git = Git::new(root);
            let message = Git::message(chrono::Local::now(), &Git::host());
            Some(match git.sync(&message) {
                Ok(done) => {
                    let mut steps = Vec::new();
                    if done.committed {
                        steps.push("committed");
                    }
                    if done.pulled {
                        steps.push("pulled");
                    }
                    if done.pushed {
                        steps.push("pushed");
                    }
                    (if steps.is_empty() { "Up to date".to_string() } else { format!("Synced: {}", steps.join(", ")) }, false)
                }
                Err(SyncFailure::Stopped(stopped)) => (format!("{stopped} Resolve it in Daftar, or run daftar-cli sync --abort"), true),
                Err(SyncFailure::Git(failure)) => (format!("Sync failed: {}", failure.output.lines().last().unwrap_or("")), true),
            })
        });
    }

    /// `r`: every file again, on a worker thread.
    pub fn reindex(&mut self) {
        self.spawn("Reading the vault", |_| Some(("Read the vault again".into(), false)));
    }

    /// Runs [work] and then reads the whole vault on a thread; `poll` takes
    /// the result. One at a time: a key that starts another waits for it.
    fn spawn(&mut self, label: &str, work: impl FnOnce(&Path) -> Option<(String, bool)> + Send + 'static) {
        if let Some((doing, _)) = &self.busy {
            self.message = Some((format!("Still {}…", doing.to_lowercase()), false));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let root = self.root.clone();
        thread::spawn(move || {
            let message = work(&root);
            let search = SearchIndex::new();
            let index = Vault::index(&root, Some(&search));
            let _ = sender.send(Loaded { index, search, message });
        });
        self.worker = Some(receiver);
        self.busy = Some((label.to_string(), Instant::now()));
        self.touched.clear();
        self.message = None;
    }

    /// The worker's result, when it is ready; whether it was.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.worker else { return false };
        let loaded = match receiver.try_recv() {
            Ok(loaded) => loaded,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.worker = None;
                self.busy = None;
                self.message = Some(("The vault could not be read".into(), true));
                return true;
            }
        };
        self.worker = None;
        self.busy = None;
        self.index = loaded.index;
        self.search = loaded.search;
        // What changed while the thread read is read again on top.
        let touched = std::mem::take(&mut self.touched);
        if !touched.is_empty() {
            self.index = Vault::update(&self.index, &touched, &self.root, Some(&self.search));
        }
        if loaded.message.is_some() {
            self.message = loaded.message;
        }
        self.refresh_nodes();
        self.rebuild_tree();
        self.reread();
        self.refresh_git();
        true
    }

    // The lists

    fn show_picker(&mut self, kind: PickerKind, title: &str, query: Option<String>, items: Vec<Item>) {
        let selected = items.iter().position(Item::selectable).unwrap_or(0);
        self.overlay = Some(Overlay::Picker(Picker { kind, title: title.into(), query, items, selected, scroll: 0 }));
    }

    pub fn picker(&mut self) -> Option<&mut Picker> {
        match &mut self.overlay {
            Some(Overlay::Picker(picker)) => Some(picker),
            _ => None,
        }
    }

    pub fn open_switcher(&mut self) {
        self.show_picker(PickerKind::Switcher, "Open", Some(String::new()), Vec::new());
        self.refilter();
    }

    pub fn open_search(&mut self) {
        self.show_picker(PickerKind::Search, "Search", Some(String::new()), Vec::new());
        self.refilter();
    }

    /// The list again for what is typed now.
    pub fn refilter(&mut self) {
        let Some(Overlay::Picker(picker)) = &self.overlay else { return };
        let query = picker.query.clone().unwrap_or_default();
        let items = match picker.kind {
            PickerKind::Switcher => self.switcher_items(&query),
            PickerKind::Search => self.search_items(&query),
            _ => return,
        };
        if let Some(picker) = self.picker() {
            picker.selected = items.iter().position(Item::selectable).unwrap_or(0);
            picker.scroll = 0;
            picker.items = items;
        }
    }

    fn switcher_items(&self, query: &str) -> Vec<Item> {
        let candidates: Vec<Candidate> = self
            .index
            .files
            .iter()
            .filter(|path| !self.is_hidden(path))
            .map(|path| {
                let aliases = self.index.notes.get(path).map(|n| n.parsed.aliases.clone()).unwrap_or_default();
                Candidate::new(path, &Vault::title(path), &aliases)
            })
            .collect();
        let found = if query.trim().is_empty() {
            let mut paths: Vec<String> = self.recent.iter().filter(|p| !self.is_hidden(p)).cloned().collect();
            paths.extend(self.index.notes.keys().filter(|p| !self.recent.contains(p) && !self.is_hidden(p)).take(LIMIT).cloned());
            paths.truncate(LIMIT);
            paths
        } else {
            QuickSwitcher::rank(query, &candidates, &self.recent, LIMIT)
        };
        let theme = self.theme;
        found
            .into_iter()
            .map(|path| {
                let title = Vault::title(&path);
                let folder = QuickSwitcher::folder(&path);
                let (in_title, in_folder) = QuickSwitcher::highlights(query, &title, &folder);
                let marked = |text: &str, marks: &BTreeSet<usize>, style: Style| -> Line<'static> {
                    Line::from(
                        text.chars()
                            .enumerate()
                            .map(|(at, c)| {
                                let style =
                                    if marks.contains(&at) { Style::new().fg(theme.accent).add_modifier(Modifier::BOLD) } else { style };
                                Span::styled(c.to_string(), style)
                            })
                            .collect::<Vec<_>>(),
                    )
                };
                let style = if Vault::is_note(&path) { Style::new().fg(theme.text) } else { theme.muted() };
                Item {
                    line: marked(&title, &in_title, style),
                    detail: (!folder.is_empty()).then(|| marked(&folder, &in_folder, theme.faint())),
                    path: Some(path),
                    ..Item::default()
                }
            })
            .collect()
    }

    fn search_items(&self, query: &str) -> Vec<Item> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let theme = self.theme;
        self.search
            .search(query, 200)
            .into_iter()
            .filter(|hit| !self.is_hidden(&hit.path))
            .take(100)
            .map(|hit| {
                let mut spans = Vec::new();
                for (n, piece) in hit.snippet.replace('\n', " ").split(['\u{1}', '\u{2}']).enumerate() {
                    let style = if n % 2 == 1 { Style::new().fg(theme.accent).add_modifier(Modifier::BOLD) } else { theme.muted() };
                    spans.push(Span::styled(piece.to_string(), style));
                }
                Item {
                    line: Line::from(Span::styled(Vault::title(&hit.path), Style::new().fg(theme.text))),
                    detail: Some(Line::from(spans)),
                    path: Some(hit.path),
                    ..Item::default()
                }
            })
            .collect()
    }

    /// The note's ` ```tasks ` queries, groups as headings; or, when it has
    /// none, every open task, the late ones first.
    pub fn open_tasks(&mut self) {
        let theme = self.theme;
        let mut tasks = self.hidden.tasks(&self.index);
        tasks.sort_by(|a, b| (&a.path, a.task.line).cmp(&(&b.path, b.task.line)));
        let today = Day::today();
        let mut items = Vec::new();
        let queries = self.note.as_ref().map(|n| (n.path.clone(), crate::query_blocks(&n.text))).filter(|(_, q)| !q.is_empty());
        let title = if let Some((path, sources)) = queries {
            for (n, source) in sources.iter().enumerate() {
                let query = TasksQuery::parse(source, Some(&path));
                items.push(heading(&format!("Query {}", n + 1), 0, &theme));
                for line in &query.not_understood {
                    items.push(Item {
                        line: Line::from(vec![
                            Span::styled("⚠ not understood: ", Style::new().fg(theme.orange)),
                            Span::styled(line.clone(), theme.muted()),
                        ]),
                        ..Item::default()
                    });
                }
                let mut open: Vec<String> = Vec::new();
                for task in query.run(tasks.clone(), today, |t| QueryTask::from(t)) {
                    for (level, name) in query.groups(&QueryTask::from(&task)).into_iter().enumerate() {
                        if open.get(level) == Some(&name) {
                            continue;
                        }
                        open.truncate(level);
                        open.push(name.clone());
                        if !name.is_empty() {
                            items.push(heading(&name, level + 1, &theme));
                        }
                    }
                    items.push(task_item(&task, &theme, &today.to_string()));
                }
            }
            format!("Tasks · {}", Vault::title(&path))
        } else {
            let mut open: Vec<VaultTask> = tasks.into_iter().filter(|t| t.task.is_open()).collect();
            open.sort_by(|a, b| {
                let day = |t: &VaultTask| t.task.day().unwrap_or("9999").to_string();
                day(a).cmp(&day(b)).then(a.path.cmp(&b.path)).then(a.task.line.cmp(&b.task.line))
            });
            let today = today.to_string();
            let mut section = "";
            for task in &open {
                let name = match task.task.day() {
                    Some(day) if day < today.as_str() => "Late",
                    Some(day) if day == today => "Today",
                    Some(_) => "Coming",
                    None => "No date",
                };
                if name != section {
                    items.push(heading(name, 0, &theme));
                    section = name;
                }
                items.push(task_item(task, &theme, &today));
            }
            "Open tasks".to_string()
        };
        if !items.iter().any(Item::selectable) {
            items.push(Item { line: Line::from(Span::styled("No tasks match.", theme.muted())), ..Item::default() });
        }
        self.show_picker(PickerKind::Tasks, &title, None, items);
    }

    pub fn open_backlinks(&mut self) {
        let Some(path) = self.note.as_ref().map(|n| n.path.clone()) else {
            self.message = Some(("Open a note to see what links to it".into(), false));
            return;
        };
        let theme = self.theme;
        let mut items: Vec<Item> = Vec::new();
        for link in self.index.backlinks(&path).iter().filter(|link| !self.hidden.contains(&link.source)) {
            let text = fs::read_to_string(self.root.join(&link.source)).unwrap_or_default();
            let line = text.split('\n').nth(link.line).unwrap_or("").trim().to_string();
            let (spans, _) = markdown::inline(&line, theme.muted(), &theme);
            items.push(Item {
                line: Line::from(vec![
                    Span::styled(Vault::title(&link.source), Style::new().fg(theme.text)),
                    Span::styled(format!("  :{}", link.line + 1), theme.faint()),
                ]),
                detail: Some(Line::from(spans)),
                path: Some(link.source.clone()),
                at: Some(link.line),
                ..Item::default()
            });
        }
        if items.is_empty() {
            items.push(Item { line: Line::from(Span::styled("No note links here.", theme.muted())), ..Item::default() });
        }
        self.show_picker(PickerKind::Backlinks, &format!("Links to {}", Vault::title(&path)), None, items);
    }

    /// The note's outline: its headings, those under a closed one left out.
    /// Enter goes to one, Space closes or opens it, `s` sorts the headings
    /// right under it.
    pub fn open_outline(&mut self) {
        self.show_outline(None);
    }

    /// The outline with heading [keep] selected, when it is shown.
    fn show_outline(&mut self, keep: Option<usize>) {
        let Some((path, text)) = self.note.as_ref().map(|n| (n.path.clone(), n.text.clone())) else {
            self.message = Some(("Open a note to see its outline".into(), false));
            return;
        };
        let theme = self.theme;
        let headings = Sections::headings(&text);
        let closed = self.closed_in(&path);
        let mut items: Vec<Item> = Vec::new();
        for index in Sections::visible(&headings, &closed) {
            let heading = &headings[index];
            let arrow = match (Sections::has_children(index, &headings), closed.contains(&Sections::key(heading))) {
                (false, _) => "  ",
                (true, false) => "▾ ",
                (true, true) => "▸ ",
            };
            items.push(Item {
                line: Line::from(vec![
                    Span::raw("  ".repeat(heading.level.saturating_sub(1))),
                    Span::styled(arrow, theme.faint()),
                    Span::styled(heading.text.clone(), theme.heading(heading.level)),
                ]),
                path: Some(path.clone()),
                at: Some(heading.line),
                heading: Some(index),
                ..Item::default()
            });
        }
        if items.is_empty() {
            items.push(Item { line: Line::from(Span::styled("No headings.", theme.muted())), ..Item::default() });
        }
        let selected = keep.and_then(|keep| items.iter().position(|i| i.heading == Some(keep)));
        self.show_picker(PickerKind::Outline, &format!("Outline of {}", Vault::title(&path)), None, items);
        if let (Some(selected), Some(picker)) = (selected, self.picker()) {
            picker.selected = selected;
        }
    }

    /// The headings closed in [path]'s outline: this session's, else the app's.
    fn closed_in(&mut self, path: &str) -> Vec<String> {
        if let Some(closed) = self.closed.get(path) {
            return closed.clone();
        }
        let places = fs::read_to_string(reading_places::state_file(&self.root, "places")).map(|data| ReadingPlaces::from_data(&data));
        let closed = places.map(|p| p.closed(path)).unwrap_or_default();
        self.closed.insert(path.to_string(), closed.clone());
        closed
    }

    /// The selected heading of the outline: the note and its place.
    fn outline_selected(&self) -> Option<(String, usize)> {
        let Some(Overlay::Picker(picker)) = &self.overlay else { return None };
        let item = picker.items.get(picker.selected)?;
        Some((item.path.clone()?, item.heading?))
    }

    /// Space in the outline: the heading closed if open, opened if closed.
    pub fn outline_toggle(&mut self) {
        let Some((path, index)) = self.outline_selected() else { return };
        let headings = self.note.as_ref().map(|n| Sections::headings(&n.text)).unwrap_or_default();
        if !Sections::has_children(index, &headings) {
            return;
        }
        let key = Sections::key(&headings[index]);
        let mut closed = self.closed_in(&path);
        if closed.contains(&key) {
            closed.retain(|k| *k != key);
        } else {
            closed.push(key);
        }
        self.closed.insert(path, closed);
        self.show_outline(Some(index));
    }

    /// `s` in the outline: the headings right under the selected one in the
    /// order of their names, each with its section, in the file.
    pub fn outline_sort(&mut self) {
        let Some((path, index)) = self.outline_selected() else { return };
        let mut why = None;
        let result = VaultEdits::apply(&path, &self.root, |text| {
            Ok(Sections::sort_headings(text, Some(index), 0).unwrap_or_else(|reason| {
                why = Some(reason);
                text.to_string()
            }))
        });
        match (result, why) {
            (Err(error), _) => self.message = Some((format!("Not changed: {error}"), true)),
            (Ok(()), Some(why)) => self.message = Some((why, false)),
            (Ok(()), None) => {
                self.changed(std::slice::from_ref(&path));
                self.message = Some(("Headings sorted by name".into(), false));
            }
        }
        self.show_outline(Some(index));
    }

    pub fn open_tags(&mut self) {
        let theme = self.theme;
        let mut items = Vec::new();
        fn walk(nodes: &[TagNode], depth: usize, theme: &Theme, out: &mut Vec<Item>) {
            for node in nodes {
                out.push(Item {
                    line: Line::from(vec![
                        Span::raw("  ".repeat(depth)),
                        Span::styled(format!("#{}", node.name), Style::new().fg(theme.accent)),
                        Span::styled(format!("  {}", node.count), theme.faint()),
                    ]),
                    tag: Some(node.tag.clone()),
                    ..Item::default()
                });
                if let Some(children) = &node.children {
                    walk(children, depth + 1, theme, out);
                }
            }
        }
        walk(&self.hidden.tag_tree(&self.index), 0, &theme, &mut items);
        self.show_picker(PickerKind::Tags, "Tags", None, items);
    }

    fn open_tag(&mut self, tag: &str) {
        let theme = self.theme;
        let fold = tag.to_lowercase();
        let mut paths: Vec<&String> = self
            .index
            .notes
            .values()
            .filter(|n| !self.hidden.contains(&n.path))
            .filter(|n| n.parsed.tags.iter().any(|t| t.to_lowercase() == fold || t.to_lowercase().starts_with(&format!("{fold}/"))))
            .map(|n| &n.path)
            .collect();
        paths.sort();
        let items = paths
            .into_iter()
            .map(|path| Item {
                line: Line::from(Span::styled(Vault::title(path), Style::new().fg(theme.text))),
                detail: Some(Line::from(Span::styled(QuickSwitcher::folder(path), theme.faint()))),
                path: Some(path.clone()),
                ..Item::default()
            })
            .collect();
        self.show_picker(PickerKind::TagNotes, &format!("#{tag}"), None, items);
    }

    /// Moves the selection [by] selectable rows, stopping at either end.
    pub fn picker_move(&mut self, by: isize) {
        let Some(picker) = self.picker() else { return };
        let step = by.signum();
        let mut at = picker.selected as isize;
        for _ in 0..by.abs() {
            let mut next = at + step;
            while next >= 0 && (next as usize) < picker.items.len() && !picker.items[next as usize].selectable() {
                next += step;
            }
            if next < 0 || next as usize >= picker.items.len() {
                break;
            }
            at = next;
        }
        picker.selected = at as usize;
    }

    /// Enter on a list.
    pub fn picker_activate(&mut self) {
        let Some(Overlay::Picker(picker)) = &self.overlay else { return };
        let Some(item) = picker.items.get(picker.selected).cloned() else { return };
        if !item.selectable() {
            return;
        }
        if let Some(tag) = item.tag {
            self.open_tag(&tag);
            return;
        }
        self.overlay = None;
        if let Some(link) = item.link {
            self.follow_link(&link);
        } else if let Some(path) = item.path {
            self.open(&path, item.at, true);
        }
    }

    /// Space on the tasks list.
    pub fn picker_toggle(&mut self) {
        let Some(Overlay::Picker(picker)) = &self.overlay else { return };
        let Some(item) = picker.items.get(picker.selected) else { return };
        if let (Some(path), Some(line), Some(status)) = (item.path.clone(), item.at, item.task) {
            self.toggle(&path, line, status);
        }
    }
}

fn flatten(nodes: &[TreeNode], depth: usize, expanded: &HashSet<String>, out: &mut Vec<TreeRow>) {
    for node in nodes {
        let open = node.is_folder() && expanded.contains(&node.path);
        out.push(TreeRow {
            path: node.path.clone(),
            name: node.name.clone(),
            depth,
            folder: node.is_folder(),
            open,
            folder_note: node.folder_note.clone(),
        });
        if open && let Some(children) = &node.children {
            flatten(children, depth + 1, expanded, out);
        }
    }
}

fn heading(name: &str, level: usize, theme: &Theme) -> Item {
    let style = theme.heading(level + 1);
    Item { line: Line::from(Span::styled(format!("{}{name}", "  ".repeat(level.saturating_sub(1))), style)), ..Item::default() }
}

fn task_item(task: &VaultTask, theme: &Theme, today: &str) -> Item {
    let t = &task.task;
    let raw = format!("- [{}] {}", t.status, t.text);
    let mut block = markdown::blocks(&raw, &[], theme, today).into_iter().next().unwrap_or_default();
    let mut spans = std::mem::take(&mut block.prefix);
    spans.extend(block.spans);
    let closed = TaskLine::is_closed(t.status);
    let status = daftar_core::TaskStatuses::current().status(t.status);
    if !status.is_default() {
        spans.push(Span::styled(format!("  {}", status.name), Style::new().fg(theme.purple)));
    }
    if let Some(reason) = &t.reason {
        spans.push(Span::styled(format!("  💬 {reason}"), Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC)));
    }
    for (mark, value, color) in [("⏳", &t.scheduled, theme.blue), ("📅", &t.due, theme.orange), ("✅", &t.done, theme.green)] {
        if let Some(value) = value {
            let color = if mark == "📅" && !closed && value.as_str() < today { theme.red } else { color };
            spans.push(Span::styled(format!("  {mark} {value}"), Style::new().fg(color)));
        }
    }
    spans.push(Span::styled(format!("  · {}", Vault::title(&task.path)), theme.faint()));
    Item { line: Line::from(spans), path: Some(task.path.clone()), at: Some(t.line), task: Some(t.status), ..Item::default() }
}
