//! Full-text search over one vault, in an SQLite FTS5 table held in memory.
//!
//! In memory because the vault on disk is the record: the index is built
//! again at each launch (a few seconds for a few thousand notes) and kept in
//! step with the files after that, so it has no schema to migrate and cannot
//! disagree with the files for longer than one change.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hit {
    pub path: String,
    /// The matching text as it reads, with the words between `\u{1}` and
    /// `\u{2}`: see [readable].
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
    /// as a prefix, and a quoted phrase matches as written. The words side by
    /// side, as typed, come first; then the words near each other; then the
    /// notes that hold them anywhere. Within each, the title counts most.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let db = self.db.lock().unwrap();
        let mut hits: Vec<Hit> = Vec::new();
        let mut seen = HashSet::new();
        for expression in Self::match_tiers(query) {
            if hits.len() >= limit {
                break;
            }
            for hit in Self::run(&db, &expression, limit) {
                if seen.insert(hit.path.clone()) {
                    hits.push(hit);
                    if hits.len() == limit {
                        break;
                    }
                }
            }
        }
        hits
    }

    /// The FTS5 expressions [search] runs, best first: with two or more
    /// unquoted words, the words as one phrase (the last one a prefix, as it
    /// may be half typed) and the words within ten of each other, before
    /// [match_expression]'s every word anywhere.
    pub fn match_tiers(query: &str) -> Vec<String> {
        let Some(every) = Self::match_expression(query) else { return vec![] };
        let words: Vec<String> = if query.contains('"') { vec![] } else { query.split_whitespace().map(|w| w.replace('"', "")).collect() };
        if words.len() < 2 {
            return vec![every];
        }
        let quoted: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
        let (last, rest) = quoted.split_last().unwrap();
        let phrase = format!("{} + {last}*", rest.join(" + "));
        let near = format!("NEAR({} {last}*, 10)", rest.join(" "));
        vec![phrase, near, every]
    }

    fn run(db: &Connection, expression: &str, limit: usize) -> Vec<Hit> {
        let Ok(mut statement) = db.prepare_cached(
            "SELECT path, snippet(note_fts, 2, char(1), char(2), '…', 16)
             FROM note_fts WHERE note_fts MATCH ?1
             ORDER BY bm25(note_fts, 0, 8, 1) LIMIT ?2",
        ) else {
            return vec![];
        };
        let rows = statement.query_map(params![expression, limit as i64], |row| {
            Ok(Hit { path: row.get(0)?, snippet: readable(&row.get::<_, Option<String>>(1)?.unwrap_or_default()) })
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

/// The lines of [snippet] with a match, as they read: no heading marks,
/// list marks, boxes, quote marks, emphasis or task emoji, and a link as its
/// text. Lines are joined by ` … `; the match marks are kept.
pub fn readable(snippet: &str) -> String {
    static LEAD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(?:\[![^\]]*\][+-]?\s*)?(?:#{1,6}\s+)?(?:(?:[-*+]|\d+[.)])\s+)?(?:\[.\]\s+)?").unwrap());
    static ALIASED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!?\[\[[^\]|]*\|([^\]]*)\]\]").unwrap());
    static WIKI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!?\[\[([^\]]*)\]\]").unwrap());
    static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!?\[([^\]]*)\]\([^)]*\)").unwrap());
    static MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\*\*|__|==|~~|`|[📅⏳🛫✅➕🔁❌⏫🔼🔽⏬🔺🆔⛔🏁\x{fe0f}]").unwrap());
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let lines: Vec<&str> = snippet.split('\n').collect();
    let matched: Vec<&str> = lines.iter().copied().filter(|line| line.contains('\u{1}')).collect();
    let lines = if matched.is_empty() { lines } else { matched };
    lines
        .into_iter()
        .map(|line| {
            let line = LEAD.replace(line, "");
            let line = ALIASED.replace_all(&line, "$1");
            let line = WIKI.replace_all(&line, "$1");
            let line = MARKDOWN.replace_all(&line, "$1");
            let line = MARKS.replace_all(&line, "");
            SPACES.replace_all(&line, " ").trim().to_string()
        })
        .filter(|line| !line.is_empty() && line != "…")
        .collect::<Vec<_>>()
        .join(" … ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_reads_as_text() {
        let raw = "…# Home\n- [ ] Book flights 📅 2026-10-28\n- [/] \u{1}Paint\u{2} the **hall** with [[Work|the team]]\n- [ ] Pay…";
        assert_eq!(readable(raw), "\u{1}Paint\u{2} the hall with the team");
        assert_eq!(readable("> [!todo] Now\n> see [[Kite]] and [the site](https://x.y)"), "Now … see Kite and the site");
        assert_eq!(readable("1. ✅ done ==\u{1}fast\u{2}=="), "done \u{1}fast\u{2}");
    }

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

    /// The words side by side come first, then near each other, then
    /// anywhere: "House II" finds the note that names it before one about a
    /// house that mentions World War II.
    #[test]
    fn the_phrase_first() {
        let index = SearchIndex::new();
        index.put(
            "war.md",
            "History",
            "A house burned down in the old town, and the long story of that night is told by the people who lived there through World War II.",
        );
        index.put("near.md", "Flats", "The house we call II is by the sea.");
        index.put("flat.md", "Identity", "Address of House II: the second flat.");
        index.put("other.md", "Other", "Nothing here.");
        assert_eq!(paths(index.search("House II", 100)), ["flat.md", "near.md", "war.md"]);
        // Half typed: the last word is a prefix in the phrase too.
        assert_eq!(paths(index.search("house i", 100)).first().map(String::as_str), Some("flat.md"));
        // Quotes keep their meaning and run alone.
        assert_eq!(SearchIndex::match_tiers("\"House II\""), ["\"House II\""]);
        assert_eq!(SearchIndex::match_tiers("house").len(), 1);
        assert_eq!(index.search("house ii", 2).len(), 2);
    }

    #[test]
    fn syntax_is_quoted() {
        assert_eq!(SearchIndex::match_expression("a AND -b:c").as_deref(), Some("\"a\"* AND \"AND\"* AND \"-b:c\"*"));
        assert_eq!(SearchIndex::match_expression("  "), None);
    }
}
