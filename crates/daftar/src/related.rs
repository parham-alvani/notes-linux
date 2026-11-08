//! The inspector's related notes: the notes nearest in meaning to the one
//! on screen (`daftar_core::semantic`), with the model and the vault's
//! vectors loaded once, on a worker thread, and kept for the next note.
//! Nothing here runs until the model is downloaded
//! (`daftar-cli semantic-index --download-model`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use daftar_core::semantic::{self, Nearby, SemanticIndex};
use daftar_core::semantic_model::{self, Model2Vec};

/// The model and one vault's vectors, once loaded.
struct Kept {
    root: PathBuf,
    model: Model2Vec,
    index: SemanticIndex,
}

static KEPT: Mutex<Option<Kept>> = Mutex::new(None);

/// Whether the model is downloaded: the inspector shows related notes only then.
pub fn available() -> bool {
    semantic_model::is_downloaded()
}

/// The notes nearest to [path] in the vault at [root], leaving out
/// [leave_out]; the vectors are brought up to [notes] (each note and its
/// change time) first. Slow the first time, as the model loads and a vault
/// never indexed is embedded: call it away from the main thread.
pub fn nearest(
    root: &Path,
    notes: &[(String, SystemTime)],
    path: &str,
    leave_out: &HashSet<String>,
    limit: usize,
) -> Result<Vec<Nearby>, String> {
    let mut kept = KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if kept.as_ref().is_none_or(|k| k.root != root) {
        let model = Model2Vec::load()?;
        let index = SemanticIndex::open(&semantic::index_file(root), &semantic_model::model_name())?;
        *kept = Some(Kept { root: root.to_path_buf(), model, index });
    }
    let Some(kept) = kept.as_mut() else { return Ok(Vec::new()) };
    kept.index.update(root, notes, &kept.model, &mut |_, _| {})?;
    Ok(kept.index.related(path, leave_out, limit))
}
