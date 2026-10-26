//! Full-text search over one vault, in an SQLite FTS5 table held in memory.
//!
//! In memory because the vault on disk is the record: the index is built
//! again at each launch (a few seconds for a few thousand notes) and kept in
//! step with the files after that, so it has no schema to migrate and cannot
//! disagree with the files for longer than one change.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hit {
    pub path: String,
    /// The matching text, with the words between `\u{1}` and `\u{2}`.
    pub snippet: String,
}

pub struct SearchIndex {
    db: Mutex<Connection>,
    /// Each note's row. `path` is not indexed in FTS5, so a row is found and
    /// removed by its rowid: a `WHERE path = ?` would read the whole table,
    /// and the first index of a large vault would take seconds.
    rows: Mutex<HashMap<String, i64>>,
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchIndex {
    pub fn new() -> SearchIndex {
        let db = Connection::open_in_memory().expect("an in-memory database");
        db.execute_batch(
            "CREATE VIRTUAL TABLE note_fts USING fts5(
                path UNINDEXED, title, body,
                tokenize = 'unicode61 remove_diacritics 2'
            )",
        )
        .expect("SQLite with FTS5");
        SearchIndex { db: Mutex::new(db), rows: Mutex::new(HashMap::new()) }
    }

    pub fn put(&self, path: &str, title: &str, body: &str) {
        let db = self.db.lock().unwrap();
        let mut rows = self.rows.lock().unwrap();
        if let Some(row) = rows.remove(path) {
            let _ = db.execute("DELETE FROM note_fts WHERE rowid = ?1", params![row]);
        }
        if db.execute("INSERT INTO note_fts (path, title, body) VALUES (?1, ?2, ?3)", params![path, title, body]).is_ok() {
            rows.insert(path.to_string(), db.last_insert_rowid());
        }
    }

    pub fn remove(&self, path: &str) {
        let db = self.db.lock().unwrap();
        if let Some(row) = self.rows.lock().unwrap().remove(path) {
            let _ = db.execute("DELETE FROM note_fts WHERE rowid = ?1", params![row]);
        }
    }

    /// Runs [work] in one transaction, which makes the first index of a large
    /// vault many times faster.
    pub fn transaction(&self, work: impl FnOnce(&SearchIndex)) {
        let _ = self.db.lock().unwrap().execute_batch("BEGIN");
        work(self);
        let _ = self.db.lock().unwrap().execute_batch("COMMIT");
    }

    /// Notes that hold every word of [query], best first. Each word matches
    /// as a prefix, and a quoted phrase matches as written.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let Some(expression) = Self::match_expression(query) else { return vec![] };
        let db = self.db.lock().unwrap();
        let Ok(mut statement) = db.prepare_cached(
            "SELECT path, snippet(note_fts, 2, char(1), char(2), '…', 16)
             FROM note_fts WHERE note_fts MATCH ?1
             ORDER BY bm25(note_fts, 0, 8, 1) LIMIT ?2",
        ) else {
            return vec![];
        };
        let rows = statement.query_map(params![expression, limit as i64], |row| {
            Ok(Hit { path: row.get(0)?, snippet: row.get::<_, Option<String>>(1)?.unwrap_or_default() })
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => vec![],
        }
    }

    /// The person's words as an FTS5 expression. Every word is quoted, so
    /// that `AND`, `-` or a `:` in what they type is a word and not syntax.
    pub fn match_expression(query: &str) -> Option<String> {
        let mut terms: Vec<String> = Vec::new();
        let flush = |terms: &mut Vec<String>, word: &str, prefix: bool| {
            let clean = word.replace('"', "");
            if clean.trim().is_empty() {
                return;
            }
            terms.push(format!("\"{clean}\"{}", if prefix { "*" } else { "" }));
        };
        let mut word = String::new();
        let mut phrase = String::new();
        let mut in_phrase = false;
        for c in query.chars() {
            if c == '"' {
                if in_phrase {
                    flush(&mut terms, &phrase, false);
                    phrase.clear();
                } else {
                    flush(&mut terms, &word, true);
                    word.clear();
                }
                in_phrase = !in_phrase;
            } else if in_phrase {
                phrase.push(c);
            } else if c.is_whitespace() {
                flush(&mut terms, &word, true);
                word.clear();
            } else {
                word.push(c);
            }
        }
        if in_phrase {
            flush(&mut terms, &phrase, false);
        } else {
            flush(&mut terms, &word, true);
        }
        (!terms.is_empty()).then(|| terms.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(hits: Vec<Hit>) -> Vec<String> {
        hits.into_iter().map(|h| h.path).collect()
    }

    #[test]
    fn every_word_as_a_prefix() {
        let index = SearchIndex::new();
        index.put("a.md", "Kubernetes", "pods and deployments");
        index.put("b.md", "Cooking", "pasta with deployment of sauce");
        index.put("c.md", "فارسی", "یادداشت درباره کتاب");
        let mut found = paths(index.search("deploy", 100));
        found.sort();
        assert_eq!(found, ["a.md", "b.md"]);
        assert_eq!(paths(index.search("deploy pods", 100)), ["a.md"]);
        assert_eq!(paths(index.search("کتاب", 100)), ["c.md"]);
        assert_eq!(paths(index.search("\"pasta with\"", 100)), ["b.md"]);
        index.remove("a.md");
        assert!(index.search("pods", 100).is_empty());
    }

    #[test]
    fn syntax_is_quoted() {
        assert_eq!(SearchIndex::match_expression("a AND -b:c").as_deref(), Some("\"a\"* AND \"AND\"* AND \"-b:c\"*"));
        assert_eq!(SearchIndex::match_expression("  "), None);
    }
}
