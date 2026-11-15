//! The links of a line where they stand: `[[target#anchor|alias]]`,
//! `![[embed]]` and `[text](path.md#anchor)` with no scheme, outside code
//! spans and fenced code; and the link being typed before the cursor.

use std::ops::Range;
use std::sync::LazyLock;

use daftar_core::Wikilink;
use percent_encoding::percent_decode_str;
use regex::Regex;

static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\]\n]+?)\]\]").unwrap());
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[[^\]\n]*\]\(<?([^)>\s]+)>?\)").unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());

/// One link on a line, its ranges in bytes of the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// As written, before `#` and `|`; empty for a heading of this note.
    pub target: String,
    pub anchor: Option<String>,
    pub embed: bool,
    /// The whole link, marks and all.
    pub whole: Range<usize>,
    /// The target as written: what a rename rewrites.
    pub target_range: Range<usize>,
}

/// The links on [line].
pub fn on_line(line: &str) -> Vec<Found> {
    if !line.contains("[[") && !line.contains("](") {
        return Vec::new();
    }
    let code: Vec<Range<usize>> = INLINE_CODE.find_iter(line).map(|m| m.range()).collect();
    let in_code = |r: &Range<usize>| code.iter().any(|c| c.start < r.end && r.start < c.end);
    let mut out = Vec::new();
    for caps in WIKILINK.captures_iter(line) {
        let (all, inner) = (caps.get(0).unwrap(), caps.get(2).unwrap());
        if in_code(&all.range()) {
            continue;
        }
        let link = Wikilink::new(inner.as_str());
        let written = inner.as_str().split(['#', '|']).next().unwrap_or("");
        let lead = written.len() - written.trim_start().len();
        let start = inner.start() + lead;
        out.push(Found {
            target: link.target,
            anchor: link.anchor.filter(|a| !a.is_empty()),
            embed: !caps[1].is_empty(),
            whole: all.range(),
            target_range: start..start + written.trim().len(),
        });
    }
    for caps in MARKDOWN_LINK.captures_iter(line) {
        let (all, url) = (caps.get(0).unwrap(), caps.get(2).unwrap());
        if in_code(&all.range())
            || SCHEME.is_match(url.as_str())
            || out.iter().any(|f| f.whole.start < all.end() && all.start() < f.whole.end)
        {
            continue;
        }
        let decoded = percent_decode_str(url.as_str()).decode_utf8_lossy().into_owned();
        let (target, anchor) = match decoded.split_once('#') {
            Some((target, anchor)) => (target.to_string(), Some(anchor.to_string()).filter(|a| !a.is_empty())),
            None => (decoded.clone(), None),
        };
        let written = url.as_str().split('#').next().unwrap_or("").len();
        out.push(Found {
            target,
            anchor,
            embed: !caps[1].is_empty(),
            whole: all.range(),
            target_range: url.start()..url.start() + written,
        });
    }
    out.sort_by_key(|f| f.whole.start);
    out
}

/// The link on [line] that byte [at] is in.
pub fn at(line: &str, at: usize) -> Option<Found> {
    on_line(line).into_iter().find(|f| f.whole.start <= at && at < f.whole.end)
}

/// Each line of [text] with its links, from 0, leaving out fenced code.
pub fn in_text(text: &str) -> Vec<(usize, Found)> {
    let mut open: Option<String> = None;
    let mut out = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        if (line.contains("```") || line.contains("~~~"))
            && let Some(caps) = FENCE.captures(line)
        {
            let marker = &caps[1];
            match &open {
                None => open = Some(marker.to_string()),
                Some(first) if marker.starts_with(&first[..1]) && marker.len() >= first.len() => open = None,
                Some(_) => {}
            }
            continue;
        }
        if open.is_none() {
            out.extend(on_line(line).into_iter().map(|f| (n, f)));
        }
    }
    out
}

/// What is being typed in a link before the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typing {
    /// `[[query`: a note; [start] is the byte after `[[`.
    Note { query: String, start: usize },
    /// `[[note#query`: a heading of [note], this one when it is empty;
    /// [start] is the byte after `#`.
    Heading { note: String, query: String, start: usize },
}

/// The link [before] (a line up to the cursor) ends in the middle of: the
/// last `[[` not closed yet. Not after `|`, the alias, nor `#^`, a block.
pub fn typing(before: &str) -> Option<Typing> {
    let open = before.rfind("[[")?;
    let inner = &before[open + 2..];
    if inner.contains("]]") || inner.contains('|') || inner.contains('[') {
        return None;
    }
    match inner.split_once('#') {
        Some((_, query)) if query.starts_with('^') => None,
        Some((note, query)) => {
            Some(Typing::Heading { note: note.trim().to_string(), query: query.to_string(), start: open + 2 + note.len() + 1 })
        }
        None => Some(Typing::Note { query: inner.to_string(), start: open + 2 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_wikilinks_embeds_and_markdown_links_but_not_in_code() {
        let line = "See [[Folder/Note#Part|shown]] and ![[pic.png]], [x](Other%20Note.md#Top), `[[Code]]`, [w](https://example.com).";
        let found = on_line(line);
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].target.as_str(), found[0].anchor.as_deref(), found[0].embed), ("Folder/Note", Some("Part"), false));
        assert_eq!(&line[found[0].target_range.clone()], "Folder/Note");
        assert_eq!(&line[found[0].whole.clone()], "[[Folder/Note#Part|shown]]");
        assert!(found[1].embed);
        assert_eq!((found[2].target.as_str(), found[2].anchor.as_deref()), ("Other Note.md", Some("Top")));
        assert_eq!(&line[found[2].target_range.clone()], "Other%20Note.md");
        assert_eq!(at(line, line.find("Folder").unwrap()).unwrap().target, "Folder/Note");
        assert_eq!(at(line, 0), None);
    }

    #[test]
    fn a_heading_of_this_note_has_no_target() {
        let found = on_line("[[#Part]] [[ Spaced ]]");
        assert_eq!((found[0].target.as_str(), found[0].anchor.as_deref()), ("", Some("Part")));
        assert_eq!(found[1].target, "Spaced");
        assert_eq!(found[1].target_range, 13..19);
    }

    #[test]
    fn links_in_fenced_code_are_none() {
        let text = "[[A]]\n```\n[[B]]\n```\n[[C]]";
        let lines: Vec<(usize, String)> = in_text(text).into_iter().map(|(n, f)| (n, f.target)).collect();
        assert_eq!(lines, [(0, "A".to_string()), (4, "C".to_string())]);
    }

    #[test]
    fn the_link_being_typed() {
        assert_eq!(typing("a [[Pro"), Some(Typing::Note { query: "Pro".into(), start: 4 }));
        assert_eq!(typing("[[یاد"), Some(Typing::Note { query: "یاد".into(), start: 2 }));
        assert_eq!(typing("[[Note#Pa"), Some(Typing::Heading { note: "Note".into(), query: "Pa".into(), start: 7 }));
        assert_eq!(typing("[[#"), Some(Typing::Heading { note: String::new(), query: String::new(), start: 3 }));
        assert_eq!(typing("[[A]] b"), None);
        assert_eq!(typing("[[A|b"), None);
        assert_eq!(typing("[[A#^b"), None);
        assert_eq!(typing("plain"), None);
    }
}
