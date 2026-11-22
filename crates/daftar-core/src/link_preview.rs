//! The first lines of a note, for the card a link shows when the pointer
//! rests on it. One pure rule, the same in every app (`docs/specs/look.md`,
//! "Link preview"). The port of `LinkPreview.swift`.
//!
//! A length is counted in characters, as every offset of the core is.

use std::sync::LazyLock;

use regex::Regex;

use crate::note_parser::{FrontMatter, Heading};
use crate::sections::Sections;
use crate::text::{slugs, trim};

pub struct LinkPreview;

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());
static BLOCK_FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(```+|~~~+)").unwrap());
static BLOCK_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}(#{1,6})[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$").unwrap());
static TRAILING_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\^([A-Za-z0-9-]+)\s*$").unwrap());
static OWN_LINE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\^([A-Za-z0-9-]+)\s*$").unwrap());
static ITEM_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d+[.)])(?:\s+\[[^\]]\])?(?:\s+|$)").unwrap());

impl LinkPreview {
    /// At most this many lines.
    pub const MAX_LINES: usize = 12;
    /// At most this many characters.
    pub const MAX_CHARACTERS: usize = 600;
    /// What the card says of a link to no note.
    pub const MISSING: &'static str = "No note yet.";
    /// What the card says of a note with no text.
    pub const EMPTY: &'static str = "The note is empty.";

    /// The first lines of [text], the note named [title].
    ///
    /// The front matter is left out, and so is a first heading whose text is
    /// the note's title. The lines start at the first one that is not blank
    /// and end after 12 lines or 600 characters, whichever comes first: at a
    /// line's end when one is in reach, with `…` when text was left out. A
    /// code block that is cut is closed, and an embed is written as its
    /// link, so nothing is followed.
    ///
    /// With [anchor] a heading (`Next`, `A#B`), the lines are those of its
    /// section without the heading itself; with a block id (`^id`), that
    /// block alone. An anchor that names nothing gives no lines.
    pub fn excerpt(text: &str, title: &str, anchor: Option<&str>) -> String {
        let lines: Vec<&str> = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
        let mut part: &[&str] = match anchor.filter(|anchor| !anchor.is_empty()) {
            Some(anchor) if anchor.starts_with('^') => match block(anchor, &lines) {
                Some((start, end)) => &lines[start..=end],
                None => return String::new(),
            },
            Some(anchor) => {
                let headings = Sections::headings(text);
                let Some(found) = resolve(anchor, &headings).and_then(|chain| chain.last().copied()) else { return String::new() };
                let end = Sections::end(found, &headings, lines.len());
                &lines[(headings[found].line + 1).min(end)..end]
            }
            None => {
                let mut part = without_blank_start(&lines[FrontMatter::parse(&lines).line_count.min(lines.len())..]);
                // The card's head says the title already.
                if part.first().and_then(|first| heading(first)).is_some_and(|text| same(&text, title)) {
                    part = &part[1..];
                }
                part
            }
        };
        part = without_blank_start(part);
        while part.last().is_some_and(|last| is_blank(last)) {
            part = &part[..part.len() - 1];
        }
        cut(part)
    }
}

fn is_blank(line: &str) -> bool {
    line.chars().all(char::is_whitespace)
}

fn without_blank_start<'a, 'b>(lines: &'a [&'b str]) -> &'a [&'b str] {
    &lines[lines.iter().position(|line| !is_blank(line)).unwrap_or(lines.len())..]
}

/// The text of the heading [line] is, as the parser reads one.
fn heading(line: &str) -> Option<String> {
    if !line.starts_with('#') {
        return None;
    }
    HEADING.captures(line).map(|caps| caps[2].to_string())
}

fn same(heading: &str, title: &str) -> bool {
    trim(heading).to_lowercase() == trim(title).to_lowercase()
}

/// The marks of a code fence that [line] opens or closes. A line that
/// starts with inline code, ```` ```a``` b ````, is no fence.
fn fence_mark(line: &str) -> Option<&str> {
    if !line.contains("```") && !line.contains("~~~") {
        return None;
    }
    let mark = FENCE.captures(line)?.get(1)?;
    if mark.as_str().starts_with('`') && line[mark.end()..].contains('`') {
        return None;
    }
    Some(mark.as_str())
}

/// The fence that is open after [line], given the one open before it.
fn fence_after<'a>(open: Option<&'a str>, line: &'a str) -> Option<&'a str> {
    let Some(mark) = fence_mark(line) else { return open };
    match open {
        Some(marks) if mark.chars().next() == marks.chars().next() && mark.len() >= marks.len() => None,
        Some(marks) => Some(marks),
        None => Some(mark),
    }
}

/// [lines] up to the limits, with what closes them.
fn cut(lines: &[&str]) -> String {
    let mut kept: Vec<String> = lines.iter().take(LinkPreview::MAX_LINES).map(|line| (*line).to_string()).collect();
    let mut more = lines.len() > kept.len();
    // Cut inside a line: no line's end was in reach.
    let mut in_line = false;
    let mut count = 0;
    for index in 0..kept.len() {
        // A line break is a character too.
        let length = kept[index].chars().count() + usize::from(index > 0);
        if count + length <= LinkPreview::MAX_CHARACTERS {
            count += length;
            continue;
        }
        more = true;
        if index == 0 {
            kept = vec![kept[0].chars().take(LinkPreview::MAX_CHARACTERS).collect()];
            in_line = true;
        } else {
            kept.truncate(index);
        }
        break;
    }
    while kept.last().is_some_and(|last| is_blank(last)) {
        kept.pop();
    }
    // A code block the cut left open is closed with its own marks.
    let open = kept.iter().fold(None, |open, line| fence_after(open, line)).map(String::from);
    let mut out = embeds_as_links(&kept);
    let closed = open.is_some();
    out.extend(open);
    let text = out.join("\n");
    if !more {
        text
    } else if in_line && !closed {
        text + "…"
    } else {
        text + "\n\n…"
    }
}

/// `![[x]]` written as `[[x]]`, outside code blocks.
fn embeds_as_links(lines: &[String]) -> Vec<String> {
    let mut open: Option<&str> = None;
    lines
        .iter()
        .map(|line| {
            if fence_mark(line).is_some() {
                open = fence_after(open, line);
                return line.clone();
            }
            if open.is_none() && line.contains("![[") { line.replace("![[", "[[") } else { line.clone() }
        })
        .collect()
}

/// How a link's heading and a heading compare: without the marks Obsidian
/// leaves out of a link, white space collapsed, case folded.
fn key(text: &str) -> String {
    slugs::heading(&text.chars().filter(|c| !"*_`=~[]#|^:%".contains(*c)).collect::<String>())
}

/// The headings an anchor `A#B#C` names, one for each part, each under the
/// one before; none when one is not found.
fn resolve(anchor: &str, headings: &[Heading]) -> Option<Vec<usize>> {
    let parts: Vec<String> = anchor.split('#').map(key).collect();
    if parts.iter().any(String::is_empty) {
        return None;
    }
    let (mut low, mut high) = (0, headings.len());
    let mut chain = Vec::new();
    for part in parts {
        let found = (low..high).find(|&at| key(&headings[at].text) == part)?;
        chain.push(found);
        low = found + 1;
        let under = headings[low..].iter().position(|h| h.level <= headings[found].level).map_or(headings.len(), |at| low + at);
        high = high.min(under);
    }
    Some(chain)
}

/// The first and last line of the block [id] names: a paragraph or a list
/// item with `^id` at its end, or a table, a quote or a code block with
/// `^id` on a line of its own after it, as Obsidian writes it.
fn block(id: &str, lines: &[&str]) -> Option<(usize, usize)> {
    let wanted = id.strip_prefix('^').unwrap_or(id).to_lowercase();
    // Each block: its first and last line, its id, and whether a line under it may name it.
    let mut blocks: Vec<(usize, usize, Option<String>, bool)> = Vec::new();
    let id_of = |line: &str| TRAILING_ID.captures(line).map(|caps| caps[1].to_lowercase());
    let starts_block = |line: &str| {
        let trimmed = line.trim_start_matches([' ', '\t']);
        ["|", ">", "```", "~~~", "$$"].iter().any(|mark| trimmed.starts_with(mark))
            || ITEM_MARK.is_match(line)
            || (trimmed.starts_with('#') && BLOCK_HEADING.is_match(line))
    };
    // To the last line of the paragraph or the item that starts at [from].
    let run = |from: usize| {
        let mut end = from;
        while end + 1 < lines.len() && !is_blank(lines[end + 1]) && !starts_block(lines[end + 1]) {
            end += 1;
        }
        end
    };
    let mut index = FrontMatter::parse(lines).line_count;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = trim(line);
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        // `^id` alone names the table, quote or code block above it.
        if let Some(caps) = OWN_LINE_ID.captures(line)
            && let Some(last) = blocks.last_mut()
            && last.3
            && last.2.is_none()
            && (last.1 + 1..index).all(|at| is_blank(lines[at]))
        {
            last.2 = Some(caps[1].to_lowercase());
            index += 1;
            continue;
        }
        let mark = fence_mark(line).and_then(|_| BLOCK_FENCE.captures(line)).map(|caps| caps[1].to_string());
        if mark.is_some() || trimmed == "$$" {
            // To its closing line, or to the end when it never closes.
            let closes = |other: &str| match &mark {
                Some(mark) => BLOCK_FENCE.captures(other).is_some_and(|caps| caps[1].chars().next() == mark.chars().next() && caps[1].len() >= mark.len()),
                None => trim(other) == "$$",
            };
            let close = (index + 1..lines.len()).find(|&at| closes(lines[at])).unwrap_or(lines.len());
            let end = close.min(lines.len() - 1);
            blocks.push((index, end, None, true));
            index = end + 1;
            continue;
        }
        if trimmed.starts_with('#') && BLOCK_HEADING.is_match(line) {
            blocks.push((index, index, None, false));
            index += 1;
            continue;
        }
        if trimmed.starts_with('|') || trimmed.starts_with('>') {
            let mark = if trimmed.starts_with('|') { '|' } else { '>' };
            let mut end = index;
            while end + 1 < lines.len() && trim(lines[end + 1]).starts_with(mark) {
                end += 1;
            }
            blocks.push((index, end, None, true));
            index = end + 1;
            continue;
        }
        // A list item is its own lines, not the items under it; any other line starts a paragraph.
        let end = run(index);
        blocks.push((index, end, id_of(lines[end]), false));
        index = end + 1;
    }
    blocks.into_iter().find(|found| found.2.as_deref() == Some(wanted.as_str())).map(|found| (found.0, found.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(count: usize) -> String {
        (1..=count).map(|n| format!("l{n}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_preview_is_the_first_lines() {
        // A front matter, the title as a heading, a blank line and 20 lines: the first 12, and `…`.
        let plan = format!("---\ntags: [a]\n---\n# Plan\n\n{}\n", numbered(20));
        assert_eq!(LinkPreview::excerpt(&plan, "Plan", None), numbered(12) + "\n\n…");
        // A link to a heading: the lines under it up to the next heading of its level or higher, without it.
        let sections = "# Plan\n\nintro\n\n## Next\n\none\n\n### Deeper\n\ntwo\n\n## Later\n\nthree\n";
        assert_eq!(LinkPreview::excerpt(sections, "Plan", Some("Next")), "one\n\n### Deeper\n\ntwo");
        assert_eq!(LinkPreview::excerpt(sections, "Plan", Some("Later")), "three");
        assert_eq!(LinkPreview::excerpt(sections, "Plan", Some("Next#Deeper")), "two");
        // One line of 900 letters: 600 of them, and `…`.
        assert_eq!(LinkPreview::excerpt(&"a".repeat(900), "Long", None), "a".repeat(600) + "…");
        // A note that is only a front matter is empty.
        assert_eq!(LinkPreview::excerpt("---\ntags: [a]\n---\n", "Plan", None), "");
        assert_eq!(LinkPreview::excerpt("", "Plan", None), "");
        // A name with no note has its own line.
        assert_eq!(LinkPreview::MISSING, "No note yet.");
        assert_eq!(LinkPreview::EMPTY, "The note is empty.");
    }

    #[test]
    fn a_short_note_is_shown_whole_with_no_dots() {
        assert_eq!(LinkPreview::excerpt("\n\none\ntwo\n\n", "Note", None), "one\ntwo");
        // Twelve lines are all of it.
        assert_eq!(LinkPreview::excerpt(&numbered(12), "Note", None), numbered(12));
        // 600 letters are all of it too.
        assert_eq!(LinkPreview::excerpt(&"b".repeat(600), "Note", None), "b".repeat(600));
    }

    #[test]
    fn only_a_first_heading_that_is_the_title_is_left_out() {
        assert_eq!(LinkPreview::excerpt("# Plan\n\ntext", "Plan", None), "text");
        assert_eq!(LinkPreview::excerpt("## plan\ntext", "Plan", None), "text");
        // Another heading stays, and so does the title when it is not the first line.
        assert_eq!(LinkPreview::excerpt("# Other\n\ntext", "Plan", None), "# Other\n\ntext");
        assert_eq!(LinkPreview::excerpt("text\n\n# Plan\n\nmore", "Plan", None), "text\n\n# Plan\n\nmore");
    }

    #[test]
    fn the_cut_is_at_a_lines_end_when_one_is_in_reach() {
        // Two lines of 400 letters: the second does not fit, so the first is all, with `…` under it.
        let line = "c".repeat(400);
        assert_eq!(LinkPreview::excerpt(&format!("{line}\n{line}"), "Note", None), format!("{line}\n\n…"));
        // The line breaks count: 300 + 1 + 299 is 600.
        let (a, b) = ("a".repeat(300), "b".repeat(299));
        assert_eq!(LinkPreview::excerpt(&format!("{a}\n{b}"), "Note", None), format!("{a}\n{b}"));
        assert_eq!(LinkPreview::excerpt(&format!("{a}\n{b}b"), "Note", None), format!("{a}\n\n…"));
        // A blank line the cut ends on is not kept.
        assert_eq!(LinkPreview::excerpt(&format!("{a}\n\n{a}"), "Note", None), format!("{a}\n\n…"));
    }

    #[test]
    fn a_code_block_that_is_cut_is_closed() {
        let code = format!("```swift\n{}\n```\nafter\n", (1..=20).map(|n| format!("let x{n} = 1")).collect::<Vec<_>>().join("\n"));
        let excerpt = LinkPreview::excerpt(&code, "Note", None);
        let lines: Vec<&str> = excerpt.split('\n').collect();
        assert_eq!(lines.len(), 15);
        assert_eq!((lines[0], lines[11]), ("```swift", "let x11 = 1"));
        assert_eq!(lines[12..], ["```", "", "…"]);
        // A longer fence is closed with its own marks, and a block that ends in the lines needs nothing.
        let tildes = format!("~~~~\n{}~~~~\n", "x\n".repeat(30));
        assert!(LinkPreview::excerpt(&tildes, "Note", None).ends_with("x\n~~~~\n\n…"));
        assert_eq!(LinkPreview::excerpt("```\na\n```\n", "Note", None), "```\na\n```");
    }

    #[test]
    fn an_embed_is_its_link_and_is_not_followed() {
        assert_eq!(LinkPreview::excerpt("See ![[Other]] and ![[a.png|100]].", "Note", None), "See [[Other]] and [[a.png|100]].");
        // In a code block it is text.
        assert_eq!(LinkPreview::excerpt("```\n![[Other]]\n```", "Note", None), "```\n![[Other]]\n```");
    }

    #[test]
    fn a_link_to_a_block_shows_that_block_alone() {
        let note = "# Note\n\nfirst\n\nThe second one. ^b1\n\n- item one\n- item two ^b2\n\nlast\n";
        assert_eq!(LinkPreview::excerpt(note, "Note", Some("^b1")), "The second one. ^b1");
        assert_eq!(LinkPreview::excerpt(note, "Note", Some("^b2")), "- item two ^b2");
        // An anchor that names nothing gives no lines.
        assert_eq!(LinkPreview::excerpt(note, "Note", Some("^none")), "");
        assert_eq!(LinkPreview::excerpt(note, "Note", Some("Nowhere")), "");
        // A table is named by the line under it, and is shown without that line.
        let table = "| A |\n| --- |\n| 1 |\n\n^t1\n\nafter ^p\n";
        assert_eq!(LinkPreview::excerpt(table, "Note", Some("^t1")), "| A |\n| --- |\n| 1 |");
        assert_eq!(LinkPreview::excerpt(table, "Note", Some("^T1")), "| A |\n| --- |\n| 1 |");
    }

    #[test]
    fn persian_text_is_cut_by_letters_not_bytes() {
        let cut = LinkPreview::excerpt(&"س".repeat(700), "یادداشت", None);
        assert_eq!(cut.chars().count(), 601);
        assert!(cut.ends_with('…'));
    }
}
