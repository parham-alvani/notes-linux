//! What the index keeps of one note, from one pass over its lines. The
//! reading view renders the note itself, in the web view.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

pub use crate::task_line::ParsedTask;
use crate::task_line::TaskLine;
use crate::text::{direction, lines, slugs, trim};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedNote {
    /// The front matter, in the file's order.
    pub properties: Vec<(String, Vec<String>)>,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    pub tasks: Vec<ParsedTask>,
    /// The text without front matter, comments and markup, for search.
    pub plain: String,
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    /// Zero-based line in the file.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Link {
    /// As written, before `#` and `|`.
    pub target: String,
    pub anchor: Option<String>,
    pub embed: bool,
    pub line: usize,
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$").unwrap());
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\]\n]+?)\]\]").unwrap());
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[[^\]\n]*\]\(<?([^)>\s]+)>?\)").unwrap());
// A URL with a scheme (`https:`, `mailto:`, `data:`, `live:`) leaves the vault.
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)#([\p{L}\p{N}_/-]+)").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());

pub struct NoteParser;

impl NoteParser {
    pub fn parse(text: &str) -> ParsedNote {
        let lines = lines(text);
        let front = FrontMatter::parse(&lines);
        let mut note = ParsedNote {
            aliases: [front.values("aliases"), front.values("alias")].concat(),
            tags: [front.values("tags"), front.values("tag")]
                .concat()
                .into_iter()
                .map(|tag| tag.strip_prefix('#').map(String::from).unwrap_or(tag))
                .collect(),
            properties: front.properties.clone(),
            ..ParsedNote::default()
        };

        // A link in a property, `up: "[[Parent]]"`, is a link as Obsidian counts them.
        let inside = front.line_count.saturating_sub(1).max(1);
        for (index, line) in lines.iter().enumerate().take(inside).skip(1).filter(|(_, line)| line.contains("[[")) {
            for caps in WIKILINK.captures_iter(line) {
                let parts = Wikilink::new(&caps[2]);
                note.links.push(Link { target: parts.target, anchor: parts.anchor, embed: !caps[1].is_empty(), line: index });
            }
        }

        let mut plain: Vec<String> = Vec::new();
        let mut fence_marker: Option<String> = None;
        let mut in_comment = false;
        let mut section = String::new();
        for (index, raw) in lines.iter().enumerate().skip(front.line_count) {
            // Each pattern runs only on a line that has the marks it needs:
            // a regular expression on every line was most of the time a
            // large vault took to open.
            if (raw.contains("```") || raw.contains("~~~"))
                && let Some(caps) = FENCE.captures(raw)
            {
                let marker = &caps[1];
                match &fence_marker {
                    None => fence_marker = Some(marker.to_string()),
                    Some(open) if marker.chars().next() == open.chars().next() && marker.len() >= open.len() => fence_marker = None,
                    Some(_) => {}
                }
                continue;
            }
            if fence_marker.is_some() {
                plain.push(raw.to_string());
                continue;
            }
            let line = strip_comments(raw, &mut in_comment);
            if trim(&line).is_empty() {
                continue;
            }

            if line.starts_with('#')
                && let Some(caps) = HEADING.captures(&line)
            {
                let text = caps[2].to_string();
                note.headings.push(Heading { level: caps[1].len(), text: text.clone(), line: index });
                section = text;
            }
            let code = if line.contains('`') { INLINE_CODE.replace_all(&line, "") } else { Cow::Borrowed(line.as_str()) };
            let links = if code.contains("[[") { WIKILINK.captures_iter(&code).collect() } else { Vec::new() };
            for caps in links {
                let parts = Wikilink::new(&caps[2]);
                note.links.push(Link { target: parts.target, anchor: parts.anchor, embed: !caps[1].is_empty(), line: index });
            }
            let links = if code.contains("](") { MARKDOWN_LINK.captures_iter(&code).collect() } else { Vec::new() };
            for caps in links {
                let url = &caps[2];
                if SCHEME.is_match(url) || url.starts_with('#') {
                    continue;
                }
                let decoded = percent_decode_str(url).decode_utf8().map(|d| d.into_owned()).unwrap_or_else(|_| url.to_string());
                let (target, anchor) = match decoded.split_once('#') {
                    Some((target, anchor)) => (target.to_string(), Some(anchor.to_string())),
                    None => (decoded, None),
                };
                note.links.push(Link { target, anchor, embed: !caps[1].is_empty(), line: index });
            }
            let tags = if may_hold_tag(&code) { TAG.captures_iter(&code).collect() } else { Vec::new() };
            for caps in tags {
                let tag = &caps[1];
                if !tag.chars().all(|c| c.is_numeric() || c == '/' || c == '-') {
                    note.tags.push(tag.to_string());
                }
            }
            if let Some(task) = TaskLine::parse(&line, index, &section) {
                note.tasks.push(task);
            }
            plain.push(line);
        }
        note.plain = plain.join("\n");
        note.rtl = direction::contains_rtl(&note.plain);
        let mut seen = HashSet::new();
        note.tags.retain(|tag| !tag.is_empty() && seen.insert(slugs::fold(tag)));
        note
    }
}

/// `target#heading|alias` in its three parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wikilink {
    pub target: String,
    pub anchor: Option<String>,
    pub alias: Option<String>,
    /// The bar was written `\|`, as it must be inside a table.
    pub escaped: bool,
}

impl Wikilink {
    pub fn new(inner: &str) -> Wikilink {
        let (mut reference, alias) = match inner.split_once('|') {
            Some((reference, alias)) => (reference, Some(alias.to_string())),
            None => (inner, None),
        };
        let mut escaped = false;
        if alias.is_some()
            && let Some(stripped) = reference.strip_suffix('\\')
        {
            reference = stripped;
            escaped = true;
        }
        let (target, anchor) = match reference.split_once('#') {
            Some((target, anchor)) => (trim(target).to_string(), Some(trim(anchor).to_string())),
            None => (trim(reference).to_string(), None),
        };
        Wikilink { target, anchor, alias, escaped }
    }
}

/// Whether a `#` here is followed by something a tag can start with: not a
/// space, a tab, another `#` or the end. A heading's marks are not.
fn may_hold_tag(line: &str) -> bool {
    line.as_bytes().windows(2).any(|pair| pair[0] == b'#' && !matches!(pair[1], b' ' | b'\t' | b'#'))
}

/// [line] without its `%% comments %%`, which may run over several lines.
fn strip_comments(line: &str, open: &mut bool) -> String {
    if !*open && !line.contains("%%") {
        return line.to_string();
    }
    let mut out = String::new();
    let mut rest = line;
    loop {
        match rest.find("%%") {
            None => {
                if !*open {
                    out.push_str(rest);
                }
                break;
            }
            Some(at) => {
                if !*open {
                    out.push_str(&rest[..at]);
                }
                *open = !*open;
                rest = &rest[at + 2..];
            }
        }
    }
    out
}

/// The front matter: a small, forgiving reading of the YAML this vault
/// writes. `key: value`, `key: [a, b]` and a block list of `- item` lines.
#[derive(Debug, Clone, Default)]
pub struct FrontMatter {
    pub properties: Vec<(String, Vec<String>)>,
    /// Lines the front matter takes, fences included; 0 when there is none.
    pub line_count: usize,
}

impl FrontMatter {
    pub fn values(&self, key: &str) -> Vec<String> {
        self.properties.iter().find(|(k, _)| k.to_lowercase() == key).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    pub fn parse(lines: &[&str]) -> FrontMatter {
        let mut result = FrontMatter::default();
        if lines.first().map(|l| trim(l)) != Some("---") {
            return result;
        }
        let Some(end) = (1..lines.len()).find(|&i| matches!(trim(lines[i]), "---" | "...")) else {
            return result;
        };
        result.line_count = end + 1;
        for line in &lines[1..end] {
            let trimmed = trim(line);
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indented = line.starts_with(char::is_whitespace);
            if let Some(item) = trimmed.strip_prefix("- ")
                && (indented || line.starts_with("- "))
                && let Some(last) = result.properties.last_mut()
            {
                last.1.push(unquote(item));
                continue;
            }
            if indented {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else { continue };
            let (key, value) = (trim(key).to_string(), trim(value));
            if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
                let items = value[1..value.len() - 1].split(',').map(|item| unquote(trim(item))).filter(|item| !item.is_empty());
                result.properties.push((key, items.collect()));
            } else {
                result.properties.push((key, if value.is_empty() { vec![] } else { vec![unquote(value)] }));
            }
        }
        result
    }
}

fn unquote(value: &str) -> String {
    let mut chars = value.chars();
    if let (Some(first), Some(last)) = (chars.next(), chars.next_back())
        && (first == '"' || first == '\'')
        && first == last
    {
        return chars.as_str().to_string();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "---
aliases: [First, \"Second\"]
tags:
  - project/alpha
---
# Heading one

A link to [[Inbox#Today|today]] and ![[diagram.png|300]], a [doc](Notes/Other%20note.md).
`[[not a link]]` and #inline-tag but not #123.
%% [[hidden]] %%

## Tasks
- [ ] Write the plan ➕ 2026-10-01 ⏳ 2026-10-04
    - [x] Draft ✅ 2026-10-02
> - [/] Inside a callout 📅 2026-10-09

```
- [ ] code, not a task
[[code link]]
```";

    #[test]
    fn front_matter() {
        let note = NoteParser::parse(TEXT);
        assert_eq!(note.aliases, ["First", "Second"]);
        assert!(note.tags.contains(&"project/alpha".to_string()));
    }

    #[test]
    fn links_outside_code_and_comments() {
        let note = NoteParser::parse(TEXT);
        let targets: Vec<&str> = note.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["Inbox", "diagram.png", "Notes/Other note.md"]);
        assert_eq!(note.links[0].anchor.as_deref(), Some("Today"));
        assert!(note.links[1].embed);
        assert_eq!(note.links[0].line, 7);
    }

    #[test]
    fn urls_with_a_scheme_are_not_notes() {
        let text = "[a](https://example.com) [b](mailto:a@example.com) ![c](data:image/png;base64,AAAA) [d](live:.cid.1) [e](Notes/E.md)";
        let targets: Vec<String> = NoteParser::parse(text).links.into_iter().map(|l| l.target).collect();
        assert_eq!(targets, ["Notes/E.md"]);
    }

    #[test]
    fn tags_headings_and_tasks() {
        let note = NoteParser::parse(TEXT);
        assert!(note.tags.contains(&"inline-tag".to_string()));
        assert!(!note.tags.contains(&"123".to_string()));
        let headings: Vec<&str> = note.headings.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(headings, ["Heading one", "Tasks"]);
        assert_eq!(note.tasks.len(), 3);
        let first = &note.tasks[0];
        assert_eq!(first.text, "Write the plan");
        assert_eq!(first.scheduled.as_deref(), Some("2026-10-04"));
        assert_eq!(first.created.as_deref(), Some("2026-10-01"));
        assert_eq!(first.section, "Tasks");
        assert_eq!(first.line, 12);
        assert!(note.tasks[1].nested && !note.tasks[1].is_open());
        let callout = &note.tasks[2];
        assert!(callout.status == '/' && callout.due.as_deref() == Some("2026-10-09") && callout.is_open());
    }

    #[test]
    fn persian_is_right_to_left() {
        assert!(NoteParser::parse("سلام، این یک یادداشت است").rtl);
        assert!(!NoteParser::parse("Plain English").rtl);
    }

    #[test]
    fn a_link_in_a_property_counts() {
        let note = NoteParser::parse("---\nup: \"[[Parent]]\"\n---\nbody");
        assert_eq!(note.links[0].target, "Parent");
        assert_eq!(note.links[0].line, 1);
    }

    #[test]
    fn wikilink_parts() {
        let link = Wikilink::new("Note#Part|label");
        assert_eq!((link.target.as_str(), link.anchor.as_deref(), link.alias.as_deref()), ("Note", Some("Part"), Some("label")));
        assert!(Wikilink::new("Note\\|label").escaped);
    }
}
