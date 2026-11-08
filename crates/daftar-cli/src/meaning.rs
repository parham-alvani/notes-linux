//! Search by meaning on the command line and in the MCP server: the model
//! loaded, the vault's vector index (in the cache) brought up to the notes,
//! and the queries `search --meaning`, `related` and `semantic-index` run.

use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use daftar_core::semantic::{self, Embedder, Nearby, SemanticIndex};
use daftar_core::semantic_model::{self, Model2Vec};
use daftar_core::{ExcludedFiles, Hit, SearchIndex, VaultIndex};

const AGENT: &str = concat!("daftar-cli/", env!("CARGO_PKG_VERSION"));

/// Loads the model: [model] for the downloaded one, a fake in tests.
pub type Load = fn() -> Result<Box<dyn Embedder>, String>;

/// The downloaded model, or [semantic_model::missing] when it is not there.
pub fn model() -> Result<Box<dyn Embedder>, String> {
    Ok(Box::new(Model2Vec::load()?))
}

/// The model and the vault's index, as a command or the server keeps them.
pub struct Meaning {
    pub model: Box<dyn Embedder>,
    pub index: SemanticIndex,
}

impl Meaning {
    /// The model and the index of the vault at [root], in the cache.
    pub fn open(root: &Path) -> Result<Meaning, String> {
        Self::with(model, &semantic::index_file(root))
    }

    /// The model [load] loads, and the index in [file].
    pub fn with(load: Load, file: &Path) -> Result<Meaning, String> {
        let model = load()?;
        let index = SemanticIndex::open(file, &model.model())?;
        Ok(Meaning { model, index })
    }

    /// The index brought up to [vault]'s notes; with [loud], the chunks
    /// embedded so far on stderr.
    pub fn update(&mut self, root: &Path, vault: &VaultIndex, loud: bool) -> Result<semantic::Updated, String> {
        let notes = SemanticIndex::notes_of(vault);
        let terminal = std::io::stderr().is_terminal();
        let mut progress = |done: usize, all: usize| {
            if !loud || all == 0 || (!terminal && done != all) {
                return;
            }
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, "{}embedding {done}/{all} sections", if terminal { "\r" } else { "" });
            if done == all {
                let _ = writeln!(stderr);
            }
            let _ = stderr.flush();
        };
        self.index.update(root, &notes, self.model.as_ref(), &mut progress)
    }

    /// The notes nearest to [query], without the hidden ones.
    pub fn search(&self, hidden: &ExcludedFiles, query: &str, limit: usize) -> Vec<Nearby> {
        let found = self.index.search(self.model.as_ref(), query, limit.saturating_add(100));
        found.into_iter().filter(|n| !hidden.contains(&n.path)).take(limit).collect()
    }

    /// [search] fused with the full-text search of [words].
    pub fn hybrid(&self, words: &SearchIndex, hidden: &ExcludedFiles, query: &str, limit: usize) -> Vec<Hit> {
        // Enough of each list that a note well placed in one is not cut off.
        let depth = limit.max(20) * 3;
        let found: Vec<Hit> =
            words.search(query, depth.saturating_add(100)).into_iter().filter(|h| !hidden.contains(&h.path)).take(depth).collect();
        semantic::fuse(&found, &self.search(hidden, query, depth), limit)
    }

    /// The notes nearest to [path] that it does not link to, without the hidden ones.
    pub fn related(&self, vault: &VaultIndex, hidden: &ExcludedFiles, path: &str, limit: usize) -> Vec<Nearby> {
        let links: HashSet<String> = vault.resolved_links(path).into_values().collect();
        self.index.related(path, &links, limit.saturating_add(100)).into_iter().filter(|n| !hidden.contains(&n.path)).take(limit).collect()
    }
}

/// What the MCP server keeps between calls: the model and the index once
/// loaded, how to load the model, and the index's file.
pub struct Kept {
    loaded: Option<Meaning>,
    load: Load,
    file: PathBuf,
}

impl Kept {
    pub fn new(root: &Path) -> Kept {
        Self::with(model, semantic::index_file(root))
    }

    pub fn with(load: Load, file: PathBuf) -> Kept {
        Kept { loaded: None, load, file }
    }

    /// The model and the index, loaded at the first call and brought up to
    /// [vault]'s notes at each.
    pub fn get(&mut self, root: &Path, vault: &VaultIndex) -> Result<&Meaning, String> {
        let meaning = match &mut self.loaded {
            Some(meaning) => meaning,
            loaded @ None => loaded.insert(Meaning::with(self.load, &self.file)?),
        };
        meaning.update(root, vault, false)?;
        Ok(meaning)
    }
}

/// A note near a query or a note, as a line: `path:line`, the cosine, then
/// the heading and the text of its nearest section.
pub fn line(near: &Nearby) -> String {
    format!("{}:{}\t{:.2}\t{}", near.path, near.line + 1, near.score, crate::snippet(&near.shown(), false))
}

/// `semantic-index`: downloads the model when asked, then indexes the vault.
pub fn build(root: &Path, vault: &VaultIndex, download: bool) -> Result<(), String> {
    if !semantic_model::is_downloaded() {
        if !download {
            return Err(semantic_model::missing());
        }
        eprintln!(
            "downloading {} at {} ({} MB) into {}",
            semantic_model::REPOSITORY,
            &semantic_model::REVISION[..8],
            semantic_model::download_size().div_ceil(1_000_000),
            semantic_model::model_dir().display()
        );
        let terminal = std::io::stderr().is_terminal();
        let mut last = u64::MAX;
        semantic_model::download(AGENT, &mut |done, all| {
            let step = done * 100 / all.max(1);
            if step != last && (terminal || step % 10 == 0) {
                last = step;
                let mut stderr = std::io::stderr().lock();
                let _ = write!(stderr, "{}{step}% of {} MB", if terminal { "\r" } else { "" }, all.div_ceil(1_000_000));
                let _ = if terminal && step < 100 { stderr.flush() } else { writeln!(stderr) };
            }
        })?;
    }
    let started = std::time::Instant::now();
    let mut meaning = Meaning::open(root)?;
    let updated = meaning.update(root, vault, true)?;
    eprintln!(
        "{} notes, {} sections in {}; {} notes read again, {} sections embedded, {} notes dropped, in {:.1} s",
        meaning.index.note_count(),
        meaning.index.chunk_count(),
        semantic::index_file(root).display(),
        updated.notes - updated.removed,
        updated.embedded,
        updated.removed,
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// A model of a few words for the tests: each vector counts the words of a
/// few topics, in English and Persian alike.
#[cfg(test)]
pub mod fake {
    use daftar_core::semantic::{Embedder, normalize};

    pub struct Fake;

    const TOPICS: [&[&str]; 3] =
        [&["broker", "queue", "consumer", "lag", "صف", "پیام"], &["garden", "basil", "mint", "tomato"], &["bank", "money"]];

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
                        TOPICS.iter().map(|words| words.iter().map(|w| text.matches(w).count() as f32).sum()).collect();
                    vector.push(0.01);
                    normalize(&mut vector);
                    vector
                })
                .collect()
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "a `Load`, as the real model's may fail")]
    pub fn load() -> Result<Box<dyn Embedder>, String> {
        Ok(Box::new(Fake))
    }
}
