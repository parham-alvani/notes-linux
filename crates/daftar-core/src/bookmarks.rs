//! Obsidian's bookmarks, `.obsidian/bookmarks.json`: files, folders,
//! searches, headings and groups of them. Daftar shows the files and
//! folders and adds or removes files; every other item is written back as
//! it was read, keys in their order, so the file stays Obsidian's.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkEntry {
    pub path: String,
    pub title: Option<String>,
    /// The group it is in, when it is in one.
    pub group: Option<String>,
    pub folder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookmarksFailure {
    /// The file is there but could not be read, so it is not written over.
    Unreadable,
    Unwritable(String),
}

impl fmt::Display for BookmarksFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookmarksFailure::Unreadable => write!(f, "bookmarks.json could not be read, so it is left as it is"),
            BookmarksFailure::Unwritable(why) => write!(f, "bookmarks.json could not be written: {why}"),
        }
    }
}

impl std::error::Error for BookmarksFailure {}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bookmarks {
    /// The items as JSON objects, in the file's order.
    pub items: Vec<Value>,
    /// True when the file was there but could not be read: the bookmarks
    /// are empty, and `save` refuses rather than lose the file.
    pub unreadable: bool,
}

impl Bookmarks {
    pub fn new(items: Vec<Value>) -> Bookmarks {
        Bookmarks { items, unreadable: false }
    }

    fn file(root: &Path) -> PathBuf {
        root.join(".obsidian/bookmarks.json")
    }

    /// No file is no bookmarks yet; a file that does not read is `unreadable`.
    pub fn load(root: &Path) -> Bookmarks {
        let file = Self::file(root);
        if !file.exists() {
            return Bookmarks::default();
        }
        let items = fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|object| object.get("items").and_then(Value::as_array).cloned())
            .filter(|items| items.iter().all(Value::is_object));
        match items {
            Some(items) => Bookmarks::new(items),
            None => Bookmarks { items: vec![], unreadable: true },
        }
    }

    pub fn save(&self, root: &Path) -> Result<(), BookmarksFailure> {
        if self.unreadable {
            return Err(BookmarksFailure::Unreadable);
        }
        let failed = |e: std::io::Error| BookmarksFailure::Unwritable(e.to_string());
        let text =
            serde_json::to_string_pretty(&json!({ "items": self.items })).map_err(|e| BookmarksFailure::Unwritable(e.to_string()))?;
        let file = Self::file(root);
        fs::create_dir_all(file.parent().unwrap()).map_err(failed)?;
        let temporary = file.with_file_name(format!(".bookmarks.json.daftar-{}", std::process::id()));
        fs::write(&temporary, text).map_err(failed)?;
        fs::rename(&temporary, &file).map_err(failed)
    }

    /// The files and folders, groups opened, in order.
    pub fn entries(&self) -> Vec<BookmarkEntry> {
        fn walk(items: &[Value], group: Option<&str>, out: &mut Vec<BookmarkEntry>) {
            for item in items {
                let text = |key: &str| item.get(key).and_then(Value::as_str);
                match text("type") {
                    Some(kind @ ("file" | "folder")) => {
                        if let Some(path) = text("path") {
                            out.push(BookmarkEntry {
                                path: path.into(),
                                title: text("title").map(String::from),
                                group: group.map(String::from),
                                folder: kind == "folder",
                            });
                        }
                    }
                    Some("group") => {
                        let inner = item.get("items").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
                        walk(inner, text("title").or(group), out);
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.items, None, &mut out);
        out
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries().iter().any(|e| !e.folder && e.path == path)
    }

    /// Adds a file at the end, or removes it from wherever it is.
    pub fn toggle(&mut self, path: &str, now: SystemTime) {
        if self.contains(path) {
            self.remove(path);
        } else {
            let ctime = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            self.items.push(json!({ "type": "file", "ctime": ctime, "path": path }));
        }
    }

    /// Takes away every bookmark of [path], or of anything in it when it is a folder.
    pub fn remove(&mut self, path: &str) {
        fn keep(items: &[Value], path: &str) -> Vec<Value> {
            items
                .iter()
                .filter(|item| {
                    let kind = item.get("type").and_then(Value::as_str);
                    let bookmarked = item.get("path").and_then(Value::as_str);
                    !(matches!(kind, Some("file" | "folder"))
                        && bookmarked.is_some_and(|b| b == path || b.starts_with(&format!("{path}/"))))
                })
                .map(|item| {
                    let mut item = item.clone();
                    if item.get("type").and_then(Value::as_str) == Some("group")
                        && let Some(inner) = item.get("items").and_then(Value::as_array).cloned()
                    {
                        item["items"] = Value::Array(keep(&inner, path));
                    }
                    item
                })
                .collect()
        }
        self.items = keep(&self.items, path);
    }

    /// After files moved, each bookmark goes where its file went. A moved
    /// folder is [folder], from and to. Returns whether anything changed.
    pub fn follow(&mut self, moves: &std::collections::BTreeMap<String, String>, folder: Option<(&str, &str)>) -> bool {
        fn walk(items: &mut [Value], moves: &std::collections::BTreeMap<String, String>, folder: Option<(&str, &str)>) -> bool {
            let mut changed = false;
            for item in items {
                let Some(object) = item.as_object_mut() else { continue };
                if let Some(path) = object.get("path").and_then(Value::as_str).map(String::from) {
                    if let Some(new) = moves.get(&path) {
                        object.insert("path".into(), Value::String(new.clone()));
                        changed = true;
                    } else if let Some((from, to)) = folder
                        && (path == from || path.starts_with(&format!("{from}/")))
                    {
                        object.insert("path".into(), Value::String(format!("{to}{}", &path[from.len()..])));
                        changed = true;
                    }
                }
                if let Some(Value::Array(inner)) = object.get_mut("items") {
                    changed |= walk(inner, moves, folder);
                }
            }
            changed
        }
        walk(&mut self.items, moves, folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::Duration;

    // Every fixture here is made up. The repository is public; the vault is not.

    const JSON: &str = r#"{
  "items": [
    {"type": "file", "ctime": 1, "path": "Home.md"},
    {"type": "group", "ctime": 2, "title": "Work", "items": [
      {"type": "file", "ctime": 3, "path": "Projects/Alpha/Plan.md", "title": "The plan"},
      {"type": "folder", "ctime": 4, "path": "Projects/Alpha"}
    ]},
    {"type": "search", "ctime": 5, "query": "tag:#work"}
  ]
}"#;

    fn make_vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::write(root.path().join(".obsidian/bookmarks.json"), JSON).unwrap();
        root
    }

    fn entry(path: &str, title: Option<&str>, group: Option<&str>, folder: bool) -> BookmarkEntry {
        BookmarkEntry { path: path.into(), title: title.map(String::from), group: group.map(String::from), folder }
    }

    fn paths(bookmarks: &Bookmarks) -> Vec<String> {
        bookmarks.entries().into_iter().map(|e| e.path).collect()
    }

    #[test]
    fn reads_files_folders_and_groups() {
        let root = make_vault();
        let bookmarks = Bookmarks::load(root.path());
        assert_eq!(
            bookmarks.entries(),
            [
                entry("Home.md", None, None, false),
                entry("Projects/Alpha/Plan.md", Some("The plan"), Some("Work"), false),
                entry("Projects/Alpha", None, Some("Work"), true),
            ]
        );
        assert!(bookmarks.contains("Projects/Alpha/Plan.md"));
        assert!(!bookmarks.contains("Projects/Alpha"));
    }

    #[test]
    fn toggles_and_keeps_what_it_does_not_show() {
        let root = make_vault();
        let mut bookmarks = Bookmarks::load(root.path());
        bookmarks.toggle("Other.md", SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        bookmarks.toggle("Projects/Alpha/Plan.md", SystemTime::now());
        bookmarks.save(root.path()).unwrap();
        let again = Bookmarks::load(root.path());
        assert_eq!(paths(&again), ["Home.md", "Projects/Alpha", "Other.md"]);
        assert!(again.items.iter().any(|i| i["type"] == "search" && i["query"] == "tag:#work"));
        assert_eq!(again.items.last().unwrap()["ctime"], 10_000);
        // Obsidian's key order is kept.
        let text = fs::read_to_string(root.path().join(".obsidian/bookmarks.json")).unwrap();
        assert!(text.find("\"type\"").unwrap() < text.find("\"ctime\"").unwrap());
    }

    #[test]
    fn follows_moves_and_deletes() {
        let root = make_vault();
        let mut bookmarks = Bookmarks::load(root.path());
        let moves = BTreeMap::from([("Projects/Alpha/Plan.md".to_string(), "Projects/Gamma/Plan.md".to_string())]);
        assert!(bookmarks.follow(&moves, Some(("Projects/Alpha", "Projects/Gamma"))));
        assert_eq!(paths(&bookmarks), ["Home.md", "Projects/Gamma/Plan.md", "Projects/Gamma"]);
        bookmarks.remove("Projects/Gamma");
        assert_eq!(paths(&bookmarks), ["Home.md"]);
        // The group stays, empty, as Obsidian keeps it.
        assert!(bookmarks.items.iter().any(|i| i["type"] == "group"));
    }

    #[test]
    fn a_file_it_cannot_read_is_never_saved_over() {
        let root = make_vault();
        let broken = "{\"items\": [ half written";
        fs::write(root.path().join(".obsidian/bookmarks.json"), broken).unwrap();
        let mut bookmarks = Bookmarks::load(root.path());
        assert!(bookmarks.entries().is_empty());
        bookmarks.toggle("Other.md", SystemTime::now());
        assert_eq!(bookmarks.save(root.path()), Err(BookmarksFailure::Unreadable));
        assert_eq!(fs::read_to_string(root.path().join(".obsidian/bookmarks.json")).unwrap(), broken);
        // No file at all is no bookmarks yet, and saving makes one.
        let empty = tempfile::tempdir().unwrap();
        let mut fresh = Bookmarks::load(empty.path());
        fresh.toggle("Home.md", SystemTime::now());
        fresh.save(empty.path()).unwrap();
        assert_eq!(paths(&Bookmarks::load(empty.path())), ["Home.md"]);
    }
}
