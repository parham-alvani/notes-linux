//! `lint`: the vault's notes against its `.markdownlint.jsonc`, as
//! `markdownlint-cli2` checks them, with no Node: `daftar_core::MarkdownLint`.
//! Prints `path:line[:column] rule/name description [detail]` for each
//! problem, or a GitHub Actions annotation in a workflow; `--fix` mends what
//! can be mended first, through `VaultEdits`.

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
pub fn notes(root: &Path, paths: &[PathBuf], ignore: &[String]) -> Vec<String> {
    notes_but(root, paths, ignore, ".markdownlintignore")
}

/// The notes of [paths], else the vault, but those [ignore_file] and
/// [ignore] name (a folder or a file each, `**` patterns taken as their
/// folder).
pub fn notes_but(root: &Path, paths: &[PathBuf], ignore: &[String], ignore_file: &str) -> Vec<String> {
    let mut ignored: Vec<String> = ignore.iter().map(|i| i.trim_matches('/').to_string()).collect();
    if let Ok(text) = fs::read_to_string(root.join(ignore_file)) {
        ignored.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split('*').next().unwrap_or(l).trim_matches('/').to_string()),
        );
    }
    let left_out = |relative: &str| {
        relative.split('/').any(|part| part.starts_with('.') || part == "node_modules")
            || ignored.iter().any(|i| relative == i || relative.starts_with(&format!("{i}/")))
    };
    let starts: Vec<PathBuf> = if paths.is_empty() { vec![root.to_path_buf()] } else { paths.to_vec() };
    let mut out = Vec::new();
    for start in starts {
        let start = if start.is_absolute() { start } else { std::env::current_dir().map(|d| d.join(&start)).unwrap_or(start) };
        for entry in WalkDir::new(&start).follow_links(false).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() || path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !left_out(&relative) {
                out.push(relative);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn run(
    root: &Path,
    paths: &[PathBuf],
    ignore: &[String],
    fix: bool,
    github: bool,
    mut print: impl FnMut(String),
) -> Result<Outcome, String> {
    let config = config(root)?;
    let notes = notes(root, paths, ignore);
    let mut fixed = 0;
    if fix {
        for (path, outcome) in notes.iter().zip(VaultEdits::apply_all(&notes, root, |text| MarkdownLint::fix(text, &config))) {
            match outcome {
                Ok(true) => fixed += 1,
                Ok(false) => {}
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
            if github {
                let title = format!("{}/{}", p.rule, p.name());
                print(report::annotation("error", path, p.line, p.column, &title, &format!("{}{detail}", p.description())));
                continue;
            }
            let column = p.column.map(|c| format!(":{c}")).unwrap_or_default();
            print(format!("{path}:{}{column} {}/{} {}{detail}", p.line, p.rule, p.name(), p.description()));
        }
    }
    Ok(Outcome { problems, notes: with, fixed, checked: notes.len() })
}
