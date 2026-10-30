//! `fmt`: the notes formatted as Prettier formats them with the vault's
//! `.prettierrc`, with no Node: `daftar_core::MarkdownFormat`. Writes each
//! note that changes, through `VaultEdits`; `--check` writes nothing and
//! lists them, with `--diff` what would change, and in GitHub Actions an
//! annotation on each note's first line that would.

use std::fs;
use std::path::{Path, PathBuf};

use daftar_core::line_diff::{Kind, LineDiff};
use daftar_core::{FormatConfig, MarkdownFormat, VaultEdits};
use rayon::prelude::*;

use crate::report;

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

pub struct Options {
    /// Writes nothing; lists the notes that are not formatted.
    pub check: bool,
    /// With [check], the lines formatting would change.
    pub diff: bool,
    /// Prints GitHub Actions annotations.
    pub github: bool,
}

pub fn run(root: &Path, paths: &[PathBuf], ignore: &[String], options: &Options, mut print: impl FnMut(String)) -> Result<Outcome, String> {
    let config = config(root)?;
    let notes = crate::lint::notes_but(root, paths, ignore, ".prettierignore");
    let mut changed = 0;
    if options.check {
        // Each note that is not formatted, with its text and the formatted one.
        let unformatted: Vec<Option<(String, String)>> = notes
            .par_iter()
            .map(|path| {
                let text = fs::read_to_string(root.join(path)).ok()?;
                let formatted = MarkdownFormat::format(&text, &config);
                (formatted != text).then_some((text, formatted))
            })
            .collect();
        for (path, (text, formatted)) in notes.iter().zip(unformatted).filter_map(|(p, u)| Some((p, u?))) {
            changed += 1;
            let lines = LineDiff::lines(&text, &formatted);
            if options.github {
                let first = lines.iter().find(|l| l.kind != Kind::Same).and_then(|l| l.old.or(l.new)).unwrap_or(1);
                print(report::annotation("error", path, first, None, "fmt", "not formatted; run daftar-cli fmt"));
            } else {
                print(path.clone());
            }
            if options.diff {
                for hunk in LineDiff::hunks(&lines, 2) {
                    let at = hunk.iter().find_map(|l| l.old).unwrap_or(1);
                    print(format!("@@ {path}:{at} @@"));
                    for line in hunk {
                        let mark = match line.kind {
                            Kind::Same => ' ',
                            Kind::Added => '+',
                            Kind::Removed => '-',
                        };
                        print(format!("{mark}{}", line.text));
                    }
                }
            }
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
