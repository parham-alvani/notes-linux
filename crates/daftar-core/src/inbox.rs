//! The capture inbox: `Scratchpad.md` at the vault's root, in dated
//! `## YYYY-MM-DD` sections, where quick capture puts what comes to mind.
//! Each item there is meant to move to its home note and leave the inbox.
//! An item is a list item with the lines indented under it, a paragraph, or
//! a fenced block; a paragraph that ends in `:` takes the list right under
//! it along, as it introduces it. Moving one adds its lines to the other
//! note first and only then takes them out of the inbox, so it is never
//! lost: at worst it is in both.

use std::sync::LazyLock;

use regex::Regex;

use crate::note_parser::NoteParser;
use crate::quick_capture::QuickCapture;
use crate::vault_edits::Failure;

static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[-*+]|\d+[.)])(?:[ \t]|$)").unwrap());
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*(?:[-*+]|\d+[.)])[ \t]+").unwrap());
static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ {0,3}(?:(?:-[ \t]*){3,}|(?:\*[ \t]*){3,}|(?:_[ \t]*){3,})$").unwrap());
static DAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());

/// One item of the inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    /// Its first line, from 0.
    pub line: usize,
    /// Its lines as the note has them, without a `\r`.
    pub lines: Vec<String>,
    /// The text of the heading it is under, `2026-10-29` in a dated section.
    pub heading: Option<String>,
    /// Its lines on one line, trimmed, the first without its list mark:
    /// what a list shows and a move is asked to find again.
    pub text: String,
}

pub struct Inbox;

impl Inbox {
    /// The inbox note when the settings name none (`scratchpad.note`).
    pub const NOTE: &'static str = "Scratchpad.md";

    /// The inbox's vault path from the `scratchpad.note` setting: the note
    /// it names, with `.md` added, or `Scratchpad.md`.
    pub fn path(setting: Option<&str>) -> String {
        let named = setting.unwrap_or("").trim().trim_matches('/');
        if named.is_empty() {
            return Self::NOTE.to_string();
        }
        if named.to_lowercase().ends_with(".md") { named.to_string() } else { format!("{named}.md") }
    }

    /// Whether an item may move to [path]: never the week notes of
    /// `Calendar/`, which the phone writes whole, nor the inbox itself.
    pub fn may_receive(path: &str, inbox: &str) -> bool {
        let folded = path.to_lowercase();
        !folded.starts_with(&format!("{}/", crate::CalendarWeek::FOLDER.to_lowercase())) && folded != inbox.to_lowercase()
    }

    /// The items of [text], in order. Headings, rules, blank lines and the
    /// front matter are not items.
    pub fn items(text: &str) -> Vec<InboxItem> {
        let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let mut items = Vec::new();
        let mut heading: Option<String> = None;
        let mut at = front_matter_end(&lines);
        while at < lines.len() {
            let line = lines[at];
            if line.trim().is_empty() || RULE.is_match(line) {
                at += 1;
                continue;
            }
            if let Some(text) = heading_text(line) {
                heading = Some(text);
                at += 1;
                continue;
            }
            let end = item_end(&lines, at);
            let taken: Vec<String> = lines[at..end].iter().map(|l| (*l).to_string()).collect();
            items.push(InboxItem { line: at, text: shown(&taken), lines: taken, heading: heading.clone() });
            at = end;
        }
        items
    }

    /// The item that starts on [line] of [text].
    pub fn item_at(text: &str, line: usize) -> Option<InboxItem> {
        Self::items(text).into_iter().find(|item| item.line == line)
    }

    /// Whether [item] is the one listed as [expected]: its text as a list
    /// shows it, or its first line.
    pub fn holds(item: &InboxItem, expected: &str) -> bool {
        let expected = expected.trim();
        item.text == expected || item.lines.first().is_some_and(|l| l.trim() == expected)
    }

    /// [text] with [lines] added: at the end of the section of the heading
    /// on [heading_line] (from 0), or at the end of the note, as
    /// `QuickCapture::append_at_heading_line` adds a line.
    pub fn added(text: &str, lines: &[String], heading_line: Option<usize>) -> String {
        let eol = if text.split('\n').next().is_some_and(|l| l.ends_with('\r')) && text.contains('\n') { "\r\n" } else { "\n" };
        QuickCapture::append_at_heading_line(&lines.join(eol), text, heading_line)
    }

    /// [text] with [heading]'s section (a heading's text, without case) or
    /// the note's end given [lines], the section made at the end when the
    /// note has no such heading, as quick capture does.
    pub fn added_under(text: &str, lines: &[String], heading: &str) -> String {
        let eol = if text.split('\n').next().is_some_and(|l| l.ends_with('\r')) && text.contains('\n') { "\r\n" } else { "\n" };
        QuickCapture::append(&lines.join(eol), text, Some(heading))
    }

    /// [text] without the item on [line], which must be [lines] still: the
    /// lines around close up to one blank line at most, and a dated section
    /// it leaves empty goes with its heading. `TextMoved` when the note no
    /// longer has those lines there.
    pub fn removed(text: &str, line: usize, lines: &[String]) -> Result<String, Failure> {
        let found = Self::item_at(text, line).ok_or(Failure::TextMoved)?;
        if found.lines != lines {
            return Err(Failure::TextMoved);
        }
        let mut all: Vec<String> = text.split('\n').map(String::from).collect();
        all.drain(line..line + lines.len());
        close_up(&mut all, line);
        // The dated section it was in, when nothing is left in it.
        let headings = NoteParser::parse(&all.join("\n")).headings;
        if let Some(index) = headings.iter().rposition(|h| h.line < line)
            && DAY.is_match(headings[index].text.trim())
        {
            let start = headings[index].line;
            let end = crate::Sections::end(index, &headings, all.len());
            if all[start + 1..end].iter().all(|l| l.trim().is_empty()) {
                // The blank lines go too, but the note's last line break stays.
                let end = if end == all.len() && all.last().is_some_and(String::is_empty) { end - 1 } else { end };
                all.drain(start..end);
                close_up(&mut all, start);
            }
        }
        Ok(all.join("\n"))
    }
}

/// The line after the front matter, or 0 when there is none.
fn front_matter_end(lines: &[&str]) -> usize {
    if lines.first().is_none_or(|l| l.trim_end() != "---") {
        return 0;
    }
    lines.iter().skip(1).position(|l| l.trim_end() == "---" || l.trim_end() == "...").map_or(0, |at| at + 2)
}

/// An ATX heading's text.
fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marks = trimmed.chars().take_while(|&c| c == '#').count();
    let rest = &trimmed[marks..];
    ((1..=6).contains(&marks) && (rest.is_empty() || rest.starts_with([' ', '\t'])))
        .then(|| rest.trim().trim_end_matches('#').trim_end().to_string())
}

fn fence(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn indented(line: &str) -> bool {
    line.starts_with([' ', '\t']) && !line.trim().is_empty()
}

/// Where a block that is not a list item or a fence stops: a blank line,
/// a heading, a rule, a fence or a list item.
fn breaks_paragraph(line: &str) -> bool {
    line.trim().is_empty() || heading_text(line).is_some() || RULE.is_match(line) || fence(line).is_some() || LIST_ITEM.is_match(line)
}

/// The line after the item that starts on [at].
fn item_end(lines: &[&str], at: usize) -> usize {
    let first = lines[at];
    if let Some(mark) = fence(first) {
        let close = lines[at + 1..].iter().position(|l| l.trim_start().starts_with(mark));
        return close.map_or(lines.len(), |c| at + 1 + c + 1);
    }
    if LIST_ITEM.is_match(first) {
        return list_item_end(lines, at);
    }
    let mut end = at + 1;
    while end < lines.len() && !breaks_paragraph(lines[end]) {
        end += 1;
    }
    // "To do in the cluster:" with its list under it is one thing.
    if lines[end - 1].trim_end().ends_with(':') {
        let mut next = end;
        if next < lines.len() && lines[next].trim().is_empty() {
            next += 1;
        }
        if next < lines.len() && LIST_ITEM.is_match(lines[next]) {
            end = next;
            while end < lines.len() && LIST_ITEM.is_match(lines[end]) {
                end = list_item_end(lines, end);
            }
        }
    }
    end
}

/// The line after the list item on [at] with the lines indented under it,
/// blank lines among them, but not the blank lines after it.
fn list_item_end(lines: &[&str], at: usize) -> usize {
    let mut end = at + 1;
    let mut last = at;
    while end < lines.len() {
        let line = lines[end];
        if indented(line) {
            last = end;
        } else if !line.trim().is_empty() {
            break;
        }
        end += 1;
    }
    last + 1
}

/// An item's lines on one line, the first without its list mark.
fn shown(lines: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let line = if at == 0 { MARK.replace(line, "").into_owned() } else { line.clone() };
        let line = line.trim().replace('\t', " ");
        if !line.is_empty() {
            parts.push(line);
        }
    }
    parts.join(" ")
}

/// The blank lines where lines were taken out at [at], made one at most;
/// none at the note's start and none before its end, whose last line
/// break stays.
fn close_up(lines: &mut Vec<String>, at: usize) {
    let blank = |l: &String| l.trim().is_empty();
    let mut start = at.min(lines.len());
    while start > 0 && blank(&lines[start - 1]) {
        start -= 1;
    }
    let mut end = at.min(lines.len());
    while end < lines.len() && blank(&lines[end]) {
        end += 1;
    }
    if start == 0 && end == lines.len() {
        lines.clear();
        return;
    }
    let keep: Vec<String> = if start == 0 {
        vec![]
    } else if end == lines.len() {
        // The note's last line break: the empty last line, as it was.
        if lines.last().is_some_and(String::is_empty) { vec![String::new()] } else { vec![] }
    } else if end - start > 0 {
        vec![lines[start].clone()]
    } else {
        vec![]
    };
    lines.splice(start..end, keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "## 2026-10-29\n\n- Ask the host about a managed database\n- [ ] Plan the secret rotation\n    with dynamic keys\n- Try the charts\n\n## 2026-10-30\n\nRead the paper on queues.\nIt is about lag.\n\nTo do in the cluster:\n\n- the playbooks\n- the network policy\n\n```sh\nkubectl get pods\n```\n";

    fn texts(text: &str) -> Vec<(usize, String, Option<String>)> {
        Inbox::items(text).into_iter().map(|i| (i.line + 1, i.text, i.heading)).collect()
    }

    #[test]
    fn the_inbox_is_the_setting_or_scratchpad() {
        assert_eq!(Inbox::path(None), "Scratchpad.md");
        assert_eq!(Inbox::path(Some(" ")), "Scratchpad.md");
        assert_eq!(Inbox::path(Some("Inbox/Later")), "Inbox/Later.md");
        assert_eq!(Inbox::path(Some("/Inbox.md/")), "Inbox.md");
    }

    #[test]
    fn items_are_list_items_paragraphs_and_blocks() {
        let day = |d: &str| Some(d.to_string());
        assert_eq!(
            texts(NOTE),
            [
                (3, "Ask the host about a managed database".into(), day("2026-10-29")),
                (4, "[ ] Plan the secret rotation with dynamic keys".into(), day("2026-10-29")),
                (6, "Try the charts".into(), day("2026-10-29")),
                (10, "Read the paper on queues. It is about lag.".into(), day("2026-10-30")),
                (13, "To do in the cluster: - the playbooks - the network policy".into(), day("2026-10-30")),
                (18, "```sh kubectl get pods ```".into(), day("2026-10-30")),
            ]
        );
        let item = Inbox::item_at(NOTE, 3).unwrap();
        assert_eq!(item.lines, ["- [ ] Plan the secret rotation", "    with dynamic keys"]);
        assert!(Inbox::holds(&item, "[ ] Plan the secret rotation with dynamic keys"));
        assert!(Inbox::holds(&item, "- [ ] Plan the secret rotation"));
        assert!(!Inbox::holds(&item, "Try the charts"));
    }

    #[test]
    fn front_matter_rules_and_text_before_a_heading() {
        let text = "---\ntags: [inbox]\n---\nLoose line\n\n---\n\n## Later\n* one\n\n  still one\n\n* two\r\n";
        assert_eq!(
            texts(text),
            [(4, "Loose line".into(), None), (9, "one still one".into(), Some("Later".into())), (13, "two".into(), Some("Later".into())),]
        );
    }

    #[test]
    fn a_removed_item_takes_its_lines_and_one_blank_line() {
        let item = Inbox::item_at(NOTE, 3).unwrap();
        let after = Inbox::removed(NOTE, 3, &item.lines).unwrap();
        assert!(after.starts_with("## 2026-10-29\n\n- Ask the host about a managed database\n- Try the charts\n\n## 2026-10-30\n"));
        // A paragraph between blank lines leaves one.
        let item = Inbox::item_at(NOTE, 9).unwrap();
        let after = Inbox::removed(NOTE, 9, &item.lines).unwrap();
        assert!(after.contains("- Try the charts\n\n## 2026-10-30\n\nTo do in the cluster:\n"), "{after}");
        // The last item: the note still ends with one line break.
        let item = Inbox::item_at(NOTE, 17).unwrap();
        let after = Inbox::removed(NOTE, 17, &item.lines).unwrap();
        assert!(after.ends_with("- the network policy\n"), "{after:?}");
    }

    #[test]
    fn an_emptied_dated_section_goes_with_its_heading() {
        let text = "## 2026-10-29\n\n- one\n\n## 2026-10-30\n\n- two\n";
        let after = Inbox::removed(text, 2, &["- one".into()]).unwrap();
        assert_eq!(after, "## 2026-10-30\n\n- two\n");
        let after = Inbox::removed(text, 6, &["- two".into()]).unwrap();
        assert_eq!(after, "## 2026-10-29\n\n- one\n");
        assert_eq!(Inbox::removed("## 2026-10-29\n\n- one\n", 2, &["- one".into()]).unwrap(), "");
        // A section of another name stays, empty: it is the person's.
        assert_eq!(Inbox::removed("## Later\n\n- one\n", 2, &["- one".into()]).unwrap(), "## Later\n");
        // A dated section with a subsection left is not empty.
        let text = "## 2026-10-29\n- one\n### Ideas\n- two\n";
        assert_eq!(Inbox::removed(text, 1, &["- one".into()]).unwrap(), "## 2026-10-29\n### Ideas\n- two\n");
    }

    #[test]
    fn a_remove_is_refused_when_the_lines_changed() {
        let text = "## 2026-10-29\n\n- one\n- two\n";
        assert_eq!(Inbox::removed(text, 2, &["- two".into()]), Err(Failure::TextMoved));
        assert_eq!(Inbox::removed(text, 1, &["- one".into()]), Err(Failure::TextMoved));
        // An item that got a line under it is not the item listed.
        assert_eq!(Inbox::removed("- one\n  more\n", 0, &["- one".into()]), Err(Failure::TextMoved));
        assert_eq!(Inbox::removed(text, 9, &["- one".into()]), Err(Failure::TextMoved));
    }

    #[test]
    fn an_item_is_added_whole_at_the_end_or_under_a_heading() {
        let item = vec!["- [ ] Plan".to_string(), "    with keys".to_string()];
        assert_eq!(Inbox::added("# Vault\n\nText.\n", &item, None), "# Vault\n\nText.\n\n- [ ] Plan\n    with keys\n");
        assert_eq!(Inbox::added("## A\n- x\n\n## B\n- y\n", &item, Some(0)), "## A\n- x\n- [ ] Plan\n    with keys\n\n## B\n- y\n");
        assert_eq!(Inbox::added_under("## A\n- x\n", &item, "b"), "## A\n- x\n\n## b\n\n- [ ] Plan\n    with keys\n");
        assert_eq!(Inbox::added("- x\r\n", &item, None), "- x\r\n- [ ] Plan\r\n    with keys\r\n");
    }

    #[test]
    fn calendar_notes_and_the_inbox_take_no_item() {
        assert!(Inbox::may_receive("Tasks/Work.md", "Scratchpad.md"));
        assert!(!Inbox::may_receive("Calendar/2026-W44.md", "Scratchpad.md"));
        assert!(!Inbox::may_receive("scratchpad.md", "Scratchpad.md"));
    }

    /// Every line that is not the item stays, in order, whatever the note.
    #[test]
    fn a_remove_keeps_every_other_filled_line() {
        let words = ["## 2026-10-29", "## Later", "text", "", "- a", "  b", "```", "---", "1. x", "end:"];
        let mut state: u64 = 7;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..500 {
            let text: Vec<&str> = (0..next(14)).map(|_| words[next(words.len())]).collect();
            let text = text.join("\n");
            for item in Inbox::items(&text) {
                let after = Inbox::removed(&text, item.line, &item.lines).unwrap();
                let lines: Vec<&str> = text.split('\n').collect();
                let kept: Vec<&str> = lines
                    .iter()
                    .enumerate()
                    .filter(|(at, l)| {
                        (*at < item.line || *at >= item.line + item.lines.len())
                            && !l.trim().is_empty()
                            && !DAY.is_match(l.trim_start_matches("## "))
                    })
                    .map(|(_, l)| *l)
                    .collect();
                let mut rest = after.split('\n');
                for line in kept {
                    assert!(rest.any(|l| l == line), "{text:?} {item:?} {after:?}");
                }
            }
        }
    }
}
