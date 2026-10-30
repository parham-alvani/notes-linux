//! `check`: what a vault's notes break that `lint` and `fmt` cannot see,
//! with Daftar's own link resolution. Errors are what is broken now: an
//! embed of a file that is not there, a link that a code span keeps from
//! working, a callout that falls apart, an H1 where the vault wants none.
//! Warnings are a backlog: a link to a note not written yet, a bare link
//! whose name more than one note has, a fence in a fence, a picture no note
//! shows, a block id written twice, and with `--urls` a web address that is
//! gone. Prints `path:line`, a tab, `error` or `warning`, a tab, the kind
//! and what it found; in GitHub Actions, an annotation for each. With
//! `--fix` it mends what has one answer: a bare link whose name more than
//! one note has gets the shortest path that names the note it goes to now,
//! and keeps the text it showed.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use chrono::NaiveDate;
use daftar_core::vault::LOGO_NAMES;
use daftar_core::web_fetch::Answer;
use daftar_core::{Vault, VaultEdits};
use percent_encoding::percent_decode_str;
use regex::Regex;

use crate::report;

pub struct Options {
    /// Fail on warnings too.
    pub strict: bool,
    /// Folders whose notes must not have an H1: their title is the file's.
    pub no_h1: Vec<String>,
    /// Callout types beyond the standard ones.
    pub callouts: Vec<String>,
    /// Asks every web address the notes link to whether it is still there.
    pub urls: bool,
    /// With [urls], lists the addresses nothing answered at too: a name no
    /// server has, a refused or broken connection, a bad certificate.
    pub unreachable: bool,
    /// Lets [urls] ask GitHub's API whether a repository is public: a
    /// 404 in a public one is a page gone, in another it may be private.
    pub github_token: Option<String>,
    /// Web addresses not to ask: a host, which takes its subdomains, or the
    /// start of an address.
    pub skip_urls: Vec<String>,
    /// Prints GitHub Actions annotations.
    pub github: bool,
    /// Mends what has one answer, and prints each change instead.
    pub fix: bool,
    /// With [fix], prints the changes and writes nothing.
    pub dry_run: bool,
}

pub struct Outcome {
    pub fixed: usize,
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
static FRAGMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[[^\]\n]*\]\(<?#([^)>\s]*)>?\)").unwrap());
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)\^([A-Za-z0-9-]+)\s*$").unwrap());
static TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*[-*+] \[.\]").unwrap());
static TASK_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(📅|⏳|🛫|✅|❌|➕)\x{FE0F}?[ \t]*(\S*)").unwrap());
static TYPO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:(?:htps?|htt|htttps?|hhttps?|ttps?)://|https?:/[^/\s]|https?//|https?:\\)").unwrap());
static SCHEMELESS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(<?(www\.[^)>\s]+)>?\)").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'`|\[\]]+"#).unwrap());

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

/// The anchors as written of the links to [target] on [line] that the
/// parser read as [parsed]: the parser leaves a code span out of a link's
/// anchor, and the reader does not.
fn written_anchors(line: &str, target: &str, parsed: &str) -> Vec<String> {
    let squeeze = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let open = format!("[[{target}#");
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(end) = after.find("]]") else { break };
        let written = after[..end].split('|').next().unwrap_or("");
        if squeeze(&INLINE_CODE.replace_all(written, "")) == squeeze(parsed) {
            out.push(written.to_string());
        }
        rest = &after[end..];
    }
    out
}

/// Whether the note at [to] has the heading or the `^block` [anchor] names.
fn has_anchor(index: &daftar_core::VaultIndex, root: &Path, to: &str, anchor: &str, texts: &mut HashMap<String, String>) -> bool {
    let Some(note) = index.notes.get(to) else { return true };
    if let Some(id) = anchor.strip_prefix('^') {
        let text = texts.entry(to.to_string()).or_insert_with(|| fs::read_to_string(root.join(to)).unwrap_or_default());
        let mark = format!("^{id}");
        return text.lines().any(|l| l.trim_end() == mark || l.trim_end().ends_with(&format!(" {mark}")));
    }
    let (wanted, slugged) = (plain(anchor), anchor.to_lowercase());
    note.parsed.headings.iter().any(|h| plain(&h.text) == wanted || slug(&h.text) == slugged)
}

/// The heading of the note at [to] that a broken [anchor] most likely
/// meant: one that starts with it or holds it (a heading renamed longer),
/// else one a few letters off (a typo). Compared as `plain` or `slug` do.
fn closest(index: &daftar_core::VaultIndex, to: &str, anchor: &str) -> Option<String> {
    let note = index.notes.get(to)?;
    let (wanted, slugged) = (plain(anchor), anchor.to_lowercase());
    if wanted.is_empty() || anchor.starts_with('^') {
        return None;
    }
    let headings = &note.parsed.headings;
    let holds = |h: &&daftar_core::Heading| {
        let (p, s) = (plain(&h.text), slug(&h.text));
        p.starts_with(&wanted) || s.starts_with(&slugged) || p.contains(&wanted)
    };
    if let Some(found) = headings.iter().filter(holds).min_by_key(|h| h.text.len()) {
        return Some(found.text.clone());
    }
    let most = (wanted.chars().count() / 4).clamp(1, 3);
    headings
        .iter()
        .map(|h| (distance(&plain(&h.text), &wanted).min(distance(&slug(&h.text), &slugged)), h))
        .filter(|(d, _)| *d <= most)
        .min_by_key(|(d, _)| *d)
        .map(|(_, h)| h.text.clone())
}

/// How many letters to add, take away or change to make [a] into [b].
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(x != *y));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// `; did you mean “X”?` for [closest]'s answer, or nothing.
fn meant(heading: Option<String>) -> String {
    heading.map(|h| format!("; did you mean “{h}”?")).unwrap_or_default()
}

/// A heading's anchor as GitHub writes it, `#my-heading`: lower case, the
/// marks left out, a dash for each space.
fn slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The web addresses on [line], without the marks a sentence or a link
/// puts after one: a stop, a closing bracket the address did not open.
fn urls(line: &str) -> Vec<String> {
    URL.find_iter(line)
        .map(|m| {
            let mut url = m.as_str();
            // A `)` the address did not open closes the link it is in.
            let mut depth = 0;
            for (i, c) in url.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' if depth == 0 => {
                        url = &url[..i];
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            while let Some('.' | ',' | ';' | ':' | '!' | '?' | '*' | '_' | '~' | '\'' | '"' | '>') = url.chars().last() {
                url = &url[..url.len() - 1];
            }
            url.to_string()
        })
        .collect()
}

/// What is wrong with how [url] is written, if anything: two addresses run
/// together, a host or port no server can have, a broken `%` escape.
fn malformed(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    if let Some(at) = rest.find("http://").or_else(|| rest.find("https://")) {
        return Some(format!("two addresses run together at `{}`", &rest[at..]));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if authority.starts_with('[') {
        (authority, None)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Some("it has no host".into());
    }
    if let Some(port) = port
        && !port.is_empty()
        && port.parse::<u16>().is_err()
    {
        return Some(format!("`{port}` is not a port"));
    }
    let bad_label = |label: &str| label.is_empty() || label.starts_with('-') || label.ends_with('-');
    if !host.starts_with('[')
        && (host.split('.').any(bad_label) || host.chars().any(|c| !(c.is_alphanumeric() || c == '-' || c == '.' || c == '_')))
    {
        return Some(format!("`{host}` is not a host name"));
    }
    let bytes = url.as_bytes();
    if let Some(at) = (0..bytes.len()).find(|&i| {
        bytes[i] == b'%' && !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    }) {
        return Some(format!("`{}` is not a % escape", &url[at..(at + 3).min(url.len())]));
    }
    None
}

/// An address only a machine on the same network reaches: CI never will.
fn is_local(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    let octets: Vec<u8> = host.split('.').filter_map(|o| o.parse().ok()).collect();
    !host.contains('.')
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".localhost")
        || (octets.len() == 4
            && (octets[0] == 10
                || octets[0] == 127
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 172 && (16..32).contains(&octets[1]))))
}

/// The `owner/repo` of a github.com address in a repository.
fn github_repo(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("https://www.github.com/"))?;
    let mut parts = rest.split(['/', '?', '#']).filter(|p| !p.is_empty());
    let (owner, repo) = (parts.next()?, parts.next()?);
    let not_owners = ["orgs", "settings", "features", "topics", "marketplace", "sponsors", "apps", "users", "search", "notifications"];
    (!not_owners.contains(&owner)).then(|| format!("{owner}/{}", repo.trim_end_matches(".git")))
}

/// Whether [skip] names [url]: a host, with its subdomains, or a start.
fn skipped(url: &str, skip: &[String]) -> bool {
    let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#', ':']).next().unwrap_or("");
    skip.iter().any(|s| url.starts_with(s.as_str()) || host == s || host.ends_with(&format!(".{s}")))
}

/// Guides and templates show Markdown in code on purpose.
fn shows_markdown(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == "CLAUDE.md" || name.starts_with("_TEMPLATE") || path.starts_with(".claude/")
}

/// The shortest path, from the end of [to], that names [to] alone and that
/// [source] resolves to it: `Folder/Note` for `Folder/Note.md`.
fn unique_target(index: &daftar_core::VaultIndex, to: &str, source: &str) -> Option<String> {
    let bare = to.strip_suffix(".md").unwrap_or(to);
    let segments: Vec<&str> = bare.split('/').collect();
    (2..=segments.len()).map(|k| segments[segments.len() - k..].join("/")).find(|target| {
        let names = |p: &String| {
            let p = p.strip_suffix(".md").unwrap_or(p).to_lowercase();
            let t = target.to_lowercase();
            p == t || p.ends_with(&format!("/{t}"))
        };
        index.notes.keys().filter(|p| names(p)).count() == 1 && index.resolve(target, source).as_deref() == Some(to)
    })
}

/// [line] with each wikilink to [target] made one to [new], showing what it
/// showed: `[[Plan]]` is `[[Work/Plan|Plan]]`, `[[Plan#Goals]]` is
/// `[[Work/Plan#Goals|Plan > Goals]]`, an alias stays, an embed gets none.
/// In a table row the `|` is escaped, as Obsidian writes it there.
fn retarget(line: &str, target: &str, new: &str) -> String {
    let bar = if line.trim_start().starts_with('|') { "\\|" } else { "|" };
    let open = format!("[[{target}");
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        let Some(end) = after.find("]]") else { break };
        let tail = &after[..end];
        if !tail.is_empty() && !tail.starts_with(['#', '|', '\\']) {
            out.push_str(&rest[..at + open.len()]);
            rest = after;
            continue;
        }
        let embed = rest[..at].ends_with('!');
        out.push_str(&rest[..at]);
        let aliased = tail.contains('|');
        let shown = match tail.strip_prefix('#') {
            _ if embed || aliased => String::new(),
            Some(anchor) => format!("{bar}{target} > {anchor}"),
            None => format!("{bar}{target}"),
        };
        out.push_str(&format!("[[{new}{tail}{shown}]]"));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
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
        CALLOUTS.iter().map(ToString::to_string).chain(options.callouts.iter().map(|c| c.to_lowercase())).collect();
    let mut found: BTreeSet<(String, usize, bool, &'static str, String)> = BTreeSet::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut texts: Vec<String> = Vec::new();
    let mut blocks_of: HashMap<String, String> = HashMap::new();
    let mut web: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    let mut fixes: BTreeSet<(String, usize, String, String)> = BTreeSet::new();
    let mut paths: Vec<&String> = index.notes.keys().filter(|p| !hidden(p)).collect();
    paths.sort();
    for path in &paths {
        let note = &index.notes[*path];
        let source = fs::read_to_string(root.join(path.as_str())).unwrap_or_default();
        let source_lines: Vec<&str> = source.lines().collect();
        for link in &note.parsed.links {
            // `[[#Heading]]` is a heading of this note.
            if (link.target.is_empty() && link.anchor.as_deref().is_none_or(str::is_empty)) || link.target.contains("://") {
                continue;
            }
            let line = link.line + 1;
            match index.resolve(&link.target, path) {
                Some(to) => {
                    if let Some(anchor) = link.anchor.as_deref().filter(|a| !a.is_empty())
                        && !has_anchor(&index, root, &to, anchor, &mut blocks_of)
                        && !written_anchors(source_lines.get(link.line).copied().unwrap_or(""), &link.target, anchor)
                            .iter()
                            .any(|a| has_anchor(&index, root, &to, a, &mut blocks_of))
                    {
                        let mark = if link.embed { "!" } else { "" };
                        let what = if anchor.starts_with('^') { "block" } else { "heading" };
                        found.insert((
                            (*path).clone(),
                            line,
                            false,
                            "missing-anchor",
                            format!("{mark}[[{}#{anchor}]]: {to} has no such {what}{}", link.target, meant(closest(&index, &to, anchor))),
                        ));
                    }
                    if !link.target.contains('/') && by_name.get(link.target.trim_end_matches(".md")).is_some_and(|&n| n > 1) {
                        let n = by_name[link.target.trim_end_matches(".md")];
                        if options.fix
                            && let Some(new) = unique_target(&index, &to, path)
                        {
                            fixes.insert(((*path).clone(), link.line, link.target.clone(), new));
                        } else {
                            found.insert((
                                (*path).clone(),
                                line,
                                false,
                                "ambiguous-link",
                                format!("[[{}]] names {n} notes; give its folder", link.target),
                            ));
                        }
                    }
                    used.insert(to);
                }
                None if link.embed => {
                    found.insert(((*path).clone(), line, true, "missing-embed", format!("![[{}]] is not in the vault", link.target)));
                }
                None => {
                    found.insert(((*path).clone(), line, false, "dangling-link", format!("[[{}]] goes to no note", link.target)));
                }
            }
        }
        let Ok(raw) = fs::read_to_string(root.join(path.as_str())) else { continue };
        let (outside, blocks) = split_code(&raw);
        let mut lines = raw.lines();
        if lines.next().is_some_and(|l| l.trim_end() == "---") && !lines.any(|l| matches!(l.trim_end(), "---" | "...")) {
            found.insert((
                (*path).clone(),
                1,
                true,
                "front-matter",
                "the front matter is never closed; the whole note is read as it".into(),
            ));
        }
        let mut block_ids: HashMap<String, usize> = HashMap::new();
        for (i, line) in outside.iter().enumerate() {
            let code = INLINE_CODE.replace_all(line, "");
            for m in FRAGMENT.captures_iter(&code) {
                let anchor = percent_decode_str(&m[2]).decode_utf8_lossy().into_owned();
                if !anchor.is_empty() && !has_anchor(&index, root, path, &anchor, &mut blocks_of) {
                    let what = if anchor.starts_with('^') { "block" } else { "heading" };
                    found.insert((
                        (*path).clone(),
                        i + 1,
                        false,
                        "missing-anchor",
                        format!("{}: this note has no such {what}{}", &m[0][m[1].len()..], meant(closest(&index, path, &anchor))),
                    ));
                }
            }
            if let Some(m) = BLOCK_ID.captures(&code) {
                if let Some(first) = block_ids.get(&m[1]) {
                    found.insert((
                        (*path).clone(),
                        i + 1,
                        false,
                        "duplicate-block",
                        format!("^{} is on line {first} too; a link goes to the first", &m[1]),
                    ));
                } else {
                    block_ids.insert(m[1].to_string(), i + 1);
                }
            }
            if TASK.is_match(&code) {
                for m in TASK_DATE.captures_iter(&code) {
                    // A word after the sign is the sentence's, not a mistaken day.
                    if m[2].starts_with(|c: char| c.is_ascii_digit())
                        && (m[2].len() != 10 || NaiveDate::parse_from_str(&m[2], "%Y-%m-%d").is_err())
                    {
                        found.insert((
                            (*path).clone(),
                            i + 1,
                            true,
                            "task-date",
                            format!("{} {} is not a day (YYYY-MM-DD); the task is read without it", &m[1], &m[2]),
                        ));
                    }
                }
            }
            for m in TYPO.find_iter(&code) {
                found.insert(((*path).clone(), i + 1, true, "bad-url", format!("`{}` is a mistyped scheme", m.as_str().trim_end())));
            }
            for m in SCHEMELESS.captures_iter(&code) {
                found.insert(((*path).clone(), i + 1, true, "bad-url", format!("{} has no https://, so it is read as a note", &m[1])));
            }
            for url in urls(&code) {
                if let Some(why) = malformed(&url) {
                    found.insert(((*path).clone(), i + 1, true, "bad-url", format!("{url}: {why}")));
                }
            }
            if options.urls {
                for url in urls(&code) {
                    if !is_local(&url) && !skipped(&url, &options.skip_urls) {
                        web.entry(url).or_default().push(((*path).clone(), i + 1));
                    }
                }
            }
        }
        if !shows_markdown(path) {
            for (i, line) in outside.iter().enumerate() {
                for span in INLINE_CODE.find_iter(line) {
                    let inner = span.as_str()[1..span.as_str().len() - 1].trim();
                    let Some(m) = WIKILINK.captures(inner) else { continue };
                    let target = m[1].split('|').next().unwrap_or("").split('#').next().unwrap_or("").trim();
                    if target.contains('/') && index.resolve(target, path).is_some() {
                        found.insert((
                            (*path).clone(),
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
                        (*path).clone(),
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
            found.insert(((*path).clone(), i + 1, true, "h1", "an H1 in a note whose title is its name; start at ##".into()));
        }
        for (i, line) in outside.iter().enumerate() {
            let Some(m) = CALLOUT.captures(line) else { continue };
            let kind = m[1].trim_end_matches(['+', '-']);
            if !callouts.contains(&kind.to_lowercase()) {
                found.insert(((*path).clone(), i + 1, true, "callout-type", format!("[!{kind}] is not a callout type")));
            }
            if STRAY_CARET.is_match(line) {
                found.insert(((*path).clone(), i + 1, true, "callout-title", "a `>` left in the callout's title".into()));
            }
            if outside.get(i + 1).is_some_and(|next| GLUED.is_match(next)) {
                found.insert((
                    (*path).clone(),
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
    if !web.is_empty() {
        let mut addresses: Vec<String> = web.keys().cloned().collect();
        addresses.sort();
        eprintln!("daftar-cli: asking {} web addresses", addresses.len());
        let agent = concat!("daftar-cli/", env!("CARGO_PKG_VERSION"));
        let answers = daftar_core::web_fetch::answers(&addresses, agent);
        let mut dead: Vec<(&String, String)> = Vec::new();
        let mut on_github: Vec<&String> = Vec::new();
        let mut unanswered = 0;
        for (url, answer) in addresses.iter().zip(answers) {
            match answer {
                // GitHub answers 404 for a private repository too.
                Answer::Status(404) if github_repo(url).is_some() => on_github.push(url),
                Answer::Status(status @ (404 | 410)) => dead.push((url, format!("answers {status}"))),
                // A server that wants a login, or turns robots away, or is
                // down for now, says nothing of whether the page is there.
                Answer::Status(_) => {}
                Answer::Unreachable(why) => {
                    unanswered += 1;
                    if options.unreachable {
                        dead.push((url, format!("does not answer: {why}")));
                    }
                }
            }
        }
        if let Some(token) = options.github_token.as_deref().filter(|t| !t.is_empty()) {
            let mut repos: Vec<String> = on_github.iter().filter_map(|u| github_repo(u)).collect();
            repos.sort();
            repos.dedup();
            let said: HashMap<String, Answer> =
                repos.iter().cloned().zip(daftar_core::web_fetch::github_repos(&repos, token, agent)).collect();
            for url in on_github {
                // The repository is public: the page in it is gone. One the
                // token cannot see may be gone, or private to others.
                if matches!(github_repo(url).and_then(|r| said.get(&r)), Some(Answer::Status(200))) {
                    dead.push((url, "answers 404".into()));
                }
            }
        } else if !on_github.is_empty() {
            eprintln!(
                "daftar-cli: {} GitHub addresses answer 404, which a private repository does too; set GH_TOKEN to tell them apart",
                on_github.len()
            );
        }
        if unanswered > 0 && !options.unreachable {
            eprintln!("daftar-cli: {unanswered} web addresses did not answer (a private network, or gone); --unreachable lists them");
        }
        for (url, why) in dead {
            for (path, line) in &web[url] {
                found.insert((path.clone(), *line, false, "dead-url", format!("{url} {why}")));
            }
        }
    }
    let fixed = mend(root, &fixes, options.dry_run, &mut print);
    let (mut errors, mut warnings) = (0, 0);
    for (path, line, error, kind, detail) in &found {
        if *error {
            errors += 1;
        } else {
            warnings += 1;
        }
        let level = if *error { "error" } else { "warning" };
        if options.github {
            print(report::annotation(level, path, *line, None, kind, detail));
        } else {
            let at = if *line > 0 { format!("{path}:{line}") } else { path.clone() };
            print(format!("{at}\t{level}\t{kind}\t{detail}"));
        }
    }
    Outcome { fixed, errors, warnings, notes: paths.len() }
}

/// Each of [fixes], `(path, line, target, new)`, made in its note through
/// `VaultEdits`, which reads the file again, and printed. The number made.
fn mend(root: &Path, fixes: &BTreeSet<(String, usize, String, String)>, dry_run: bool, print: &mut impl FnMut(String)) -> usize {
    let mut made = 0;
    let mut by_note: HashMap<&str, Vec<(usize, &str, &str)>> = HashMap::new();
    for (path, line, target, new) in fixes {
        by_note.entry(path).or_default().push((*line, target, new));
    }
    let mut notes: Vec<_> = by_note.into_iter().collect();
    notes.sort();
    for (path, fixes) in notes {
        let mut changes = Vec::new();
        let edit = VaultEdits::apply(path, root, |text| {
            let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
            for (line, target, new) in &fixes {
                let Some(old) = lines.get(*line) else { continue };
                let mended = retarget(old, target, new);
                if mended != *old {
                    changes.push((*line, old.trim().to_string(), mended.trim().to_string()));
                    lines[*line] = mended;
                }
            }
            Ok(if dry_run { text.to_string() } else { lines.join("\n") })
        });
        if let Err(error) = edit {
            eprintln!("daftar-cli: {path}: {error}");
            continue;
        }
        for (line, old, new) in changes {
            made += 1;
            print(format!("{path}:{}\tfixed\tambiguous-link\t{old} → {new}", line + 1));
        }
    }
    made
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
        Options {
            strict: false,
            no_h1: vec!["Learning".into()],
            callouts: vec!["read".into()],
            urls: false,
            unreachable: false,
            github_token: None,
            skip_urls: Vec::new(),
            github: false,
            fix: false,
            dry_run: false,
        }
    }

    #[test]
    fn finds_what_is_broken() {
        let dir = vault(&[
            ("D.md", "[[C#No such]] [[C#^nope]] [[Learning/B#Title]] [[C#^here]] [[E#`code` heading]]\n"),
            ("E.md", "## `code` heading\n"),
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
    fn a_broken_heading_link_names_the_heading_it_likely_meant() {
        let dir = vault(&[(
            "A.md",
            "## Checked: Mar Tirrena 1\n## Travel Plans\n\n[[#Checked]] [[#Travle Plans]] [[#Nothing like it]] [x](#travel)\n",
        )]);
        let lines = findings(dir.path(), &options());
        assert_eq!(lines.len(), 4, "{lines:#?}");
        let of = |link: &str| lines.iter().find(|l| l.contains(link)).unwrap().clone();
        assert!(of("[[#Checked]]").ends_with("did you mean “Checked: Mar Tirrena 1”?"));
        assert!(of("[[#Travle Plans]]").ends_with("did you mean “Travel Plans”?"));
        assert!(of("[[#Nothing like it]]").ends_with("has no such heading"));
        assert!(of("(#travel)").ends_with("did you mean “Travel Plans”?"));
    }

    #[test]
    fn fix_gives_an_ambiguous_link_its_folder() {
        let note = "[[Plan]] and [[Plan#Goals]] and [[Plan|the plan]] and ![[Plan]] and [[Planner]]\n| a | [[Plan]] |\n";
        let dir = vault(&[("Work/Plan.md", "## Goals\n"), ("Home/Plan.md", ""), ("Work/Note.md", note), ("Planner.md", "")]);
        let mut options = options();
        options.fix = true;
        options.dry_run = true;
        let lines = findings(dir.path(), &options);
        assert_eq!(lines.len(), 2, "{lines:#?}");
        assert_eq!(fs::read_to_string(dir.path().join("Work/Note.md")).unwrap(), note);
        options.dry_run = false;
        assert_eq!(findings(dir.path(), &options).len(), 2);
        assert_eq!(
            fs::read_to_string(dir.path().join("Work/Note.md")).unwrap(),
            "[[Work/Plan|Plan]] and [[Work/Plan#Goals|Plan > Goals]] and [[Work/Plan|the plan]] and ![[Work/Plan]] and [[Planner]]\n\
             | a | [[Work/Plan\\|Plan]] |\n"
        );
        // Nothing is ambiguous any more.
        assert!(findings(dir.path(), &options).is_empty());
    }

    #[test]
    fn finds_anchors_days_and_blocks() {
        let dir = vault(&[
            (
                "A.md",
                "## My Heading\n\n[[#My Heading]] [[#Nope]] [up](#my-heading) [x](#gone) [y](B.md#second-part)\n\none ^id\ntwo ^id\n\n- [ ] a 📅 2026-02-30\n- [ ] b 📅 2026-1-5 ⏳ 2026-10-01\n- [ ] c ⏳ waiting\n",
            ),
            ("B.md", "---\ntitle: b\n\n## Second Part\n"),
        ]);
        let lines = findings(dir.path(), &options());
        let kinds: Vec<(&str, &str)> = lines.iter().map(|l| (l.split('\t').next().unwrap(), l.split('\t').nth(2).unwrap())).collect();
        assert_eq!(
            kinds,
            [
                ("A.md:3", "missing-anchor"),
                ("A.md:3", "missing-anchor"),
                ("A.md:6", "duplicate-block"),
                ("A.md:8", "task-date"),
                ("A.md:9", "task-date"),
                ("B.md:1", "front-matter")
            ],
            "{lines:#?}"
        );
    }

    #[test]
    fn reads_web_addresses_as_written() {
        assert_eq!(
            urls("[a](https://e.org/a_(b)) and <https://e.org/x>. See **https://e.org/y**, https://e.org/z. (https://e.org/w)/"),
            ["https://e.org/a_(b)", "https://e.org/x", "https://e.org/y", "https://e.org/z", "https://e.org/w"]
        );
        assert_eq!(
            malformed("https://metrics.nhttps://grafana.example.org/login").as_deref(),
            Some("two addresses run together at `https://grafana.example.org/login`")
        );
        assert!(
            malformed("https://a..b.org/").is_some()
                && malformed("https://e.org:80a/").is_some()
                && malformed("https://e.org/%zz").is_some()
        );
        assert!(
            malformed("https://e.org:8080/a%20b?q=1#x").is_none()
                && malformed("http://[::1]:80/").is_none()
                && malformed("https://bücher.de/").is_none()
        );
        assert!(TYPO.is_match("see htps://e.org") && TYPO.is_match("https//e.org") && TYPO.is_match("http:/e.org"));
        assert!(!TYPO.is_match("https://e.org and http://e.org"));
        assert!(is_local("http://localhost:8080/x") && is_local("https://10.1.2.3/") && is_local("http://printer.local"));
        assert!(!is_local("https://example.org/") && !is_local("https://8.8.8.8/"));
        let skip = vec!["example.org".to_string(), "https://e.org/private".to_string()];
        assert!(skipped("https://docs.example.org/a", &skip) && skipped("https://e.org/private/x", &skip));
        assert!(!skipped("https://e.org/public", &skip) && !skipped("https://notexample.org/", &skip));
        assert_eq!(github_repo("https://github.com/o/r.git/issues/1#x").as_deref(), Some("o/r"));
        assert_eq!(github_repo("https://github.com/orgs/o/people"), None);
        assert_eq!(github_repo("https://github.com/o"), None);
    }

    #[test]
    fn a_clean_vault_has_nothing() {
        let dir = vault(&[("A.md", "See [[B]].\n\n> [!note] Title\n>\n> Body\n"), ("B.md", "![](uploads/x.png)\n"), ("uploads/x.png", "")]);
        assert!(findings(dir.path(), &options()).is_empty());
    }
}
