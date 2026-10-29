//! `check`: what a vault's notes break that `lint` and `fmt` cannot see,
//! with Daftar's own link resolution. Errors are what is broken now: an
//! embed of a file that is not there, a link that a code span keeps from
//! working, a callout that falls apart, an H1 where the vault wants none.
//! Warnings are a backlog: a link to a note not written yet, a bare link
//! whose name more than one note has, a fence in a fence, a picture no note
//! shows. Prints `path:line`, a tab, `error` or `warning`, a tab, the kind
//! and what it found.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use daftar_core::Vault;
use daftar_core::vault::LOGO_NAMES;
use regex::Regex;

pub struct Options {
    /// Fail on warnings too.
    pub strict: bool,
    /// Folders whose notes must not have an H1: their title is the file's.
    pub no_h1: Vec<String>,
    /// Callout types beyond the standard ones.
    pub callouts: Vec<String>,
}

pub struct Outcome {
    pub errors: usize,
    pub warnings: usize,
    pub notes: usize,
}

/// The callout types Obsidian and Daftar's reader know.
const CALLOUTS: [&str; 27] = [
    "note",
    "warning",
    "important",
    "info",
    "tip",
    "quote",
    "summary",
    "example",
    "danger",
    "success",
    "todo",
    "caution",
    "bug",
    "done",
    "abstract",
    "question",
    "failure",
    "error",
    "hint",
    "check",
    "attention",
    "cite",
    "tldr",
    "faq",
    "help",
    "missing",
    "fail",
];

const PICTURES: [&str; 12] = ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "avif", "heic", "tif", "tiff", "ico"];

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)(`{3,})(.*)$").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\[([^\]]+)\]\]$").unwrap());
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>\s*\[!([^\]]*)\]").unwrap());
static STRAY_CARET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>\s*\[![^\]]*\][+-]?\s+>").unwrap());
static GLUED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*>\s*\S").unwrap());

/// The lines with fenced code blanked, by CommonMark's rule for closing a
/// fence (a run as long and no info string), and each block with the line
/// it opens on.
fn split_code(text: &str) -> (Vec<String>, Vec<(usize, Vec<String>)>) {
    let (mut out, mut blocks, mut body) = (Vec::new(), Vec::new(), Vec::new());
    let mut open: Option<(usize, usize)> = None;
    for (number, line) in text.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        if let Some(m) = FENCE.captures(line) {
            match open {
                None => {
                    open = Some((m[2].len(), number));
                    body = Vec::new();
                }
                Some((marker, start)) => {
                    if m[2].len() >= marker && m[3].trim().is_empty() {
                        blocks.push((start, std::mem::take(&mut body)));
                        open = None;
                    } else {
                        body.push(line.to_string());
                    }
                }
            }
            out.push(String::new());
            continue;
        }
        if open.is_some() {
            body.push(line.to_string());
            out.push(String::new());
        } else {
            out.push(line.to_string());
        }
    }
    if let Some((_, start)) = open {
        blocks.push((start, body));
    }
    (out, blocks)
}

/// The reader's rule for a heading a link names: compared without marks,
/// spaces made one, case folded.
fn plain(text: &str) -> String {
    let kept: String = text.chars().filter(|c| !"*_`=~[]#|^:%".contains(*c)).collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Whether the note at [to] has the heading or the `^block` [anchor] names.
fn has_anchor(index: &daftar_core::VaultIndex, root: &Path, to: &str, anchor: &str, texts: &mut HashMap<String, String>) -> bool {
    let Some(note) = index.notes.get(to) else { return true };
    if let Some(id) = anchor.strip_prefix('^') {
        let text = texts.entry(to.to_string()).or_insert_with(|| fs::read_to_string(root.join(to)).unwrap_or_default());
        let mark = format!("^{id}");
        return text.lines().any(|l| l.trim_end() == mark || l.trim_end().ends_with(&format!(" {mark}")));
    }
    let wanted = plain(anchor);
    note.parsed.headings.iter().any(|h| plain(&h.text) == wanted)
}

/// Guides and templates show Markdown in code on purpose.
fn shows_markdown(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "CLAUDE.md" || name.starts_with("_TEMPLATE") || path.starts_with(".claude/")
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

pub fn run(root: &Path, hidden: impl Fn(&str) -> bool, options: &Options, mut print: impl FnMut(String)) -> Outcome {
    let index = Vault::index(root, None);
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for path in index.notes.keys() {
        *by_name.entry(stem(path)).or_default() += 1;
    }
    let callouts: HashSet<String> =
        CALLOUTS.iter().map(|c| c.to_string()).chain(options.callouts.iter().map(|c| c.to_lowercase())).collect();
    let mut found: BTreeSet<(String, usize, bool, &'static str, String)> = BTreeSet::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut texts: Vec<String> = Vec::new();
    let mut blocks_of: HashMap<String, String> = HashMap::new();
    let mut paths: Vec<&String> = index.notes.keys().filter(|p| !hidden(p)).collect();
    paths.sort();
    for path in &paths {
        let note = &index.notes[*path];
        for link in &note.parsed.links {
            if link.target.is_empty() || link.target.contains("://") {
                continue;
            }
            let line = link.line + 1;
            match index.resolve(&link.target, path) {
                Some(to) => {
                    if let Some(anchor) = link.anchor.as_deref().filter(|a| !a.is_empty())
                        && !has_anchor(&index, root, &to, anchor, &mut blocks_of)
                    {
                        let mark = if link.embed { "!" } else { "" };
                        let what = if anchor.starts_with('^') { "block" } else { "heading" };
                        found.insert((
                            path.to_string(),
                            line,
                            false,
                            "missing-anchor",
                            format!("{mark}[[{}#{anchor}]]: {to} has no such {what}", link.target),
                        ));
                    }
                    if !link.target.contains('/') && by_name.get(link.target.trim_end_matches(".md")).is_some_and(|&n| n > 1) {
                        let n = by_name[link.target.trim_end_matches(".md")];
                        found.insert((
                            path.to_string(),
                            line,
                            false,
                            "ambiguous-link",
                            format!("[[{}]] names {n} notes; give its folder", link.target),
                        ));
                    }
                    used.insert(to);
                }
                None if link.embed => {
                    found.insert((path.to_string(), line, true, "missing-embed", format!("![[{}]] is not in the vault", link.target)));
                }
                None => {
                    found.insert((path.to_string(), line, false, "dangling-link", format!("[[{}]] goes to no note", link.target)));
                }
            }
        }
        let Ok(raw) = fs::read_to_string(root.join(path.as_str())) else { continue };
        let (outside, blocks) = split_code(&raw);
        if !shows_markdown(path) {
            for (i, line) in outside.iter().enumerate() {
                for span in INLINE_CODE.find_iter(line) {
                    let inner = span.as_str()[1..span.as_str().len() - 1].trim();
                    let Some(m) = WIKILINK.captures(inner) else { continue };
                    let target = m[1].split('|').next().unwrap_or("").split('#').next().unwrap_or("").trim();
                    if target.contains('/') && index.resolve(target, path).is_some() {
                        found.insert((
                            path.to_string(),
                            i + 1,
                            true,
                            "link-in-code",
                            format!("`[[{target}]]` shows as code and never links"),
                        ));
                    }
                }
            }
            for (start, body) in &blocks {
                if body.iter().any(|l| l.starts_with("```")) {
                    found.insert((
                        path.to_string(),
                        *start,
                        false,
                        "nested-fence",
                        "a fence in a fence: the outer block swallows the text between".into(),
                    ));
                }
            }
        }
        if options.no_h1.iter().any(|f| path.starts_with(&format!("{}/", f.trim_matches('/'))))
            && stem(path) != "CLAUDE"
            && let Some(i) = outside.iter().position(|l| l.starts_with("# "))
        {
            found.insert((path.to_string(), i + 1, true, "h1", "an H1 in a note whose title is its name; start at ##".into()));
        }
        for (i, line) in outside.iter().enumerate() {
            let Some(m) = CALLOUT.captures(line) else { continue };
            let kind = m[1].trim_end_matches(['+', '-']);
            if !callouts.contains(&kind.to_lowercase()) {
                found.insert((path.to_string(), i + 1, true, "callout-type", format!("[!{kind}] is not a callout type")));
            }
            if STRAY_CARET.is_match(line) {
                found.insert((path.to_string(), i + 1, true, "callout-title", "a `>` left in the callout's title".into()));
            }
            if outside.get(i + 1).is_some_and(|next| GLUED.is_match(next)) {
                found.insert((
                    path.to_string(),
                    i + 1,
                    true,
                    "callout-body",
                    "the body is joined to the title; put a bare `>` line between".into(),
                ));
            }
        }
        texts.push(raw);
    }
    // A canvas or a base can show a picture too.
    for file in index.files.iter().filter(|f| f.ends_with(".canvas") || f.ends_with(".base")) {
        if let Ok(text) = fs::read_to_string(root.join(file)) {
            texts.push(text);
        }
    }
    for file in &index.files {
        // A folder's logo is its icon in the tree.
        if LOGO_NAMES.contains(&file.rsplit('/').next().unwrap_or(file)) {
            continue;
        }
        let is_picture = file.rsplit_once('.').is_some_and(|(_, ext)| PICTURES.contains(&ext.to_lowercase().as_str()));
        if !is_picture || used.contains(file) || hidden(file) {
            continue;
        }
        let name = file.rsplit('/').next().unwrap_or(file);
        if !texts.iter().any(|t| t.contains(name)) {
            found.insert((file.clone(), 0, false, "unused-picture", "no note shows it".into()));
        }
    }
    let (mut errors, mut warnings) = (0, 0);
    for (path, line, error, kind, detail) in &found {
        if *error {
            errors += 1;
        } else {
            warnings += 1;
        }
        let at = if *line > 0 { format!("{path}:{line}") } else { path.clone() };
        print(format!("{at}\t{}\t{kind}\t{detail}", if *error { "error" } else { "warning" }));
    }
    Outcome { errors, warnings, notes: paths.len() }
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

    fn findings(dir: &Path, options: &Options) -> Vec<String> {
        let mut lines = Vec::new();
        run(dir, |_| false, options, |l| lines.push(l));
        lines
    }

    fn options() -> Options {
        Options { strict: false, no_h1: vec!["Learning".into()], callouts: vec!["read".into()] }
    }

    #[test]
    fn finds_what_is_broken() {
        let dir = vault(&[
            ("D.md", "[[C#No such]] [[C#^nope]] [[Learning/B#Title]] [[C#^here]]\n"),
            ("A.md", "See [[Nowhere]] and ![[gone.png]].\n\n> [!read] Book\n> glued\n\n> [!nope]\n"),
            ("Learning/B.md", "# Title\n\nUse `[[Learning/B]]` here.\n"),
            ("uploads/used.png", ""),
            ("uploads/lost.png", ""),
            ("Company/logo.svg", ""),
            ("C.md", "![[used.png]] ^here\n\n````md\n```\ninner\n```\n````\n"),
        ]);
        let lines = findings(dir.path(), &options());
        let kinds: Vec<&str> = lines.iter().map(|l| l.split('\t').nth(2).unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "dangling-link",
                "missing-embed",
                "callout-body",
                "callout-type",
                "nested-fence",
                "missing-anchor",
                "missing-anchor",
                "h1",
                "link-in-code",
                "unused-picture"
            ],
            "{lines:#?}"
        );
        assert!(lines[0].starts_with("A.md:1\twarning\t"));
        assert!(lines.last().unwrap().starts_with("uploads/lost.png\twarning"));
    }

    #[test]
    fn a_clean_vault_has_nothing() {
        let dir = vault(&[("A.md", "See [[B]].\n\n> [!note] Title\n>\n> Body\n"), ("B.md", "![](uploads/x.png)\n"), ("uploads/x.png", "")]);
        assert!(findings(dir.path(), &options()).is_empty());
    }
}
