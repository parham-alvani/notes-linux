//! Search by meaning, beside the words of `search.rs`: each note cut into
//! its heading sections, each section a vector from an [Embedder], the
//! vectors kept in an SQLite file in the cache so that only the notes that
//! changed are embedded again. A query is a vector too, and the notes whose
//! sections lie nearest to it, by cosine, come first; [fuse] ranks them
//! together with the full-text search's notes (reciprocal rank fusion).
//!
//! Nothing here needs the model: the embedder is a trait, so the tests run
//! on a fake one, and the model (`semantic_model.rs`, the `semantic`
//! feature) is downloaded only when the person asks.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use regex::Regex;
use rusqlite::{Connection, params};

use crate::note_parser::{FrontMatter, NoteParser};
use crate::search::{Hit, readable};
use crate::vault::{Vault, VaultIndex};

/// Turns texts into vectors of one length, each of length 1.
pub trait Embedder: Send + Sync {
    /// The model's name and revision: vectors of another model are never
    /// compared, so a new one indexes the vault again.
    fn model(&self) -> String;
    fn embed(&self, texts: &[String]) -> Vec<Vec<f32>>;
}

/// How notes are cut into chunks. A change to [chunks] raises it, and an
/// index of an older one is made again.
const CHUNKING: &str = "3";

/// The most a chunk holds, in characters: a static model averages its
/// tokens, so a long text says less, and Model2Vec reads 512 tokens at most.
const CHUNK_CHARS: usize = 1500;

/// How many lines of a code block are embedded.
const CODE_LINES: usize = 4;

/// How many chunks go to the embedder at once, between progress reports.
const BATCH: usize = 256;

/// The `k` of reciprocal rank fusion: a note's score is the sum of
/// `1 / (k + rank)` over the lists it is in. 60 is the paper's.
const FUSION_K: f32 = 60.0;

/// A section of a note as it is embedded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The headings above it and its own, joined by ` > `; empty above the first.
    pub heading: String,
    /// The zero-based line it starts on.
    pub line: usize,
    /// What is embedded: the note's title (and, for the first chunk, its
    /// aliases and tags), the heading path, then the text.
    pub text: String,
    /// The text as it reads, for a snippet.
    pub snippet: String,
}

/// A note near a query or a note, with its nearest chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Nearby {
    pub path: String,
    pub heading: String,
    pub line: usize,
    /// The cosine of the chunk with the query, from -1 to 1.
    pub score: f32,
    pub snippet: String,
}

/// What an [SemanticIndex::update] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Updated {
    /// Notes read again because they changed, came or went.
    pub notes: usize,
    /// Chunks embedded; the others of a changed note kept their vector.
    pub embedded: usize,
    pub removed: usize,
}

/// One chunk in memory, as the file has it.
#[derive(Debug, Clone)]
struct Row {
    path: String,
    heading: String,
    line: usize,
    hash: String,
    snippet: String,
    vector: Vec<f32>,
}

pub struct SemanticIndex {
    db: Connection,
    model: String,
    rows: Vec<Row>,
    /// Each note's change time, in milliseconds, as last embedded.
    modified: HashMap<String, i64>,
}

impl SemanticIndex {
    /// The index in [file], made when there is none, for [model]: an index
    /// of another model or chunking is emptied.
    pub fn open(file: &Path, model: &str) -> Result<SemanticIndex, String> {
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        let db = Connection::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
        Self::with(db, model)
    }

    /// An index kept only in memory, for tests.
    pub fn in_memory(model: &str) -> SemanticIndex {
        Self::with(Connection::open_in_memory().expect("an in-memory database"), model).expect("the schema")
    }

    fn with(db: Connection, model: &str) -> Result<SemanticIndex, String> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS notes (path TEXT PRIMARY KEY, modified INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS chunks (
                path TEXT NOT NULL, position INTEGER NOT NULL, heading TEXT NOT NULL, line INTEGER NOT NULL,
                hash TEXT NOT NULL, snippet TEXT NOT NULL, vector BLOB NOT NULL,
                PRIMARY KEY (path, position)
             );",
        )
        .map_err(|e| e.to_string())?;
        let kept: Option<String> = db.query_row("SELECT value FROM meta WHERE key = 'model'", [], |row| row.get(0)).ok();
        let wanted = format!("{model}\n{CHUNKING}");
        if kept.as_deref() != Some(wanted.as_str()) {
            db.execute_batch("DELETE FROM chunks; DELETE FROM notes;").map_err(|e| e.to_string())?;
            db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('model', ?1)", params![wanted]).map_err(|e| e.to_string())?;
        }
        let mut index = SemanticIndex { db, model: model.to_string(), rows: Vec::new(), modified: HashMap::new() };
        index.load().map_err(|e| e.to_string())?;
        Ok(index)
    }

    fn load(&mut self) -> rusqlite::Result<()> {
        let mut statement = self.db.prepare("SELECT path, modified FROM notes")?;
        self.modified = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut statement = self.db.prepare("SELECT path, heading, line, hash, snippet, vector FROM chunks ORDER BY path, position")?;
        self.rows = statement
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(5)?;
                Ok(Row {
                    path: row.get(0)?,
                    heading: row.get(1)?,
                    line: row.get::<_, i64>(2)? as usize,
                    hash: row.get(3)?,
                    snippet: row.get(4)?,
                    vector: blob.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(())
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// How many notes have vectors.
    pub fn note_count(&self) -> usize {
        self.modified.len()
    }

    pub fn chunk_count(&self) -> usize {
        self.rows.len()
    }

    /// The notes of [index] with their change times, as [update] takes them.
    pub fn notes_of(index: &VaultIndex) -> Vec<(String, SystemTime)> {
        index.notes.values().map(|note| (note.path.clone(), note.modified)).collect()
    }

    /// Brings the index up to [notes], the vault's notes and their change
    /// times: a note whose time changed is cut again and only its new
    /// chunks are embedded; a note no longer there is dropped. [progress]
    /// hears the chunks embedded so far and how many there are to embed.
    pub fn update(
        &mut self,
        root: &Path,
        notes: &[(String, SystemTime)],
        embedder: &dyn Embedder,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<Updated, String> {
        if embedder.model() != self.model {
            return Err(format!("the index is of {}, not {}", self.model, embedder.model()));
        }
        let present: HashSet<&str> = notes.iter().map(|(path, _)| path.as_str()).collect();
        let gone: Vec<String> = self.modified.keys().filter(|path| !present.contains(path.as_str())).cloned().collect();
        let changed: Vec<(&str, i64)> = notes
            .iter()
            .map(|(path, time)| (path.as_str(), millis(*time)))
            .filter(|(path, time)| self.modified.get(*path) != Some(time))
            .collect();
        let mut updated = Updated { notes: gone.len() + changed.len(), removed: gone.len(), ..Updated::default() };
        if updated.notes == 0 {
            return Ok(updated);
        }

        // The vectors a changed note already has, by its chunks' text.
        let mut known: HashMap<(&str, &str), &[f32]> = HashMap::new();
        for row in &self.rows {
            known.insert((row.path.as_str(), row.hash.as_str()), &row.vector);
        }
        let mut cut: Vec<(&str, i64, Vec<Chunk>)> = Vec::new();
        let mut wanted: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for &(path, time) in &changed {
            let text = fs::read(root.join(path)).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
            let chunks = chunks(path, &text);
            for chunk in &chunks {
                let hash = hash(&chunk.text);
                if !known.contains_key(&(path, hash.as_str())) && seen.insert(hash) {
                    wanted.push(chunk.text.clone());
                }
            }
            cut.push((path, time, chunks));
        }
        let mut fresh: HashMap<String, Vec<f32>> = HashMap::new();
        progress(0, wanted.len());
        for batch in wanted.chunks(BATCH) {
            let vectors = embedder.embed(batch);
            for (text, vector) in batch.iter().zip(vectors) {
                fresh.insert(hash(text), vector);
            }
            updated.embedded += batch.len();
            progress(updated.embedded, wanted.len());
        }

        let mut rows: Vec<Row> = Vec::new();
        for (path, _, chunks) in &cut {
            for chunk in chunks {
                let hash = hash(&chunk.text);
                let Some(vector) = fresh.get(&hash).map(Vec::as_slice).or_else(|| known.get(&(*path, hash.as_str())).copied()) else {
                    continue;
                };
                rows.push(Row {
                    path: (*path).to_string(),
                    heading: chunk.heading.clone(),
                    line: chunk.line,
                    hash,
                    snippet: chunk.snippet.clone(),
                    vector: vector.to_vec(),
                });
            }
        }
        drop(known);

        let transaction = self.db.transaction().map_err(|e| e.to_string())?;
        for path in gone.iter().map(String::as_str).chain(changed.iter().map(|(path, _)| *path)) {
            transaction.execute("DELETE FROM chunks WHERE path = ?1", params![path]).map_err(|e| e.to_string())?;
            transaction.execute("DELETE FROM notes WHERE path = ?1", params![path]).map_err(|e| e.to_string())?;
        }
        {
            let mut chunk = transaction
                .prepare("INSERT INTO chunks (path, position, heading, line, hash, snippet, vector) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")
                .map_err(|e| e.to_string())?;
            let mut position: HashMap<&str, i64> = HashMap::new();
            for row in &rows {
                let at = position.entry(row.path.as_str()).or_insert(0);
                let blob: Vec<u8> = row.vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                chunk
                    .execute(params![row.path, *at, row.heading, row.line as i64, row.hash, row.snippet, blob])
                    .map_err(|e| e.to_string())?;
                *at += 1;
            }
            let mut note = transaction.prepare("INSERT INTO notes (path, modified) VALUES (?1, ?2)").map_err(|e| e.to_string())?;
            for (path, time) in &changed {
                note.execute(params![path, time]).map_err(|e| e.to_string())?;
            }
        }
        transaction.commit().map_err(|e| e.to_string())?;

        let dropped: HashSet<&str> = gone.iter().map(String::as_str).chain(changed.iter().map(|(path, _)| *path)).collect();
        self.rows.retain(|row| !dropped.contains(row.path.as_str()));
        self.rows.extend(rows);
        for path in &gone {
            self.modified.remove(path);
        }
        for (path, time) in changed {
            self.modified.insert(path.to_string(), time);
        }
        Ok(updated)
    }

    /// The notes nearest to [query], each by its nearest chunk, best first.
    pub fn search(&self, embedder: &dyn Embedder, query: &str, limit: usize) -> Vec<Nearby> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(vector) = embedder.embed(&[query.to_string()]).pop() else { return Vec::new() };
        self.nearest(&vector, |_| true, limit)
    }

    /// The notes nearest to the note at [path] (to the mean of its chunks),
    /// leaving out [leave_out] (the notes it links to, say) and itself.
    /// Empty when the note has no vectors.
    pub fn related(&self, path: &str, leave_out: &HashSet<String>, limit: usize) -> Vec<Nearby> {
        let own: Vec<&Row> = self.rows.iter().filter(|row| row.path == path).collect();
        let Some(first) = own.first() else { return Vec::new() };
        let mut mean = vec![0.0; first.vector.len()];
        for row in &own {
            for (sum, value) in mean.iter_mut().zip(&row.vector) {
                *sum += value;
            }
        }
        normalize(&mut mean);
        self.nearest(&mean, |row| row.path != path && !leave_out.contains(&row.path), limit)
    }

    /// The best chunk of each note that [keep] keeps, by cosine with [vector].
    fn nearest(&self, vector: &[f32], keep: impl Fn(&Row) -> bool, limit: usize) -> Vec<Nearby> {
        let mut best: HashMap<&str, (f32, &Row)> = HashMap::new();
        for row in self.rows.iter().filter(|row| keep(row)) {
            let score = dot(vector, &row.vector);
            let entry = best.entry(row.path.as_str()).or_insert((score, row));
            if score > entry.0 {
                *entry = (score, row);
            }
        }
        let mut found: Vec<(f32, &Row)> = best.into_values().collect();
        found.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
        found
            .into_iter()
            .take(limit)
            .map(|(score, row)| Nearby {
                path: row.path.clone(),
                heading: row.heading.clone(),
                line: row.line,
                score,
                snippet: row.snippet.clone(),
            })
            .collect()
    }
}

/// [words] (the full-text search's notes, best first) and [meaning] (the
/// nearest notes, best first) as one list, by reciprocal rank fusion: a
/// note in both lists comes before one that tops only one. A note found by
/// meaning shows its nearest chunk, under its heading; one found only by
/// its words shows where they are.
pub fn fuse(words: &[Hit], meaning: &[Nearby], limit: usize) -> Vec<Hit> {
    let mut scores: HashMap<&str, f32> = HashMap::new();
    let mut snippets: HashMap<&str, String> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for (rank, hit) in words.iter().enumerate() {
        *scores.entry(&hit.path).or_insert_with(|| {
            order.push(&hit.path);
            0.0
        }) += 1.0 / (FUSION_K + rank as f32 + 1.0);
        snippets.entry(&hit.path).or_insert_with(|| hit.snippet.clone());
    }
    for (rank, near) in meaning.iter().enumerate() {
        *scores.entry(&near.path).or_insert_with(|| {
            order.push(&near.path);
            0.0
        }) += 1.0 / (FUSION_K + rank as f32 + 1.0);
        snippets.insert(&near.path, near.shown());
    }
    let mut paths = order;
    // Stable: a tie keeps the full-text search's order.
    paths.sort_by(|a, b| scores[b].total_cmp(&scores[a]));
    paths.into_iter().take(limit).map(|path| Hit { path: path.to_string(), snippet: snippets.remove(path).unwrap_or_default() }).collect()
}

impl Nearby {
    /// The chunk's heading and its text, as a search shows a snippet.
    pub fn shown(&self) -> String {
        if self.heading.is_empty() { self.snippet.clone() } else { format!("{} · {}", self.heading, self.snippet) }
    }
}

/// [path]'s note cut into its heading sections: the text above the first
/// heading, then each heading's own text (to the next heading of any
/// level), each under the path of headings above it. Of a code block only
/// the first few lines count, as a dump of a configuration or a log says
/// little of what the section is about and pulls every note with one
/// together, while its first lines (a command, a file's name) say much. A
/// section with no text is left out, as its heading is in
/// the path of the ones under it; a long one is cut at a line into pieces
/// of [CHUNK_CHARS] or fewer. A note with no text at all is one chunk of
/// its title.
pub fn chunks(path: &str, text: &str) -> Vec<Chunk> {
    let title = Vault::title(path);
    let parsed = NoteParser::parse(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let front = FrontMatter::parse(&lines).line_count;
    let mut about: Vec<String> = parsed.aliases.clone();
    about.extend(parsed.tags.iter().map(|tag| format!("#{tag}")));
    let mut bounds: Vec<(usize, usize, String)> = Vec::new();
    let mut trail: Vec<(usize, String)> = Vec::new();
    let mut start = front;
    let mut heading = String::new();
    for found in &parsed.headings {
        bounds.push((start, found.line, heading.clone()));
        trail.retain(|(level, _)| *level < found.level);
        trail.push((found.level, found.text.clone()));
        heading = trail.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join(" > ");
        start = found.line + 1;
    }
    bounds.push((start, lines.len(), heading));

    let mut found: Vec<Chunk> = Vec::new();
    for (from, to, heading) in bounds {
        let body = prose(lines.get(from..to.max(from)).unwrap_or_default());
        for (line, piece) in pieces(&body, from) {
            // A blank line first, so a rule (`---`) on the section's first
            // line is not read as front matter.
            let plain = NoteParser::parse(&format!("\n{piece}")).plain;
            if plain.trim().is_empty() {
                continue;
            }
            let mut head = title.clone();
            if found.is_empty() && !about.is_empty() {
                head.push_str(&format!(" ({})", about.join(", ")));
            }
            if !heading.is_empty() {
                head.push('\n');
                head.push_str(&heading);
            }
            found.push(Chunk {
                heading: heading.clone(),
                line,
                text: format!("{head}\n{plain}"),
                snippet: shorten(&readable(&plain), 240),
            });
        }
    }
    if found.is_empty() {
        let head = if about.is_empty() { title } else { format!("{title} ({})", about.join(", ")) };
        found.push(Chunk { heading: String::new(), line: 0, text: head, snippet: String::new() });
    }
    found
}

/// [lines] with no more than the first [CODE_LINES] lines of each code
/// block, the others blank.
fn prose<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());
    let mut kept = Vec::with_capacity(lines.len());
    let mut open: Option<(&str, usize)> = None;
    for line in lines {
        let fence = FENCE.captures(line).and_then(|caps| caps.get(1)).map(|m| m.as_str());
        match (open, fence) {
            (None, Some(marker)) => open = Some((marker, 0)),
            (Some((marker, _)), Some(close)) if close.starts_with(marker) && close.chars().all(|c| marker.starts_with(c)) => open = None,
            (Some((marker, seen)), _) => {
                open = Some((marker, seen + 1));
                if seen >= CODE_LINES {
                    kept.push("");
                    continue;
                }
            }
            (None, None) => {}
        }
        kept.push(*line);
    }
    kept
}

/// [lines] (from line [first] of the note) in pieces of at most
/// [CHUNK_CHARS] characters, cut at a blank line when one is near, else at
/// any line; each with the first line of it that has text.
fn pieces(lines: &[&str], first: usize) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut piece = String::new();
    let mut start = first;
    let mut size = 0;
    for (offset, line) in lines.iter().enumerate() {
        let length = line.chars().count() + 1;
        let full = size + length > CHUNK_CHARS || (size > CHUNK_CHARS * 2 / 3 && line.trim().is_empty());
        if full && !piece.trim().is_empty() {
            found.push((start, std::mem::take(&mut piece)));
            size = 0;
        }
        // A piece starts on its first line with text.
        if piece.trim().is_empty() {
            start = first + offset;
        }
        piece.push_str(line);
        piece.push('\n');
        size += length;
    }
    if !piece.trim().is_empty() {
        found.push((start, piece));
    }
    found
}

/// [text] to at most [most] characters, cut at a space when one is near.
fn shorten(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_string();
    }
    let cut: String = text.chars().take(most).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > cut.len() / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// [vector] scaled to length 1, unless it is all zeros.
pub fn normalize(vector: &mut [f32]) {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 {
        for value in vector {
            *value /= length;
        }
    }
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// FNV-1a, 64 bits, in hex: the same on every machine and every Rust, as
/// std's hasher is not promised to be.
fn hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Daftar's cache: `$XDG_CACHE_HOME/daftar`, else `~/.cache/daftar`.
pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| std::env::home_dir().unwrap_or_default().join(".cache"), PathBuf::from);
    base.join("daftar")
}

/// The index file of the vault at [root]: one per vault, named by a hash of
/// its folder's path.
pub fn index_file(root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    cache_dir().join("semantic").join(format!("{}.sqlite", hash(&root.to_string_lossy())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model of a few words: each vector counts the words of a few topics,
    /// in English and Persian alike, so that "broker" and "پیام" are near.
    struct Fake;

    const TOPICS: [&[&str]; 4] = [
        &["broker", "queue", "consumer", "lag", "nats", "kafka", "پیام", "صف"],
        &["garden", "plants", "basil", "mint", "باغ", "گیاه"],
        &["swim", "pool", "swimming", "شنا", "استخر"],
        &["tax", "bank", "money", "بانک", "پول"],
    ];

    impl Embedder for Fake {
        fn model(&self) -> String {
            "fake".into()
        }

        fn embed(&self, texts: &[String]) -> Vec<Vec<f32>> {
            texts
                .iter()
                .map(|text| {
                    let text = text.to_lowercase();
                    let mut vector: Vec<f32> =
                        TOPICS.iter().map(|words| words.iter().map(|w| text.matches(w).count()).sum::<usize>() as f32).collect();
                    vector.push(0.01);
                    normalize(&mut vector);
                    vector
                })
                .collect()
        }
    }

    fn vault(notes: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in notes {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    fn notes(root: &Path) -> Vec<(String, SystemTime)> {
        SemanticIndex::notes_of(&Vault::index(root, None))
    }

    fn paths(found: &[Nearby]) -> Vec<&str> {
        found.iter().map(|n| n.path.as_str()).collect()
    }

    #[test]
    fn a_note_is_cut_at_its_headings_under_its_title() {
        let text = "---\naliases: [Queue]\ntags: [work]\n---\nIntro line.\n\n# Broker\n\n## Lag\n\nConsumers fall behind.\n\n## Empty\n\n# Other\n---\nAfter a rule.\n";
        let found = chunks("Work/NATS.md", text);
        let shown: Vec<(&str, usize)> = found.iter().map(|c| (c.heading.as_str(), c.line)).collect();
        assert_eq!(shown, [("", 4), ("Broker > Lag", 10), ("Other", 15)]);
        assert_eq!(found[0].text, "NATS (Queue, #work)\nIntro line.");
        assert_eq!(found[1].text, "NATS\nBroker > Lag\nConsumers fall behind.");
        assert_eq!(found[1].snippet, "Consumers fall behind.");
        assert!(found[2].text.ends_with("After a rule."));
        // A note with no text is its title.
        assert_eq!(chunks("Empty.md", "# \n")[0].text, "Empty");
    }

    #[test]
    fn only_the_first_lines_of_a_code_block_count() {
        let text = "# Stream\n\nThe stream of rides.\n\n```text\n1\n2\n3\n4\n5\n6\n```\n\nAfter.\n\n# Only code\n\n~~~\nmake all\n~~~\n";
        let found = chunks("NATS.md", text);
        assert_eq!(found[0].text, "NATS\nStream\nThe stream of rides.\n1\n2\n3\n4\n\n\nAfter.");
        assert_eq!(found[1].text, "NATS\nOnly code\nmake all");
    }

    #[test]
    fn a_long_section_is_cut_into_pieces() {
        let paragraph = "word ".repeat(60);
        let text = format!("# Long\n{}", format!("{paragraph}\n\n").repeat(12));
        let found = chunks("Long.md", &text);
        assert!(found.len() > 1);
        assert!(
            found.iter().all(|c| c.text.chars().count() <= CHUNK_CHARS + 20),
            "{:?}",
            found.iter().map(|c| c.text.len()).collect::<Vec<_>>()
        );
        assert!(found.iter().all(|c| c.heading == "Long"));
        assert!(found.windows(2).all(|w| w[0].line < w[1].line));
    }

    #[test]
    fn notes_are_found_by_meaning_in_either_language() {
        let root = vault(&[
            ("Work/Incident.md", "# Incident\n\nThe consumer lag on NATS grew all night.\n"),
            ("Garden.md", "# Garden\n\nBasil and mint need sun.\n"),
            ("شنا.md", "# شنا\n\nهر هفته به استخر می‌روم.\n"),
        ]);
        let mut index = SemanticIndex::in_memory("fake");
        let updated = index.update(root.path(), &notes(root.path()), &Fake, &mut |_, _| {}).unwrap();
        assert_eq!(updated, Updated { notes: 3, embedded: 3, removed: 0 });
        assert_eq!(paths(&index.search(&Fake, "kafka queue", 1)), ["Work/Incident.md"]);
        assert_eq!(paths(&index.search(&Fake, "صف پیام", 1)), ["Work/Incident.md"]);
        assert_eq!(paths(&index.search(&Fake, "swimming pool", 1)), ["شنا.md"]);
        let best = &index.search(&Fake, "plants", 3)[0];
        assert_eq!((best.heading.as_str(), best.line, best.snippet.as_str()), ("Garden", 2, "Basil and mint need sun."));
        assert!(index.search(&Fake, "  ", 3).is_empty());
    }

    #[test]
    fn only_what_changed_is_embedded_again() {
        let root = vault(&[("A.md", "# One\n\nbroker\n\n# Two\n\ngarden\n"), ("B.md", "pool\n")]);
        let file = root.path().join(".cache/index.sqlite");
        let mut index = SemanticIndex::open(&file, "fake").unwrap();
        assert_eq!(index.update(root.path(), &notes(root.path()), &Fake, &mut |_, _| {}).unwrap().embedded, 3);
        // Nothing changed: nothing is read.
        assert_eq!(index.update(root.path(), &notes(root.path()), &Fake, &mut |_, _| {}).unwrap(), Updated::default());

        // One section of A changed, B is gone, C is new: the file keeps them all.
        fs::write(root.path().join("A.md"), "# One\n\nbroker\n\n# Two\n\nbank money\n").unwrap();
        fs::remove_file(root.path().join("B.md")).unwrap();
        fs::write(root.path().join("C.md"), "mint\n").unwrap();
        let mut notes = notes(root.path());
        for (path, time) in &mut notes {
            if path == "A.md" {
                *time += std::time::Duration::from_secs(5);
            }
        }
        let mut heard = Vec::new();
        let updated = index.update(root.path(), &notes, &Fake, &mut |done, all| heard.push((done, all))).unwrap();
        assert_eq!(updated, Updated { notes: 3, embedded: 2, removed: 1 });
        assert_eq!(heard, [(0, 2), (2, 2)]);
        drop(index);

        let index = SemanticIndex::open(&file, "fake").unwrap();
        assert_eq!((index.note_count(), index.chunk_count()), (2, 3));
        assert_eq!(paths(&index.search(&Fake, "money", 1)), ["A.md"]);
        assert!(index.search(&Fake, "pool", 5).iter().all(|n| n.path != "B.md"));
        // Another model starts again, and takes no vectors of this one.
        let mut other = SemanticIndex::open(&file, "other").unwrap();
        assert_eq!(other.chunk_count(), 0);
        assert!(other.update(root.path(), &[], &Fake, &mut |_, _| {}).is_err());
    }

    #[test]
    fn related_notes_leave_out_the_note_and_its_links() {
        let root = vault(&[
            ("Incident.md", "NATS consumer lag. See [[Kafka]].\n"),
            ("Kafka.md", "Kafka is a broker with a queue.\n"),
            ("Queue.md", "# Queue\n\nThe broker queue and its consumer.\n\n# Aside\n\nA bank.\n"),
            ("Garden.md", "Basil.\n"),
        ]);
        let mut index = SemanticIndex::in_memory("fake");
        index.update(root.path(), &notes(root.path()), &Fake, &mut |_, _| {}).unwrap();
        let near = index.related("Incident.md", &HashSet::from(["Kafka.md".to_string()]), 2);
        assert_eq!(paths(&near)[0], "Queue.md");
        assert_eq!(near[0].heading, "Queue");
        assert!(paths(&near).iter().all(|p| *p != "Incident.md" && *p != "Kafka.md"));
        assert!(index.related("Nowhere.md", &HashSet::new(), 5).is_empty());
    }

    #[test]
    fn fusion_puts_a_note_both_lists_found_first() {
        let hit = |path: &str| Hit { path: path.into(), snippet: format!("words of {path}") };
        let near =
            |path: &str, heading: &str| Nearby { path: path.into(), heading: heading.into(), line: 0, score: 0.5, snippet: "chunk".into() };
        let fused = fuse(&[hit("a"), hit("b"), hit("c")], &[near("c", "Lag"), near("d", ""), near("a", "")], 10);
        let order: Vec<&str> = fused.iter().map(|h| h.path.as_str()).collect();
        assert_eq!(order, ["a", "c", "b", "d"]);
        assert_eq!(fused[1].snippet, "Lag · chunk");
        assert_eq!(fused[2].snippet, "words of b");
        assert_eq!(fuse(&[hit("a")], &[], 0), []);
    }

    #[test]
    fn the_index_file_is_in_the_cache_by_the_vaults_path() {
        let file = index_file(Path::new("/nowhere/vault"));
        assert!(file.starts_with(cache_dir().join("semantic")));
        assert_eq!(file, index_file(Path::new("/nowhere/vault")));
        assert_ne!(file, index_file(Path::new("/nowhere/other")));
        assert_eq!(hash(""), "cbf29ce484222325");
    }
}
