//! Where each note was being read: the source line of the reading cursor,
//! kept across launches so a note opens where it was left. The notes read
//! last are kept, up to `LIMIT`; a move or a rename takes its place along.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadingPlaces {
    lines: HashMap<String, usize>,
    /// Read first to read last.
    order: Vec<String>,
}

impl ReadingPlaces {
    pub const LIMIT: usize = 1000;

    pub fn new() -> ReadingPlaces {
        ReadingPlaces::default()
    }

    pub fn get(&self, path: &str) -> Option<usize> {
        self.lines.get(path).copied()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn set(&mut self, path: &str, line: usize) {
        if self.lines.insert(path.to_string(), line).is_some() {
            if self.order.last().map(String::as_str) == Some(path) {
                return;
            }
            self.order.retain(|p| p != path);
        }
        self.order.push(path.to_string());
        while self.order.len() > Self::LIMIT {
            let oldest = self.order.remove(0);
            self.lines.remove(&oldest);
        }
    }

    /// After a move or a rename: [moves] maps each file's old path to its new one.
    pub fn moved(&mut self, moves: &BTreeMap<String, String>) {
        if !moves.keys().any(|k| self.lines.contains_key(k)) {
            return;
        }
        let rename = |p: &String| moves.get(p).unwrap_or(p).clone();
        self.lines = self.lines.iter().map(|(path, line)| (rename(path), *line)).collect();
        self.order = self.order.iter().map(rename).collect();
    }

    /// After a delete: the file, or every file in the folder.
    pub fn forget(&mut self, path: &str) {
        let gone = |other: &str| other == path || other.starts_with(&format!("{path}/"));
        self.order.retain(|p| !gone(p));
        self.lines.retain(|p, _| !gone(p));
    }

    /// `{"places": [{"Folder/Note.md": 12}, …]}`, read first to read last.
    pub fn data(&self) -> String {
        let places: Vec<Value> = self
            .order
            .iter()
            .filter_map(|path| self.lines.get(path).map(|line| Value::Object([(path.clone(), json!(line))].into_iter().collect())))
            .collect();
        json!({ "places": places }).to_string()
    }

    /// Places from [data]; none from a file that is missing or broken.
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
}
