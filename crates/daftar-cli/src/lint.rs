//! `lint`: the vault's notes against its `.markdownlint.jsonc`, as
//! `markdownlint-cli2` checks them, with no Node: `daftar_core::MarkdownLint`.
//! Prints `path:line[:column] rule/name description [detail]` for each
//! problem, a GitHub Actions annotation in a workflow, or with `--json` a
//! JSON line (`report.rs`); `--fix` mends what can be mended first, through
//! `VaultEdits`.

use std::fs;
use std::path::{Path, PathBuf};

use daftar_core::{LintConfig, MarkdownLint, VaultEdits};
use walkdir::WalkDir;

use crate::report;

/// What a run found: the problems left, and the notes they are in.
pub struct Outcome {
    pub problems: usize,
    pub notes: usize,
    pub fixed: usize,
    pub checked: usize,
}

/// The config the vault has, `.markdownlint.jsonc` or `.markdownlint.json`;
/// every rule with its defaults when it has none.
pub fn config(root: &Path) -> Result<LintConfig, String> {
    for name in [".markdownlint.jsonc", ".markdownlint.json"] {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            return LintConfig::parse(&text).map_err(|e| format!("{name}: {e}"));
        }
    }
    Ok(LintConfig::all())
}

/// The notes to lint: [paths] (files or folders), else the whole vault;
/// hidden folders, `node_modules`, the lines of `.markdownlintignore` and
/// [ignore] are left out.
pub fn notes(root: &Path, paths: &[PathBuf], ignore: &[String]) -> Result<Vec<String>, String> {
    notes_but(root, paths, ignore, Some(".markdownlintignore"))
}

/// Whether a file, by its path from the vault's folder, is left out: in a
/// hidden folder or `node_modules`, or named by [ignore] or a line of
/// [ignore_file] (a folder or a file each, `**` patterns taken as their
/// folder).
pub fn left_out(root: &Path, ignore: &[String], ignore_file: Option<&str>) -> impl Fn(&str) -> bool + use<> {
    let mut ignored: Vec<String> = ignore.iter().map(|i| i.trim_matches('/').to_string()).collect();
    if let Some(text) = ignore_file.and_then(|name| fs::read_to_string(root.join(name)).ok()) {
        ignored.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split('*').next().unwrap_or(l).trim_matches('/').to_string()),
        );
    }
    move |relative: &str| {
        relative.split('/').any(|part| part.starts_with('.') || part == "node_modules")
            || ignored.iter().any(|i| relative == i || relative.starts_with(&format!("{i}/")))
    }
}

/// [path] as given on the command line: from the current folder when it is
/// there and in the vault, else from the vault's folder, so that an app can
/// name a note by its path in the vault from anywhere.
pub fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let here = std::env::current_dir().map_or_else(|_| path.to_path_buf(), |d| d.join(path));
    let in_vault = |p: &Path| p.canonicalize().ok().zip(root.canonicalize().ok()).is_some_and(|(p, r)| p.starts_with(r));
    if here.exists() && in_vault(&here) { here } else { root.join(path) }
}

/// The notes of [paths], else the vault, but those [ignore_file] and
/// [ignore] name: see [left_out]. A path that is not there is an error.
pub fn notes_but(root: &Path, paths: &[PathBuf], ignore: &[String], ignore_file: Option<&str>) -> Result<Vec<String>, String> {
    let left_out = left_out(root, ignore, ignore_file);
    let starts: Vec<PathBuf> = if paths.is_empty() { vec![root.to_path_buf()] } else { paths.iter().map(|p| resolve(root, p)).collect() };
    // The vault's folder as the paths under it are written, which may be
    // through a symlink (/tmp on macOS is /private/tmp).
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for start in starts {
        if !start.exists() {
            return Err(format!("{} is not there", start.display()));
        }
        for entry in WalkDir::new(&start).follow_links(false).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() || path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let relative = match path.strip_prefix(root) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => match path.canonicalize().ok().and_then(|p| p.strip_prefix(&canonical).ok().map(Path::to_path_buf)) {
                    Some(relative) => relative,
                    None => continue,
                },
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !left_out(&relative) {
                out.push(relative);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn run(
    root: &Path,
    paths: &[PathBuf],
    ignore: &[String],
    fix: bool,
    mode: report::Mode,
    mut print: impl FnMut(String),
) -> Result<Outcome, String> {
    let config = config(root)?;
    let notes = notes(root, paths, ignore)?;
    let mut fixed = 0;
    if fix {
        for (path, outcome) in notes.iter().zip(VaultEdits::apply_all(&notes, root, |text| MarkdownLint::fix(text, &config))) {
            match outcome {
                Ok(true) => fixed += 1,
                Ok(false) => {}
                // Only findings go to stdout as JSON.
                Err(failure) if mode == report::Mode::Json => eprintln!("daftar-cli: {path}: not fixed: {failure}"),
                Err(failure) => print(format!("{path}: not fixed: {failure}")),
            }
        }
    }
    let (mut problems, mut with) = (0, 0);
    for path in &notes {
        let Ok(bytes) = fs::read(root.join(path)) else { continue };
        let found = MarkdownLint::lint_bytes(&bytes, &config);
        if !found.is_empty() {
            with += 1;
        }
        for p in &found {
            problems += 1;
            let detail = if p.detail.is_empty() { String::new() } else { format!(" {}", p.detail) };
            let code = format!("{}/{}", p.rule, p.name());
            let message = format!("{}{detail}", p.description());
            let finding = report::Finding {
                tool: "lint",
                level: "error",
                path,
                line: p.line,
                column: p.column,
                code: &code,
                message: &message,
                fixable: p.fixable(),
            };
            if let Some(line) = finding.printed(mode) {
                print(line);
                continue;
            }
            let column = p.column.map(|c| format!(":{c}")).unwrap_or_default();
            print(format!("{path}:{}{column} {}/{} {}{detail}", p.line, p.rule, p.name(), p.description()));
        }
    }
    Ok(Outcome { problems, notes: with, fixed, checked: notes.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn a_problem_is_a_json_line_that_says_whether_fix_mends_it() {
        let dir = vault(&[("Notes/A.md", "# One\n\n# Two \n"), ("Notes/B.md", "# Fine\n")]);
        let mut lines = Vec::new();
        // A path from the vault's folder, whatever the current folder is.
        let outcome = run(dir.path(), &[PathBuf::from("Notes/A.md")], &[], false, report::Mode::Json, |l| lines.push(l)).unwrap();
        assert_eq!((outcome.problems, outcome.checked), (2, 1));
        let found: Vec<serde_json::Value> = lines.iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(found[0]["code"], "MD025/single-title");
        assert_eq!(found[0]["fixable"], false);
        assert_eq!(found[1]["code"], "MD009/no-trailing-spaces");
        assert_eq!(
            found[1],
            serde_json::json!({"tool": "lint", "level": "error", "path": "Notes/A.md", "line": 3, "column": 6,
                "code": "MD009/no-trailing-spaces", "message": "Trailing spaces [Expected: 0 or 2; Actual: 1]", "fixable": true})
        );
        // --fix mends that one note.
        let outcome = run(dir.path(), &[PathBuf::from("Notes/A.md")], &[], true, report::Mode::Json, |_| {}).unwrap();
        assert_eq!((outcome.fixed, outcome.problems), (1, 1));
        assert_eq!(fs::read_to_string(dir.path().join("Notes/A.md")).unwrap(), "# One\n\n# Two\n");
    }

    #[test]
    fn a_path_that_is_not_there_is_an_error() {
        let dir = vault(&[("A.md", "# A\n")]);
        assert!(notes(dir.path(), &[PathBuf::from("Nowhere/B.md")], &[]).unwrap_err().contains("is not there"));
        assert_eq!(notes(dir.path(), &[dir.path().join("A.md")], &[]).unwrap(), ["A.md"]);
    }
}
