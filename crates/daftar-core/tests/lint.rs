//! The lint against markdownlint-cli2: each note in tests/lint breaks rules
//! on purpose, and expected.txt is what markdownlint-cli2 0.23 reported for
//! them with tests/lint/.markdownlint.jsonc (`npx markdownlint-cli2 "*.md"`),
//! as `file:line[:column] rule`. Lines and rules must match; columns are
//! markdownlint's choice within a line and are not compared.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use daftar_core::{LintConfig, MarkdownLint};

#[test]
fn the_lint_finds_what_markdownlint_finds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lint");
    let config = LintConfig::parse(&fs::read_to_string(dir.join(".markdownlint.jsonc")).unwrap()).unwrap();
    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in fs::read_to_string(dir.join("expected.txt")).unwrap().lines() {
        let (place, rule) = line.split_once(' ').unwrap();
        let mut parts = place.split(':');
        let (file, number) = (parts.next().unwrap(), parts.next().unwrap());
        expected.entry(file.to_string()).or_default().push(format!("{number} {rule}"));
    }
    let mut failures = Vec::new();
    let mut files: Vec<_> =
        fs::read_dir(&dir).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
    files.sort();
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(&path).unwrap();
        let mut found: Vec<String> = MarkdownLint::lint(&text, &config).iter().map(|p| format!("{} {}", p.line, p.rule)).collect();
        let mut want = expected.remove(&name).unwrap_or_default();
        found.sort();
        want.sort();
        if found != want {
            let extra: Vec<_> = found.iter().filter(|f| !want.contains(f)).collect();
            let missing: Vec<_> = want.iter().filter(|w| !found.contains(w)).collect();
            failures.push(format!("{name}: extra {extra:?}, missing {missing:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
