//! The files of a vault on disk and everything known about them at one
//! moment: the notes, the links between them, the tree and the tags.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use rayon::prelude::*;
use regex::Regex;
use walkdir::WalkDir;

use crate::link_resolver::LinkResolver;
use crate::note_parser::{NoteParser, ParsedNote};
use crate::search::SearchIndex;
use crate::tag_tree::TagNode;
use crate::task_line::ParsedTask;
use crate::task_status::TaskStatuses;
use crate::text::{slugs, standard_compare, trim};

/// One note in the index.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteEntry {
    pub path: String,
    pub modified: SystemTime,
    pub parsed: ParsedNote,
}

impl NoteEntry {
    /// The file name without `.md`, which is what Obsidian shows and links by.
    pub fn title(&self) -> String {
        Vault::title(&self.path)
    }
}

/// A link into a note from another one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Backlink {
    pub source: String,
    pub line: usize,
}

/// A task with the note it is in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaultTask {
    pub path: String,
    pub task: ParsedTask,
}

impl VaultTask {
    pub fn id(&self) -> String {
        format!("{}:{}", self.path, self.task.line)
    }
}

/// A folder or a file in the tree, in the order the sidebar shows them:
/// folders first, then files, each by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    /// `Some` for a folder, even an empty one.
    pub children: Option<Vec<TreeNode>>,
    /// For a folder: its folder note `X/X.md`, which opens when the folder is clicked.
    pub folder_note: Option<String>,
    /// For a folder: its `logo.svg` (or `.png`, `.jpg`, `.webp`), drawn as its icon.
    pub logo: Option<String>,
}

impl TreeNode {
    pub fn is_folder(&self) -> bool {
        self.children.is_some()
    }
}

/// Everything known about one vault at one moment. A value: a change to the
/// files makes a new one, so a view never reads half of an update.
#[derive(Debug, Clone, Default)]
pub struct VaultIndex {
    pub notes: HashMap<String, NoteEntry>,
    /// Every file the vault holds, notes and attachments, by vault path.
    pub files: BTreeSet<String>,
    /// Folders with no file in them at any depth, which the tree still
    /// shows: a new folder is empty until something goes into it.
    pub empty_folders: BTreeSet<String>,
    resolver: LinkResolver,
    backlinks: HashMap<String, Vec<Backlink>>,
    tree: Vec<TreeNode>,
    tags: HashMap<String, usize>,
    tag_tree: Vec<TagNode>,
    /// What the resolver and the tree were made from. A saved note changes
    /// neither, and with 20,000 files they took most of a second to make.
    resolved: Option<(BTreeSet<String>, Aliases)>,
    treed: Option<(BTreeSet<String>, BTreeSet<String>)>,
}

/// Each note's aliases, by its path.
type Aliases = HashMap<String, Vec<String>>;

impl VaultIndex {
    pub fn resolver(&self) -> &LinkResolver {
        &self.resolver
    }

    pub fn backlinks(&self, path: &str) -> &[Backlink] {
        self.backlinks.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn tree(&self) -> &[TreeNode] {
        &self.tree
    }

    /// Notes per tag.
    pub fn tags(&self) -> &HashMap<String, usize> {
        &self.tags
    }

    /// The tags as the tag pane shows them, each note counted once per branch.
    pub fn tag_tree(&self) -> &[TagNode] {
        &self.tag_tree
    }

    /// The path [target] names from [source], or `None` when nothing does.
    pub fn resolve(&self, target: &str, source: &str) -> Option<String> {
        self.resolver.resolve(target, source)
    }

    /// Every link target in the note, with where it goes, for the reader.
    pub fn resolved_links(&self, path: &str) -> BTreeMap<String, String> {
        match self.notes.get(path) {
            Some(note) => self.resolved_links_in(&note.parsed, path),
            None => BTreeMap::new(),
        }
    }

    /// The same, for a text that is not in the index yet.
    pub fn resolved_links_in(&self, parsed: &ParsedNote, path: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for link in &parsed.links {
            if out.contains_key(&link.target) {
                continue;
            }
            if let Some(found) = self.resolver.resolve(&link.target, path) {
                out.insert(link.target.clone(), found);
            }
        }
        out
    }

    /// The folder at [path] in the tree; "" is the vault itself.
    pub fn folder(&self, path: &str) -> Option<&[TreeNode]> {
        Vault::folder(path, &self.tree)
    }

    /// Every task in the vault.
    pub fn tasks(&self) -> Vec<VaultTask> {
        self.notes
            .values()
            .flat_map(|note| note.parsed.tasks.iter().map(|task| VaultTask { path: note.path.clone(), task: task.clone() }))
            .collect()
    }

    /// The links, tags, tree and resolver again, after [notes] or [files] changed.
    pub fn derive(&mut self) {
        let aliases: HashMap<String, Vec<String>> = self
            .notes
            .values()
            .filter(|note| !note.parsed.aliases.is_empty())
            .map(|note| (note.path.clone(), note.parsed.aliases.clone()))
            .collect();
        if self.resolved.as_ref().is_none_or(|(files, known)| *files != self.files || *known != aliases) {
            self.resolver = LinkResolver::new(&self.files, &aliases);
            self.resolved = Some((self.files.clone(), aliases));
        }

        // Each note's links are resolved on their own, several notes at a time.
        let resolver = &self.resolver;
        let found: Vec<(String, Backlink)> = self
            .notes
            .par_iter()
            .flat_map_iter(|(_, note)| {
                let mut seen = HashSet::new();
                note.parsed
                    .links
                    .iter()
                    .filter_map(|link| {
                        let target = resolver.resolve(&link.target, &note.path)?;
                        (target != note.path && seen.insert(target.clone()))
                            .then(|| (target, Backlink { source: note.path.clone(), line: link.line }))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut back: HashMap<String, Vec<Backlink>> = HashMap::new();
        for (target, link) in found {
            back.entry(target).or_default().push(link);
        }
        let mut tags: HashMap<String, usize> = HashMap::new();
        for note in self.notes.values() {
            for tag in &note.parsed.tags {
                *tags.entry(tag.clone()).or_default() += 1;
            }
        }
        for list in back.values_mut() {
            list.sort_by(|a, b| a.source.cmp(&b.source));
        }
        self.backlinks = back;
        self.tags = tags;
        self.tag_tree = TagNode::tree_of_notes(self.notes.values().map(|note| note.parsed.tags.as_slice()));
        if self.treed.as_ref().is_none_or(|(files, empty)| *files != self.files || *empty != self.empty_folders) {
            self.tree = Vault::tree(&self.files, &self.empty_folders);
            self.treed = Some((self.files.clone(), self.empty_folders.clone()));
        }
    }
}

/// The files of a vault on disk: which ones count, and how they are read.
pub struct Vault;

/// Folders never read: the app's settings, git, dependencies, the trash.
const SKIPPED: [&str; 5] = [".obsidian", ".git", ".trash", "node_modules", ".github"];

/// The names a folder's logo may have, best first: a drawing before a picture.
pub const LOGO_NAMES: [&str; 5] = ["logo.svg", "logo.png", "logo.webp", "logo.jpg", "logo.jpeg"];

fn last_part(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The vault path of [entry] under [root], with `/` between the parts.
fn relative(root: &Path, entry: &Path) -> Option<String> {
    let parts: Vec<&str> = entry.strip_prefix(root).ok()?.iter().map(|p| p.to_str()).collect::<Option<_>>()?;
    (!parts.is_empty()).then(|| slugs::normalize(&parts.join("/")))
}

/// A walk of [root] that never enters hidden or skipped folders.
fn walk(root: &Path) -> impl Iterator<Item = walkdir::DirEntry> {
    WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !name.starts_with('.') && !SKIPPED.contains(&name.as_ref())
        })
        .filter_map(Result::ok)
}

impl Vault {
    pub fn title(path: &str) -> String {
        let name = last_part(path);
        name.strip_suffix(".md").unwrap_or(name).to_string()
    }

    pub fn is_note(path: &str) -> bool {
        path.to_lowercase().ends_with(".md")
    }

    pub fn is_base(path: &str) -> bool {
        path.to_lowercase().ends_with(".base")
    }

    /// A note, a base or a canvas: a file the app shows itself rather than
    /// handing it to the app the system gives it to.
    pub fn opens_in_app(path: &str) -> bool {
        Self::is_note(path) || Self::is_base(path) || crate::canvas::Canvas::is_canvas(path)
    }

    /// Whether a vault path is one the app reads at all.
    pub fn counts(path: &str) -> bool {
        !path.split('/').any(|part| part.starts_with('.') || SKIPPED.contains(&part))
    }

    /// Every file under [root] that counts, by vault path.
    pub fn scan(root: &Path) -> BTreeSet<String> {
        walk(root).filter(|entry| entry.file_type().is_file()).filter_map(|entry| relative(root, entry.path())).collect()
    }

    /// Reads and parses each note in [paths], several at a time.
    pub fn read_all(paths: &[String], root: &Path) -> Vec<NoteEntry> {
        paths.par_iter().filter_map(|path| Self::read(path, root)).collect()
    }

    pub fn read(path: &str, root: &Path) -> Option<NoteEntry> {
        let file = root.join(path);
        let bytes = fs::read(&file).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let modified = fs::metadata(&file).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        Some(NoteEntry { path: path.to_string(), modified, parsed: NoteParser::parse(&text) })
    }

    /// Builds the whole index of the vault at [root].
    /// The vault's task statuses are read again too, for the rules given only
    /// a symbol: see `TaskStatuses::current`.
    pub fn index(root: &Path, search: Option<&SearchIndex>) -> VaultIndex {
        TaskStatuses::set_current(TaskStatuses::load(root));
        let mut index = VaultIndex { files: Self::scan(root), ..VaultIndex::default() };
        index.empty_folders = Self::empty_folders(root, &index.files);
        let paths: Vec<String> = index.files.iter().filter(|p| Self::is_note(p)).cloned().collect();
        let notes = Self::read_all(&paths, root);
        if let Some(search) = search {
            search.transaction(|search| {
                for note in &notes {
                    search.put(&note.path, &note.title(), &note.parsed.plain);
                }
            });
        }
        index.notes = notes.into_iter().map(|note| (note.path.clone(), note)).collect();
        index.derive();
        index
    }

    /// The index after the files at [changed] changed on disk: each is read
    /// again, or dropped when it is gone.
    pub fn update(index: &VaultIndex, changed: &BTreeSet<String>, root: &Path, search: Option<&SearchIndex>) -> VaultIndex {
        let mut index = index.clone();
        Self::update_in_place(&mut index, changed, root, search);
        index
    }

    /// [update] on [index] itself, with no copy of it: a vault of 20,000
    /// notes takes a sixth of a second to copy, at every change.
    pub fn update_in_place(index: &mut VaultIndex, changed: &BTreeSet<String>, root: &Path, search: Option<&SearchIndex>) {
        if changed.contains(TaskStatuses::PATH) {
            TaskStatuses::set_current(TaskStatuses::load(root));
        }
        let mut reread: Vec<String> = Vec::new();
        // A folder came or went, or its last file did: the empty ones are found again.
        let mut folders = false;
        let forget = |index: &mut VaultIndex, gone: &str| {
            index.files.remove(gone);
            index.notes.remove(gone);
            if let Some(search) = search {
                search.remove(gone);
            }
        };
        for path in changed.iter().filter(|p| Self::counts(p)) {
            let metadata = fs::metadata(root.join(path));
            let base = format!("{}/", path.trim_end_matches('/'));
            match metadata {
                Ok(m) if m.is_dir() => {
                    folders = true;
                    // A folder that came or went: its files are found again.
                    let now: BTreeSet<String> = Self::scan(root).into_iter().filter(|p| p.starts_with(&base)).collect();
                    let before: Vec<String> = index.files.iter().filter(|p| p.starts_with(&base)).cloned().collect();
                    for gone in before.iter().filter(|p| !now.contains(*p)) {
                        forget(index, gone);
                    }
                    for added in now {
                        if Self::is_note(&added) {
                            reread.push(added.clone());
                        }
                        index.files.insert(added);
                    }
                }
                Ok(_) => {
                    index.files.insert(path.clone());
                    if Self::is_note(path) {
                        reread.push(path.clone());
                    }
                }
                Err(_) => {
                    folders = true;
                    let gone: Vec<String> = index.files.iter().filter(|p| *p == path || p.starts_with(&base)).cloned().collect();
                    for gone in gone {
                        forget(index, &gone);
                    }
                }
            }
        }
        for note in Self::read_all(&reread, root) {
            if let Some(search) = search {
                search.put(&note.path, &note.title(), &note.parsed.plain);
            }
            index.notes.insert(note.path.clone(), note);
        }
        if folders {
            index.empty_folders = Self::empty_folders(root, &index.files);
        }
        index.derive();
    }

    /// Every folder under [root] that holds none of [files], however deep.
    pub fn empty_folders(root: &Path, files: &BTreeSet<String>) -> BTreeSet<String> {
        let mut full = HashSet::new();
        for file in files {
            let mut parts: Vec<&str> = file.split('/').collect();
            parts.pop();
            while !parts.is_empty() {
                if !full.insert(parts.join("/")) {
                    break;
                }
                parts.pop();
            }
        }
        walk(root)
            .filter(|entry| entry.file_type().is_dir())
            .filter_map(|entry| relative(root, entry.path()))
            .filter(|path| !full.contains(path))
            .collect()
    }

    /// The logo among a folder's own [files], when it has one.
    fn logo_in(files: &[String]) -> Option<String> {
        LOGO_NAMES.iter().find_map(|name| files.iter().find(|f| last_part(f).to_lowercase() == *name).cloned())
    }

    /// The logo of the folder at [folder], from the vault's files.
    pub fn logo(folder: &str, files: &BTreeSet<String>) -> Option<String> {
        let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
        LOGO_NAMES.iter().find_map(|name| {
            files
                .iter()
                .find(|f| f.len() == prefix.len() + name.len() && f.to_lowercase() == format!("{prefix}{name}").to_lowercase())
                .cloned()
        })
    }

    /// The folder tree. A folder note `X/X.md` is not listed inside its
    /// folder; the folder opens it, as the folder-notes plugin does.
    /// The folder at [path] in [tree]; "" is the vault itself.
    pub fn folder<'a>(path: &str, tree: &'a [TreeNode]) -> Option<&'a [TreeNode]> {
        let mut level = tree;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let next = level.iter().find(|n| n.is_folder() && n.name == part)?;
            level = next.children.as_deref().unwrap_or(&[]);
        }
        Some(level)
    }

    pub fn tree(files: &BTreeSet<String>, empty_folders: &BTreeSet<String>) -> Vec<TreeNode> {
        #[derive(Default)]
        struct Folder {
            folders: BTreeMap<String, Folder>,
            files: Vec<String>,
        }
        let mut root = Folder::default();
        for path in empty_folders {
            let mut folder = &mut root;
            for part in path.split('/') {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
        }
        for path in files {
            let mut folder = &mut root;
            let parts: Vec<&str> = path.split('/').collect();
            for part in &parts[..parts.len() - 1] {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
            folder.files.push(path.clone());
        }
        fn build(folder: &Folder, prefix: &str) -> Vec<TreeNode> {
            let mut names: Vec<&String> = folder.folders.keys().collect();
            names.sort_by(|a, b| standard_compare(a, b));
            let mut nodes: Vec<TreeNode> = names
                .into_iter()
                .map(|name| {
                    let path = format!("{prefix}{name}");
                    let child = &folder.folders[name];
                    let note = format!("{path}/{name}.md");
                    let has_note = child.files.contains(&note);
                    let logo = Vault::logo_in(&child.files);
                    let mut children = build(child, &format!("{path}/"));
                    // The folder note and the logo are the folder, not files in it.
                    children.retain(|c| c.path != note && (c.is_folder() || !LOGO_NAMES.contains(&c.name.to_lowercase().as_str())));
                    TreeNode { path, name: name.clone(), children: Some(children), folder_note: has_note.then_some(note), logo }
                })
                .collect();
            let mut leaves = folder.files.clone();
            leaves.sort_by(|a, b| standard_compare(&Vault::title(a), &Vault::title(b)));
            nodes.extend(leaves.into_iter().map(|path| TreeNode {
                name: if Vault::is_note(&path) { Vault::title(&path) } else { last_part(&path).to_string() },
                path,
                children: None,
                folder_note: None,
                logo: None,
            }));
            nodes
        }
        build(&root, "")
    }
}

/// The names `files.hidden` has when the settings file says none: the
/// files for AI agents.
pub const HIDDEN_NAMES: [&str; 4] = ["CLAUDE.md", "AGENTS.md", "GEMINI.md", "CLAUDE.local.md"];

/// Obsidian's "excluded files" (`userIgnoreFilters` in `.obsidian/app.json`)
/// and the files for AI agents and other tools (`files.hidden` in the
/// config, by name in any folder): kept out of the tree, search, the
/// switcher and the graph. Links to them still work, and they still open
/// by their path. A filter is a path prefix (`uploads/` hides the folder and
/// all in it), or a regular expression between slashes.
#[derive(Debug, Clone, Default)]
pub struct ExcludedFiles {
    pub prefixes: Vec<String>,
    pub patterns: Vec<Regex>,
    /// Whole file names, folded: `CLAUDE.md` hides `Work/CLAUDE.md`, not `MyCLAUDE.md`.
    pub names: HashSet<String>,
}

impl ExcludedFiles {
    /// [filters] as Obsidian has them, and the file [names] hidden in any folder.
    pub fn with_names<S: AsRef<str>, N: AsRef<str>>(filters: &[S], names: &[N]) -> ExcludedFiles {
        let mut excluded = ExcludedFiles::new(filters);
        excluded.names = names.iter().map(|n| slugs::fold(n.as_ref().trim())).filter(|n| !n.is_empty()).collect();
        excluded
    }

    pub fn new<S: AsRef<str>>(filters: &[S]) -> ExcludedFiles {
        let mut excluded = ExcludedFiles::default();
        for filter in filters {
            let trimmed = trim(filter.as_ref());
            if trimmed.len() > 2 && trimmed.starts_with('/') && trimmed.ends_with('/') {
                if let Ok(regex) = Regex::new(&trimmed[1..trimmed.len() - 1]) {
                    excluded.patterns.push(regex);
                }
            } else if !trimmed.is_empty() {
                excluded.prefixes.push(slugs::fold(trimmed));
            }
        }
        excluded
    }

    /// Obsidian's filters for the vault at [root], and the file [names].
    pub fn load_with_names<N: AsRef<str>>(root: &Path, names: &[N]) -> ExcludedFiles {
        let mut excluded = ExcludedFiles::load(root);
        excluded.names = ExcludedFiles::with_names::<&str, N>(&[], names).names;
        excluded
    }

    pub fn is_empty(&self) -> bool {
        self.prefixes.is_empty() && self.patterns.is_empty() && self.names.is_empty()
    }

    /// The folder at [path] is excluded itself, as `uploads/` excludes `uploads`.
    pub fn contains_folder(&self, path: &str) -> bool {
        let folded = slugs::fold(path) + "/";
        let folder = format!("{path}/");
        self.prefixes.iter().any(|p| folded.starts_with(p.as_str())) || self.patterns.iter().any(|r| r.is_match(&folder))
    }

    /// The tree without the excluded files and folders. A folder that holds
    /// only hidden files (a `CLAUDE.md`) stays, as an empty folder.
    pub fn tree(&self, files: &BTreeSet<String>, empty_folders: &BTreeSet<String>) -> Vec<TreeNode> {
        if self.is_empty() {
            return Vault::tree(files, empty_folders);
        }
        let mut shown = BTreeSet::new();
        let mut empty = BTreeSet::new();
        for path in files {
            if !self.contains(path) {
                shown.insert(path.clone());
                continue;
            }
            // The nearest folder above a hidden file that is not hidden itself.
            let mut folder = path.as_str();
            while let Some(slash) = folder.rfind('/') {
                folder = &folder[..slash];
                if !self.contains_folder(folder) {
                    empty.insert(folder.to_string());
                    break;
                }
            }
        }
        empty.extend(empty_folders.iter().filter(|f| !self.contains_folder(f)).cloned());
        Vault::tree(&shown, &empty)
    }

    pub fn load(root: &Path) -> ExcludedFiles {
        let filters = fs::read(root.join(".obsidian/app.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|json| {
                json.get("userIgnoreFilters")?
                    .as_array()
                    .map(|list| list.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        ExcludedFiles::new(&filters)
    }

    pub fn contains(&self, path: &str) -> bool {
        let folded = slugs::fold(path);
        self.prefixes.iter().any(|p| folded.starts_with(p.as_str()))
            || (!self.names.is_empty() && self.names.contains(last_part(&folded)))
            || self.patterns.iter().any(|r| r.is_match(path))
    }

    /// What the lists leave out in the vault at [root], as the settings file
    /// says: Obsidian's filters and `files.hidden` (the files for AI agents
    /// when it has no such key); nothing when `files.show_hidden` is on, or
    /// when [show] asks for every file, as `--hidden` on the command line does.
    pub fn from_config(root: &Path, config: &crate::config::ConfigFile, show: bool) -> ExcludedFiles {
        if show || config.bool("files.show_hidden").unwrap_or(false) {
            return ExcludedFiles::default();
        }
        ExcludedFiles::load_with_names(root, &Self::hidden_names(config))
    }

    /// `files.hidden` in [config], or [HIDDEN_NAMES] when it has no such key.
    pub fn hidden_names(config: &crate::config::ConfigFile) -> Vec<String> {
        config.strings("files.hidden").unwrap_or_else(|| HIDDEN_NAMES.map(String::from).to_vec())
    }

    /// The tasks of [index] in the notes that are not left out.
    pub fn tasks(&self, index: &VaultIndex) -> Vec<VaultTask> {
        index
            .notes
            .values()
            .filter(|note| !self.contains(&note.path))
            .flat_map(|note| note.parsed.tasks.iter().map(|task| VaultTask { path: note.path.clone(), task: task.clone() }))
            .collect()
    }

    /// Notes per tag, counting only the notes that are not left out.
    pub fn tags(&self, index: &VaultIndex) -> HashMap<String, usize> {
        if self.is_empty() {
            return index.tags().clone();
        }
        let mut tags: HashMap<String, usize> = HashMap::new();
        for note in index.notes.values().filter(|note| !self.contains(&note.path)) {
            for tag in &note.parsed.tags {
                *tags.entry(tag.clone()).or_default() += 1;
            }
        }
        tags
    }

    /// The tag pane's tree, from the notes that are not left out.
    pub fn tag_tree(&self, index: &VaultIndex) -> Vec<TagNode> {
        if self.is_empty() {
            return index.tag_tree().to_vec();
        }
        TagNode::tree_of_notes(index.notes.values().filter(|note| !self.contains(&note.path)).map(|note| note.parsed.tags.as_slice()))
    }
}

impl Vault {
    /// Where a new picture goes: Obsidian's attachment folder for the vault,
    /// `attachmentFolderPath` in `.obsidian/app.json`; the vault's root
    /// without one, or with one beside the note (`./`), as the app pastes.
    pub fn attachment_folder(root: &Path) -> String {
        let Ok(text) = fs::read_to_string(root.join(".obsidian/app.json")) else { return String::new() };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return String::new() };
        match json.get("attachmentFolderPath").and_then(|v| v.as_str()) {
            Some(folder) if !folder.is_empty() && folder != "/" && !folder.starts_with("./") => folder.trim_matches('/').to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn make_vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let files = [
            ("Notes/Inbox.md", "# Inbox\n- [ ] Call [[Alpha]] ⏳ 2026-10-04\n"),
            ("Projects/Alpha/Alpha.md", "Alpha project, see [[Inbox]]."),
            ("Projects/Alpha/Plan.md", "Plan for [[Projects/Alpha]]."),
            ("uploads/logo.png", "png"),
            (".obsidian/app.json", "{}"),
            ("node_modules/x/README.md", "skip"),
        ];
        for (path, text) in files {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn names(nodes: &[TreeNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn indexes_the_vault() {
        let root = make_vault();
        let index = Vault::index(root.path(), None);
        assert_eq!(index.files, set(&["Notes/Inbox.md", "Projects/Alpha/Alpha.md", "Projects/Alpha/Plan.md", "uploads/logo.png"]));
        let sources: Vec<&str> = index.backlinks("Projects/Alpha/Alpha.md").iter().map(|b| b.source.as_str()).collect();
        assert_eq!(sources, ["Notes/Inbox.md", "Projects/Alpha/Plan.md"]);
        assert_eq!(index.tasks().len(), 1);
        let projects = index.tree().iter().find(|n| n.name == "Projects").unwrap();
        let alpha = &projects.children.as_ref().unwrap()[0];
        assert_eq!(alpha.folder_note.as_deref(), Some("Projects/Alpha/Alpha.md"));
        assert_eq!(names(alpha.children.as_ref().unwrap()), ["Plan"]);
    }

    fn hidden() -> ExcludedFiles {
        ExcludedFiles::with_names(&["uploads/", "node_modules/"], &HIDDEN_NAMES)
    }

    #[test]
    fn a_folder_filter_hides_the_folder_and_all_in_it() {
        let hidden = hidden();
        assert!(hidden.contains("uploads/a.png") && hidden.contains("uploads/2026/b.pdf"));
        assert!(hidden.contains_folder("uploads") && hidden.contains_folder("uploads/2026") && hidden.contains_folder("node_modules"));
        assert!(!hidden.contains_folder("Notes") && !hidden.contains_folder("Notes/uploads"));
        assert!(!hidden.contains("Notes/uploads/a.png") && !hidden.contains("uploads.md"));
    }

    #[test]
    fn a_name_hides_the_file_in_any_folder() {
        let hidden = hidden();
        for path in [
            "CLAUDE.md",
            "Work/CLAUDE.md",
            "Work/Projects/Alpha/AGENTS.md",
            "Calendar/GEMINI.md",
            "Notes/CLAUDE.local.md",
            "Notes/claude.md",
        ] {
            assert!(hidden.contains(path), "{path}");
        }
        // Only the whole name: a note that has it in its name is a note.
        for path in ["MyCLAUDE.md", "Work/CLAUDE.md.bak", "Work/CLAUDE", "CLAUDE.md/Plan.md"] {
            assert!(!hidden.contains(path), "{path}");
        }
        assert!(!hidden.contains_folder("Work"));
        let none = ExcludedFiles::with_names::<&str, &str>(&[], &[" ", ""]);
        assert!(none.is_empty() && !none.contains("CLAUDE.md"));
        assert!(!ExcludedFiles::new(&["uploads/"]).contains("CLAUDE.md"));
    }

    #[test]
    fn the_tree_leaves_them_out() {
        let files = set(&[
            "Home.md",
            "CLAUDE.md",
            "AGENTS.md",
            "MyCLAUDE.md",
            "Work/Plan.md",
            "Work/CLAUDE.md",
            "Work/CLAUDE.md.bak",
            "Agents/CLAUDE.md",
            "uploads/a.png",
            "uploads/2026/b.pdf",
        ]);
        let tree = hidden().tree(&files, &set(&["uploads/empty", "Empty"]));
        assert_eq!(tree.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Agents", "Empty", "Work", "Home.md", "MyCLAUDE.md"]);
        let work = Vault::folder("Work", &tree).unwrap();
        assert_eq!(work.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Work/CLAUDE.md.bak", "Work/Plan.md"]);
        // A folder with only a hidden file in it is still there, empty.
        assert!(Vault::folder("Agents", &tree).unwrap().is_empty());
        assert!(Vault::folder("uploads", &tree).is_none());
        assert_eq!(ExcludedFiles::default().tree(&files, &BTreeSet::new()), Vault::tree(&files, &BTreeSet::new()));
    }

    #[test]
    fn links_to_hidden_files_still_resolve() {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("Home.md", "![[diagram.png]] and [[CLAUDE]] and [[Work/CLAUDE.md|rules]]"),
            ("uploads/diagram.png", "png"),
            ("Work/CLAUDE.md", "Rules for agents. See [[Home]]."),
            (".obsidian/app.json", r#"{"userIgnoreFilters":["uploads/","node_modules/"]}"#),
        ] {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        let excluded = ExcludedFiles::load_with_names(root.path(), &HIDDEN_NAMES);
        assert!(excluded.contains("uploads/diagram.png") && excluded.contains("Work/CLAUDE.md"));
        let index = Vault::index(root.path(), None);
        assert_eq!(index.resolve("diagram.png", "Home.md").as_deref(), Some("uploads/diagram.png"));
        assert_eq!(index.resolve("CLAUDE", "Home.md").as_deref(), Some("Work/CLAUDE.md"));
        let paths = |graph: crate::NoteGraph| graph.nodes.into_iter().map(|n| n.path).collect::<Vec<_>>();
        assert_eq!(paths(crate::NoteGraph::vault(&index, Some(&excluded))), ["Home.md"]);
        assert_eq!(paths(crate::NoteGraph::local(&index, "Home.md", 2, Some(&excluded))), ["Home.md"]);
        assert_eq!(paths(crate::NoteGraph::local(&index, "Home.md", 2, None)), ["Home.md", "Work/CLAUDE.md"]);
        // Opened by its path, a hidden note is the centre of its own graph.
        assert_eq!(paths(crate::NoteGraph::local(&index, "Work/CLAUDE.md", 2, Some(&excluded))), ["Work/CLAUDE.md", "Home.md"]);
    }

    #[test]
    fn a_folders_logo_is_its_icon() {
        let files = set(&[
            "Companies/Acme/Acme.md",
            "Companies/Acme/logo.png",
            "Companies/Acme/Logo.SVG",
            "Companies/Acme/Notes.md",
            "Companies/Beta/b.md",
            "logo.svg",
        ]);
        let tree = Vault::tree(&files, &BTreeSet::new());
        let companies = tree.iter().find(|n| n.name == "Companies").unwrap().children.as_ref().unwrap();
        let acme = companies.iter().find(|n| n.name == "Acme").unwrap();
        // A drawing wins over a picture, whatever the case of its name.
        assert_eq!(acme.logo.as_deref(), Some("Companies/Acme/Logo.SVG"));
        assert_eq!(names(acme.children.as_ref().unwrap()), ["Notes"]);
        assert_eq!(companies.iter().find(|n| n.name == "Beta").unwrap().logo, None);
        assert_eq!(Vault::logo("Companies/Acme", &files).as_deref(), Some("Companies/Acme/Logo.SVG"));
        assert_eq!(Vault::logo("Companies/Beta", &files), None);
        // The vault's own logo.svg at the top is an ordinary file in the tree.
        assert!(tree.iter().any(|n| n.path == "logo.svg"));
    }

    #[test]
    fn finds_a_folder_in_the_tree() {
        let root = make_vault();
        let index = Vault::index(root.path(), None);
        assert_eq!(names(index.folder("").unwrap()), ["Notes", "Projects", "uploads"]);
        assert_eq!(names(index.folder("Projects/Alpha").unwrap()), ["Plan"]);
        assert_eq!(index.folder("Projects").unwrap()[0].folder_note.as_deref(), Some("Projects/Alpha/Alpha.md"));
        assert!(index.folder("Nowhere").is_none());
    }

    #[test]
    fn an_empty_folder_is_in_the_tree() {
        let root = make_vault();
        fs::create_dir_all(root.path().join("Drafts/New")).unwrap();
        let index = Vault::index(root.path(), None);
        assert_eq!(index.empty_folders, set(&["Drafts", "Drafts/New"]));
        assert_eq!(names(index.folder("Drafts").unwrap()), ["New"]);
    }

    #[test]
    fn update_reads_changes_and_drops_deletions() {
        let root = make_vault();
        let search = SearchIndex::new();
        let mut index = Vault::index(root.path(), Some(&search));
        fs::write(root.path().join("New.md"), "New note about [[Inbox]] and zebras").unwrap();
        fs::remove_file(root.path().join("Projects/Alpha/Plan.md")).unwrap();
        index = Vault::update(&index, &set(&["New.md", "Projects/Alpha/Plan.md"]), root.path(), Some(&search));
        assert!(index.notes.contains_key("New.md"));
        assert!(!index.notes.contains_key("Projects/Alpha/Plan.md"));
        let sources: Vec<&str> = index.backlinks("Notes/Inbox.md").iter().map(|b| b.source.as_str()).collect();
        assert_eq!(sources, ["New.md", "Projects/Alpha/Alpha.md"]);
        assert_eq!(search.search("zebras", 100).into_iter().map(|h| h.path).collect::<Vec<_>>(), ["New.md"]);
        assert!(search.search("Plan", 100).is_empty());
    }

    #[test]
    fn a_note_counts_once_under_a_parent_tag() {
        let mut index = VaultIndex::default();
        for (path, text) in [("One.md", "#a and #a/b"), ("Two.md", "#a/b and #a/c"), ("Three.md", "#A")] {
            index
                .notes
                .insert(path.into(), NoteEntry { path: path.into(), modified: SystemTime::UNIX_EPOCH, parsed: NoteParser::parse(text) });
        }
        index.files = index.notes.keys().cloned().collect();
        index.derive();
        let a = &index.tag_tree()[0];
        assert!(a.tag == "a" && a.count == 3);
        assert_eq!(a.children.as_ref().unwrap().iter().map(|t| t.count).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn the_settings_file_says_what_is_hidden() {
        let root = make_vault();
        fs::write(root.path().join(".obsidian/app.json"), r#"{"userIgnoreFilters":["uploads/"],"attachmentFolderPath":"uploads/"}"#)
            .unwrap();
        let config = crate::ConfigFile::new("[files]\nhidden = [\"Plan.md\"]\n");
        let hidden = ExcludedFiles::from_config(root.path(), &config, false);
        assert!(hidden.contains("Projects/Alpha/Plan.md") && hidden.contains("uploads/logo.png"));
        assert!(!hidden.contains("CLAUDE.md"));
        // With no key, the files for AI agents.
        assert!(ExcludedFiles::from_config(root.path(), &crate::ConfigFile::new(""), false).contains("Work/AGENTS.md"));
        assert!(ExcludedFiles::from_config(root.path(), &config, true).is_empty());
        let shown = crate::ConfigFile::new("[files]\nshow_hidden = true\n");
        assert!(ExcludedFiles::from_config(root.path(), &shown, false).is_empty());
        let index = Vault::index(root.path(), None);
        assert!(hidden.tasks(&index).iter().all(|t| t.path != "Projects/Alpha/Plan.md"));
        assert_eq!(Vault::attachment_folder(root.path()), "uploads");
    }

    #[test]
    fn hidden_notes_tags_and_tasks_are_left_out() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("A.md"), "#shared #mine\n- [ ] One").unwrap();
        fs::write(root.path().join("CLAUDE.md"), "#shared #agents\n- [ ] Two").unwrap();
        let index = Vault::index(root.path(), None);
        let hidden = ExcludedFiles::with_names::<&str, &str>(&[], &HIDDEN_NAMES);
        let tags = hidden.tags(&index);
        assert_eq!((tags.get("shared"), tags.get("agents")), (Some(&1), None));
        let mut tree: Vec<String> = hidden.tag_tree(&index).into_iter().map(|t| t.tag).collect();
        tree.sort();
        assert_eq!(tree, ["mine", "shared"]);
        assert_eq!(hidden.tasks(&index).len(), 1);
        assert_eq!(ExcludedFiles::default().tasks(&index).len(), 2);
    }

    #[test]
    fn excluded_files() {
        let excluded = ExcludedFiles::new(&["Archive/", "/\\.excalidraw\\.md$/"]);
        assert!(excluded.contains("archive/old.md"));
        assert!(excluded.contains("Drawings/a.excalidraw.md"));
        assert!(!excluded.contains("Notes/Inbox.md"));
    }
}
