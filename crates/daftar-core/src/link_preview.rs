//! The link preview (`docs/specs/look.md`, part 2): resting the pointer on
//! a `[[link]]` shows a small card with the note's title and its first
//! lines. What the card says is one pure rule over text, the same in the
//! reader and the editor of every app; how it is drawn is each app's.

use crate::note_parser::NoteParser;
use crate::sections::Sections;

/// What a `[[link]]` names: the note, and a heading or a block of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    /// As written before `#` and `|`; empty for a link into the same note.
    pub name: String,
    pub heading: Option<String>,
    /// Without its `^`.
    pub block: Option<String>,
}

/// The card's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewBody {
    /// The first lines, as Markdown.
    Lines(String),
    /// The note has no text: "The note is empty.".
    Empty,
    /// No note has the name: "No note yet.".
    Missing,
}

/// The card of one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCard {
    /// The note's title, or the link's name when there is no such note.
    pub title: String,
    /// The heading the link names, when the note has it.
    pub heading: Option<String>,
    /// The note's folder, as `A › B`; empty at the vault's root.
    pub folder: String,
    pub body: PreviewBody,
}

impl PreviewBody {
    /// What the card says where there are no lines to show.
    pub fn words(&self) -> Option<&'static str> {
        match self {
            PreviewBody::Lines(_) => None,
            PreviewBody::Empty => Some(LinkPreview::EMPTY),
            PreviewBody::Missing => Some(LinkPreview::MISSING),
        }
    }
}

pub struct LinkPreview;

impl LinkPreview {
    /// The pointer rests this long on a link before its card shows, and
    /// the card stays this long after the pointer left it, in ms.
    pub const REST: u64 = 350;
    pub const LINGER: u64 = 250;
    /// The most lines and characters of a note a card shows.
    pub const LINES: usize = 12;
    pub const CHARACTERS: usize = 600;
    pub const EMPTY: &'static str = "The note is empty.";
    pub const MISSING: &'static str = "No note yet.";

    /// What the text of a link names: `[[Plan#Next|the plan]]`, with or
    /// without its brackets. None for an embed (`![[…]]`) and for a link
    /// that names nothing.
    pub fn target(link: &str) -> Option<LinkTarget> {
        let link = link.trim();
        if link.starts_with('!') {
            return None;
        }
        let inner = link.strip_prefix("[[").unwrap_or(link);
        let inner = inner.strip_suffix("]]").unwrap_or(inner);
        // `\|` is the alias bar in a table's cell.
        let target = inner.split('|').next().unwrap_or("").trim_end_matches('\\');
        let (name, anchor) = target.split_once('#').map_or((target, None), |(name, anchor)| (name, Some(anchor)));
        // `#A#B` names B under A: the last heading is the one shown.
        let anchor = anchor.map(|anchor| anchor.rsplit('#').next().unwrap_or(anchor).trim()).filter(|anchor| !anchor.is_empty());
        let (heading, block) = match anchor {
            Some(anchor) => match anchor.strip_prefix('^') {
                Some(block) => (None, Some(block.to_string())),
                None => (Some(anchor.to_string()), None),
            },
            None => (None, None),
        };
        let name = name.trim().to_string();
        (!name.is_empty() || heading.is_some() || block.is_some()).then_some(LinkTarget { name, heading, block })
    }

    /// The card of a link to [target]: [path] is the note it resolves to
    /// and [text] that note's text, both None when no note has the name.
    pub fn card(target: &LinkTarget, path: Option<&str>, text: Option<&str>) -> LinkCard {
        let (Some(path), Some(text)) = (path, text) else {
            return LinkCard { title: target.name.clone(), heading: None, folder: String::new(), body: PreviewBody::Missing };
        };
        let title = crate::vault::Vault::title(path);
        let folder =
            path.rsplit_once('/').map_or_default(|(folder, _)| folder.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" › "));
        let section = target.heading.as_deref().and_then(|heading| Self::section(text, heading));
        let block = target.block.as_deref().and_then(|id| Self::block(text, id));
        let heading = section.as_ref().map(|(heading, _)| heading.clone());
        let lines = match (section, block) {
            (Some((_, lines)), _) => lines,
            (None, Some(block)) => block,
            (None, None) => Self::excerpt(text, &title),
        };
        let body = if lines.trim().is_empty() { PreviewBody::Empty } else { PreviewBody::Lines(lines) };
        LinkCard { title, heading, folder, body }
    }

    /// The first lines of a note: its text without its front matter and
    /// without a first heading whose text is [title], from the first line
    /// that is not blank, cut as `cut` cuts.
    pub fn excerpt(text: &str, title: &str) -> String {
        let mut lines: Vec<&str> = Self::body(text).lines().skip_while(|line| line.trim().is_empty()).collect();
        let is_title = lines.first().is_some_and(|line| {
            let line = line.trim();
            let text = line.trim_start_matches('#');
            let level = line.len() - text.len();
            (1..=6).contains(&level) && text.starts_with([' ', '\t']) && text.trim().trim_end_matches('#').trim() == title.trim()
        });
        if is_title {
            lines.remove(0);
        }
        Self::cut(&lines)
    }

    /// [text] without its front matter: the lines from a first `---` line
    /// to the `---` or `...` line that closes it.
    fn body(text: &str) -> &str {
        let mut lines = text.split_inclusive('\n');
        let bare = |line: &str| line.trim_end_matches(['\n', '\r']).trim_matches([' ', '\t']).to_string();
        let Some(first) = lines.next().filter(|first| bare(first) == "---") else { return text };
        let mut at = first.len();
        for line in lines {
            at += line.len();
            if matches!(bare(line).as_str(), "---" | "...") {
                return &text[at..];
            }
        }
        text
    }

    /// The lines under the heading [heading] of [text], without the
    /// heading itself, up to the next heading of its level or higher; with
    /// the heading as the note writes it. None when the note has no such heading.
    pub fn section(text: &str, heading: &str) -> Option<(String, String)> {
        let headings = NoteParser::parse(text).headings;
        let wanted = heading.trim().to_lowercase();
        let index = headings.iter().position(|h| h.text.trim().to_lowercase() == wanted)?;
        let lines: Vec<&str> = text.lines().collect();
        let end = Sections::end(index, &headings, lines.len()).min(lines.len());
        let under = lines.get(headings[index].line + 1..end).unwrap_or(&[]);
        Some((headings[index].text.clone(), Self::cut(under)))
    }

    /// The block with the id [id] (`^id` at the end of its last line, or on
    /// the line under it), alone and without the id.
    pub fn block(text: &str, id: &str) -> Option<String> {
        let mark = format!("^{id}");
        let lines: Vec<&str> = text.lines().collect();
        let at = lines.iter().position(|line| {
            let line = line.trim_end();
            line == mark || line.ends_with(&format!(" {mark}"))
        })?;
        let own = lines[at].trim_end();
        let last = own.strip_suffix(&mark).unwrap_or(own).trim_end();
        // An id on a line of its own names the block over it.
        let end = if last.is_empty() { at } else { at + 1 };
        let mut start = end.min(at);
        while start > 0 && !lines[start - 1].trim().is_empty() {
            start -= 1;
        }
        let mut block: Vec<&str> = lines[start..end].to_vec();
        if !last.is_empty() {
            block.pop();
            block.push(last);
        }
        Some(Self::cut(&block))
    }

    /// [lines] from the first that is not blank, up to `LINES` lines or
    /// `CHARACTERS` characters, whichever comes first: cut at a line's end
    /// when one is in reach, with `…` when text was left out. A code block
    /// that is cut is closed, and an embed is shown as its link.
    pub fn cut(lines: &[&str]) -> String {
        let lines: Vec<&str> = lines.iter().copied().skip_while(|line| line.trim().is_empty()).collect();
        let mut end = lines.len();
        while end > 0 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let lines = &lines[..end];
        let mut kept: Vec<String> = Vec::new();
        let (mut count, mut left_out, mut whole) = (0, false, true);
        for line in lines {
            let length = line.chars().count();
            // With the line break before it.
            let needed = length + usize::from(!kept.is_empty());
            if kept.len() == Self::LINES || count + needed > Self::CHARACTERS {
                left_out = true;
                if kept.is_empty() {
                    // One line longer than the card holds: its start.
                    kept.push(line.chars().take(Self::CHARACTERS).collect());
                    whole = false;
                }
                break;
            }
            count += needed;
            kept.push(Self::without_embeds(line));
        }
        // A fence the cut left open is closed, so the code does not run on.
        let mut open: Option<String> = None;
        for line in &kept {
            let rest = line.trim_start_matches([' ', '\t', '>']);
            let mark: String = rest.chars().take_while(|c| *c == '`' || *c == '~').collect();
            match &open {
                None if mark.len() >= 3 => open = Some(mark),
                Some(fence) if mark.len() >= fence.len() && mark.starts_with(fence.as_str()) && rest[mark.len()..].trim().is_empty() => {
                    open = None;
                }
                _ => {}
            }
        }
        let mut out = kept.join("\n");
        if let Some(fence) = open {
            out.push('\n');
            out.push_str(&fence);
        }
        if left_out {
            out.push_str(if whole { "\n…" } else { "…" });
        }
        out
    }

    /// `![[note]]` as `[[note]]`: an embed in a card is shown as its link, not followed.
    fn without_embeds(line: &str) -> String {
        line.replace("![[", "[[")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(body: &PreviewBody) -> &str {
        match body {
            PreviewBody::Lines(lines) => lines,
            other => panic!("no lines: {other:?}"),
        }
    }

    // docs/specs/look.md, part 2, case 18.
    #[test]
    fn the_preview_is_the_first_lines() {
        let twenty: String = (1..=20).map(|n| format!("l{n}\n")).collect();
        let plan = format!("---\ntags: [a]\n---\n# Plan\n\n{twenty}\n## Next\n\nn1\nn2\n### Deeper\nn3\n## After\nn4\n");
        let twelve: String = (1..=12).map(|n| format!("l{n}\n")).collect();
        assert_eq!(LinkPreview::excerpt(&plan, "Plan"), format!("{twelve}…"));
        // A first heading that is not the title stays.
        assert!(LinkPreview::excerpt("# Other\n\ntext\n", "Plan").starts_with("# Other\n"));

        // A link to a heading: the lines under it, up to the next heading of its level or higher.
        let target = LinkPreview::target("[[Plan#Next]]").unwrap();
        assert_eq!(target, LinkTarget { name: "Plan".into(), heading: Some("Next".into()), block: None });
        let card = LinkPreview::card(&target, Some("Work/Plans/Plan.md"), Some(&plan));
        assert_eq!((card.title.as_str(), card.heading.as_deref(), card.folder.as_str()), ("Plan", Some("Next"), "Work › Plans"));
        assert_eq!(lines(&card.body), "n1\nn2\n### Deeper\nn3");

        // One line of 900 letters: 600 and the mark.
        let long = "x".repeat(900);
        assert_eq!(LinkPreview::excerpt(&long, "Long"), format!("{}…", "x".repeat(600)));
        // A line's end in reach: the cut is there.
        let two = format!("{}\n{}\n", "a".repeat(500), "b".repeat(200));
        assert_eq!(LinkPreview::excerpt(&two, "Two"), format!("{}\n…", "a".repeat(500)));

        // A note that is only a front matter is empty, and a name with no note says so.
        let empty = LinkPreview::card(&LinkPreview::target("Plan").unwrap(), Some("Plan.md"), Some("---\na: 1\n---\n"));
        assert_eq!((empty.body.clone(), empty.body.words()), (PreviewBody::Empty, Some("The note is empty.")));
        let missing = LinkPreview::card(&LinkPreview::target("[[Nowhere|there]]").unwrap(), None, None);
        assert_eq!((missing.title.as_str(), missing.body.words()), ("Nowhere", Some("No note yet.")));
    }

    #[test]
    fn a_link_names_a_note_a_heading_or_a_block() {
        let target = |link: &str| LinkPreview::target(link).map(|t| (t.name, t.heading, t.block));
        assert_eq!(target("[[Plan]]"), Some(("Plan".into(), None, None)));
        assert_eq!(target("[[Notes/Plan|the plan]]"), Some(("Notes/Plan".into(), None, None)));
        assert_eq!(target(r"[[Plan#Next\|x]]"), Some(("Plan".into(), Some("Next".into()), None)));
        assert_eq!(target("[[Plan#A#B]]"), Some(("Plan".into(), Some("B".into()), None)));
        assert_eq!(target("[[Plan#^abc]]"), Some(("Plan".into(), None, Some("abc".into()))));
        assert_eq!(target("[[#Here]]"), Some((String::new(), Some("Here".into()), None)));
        // An embed has no card, and neither has a link that names nothing.
        assert_eq!(target("![[Plan]]"), None);
        assert_eq!(target("[[]]"), None);
    }

    #[test]
    fn a_block_alone_and_a_cut_code_block_closed() {
        let text = "one\n\ntwo\nthree ^abc\n\nfour\n\nfive\nsix\n^def\n";
        assert_eq!(LinkPreview::block(text, "abc").as_deref(), Some("two\nthree"));
        assert_eq!(LinkPreview::block(text, "def").as_deref(), Some("five\nsix"));
        assert_eq!(LinkPreview::block(text, "nope"), None);
        let card = LinkPreview::card(&LinkPreview::target("[[N#^abc]]").unwrap(), Some("N.md"), Some(text));
        assert_eq!(lines(&card.body), "two\nthree");
        // A heading the note does not have: the note's first lines, and no heading in the head.
        let lost = LinkPreview::card(&LinkPreview::target("[[N#Gone]]").unwrap(), Some("N.md"), Some(text));
        assert_eq!((lost.heading, lines(&lost.body).starts_with("one")), (None, true));

        // A code block the cut goes through is closed before the mark.
        let code: String = (1..=20).map(|n| format!("c{n}\n")).collect();
        let cut = LinkPreview::excerpt(&format!("intro\n```rust\n{code}```\n"), "Code");
        assert!(cut.ends_with("c10\n```\n…"), "{cut}");
        assert_eq!(cut.lines().count(), 14);
        // One that fits is left as it is, and an embed is shown as its link.
        assert_eq!(LinkPreview::excerpt("```\na\n```\n![[Other]] and ![[pic.png]]\n", "X"), "```\na\n```\n[[Other]] and [[pic.png]]");
        assert_eq!(LinkPreview::excerpt("\n\n  \n", "X"), "");
    }
}
