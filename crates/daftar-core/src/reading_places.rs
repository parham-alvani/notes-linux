//! What the app remembers of each note it showed, kept across launches: the
//! source line of the reading cursor, so a note opens where it was left; the
//! sections folded shut; and when the note was last read, so a note changed
//! by a pull since then can say so. The notes read last are kept, up to
//! `LIMIT`; a move or a rename takes its entry along.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub line: Option<usize>,
    /// Each folded heading as `level:text`, which an edit above it does not move.
    pub folds: Option<Vec<String>>,
    /// When the note was last on screen, in whole seconds since 1970.
    pub read: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadingPlaces {
    entries: HashMap<String, Entry>,
    /// Read first to read last.
    order: Vec<String>,
}

impl ReadingPlaces {
    pub const LIMIT: usize = 1000;

    pub fn new() -> ReadingPlaces {
        ReadingPlaces::default()
    }

    pub fn get(&self, path: &str) -> Option<usize> {
        self.entries.get(path).and_then(|e| e.line)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn folds(&self, path: &str) -> Vec<String> {
        self.entries.get(path).and_then(|e| e.folds.clone()).unwrap_or_default()
    }

    /// When [path] was last read, in seconds since 1970.
    pub fn last_read(&self, path: &str) -> Option<i64> {
        self.entries.get(path).and_then(|e| e.read)
    }

    /// The notes read last, newest first, up to [limit]. A note with no read
    /// time, kept by a version that did not record one, comes after those
    /// with one, by when its place was set.
    pub fn recently_read(&self, limit: usize) -> Vec<String> {
        let mut read: Vec<(&String, Option<i64>, usize)> =
            self.order.iter().enumerate().filter_map(|(offset, path)| self.entries.get(path).map(|e| (path, e.read, offset))).collect();
        // The order breaks a tie: one second can hold many reads.
        read.sort_by_key(|r| std::cmp::Reverse((r.1, r.2)));
        read.into_iter().take(limit).map(|(path, _, _)| path.clone()).collect()
    }

    pub fn set(&mut self, path: &str, line: usize) {
        self.update(path, |e| e.line = Some(line));
    }

    pub fn set_folds(&mut self, path: &str, folds: Vec<String>) {
        self.update(path, |e| e.folds = if folds.is_empty() { None } else { Some(folds) });
    }

    /// [at] in seconds since 1970; whole seconds, as the file keeps them.
    pub fn mark_read(&mut self, path: &str, at: i64) {
        self.update(path, |e| e.read = Some(at));
    }

    fn update(&mut self, path: &str, change: impl FnOnce(&mut Entry)) {
        let before = self.entries.get(path).cloned();
        let mut entry = before.clone().unwrap_or_default();
        change(&mut entry);
        if before.as_ref() == Some(&entry) {
            return;
        }
        self.entries.insert(path.to_string(), entry);
        if before.is_some() {
            if self.order.last().map(String::as_str) == Some(path) {
                return;
            }
            self.order.retain(|p| p != path);
        }
        self.order.push(path.to_string());
        while self.order.len() > Self::LIMIT {
            let oldest = self.order.remove(0);
            self.entries.remove(&oldest);
        }
    }

    /// After a move or a rename: [moves] maps each file's old path to its new one.
    pub fn moved(&mut self, moves: &BTreeMap<String, String>) {
        if !moves.keys().any(|k| self.entries.contains_key(k)) {
            return;
        }
        let rename = |p: &String| moves.get(p).unwrap_or(p).clone();
        self.entries = self.entries.drain().map(|(path, entry)| (rename(&path), entry)).collect();
        self.order = self.order.iter().map(rename).collect();
    }

    /// After a delete: the file, or every file in the folder.
    pub fn forget(&mut self, path: &str) {
        let gone = |other: &str| other == path || other.starts_with(&format!("{path}/"));
        self.order.retain(|p| !gone(p));
        self.entries.retain(|p, _| !gone(p));
    }

    /// `{"notes": [{"path": "Folder/Note.md", "line": 12, "folds": ["2:Plan"], "read": "2027-01-15T08:00:00Z"}, …]}`,
    /// read first to read last.
    pub fn data(&self) -> String {
        let notes: Vec<Value> = self
            .order
            .iter()
            .filter_map(|path| {
                let entry = self.entries.get(path)?;
                let mut note = Map::new();
                note.insert("path".into(), json!(path));
                if let Some(line) = entry.line {
                    note.insert("line".into(), json!(line));
                }
                if let Some(folds) = &entry.folds {
                    note.insert("folds".into(), json!(folds));
                }
                if let Some(read) = entry.read.and_then(|s| DateTime::<Utc>::from_timestamp(s, 0)) {
                    note.insert("read".into(), json!(read.format("%Y-%m-%dT%H:%M:%SZ").to_string()));
                }
                Some(Value::Object(note))
            })
            .collect();
        json!({ "notes": notes }).to_string()
    }

    /// Entries from [data]; none from a file that is missing or broken. The
    /// first format, lines only (`{"places": [{"Folder/Note.md": 12}]}`), is read too.
    pub fn from_data(data: &str) -> ReadingPlaces {
        let mut places = ReadingPlaces::new();
        let Ok(file) = serde_json::from_str::<Value>(data) else { return places };
        for entry in file.get("places").and_then(Value::as_array).into_iter().flatten() {
            for (path, line) in entry.as_object().into_iter().flatten() {
                if let Some(line) = line.as_u64() {
                    places.set(path, line as usize);
                }
            }
        }
        for note in file.get("notes").and_then(Value::as_array).into_iter().flatten() {
            let Some(path) = note.get("path").and_then(Value::as_str) else { continue };
            let entry = Entry {
                line: note.get("line").and_then(Value::as_u64).map(|l| l as usize),
                folds: note
                    .get("folds")
                    .and_then(Value::as_array)
                    .map(|folds| folds.iter().filter_map(|f| f.as_str().map(String::from)).collect()),
                read: note.get("read").and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp()),
            };
            places.update(path, |e| *e = entry);
        }
        places
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_come_back_from_their_file() {
        let mut places = ReadingPlaces::new();
        places.set("Notes/Plan.md", 12);
        places.set("Notes/Inbox.md", 3);
        places.set("Notes/Plan.md", 40);
        let back = ReadingPlaces::from_data(&places.data());
        assert_eq!(back, places);
        assert_eq!(back.get("Notes/Plan.md"), Some(40));
        assert_eq!(back.get("Notes/Inbox.md"), Some(3));
        assert_eq!(ReadingPlaces::from_data("not json").len(), 0);
    }

    #[test]
    fn the_oldest_go_first() {
        let mut places = ReadingPlaces::new();
        for index in 0..ReadingPlaces::LIMIT + 5 {
            places.set(&format!("Note {index}.md"), index);
        }
        // A note read again is new again.
        places.set("Note 5.md", 1);
        places.set("One more.md", 1);
        assert_eq!(places.len(), ReadingPlaces::LIMIT);
        assert_eq!(places.get("Note 0.md"), None);
        assert_eq!(places.get("Note 5.md"), Some(1));
        assert_eq!(places.get("Note 6.md"), None);
    }

    #[test]
    fn reading_places_list_the_notes_read_last() {
        let mut places = ReadingPlaces::new();
        places.set("Old.md", 1);
        places.set("New.md", 2);
        places.set("Old.md", 9);
        assert_eq!(places.recently_read(10), vec!["Old.md", "New.md"]);
        assert_eq!(places.recently_read(1), vec!["Old.md"]);
        assert_eq!(ReadingPlaces::from_data(&places.data()).recently_read(10), places.recently_read(10));
    }

    #[test]
    fn places_follow_moves_and_deletes() {
        let mut places = ReadingPlaces::new();
        places.set("Old/Plan.md", 7);
        places.set("Old/Sub/Deep.md", 2);
        places.set("Keep.md", 1);
        places.moved(&BTreeMap::from([("Old/Plan.md".to_string(), "New/Plan.md".to_string())]));
        assert_eq!(places.get("New/Plan.md"), Some(7));
        assert_eq!(places.get("Old/Plan.md"), None);
        places.forget("Old");
        assert_eq!(places.get("Old/Sub/Deep.md"), None);
        assert_eq!(places.get("Keep.md"), Some(1));
        assert_eq!(ReadingPlaces::from_data(&places.data()), places);
    }

    #[test]
    fn folds_and_read_times_come_back() {
        let mut places = ReadingPlaces::new();
        places.set_folds("Plan.md", vec!["2:Done".into(), "3:Old ideas".into()]);
        places.mark_read("Plan.md", 1_800_000_000);
        places.set("Plan.md", 4);
        let back = ReadingPlaces::from_data(&places.data());
        assert_eq!(back, places);
        assert_eq!(back.folds("Plan.md"), vec!["2:Done", "3:Old ideas"]);
        assert_eq!(back.last_read("Plan.md"), Some(1_800_000_000));
        assert_eq!(back.get("Plan.md"), Some(4));
        places.set_folds("Plan.md", vec![]);
        assert!(places.folds("Plan.md").is_empty());
        places.moved(&BTreeMap::from([("Plan.md".to_string(), "Roadmap.md".to_string())]));
        assert!(places.last_read("Roadmap.md").is_some());
    }

    #[test]
    fn the_first_format_is_still_read() {
        let old = ReadingPlaces::from_data(r#"{"places":[{"A.md":3},{"B.md":9}]}"#);
        assert_eq!(old.get("A.md"), Some(3));
        assert_eq!(old.get("B.md"), Some(9));
    }
}
