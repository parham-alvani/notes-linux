//! `fmt`: the notes formatted as Prettier formats them with the vault's
//! `.prettierrc`, with no Node: `daftar_core::MarkdownFormat`. Writes each
//! note that changes, through `VaultEdits`; `--check` writes nothing and
//! lists them, with `--diff` what would change, in GitHub Actions an
//! annotation on each note's first line that would, and with `--json` a JSON
//! line there (`report.rs`). `--stdin` formats one note from stdin and
//! prints it, writing nothing.

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
    /// How a note that is not formatted is printed with [check].
    pub mode: report::Mode,
}

pub fn run(root: &Path, paths: &[PathBuf], ignore: &[String], options: &Options, mut print: impl FnMut(String)) -> Result<Outcome, String> {
    let config = config(root)?;
    let notes = crate::lint::notes_but(root, paths, ignore, Some(".prettierignore"))?;
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
            let first = lines.iter().find(|l| l.kind != Kind::Same).and_then(|l| l.old.or(l.new)).unwrap_or(1);
            let finding = report::Finding {
                tool: "fmt",
                level: "error",
                path,
                line: first,
                column: None,
                code: "fmt",
                message: "not formatted; run daftar-cli fmt",
                fixable: true,
            };
            print(finding.printed(options.mode).unwrap_or_else(|| path.clone()));
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

/// [text] as `fmt` would write it, for `fmt --stdin`; unchanged when
/// [path] (from the vault's folder) is a file `fmt` leaves alone: one
/// `.prettierignore` or [ignore] names, or in a hidden folder.
pub fn formatted(root: &Path, text: &str, path: Option<&str>, ignore: &[String]) -> Result<String, String> {
    let config = config(root)?;
    let left_out = crate::lint::left_out(root, ignore, Some(".prettierignore"));
    if path.is_some_and(|p| left_out(p.trim_start_matches("./"))) {
        return Ok(text.to_string());
    }
    Ok(MarkdownFormat::format(text, &config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_from_stdin_is_formatted_as_fmt_writes_it_unless_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".prettierignore"), "Archive/\n").unwrap();
        let text = "# A\n* x\n";
        assert_eq!(formatted(dir.path(), text, None, &[]).unwrap(), "# A\n\n- x\n");
        assert_eq!(formatted(dir.path(), text, Some("Notes/A.md"), &[]).unwrap(), "# A\n\n- x\n");
        assert_eq!(formatted(dir.path(), text, Some("Archive/A.md"), &[]).unwrap(), text);
        assert_eq!(formatted(dir.path(), text, Some(".github/A.md"), &[]).unwrap(), text);
        assert_eq!(formatted(dir.path(), text, Some("Old/A.md"), &["Old".into()]).unwrap(), text);
        // What fmt writes is the same.
        fs::write(dir.path().join("A.md"), text).unwrap();
        let options = Options { check: false, diff: false, mode: report::Mode::Plain };
        run(dir.path(), &[PathBuf::from("A.md")], &[], &options, |_| {}).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("A.md")).unwrap(), formatted(dir.path(), text, None, &[]).unwrap());
    }

    #[test]
    fn check_prints_a_json_line_at_the_first_line_that_changes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("A.md"), "# A\n\ntext\n* x\n").unwrap();
        fs::write(dir.path().join("B.md"), "# B\n").unwrap();
        let options = Options { check: true, diff: false, mode: report::Mode::Json };
        let mut lines = Vec::new();
        let outcome = run(dir.path(), &[], &[], &options, |l| lines.push(l)).unwrap();
        assert_eq!((outcome.changed, outcome.checked), (1, 2));
        assert_eq!(
            lines,
            [
                r#"{"tool":"fmt","level":"error","path":"A.md","line":4,"column":null,"code":"fmt","message":"not formatted; run daftar-cli fmt","fixable":true}"#
            ]
        );
    }
}
