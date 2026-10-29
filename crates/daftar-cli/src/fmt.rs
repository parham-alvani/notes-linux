//! `fmt`: the notes formatted as Prettier formats them with the vault's
//! `.prettierrc`, with no Node: `daftar_core::MarkdownFormat`. Writes each
//! note that changes, through `VaultEdits`; `--check` writes nothing and
//! lists them.

use std::fs;
use std::path::{Path, PathBuf};

use daftar_core::{FormatConfig, MarkdownFormat, VaultEdits};
use rayon::prelude::*;

pub struct Outcome {
    /// The notes that are (or were) not formatted.
    pub changed: usize,
    pub checked: usize,
}

/// The vault's `.prettierrc` (or `.prettierrc.json`); Prettier's defaults
/// when it has none.
pub fn config(root: &Path) -> Result<FormatConfig, String> {
    for name in [".prettierrc", ".prettierrc.json"] {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            return FormatConfig::parse(&text).map_err(|e| format!("{name}: {e}"));
        }
    }
    Ok(FormatConfig::default())
}

pub fn run(root: &Path, paths: &[PathBuf], ignore: &[String], check: bool, mut print: impl FnMut(String)) -> Result<Outcome, String> {
    let config = config(root)?;
    let notes = crate::lint::notes_but(root, paths, ignore, ".prettierignore");
    let mut changed = 0;
    if check {
        let unformatted: Vec<bool> = notes
            .par_iter()
            .map(|path| fs::read_to_string(root.join(path)).is_ok_and(|text| MarkdownFormat::format(&text, &config) != text))
            .collect();
        for (path, _) in notes.iter().zip(unformatted).filter(|(_, u)| *u) {
            changed += 1;
            print(path.clone());
        }
    } else {
        for (path, outcome) in notes.iter().zip(VaultEdits::apply_all(&notes, root, |text| MarkdownFormat::format(text, &config))) {
            match outcome {
                Ok(true) => {
                    changed += 1;
                    print(path.clone());
                }
                Ok(false) => {}
                Err(failure) => print(format!("{path}: not formatted: {failure}")),
            }
        }
    }
    Ok(Outcome { changed, checked: notes.len() })
}
