//! What the editor colours in a note's source: one span per piece of
//! markdown, in **character** ranges so a GtkTextBuffer can take them as they
//! are (the Mac uses UTF-16 ranges for NSTextStorage). The markup itself
//! (`**`, `#`, `[[`) is a span of its own, so the editor can dim it while the
//! text it marks stands out.
//!
//! Line based, like the parser: a fenced block, the front matter and a
//! multi-line comment are the only state carried from one line to the next,
//! and `block_state` finds that state for any line, so a paragraph can be
//! coloured again on its own after a keystroke.

use std::ops::Range;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::editor_commands::line_range;
use crate::task_line::TaskReason;
use crate::task_status::{TaskStatuses, TaskType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Heading(u8),
    Markup,
    Bold,
    Italic,
    Strike,
    Highlight,
    Code,
    CodeBlock,
    Fence,
    Link,
    Url,
    Tag,
    Embed,
    TaskOpen,
    TaskDone,
    TaskCancelled,
    TaskOther,
    ListMarker,
    Quote,
    CalloutTitle,
    FrontMatter,
    Comment,
    Math,
    Rule,
    BlockId,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    pub kind: Kind,
    /// Characters, from the start of the text.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockState {
    Normal,
    FrontMatter,
    /// The fence that opened it, as ```` ``` ```` or `~~~~`.
    Fence(String),
    Comment,
    Math,
}

fn make(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap()
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| make(r"^(#{1,6})[ \t]"));
static RULE: LazyLock<Regex> = LazyLock::new(|| make(r"^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$"));
static QUOTE: LazyLock<Regex> = LazyLock::new(|| make(r"^(?:[ \t]*>)+[ \t]?"));
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| make(r"^(\[![\w-]+\][+-]?)"));
// The Mac's `\G`: these are matched on the line from where the quote ends.
static TASK: LazyLock<Regex> = LazyLock::new(|| make(r"^([ \t]*(?:[-*+]|\d+[.)])[ \t]+)(\[([^\]])\])"));
static LIST: LazyLock<Regex> = LazyLock::new(|| make(r"^([ \t]*(?:[-*+]|\d+[.)]))[ \t]+"));
static TASK_METADATA: LazyLock<Regex> =
    LazyLock::new(|| make(r"(?:✅|➕|⏳|❌|📅|🔁|⛔|🛫|🆔|🔺|⏫|🔼|🔽|⏬)(?:[ \t]*[0-9A-Za-z/,:.-]+(?:[ \t]+[a-z]+)*)?"));
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| make(r"(`+)(?:(?!\1).)+?\1"));
static COMMENT: LazyLock<Regex> = LazyLock::new(|| make(r"%%.*?%%"));
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| make(r"(!?)\[\[([^\]\n]+?)\]\]"));
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| make(r"!?(\[[^\]\n]*\])(\([^)\n]*\))"));
static AUTOLINK: LazyLock<Regex> = LazyLock::new(|| make(r"<(?:https?|mailto):[^>\s]+>"));
static BARE_URL: LazyLock<Regex> = LazyLock::new(|| make(r"(?<![<(])\bhttps?://[^\s<>)\]]+"));
static BOLD_ITALIC: LazyLock<Regex> = LazyLock::new(|| make(r"((\*\*\*|___)(?=\S)(?:.+?)(?<=\S)\2)"));
static BOLD: LazyLock<Regex> = LazyLock::new(|| make(r"((?:\*\*|__)(?=\S)(?:.+?)(?<=\S)(?:\*\*|__))"));
static ITALIC: LazyLock<Regex> = LazyLock::new(|| make(r"(?<![*_\w])((?:\*|_)(?=[^\s*_])(?:[^*_\n]+?)(?<=[^\s*_])(?:\*|_))(?![*_\w])"));
static STRIKE: LazyLock<Regex> = LazyLock::new(|| make(r"(~~(?=\S).+?(?<=\S)~~)"));
static HIGHLIGHT: LazyLock<Regex> = LazyLock::new(|| make(r"(==(?=\S).+?(?<=\S)==)"));
static INLINE_MATH: LazyLock<Regex> = LazyLock::new(|| make(r"(?<![\\$])\$(?=\S)[^$\n]+?(?<=\S)\$(?!\d)"));
static TAG: LazyLock<Regex> = LazyLock::new(|| make(r"(?:^|\s)(#[\p{L}\p{N}_/-]*[\p{L}_/-][\p{L}\p{N}_/-]*)"));
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| make(r"\s(\^[A-Za-z0-9-]+)\s*$"));

pub struct MarkdownHighlighter;

impl MarkdownHighlighter {
    /// The state the line starting at character [location] begins in. Read
    /// line by line in place: only a line that can open or close a block is
    /// looked at closely, so the end of a long note is found quickly.
    pub fn block_state(location: usize, text: &str) -> BlockState {
        let mut state = BlockState::Normal;
        let mut at = 0;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            if at >= location {
                break;
            }
            at += line.chars().count();
            let plain = state == BlockState::Normal && index > 0 && !line.contains(['`', '~', '$', '%']);
            if !plain {
                state = Self::next(line.trim_end_matches(['\n', '\r']), &state, index == 0);
            }
        }
        state
    }

    /// The spans of [lines], whole lines of a note that start at character
    /// [offset], in the block [state] the first of them begins in; and the
    /// state after the last. A note styled a part at a time carries the
    /// state from one part to the next instead of reading it from the top.
    pub fn spans_of_lines(lines: &str, offset: usize, state: BlockState) -> (Vec<Span>, BlockState) {
        let chars: Vec<char> = lines.chars().collect();
        let mut state = state;
        let mut out = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let range = line_range(&chars, start..start);
            if range.is_empty() {
                break;
            }
            let line = content(&chars, &range);
            let before = state.clone();
            state = Self::next(&line, &state, offset + range.start == 0);
            style(&line, offset + range.start, &before, &state, &mut out);
            start = range.end;
        }
        (out, state)
    }

    fn state_before(location: usize, chars: &[char]) -> BlockState {
        let mut state = BlockState::Normal;
        let mut start = 0;
        let mut first = true;
        while start < location && start < chars.len() {
            let range = line_range(chars, start..start);
            state = Self::next(&content(chars, &range), &state, first);
            first = false;
            start = range.end;
        }
        state
    }

    fn next(line: &str, state: &BlockState, first_line: bool) -> BlockState {
        let trimmed = line.trim_matches([' ', '\t']);
        match state {
            BlockState::FrontMatter => {
                if trimmed == "---" || trimmed == "..." {
                    BlockState::Normal
                } else {
                    BlockState::FrontMatter
                }
            }
            BlockState::Fence(marker) => {
                let mark = marker.chars().next().unwrap();
                let run = trimmed.chars().take_while(|c| *c == mark).count();
                let closes = run >= marker.chars().count() && trimmed.trim_start_matches(mark).trim().is_empty();
                if closes { BlockState::Normal } else { state.clone() }
            }
            BlockState::Comment => {
                if line.matches("%%").count() % 2 == 1 {
                    BlockState::Normal
                } else {
                    BlockState::Comment
                }
            }
            BlockState::Math => {
                if trimmed.ends_with("$$") {
                    BlockState::Normal
                } else {
                    BlockState::Math
                }
            }
            BlockState::Normal => {
                if first_line && trimmed == "---" {
                    return BlockState::FrontMatter;
                }
                let quoteless = trimmed.trim_start_matches(['>', ' ']);
                if let Some(marker) = fence_marker(quoteless) {
                    return BlockState::Fence(marker);
                }
                if trimmed.starts_with("$$") && !(trimmed.chars().count() > 2 && trimmed[2..].ends_with("$$")) {
                    return BlockState::Math;
                }
                if line.matches("%%").count() % 2 == 1 {
                    return BlockState::Comment;
                }
                BlockState::Normal
            }
        }
    }

    /// The spans of every line that overlaps the characters in [range].
    pub fn spans_in(text: &str, range: Range<usize>) -> Vec<Span> {
        let chars: Vec<char> = text.chars().collect();
        let lines = line_range(&chars, range.start.min(chars.len())..range.end.min(chars.len()));
        let mut state = Self::state_before(lines.start, &chars);
        let mut out = Vec::new();
        let mut start = lines.start;
        while start < lines.end {
            let range = line_range(&chars, start..start);
            if range.is_empty() {
                break;
            }
            let line = content(&chars, &range);
            let before = state.clone();
            state = Self::next(&line, &state, range.start == 0);
            style(&line, range.start, &before, &state, &mut out);
            start = range.end;
        }
        out
    }

    pub fn spans(text: &str) -> Vec<Span> {
        Self::spans_in(text, 0..text.chars().count())
    }
}

/// The line in [range] without its line ending.
fn content(chars: &[char], range: &Range<usize>) -> String {
    let line: String = chars[range.clone()].iter().collect();
    line.trim_end_matches(['\n', '\r']).to_string()
}

fn fence_marker(line: &str) -> Option<String> {
    for mark in ['`', '~'] {
        if line.starts_with(&mark.to_string().repeat(3)) {
            return Some(line.chars().take_while(|c| *c == mark).collect());
        }
    }
    None
}

/// Byte ranges in one line, turned into character ranges in the text.
struct Line<'a> {
    text: &'a str,
    start: usize,
}

impl Line<'_> {
    fn at(&self, bytes: Range<usize>) -> Range<usize> {
        let s = self.start + self.text[..bytes.start].chars().count();
        s..s + self.text[bytes].chars().count()
    }
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn each(pattern: &Regex, text: &str, mut body: impl FnMut(&fancy_regex::Captures<'_, str>)) {
    for caps in pattern.captures_iter(text).flatten() {
        body(&caps);
    }
}

fn style(line: &str, start: usize, before: &BlockState, after: &BlockState, out: &mut Vec<Span>) {
    if line.is_empty() {
        return;
    }
    let l = Line { text: line, start };
    let whole = l.at(0..line.len());
    let mut push = |kind: Kind, range: Range<usize>| out.push(Span { kind, range });
    match before {
        BlockState::FrontMatter => return push(Kind::FrontMatter, whole),
        BlockState::Fence(_) => return push(if *after == BlockState::Normal { Kind::Fence } else { Kind::CodeBlock }, whole),
        BlockState::Comment => return push(Kind::Comment, whole),
        BlockState::Math => return push(Kind::Math, whole),
        BlockState::Normal => {}
    }
    match after {
        BlockState::FrontMatter => return push(Kind::FrontMatter, whole),
        BlockState::Fence(_) => return push(Kind::Fence, whole),
        _ => {}
    }
    if *after == BlockState::Math || line.trim_matches([' ', '\t']).starts_with("$$") {
        return push(Kind::Math, whole);
    }
    if *after == BlockState::Comment {
        // A comment opens on this line and runs past it: from its `%%` on.
        let open = line.rfind("%%").unwrap_or(0);
        push(Kind::Comment, l.at(open..line.len()));
    }

    // Block-level: headings, quotes, lists, tasks, rules.
    if let Ok(Some(caps)) = HEADING.captures(line) {
        let marks = caps.get(1).unwrap();
        push(Kind::Heading(marks.as_str().len() as u8), whole.clone());
        push(Kind::Markup, l.at(marks.range()));
    }
    if RULE.is_match(line).unwrap_or(false) {
        return push(Kind::Rule, whole);
    }
    let mut body = 0;
    if let Ok(Some(found)) = QUOTE.find(line) {
        push(Kind::Quote, whole.clone());
        push(Kind::Markup, l.at(found.range()));
        body = found.end();
        if let Ok(Some(callout)) = CALLOUT.captures(&line[body..]) {
            push(Kind::CalloutTitle, l.at(body..line.len()));
            let marks = callout.get(1).unwrap();
            push(Kind::Markup, l.at(body + marks.start()..body + marks.end()));
        }
    }
    let rest = &line[body..];
    if let Ok(Some(caps)) = TASK.captures(rest) {
        let shift = |r: Range<usize>| body + r.start..body + r.end;
        push(Kind::ListMarker, l.at(shift(caps.get(1).unwrap().range())));
        // By the type the vault's Tasks plugin settings give the status.
        let symbol = caps.get(3).unwrap().as_str().chars().next().unwrap_or(' ');
        let kind = match (symbol, TaskStatuses::current().kind(symbol)) {
            (' ', _) => Kind::TaskOpen,
            (_, TaskType::Done) => Kind::TaskDone,
            (_, TaskType::Cancelled | TaskType::NonTask) => Kind::TaskCancelled,
            _ => Kind::TaskOther,
        };
        push(kind, l.at(shift(caps.get(2).unwrap().range())));
        let end = body + caps.get(0).unwrap().end();
        if matches!(kind, Kind::TaskDone | Kind::TaskCancelled) && end < line.len() {
            push(kind, l.at(end..line.len()));
        }
        each(&TASK_METADATA, line, |m| push(Kind::Metadata, l.at(m.get(0).unwrap().range())));
        if let Some(reason) = TaskReason::range(&line[end..]) {
            push(Kind::Metadata, l.at(end + reason.start..end + reason.end));
        }
    } else if let Ok(Some(caps)) = LIST.captures(rest) {
        let marker = caps.get(1).unwrap();
        push(Kind::ListMarker, l.at(body + marker.start()..body + marker.end()));
    }

    // Inline: code first, so that markup inside code is code. Ranges are in
    // bytes of the line until they are pushed.
    let mut code: Vec<Range<usize>> = Vec::new();
    each(&INLINE_CODE, line, |m| {
        let all = m.get(0).unwrap().range();
        let ticks = m.get(1).unwrap().as_str().len();
        push(Kind::Code, l.at(all.clone()));
        push(Kind::Markup, l.at(all.start..all.start + ticks));
        push(Kind::Markup, l.at(all.end - ticks..all.end));
        code.push(all);
    });
    let free = |r: &Range<usize>| !code.iter().any(|c| overlaps(c, r));

    each(&COMMENT, line, |m| {
        let r = m.get(0).unwrap().range();
        if free(&r) {
            push(Kind::Comment, l.at(r));
        }
    });
    each(&WIKILINK, line, |m| {
        let all = m.get(0).unwrap().range();
        if !free(&all) {
            return;
        }
        let bang = m.get(1).unwrap().as_str().len();
        push(if bang > 0 { Kind::Embed } else { Kind::Link }, l.at(all.clone()));
        push(Kind::Markup, l.at(all.start..all.start + bang + 2));
        push(Kind::Markup, l.at(all.end - 2..all.end));
        // `[[target|label]]`: the target is markup too; the label is what reads.
        let inner = m.get(2).unwrap();
        if let Some(bar) = inner.as_str().find('|') {
            push(Kind::Markup, l.at(inner.start()..inner.start() + bar + 1));
        }
    });
    each(&MARKDOWN_LINK, line, |m| {
        if free(&m.get(0).unwrap().range()) {
            push(Kind::Link, l.at(m.get(1).unwrap().range()));
            push(Kind::Url, l.at(m.get(2).unwrap().range()));
        }
    });
    for pattern in [&AUTOLINK, &BARE_URL] {
        each(pattern, line, |m| {
            let r = m.get(0).unwrap().range();
            if free(&r) {
                push(Kind::Url, l.at(r));
            }
        });
    }
    // `***both***` is bold and italic at once; its runs of three are no
    // one else's, so `**` and `*` do not match half of them.
    let mut triples: Vec<Range<usize>> = Vec::new();
    each(&BOLD_ITALIC, line, |m| {
        let all = m.get(1).unwrap().range();
        if !free(&all) {
            return;
        }
        push(Kind::Bold, l.at(all.clone()));
        push(Kind::Italic, l.at(all.clone()));
        let (open, close) = (all.start..all.start + 3, all.end - 3..all.end);
        push(Kind::Markup, l.at(open.clone()));
        push(Kind::Markup, l.at(close.clone()));
        triples.extend([open, close]);
    });
    for (pattern, kind, width) in
        [(&BOLD, Kind::Bold, 2), (&STRIKE, Kind::Strike, 2), (&HIGHLIGHT, Kind::Highlight, 2), (&ITALIC, Kind::Italic, 1)]
    {
        each(pattern, line, |m| {
            let all = m.get(1).unwrap().range();
            let (open, close) = (all.start..all.start + width, all.end - width..all.end);
            // Code may be inside (`==the `x` flag==`); the marks may not.
            if !free(&open) || !free(&close) {
                return;
            }
            if matches!(kind, Kind::Bold | Kind::Italic) && [&open, &close].iter().any(|end| triples.iter().any(|t| overlaps(t, end))) {
                return;
            }
            push(kind, l.at(all.clone()));
            push(Kind::Markup, l.at(open));
            push(Kind::Markup, l.at(close));
        });
    }
    each(&INLINE_MATH, line, |m| {
        let r = m.get(0).unwrap().range();
        if free(&r) {
            push(Kind::Math, l.at(r));
        }
    });
    each(&TAG, line, |m| {
        let r = m.get(1).unwrap().range();
        if free(&r) {
            push(Kind::Tag, l.at(r));
        }
    });
    each(&BLOCK_ID, line, |m| push(Kind::BlockId, l.at(m.get(1).unwrap().range())));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_note_styled_in_parts_is_styled_as_a_whole() {
        let text = "---\ntags: [a]\n---\n# T\n**b** and `c`\n```rust\nlet x = 1;\n```\n$$\nx^2\n$$\na %% hidden\nstill %% b\n- [ ] task\n";
        let whole = MarkdownHighlighter::spans(text);
        // In parts of three lines, the state carried from part to part.
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let (mut parts, mut state, mut offset) = (Vec::new(), BlockState::Normal, 0);
        for chunk in lines.chunks(3) {
            let joined = chunk.concat();
            let (spans, next) = MarkdownHighlighter::spans_of_lines(&joined, offset, state);
            parts.extend(spans);
            state = next;
            offset += joined.chars().count();
        }
        assert_eq!(parts, whole);
        // The state each line starts in, read from the top, matches.
        let mut at = 0;
        let mut carried = BlockState::Normal;
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(MarkdownHighlighter::block_state(at, text), carried, "line {index}");
            let (_, next) = MarkdownHighlighter::spans_of_lines(line, at, carried);
            carried = next;
            at += line.chars().count();
        }
    }

    use super::*;
    use std::collections::HashMap;

    fn kinds(text: &str) -> HashMap<Kind, Vec<String>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out: HashMap<Kind, Vec<String>> = HashMap::new();
        for span in MarkdownHighlighter::spans(text) {
            out.entry(span.kind).or_default().push(chars[span.range].iter().collect());
        }
        out
    }

    fn get(found: &HashMap<Kind, Vec<String>>, kind: Kind) -> Option<Vec<&str>> {
        found.get(&kind).map(|v| v.iter().map(String::as_str).collect())
    }

    #[test]
    fn bold_and_italic_with_highlights() {
        let inside = kinds("==a **bold** and *it* word==");
        assert_eq!(get(&inside, Kind::Highlight), Some(vec!["==a **bold** and *it* word=="]));
        assert_eq!(get(&inside, Kind::Bold), Some(vec!["**bold**"]));
        assert_eq!(get(&inside, Kind::Italic), Some(vec!["*it*"]));
        let both = kinds("***==both==*** and **==mark==**");
        assert_eq!(get(&both, Kind::Bold), Some(vec!["***==both==***", "**==mark==**"]));
        assert_eq!(get(&both, Kind::Italic), Some(vec!["***==both==***"]));
        assert_eq!(get(&both, Kind::Highlight), Some(vec!["==both==", "==mark=="]));
        assert_eq!(both[&Kind::Markup].iter().filter(|m| *m == "***").count(), 2);
        assert_eq!(get(&kinds("___under___"), Kind::Italic), Some(vec!["___under___"]));
        // Code inside a highlight; marks inside code are not marks.
        assert_eq!(get(&kinds("==the `x` flag=="), Kind::Highlight), Some(vec!["==the `x` flag=="]));
        assert_eq!(get(&kinds("`a*b` and `c*d`"), Kind::Italic), None);
        assert_eq!(get(&kinds("`==a` b=="), Kind::Highlight), None);
    }

    #[test]
    fn headings_and_inline() {
        let text = "## Title **bold** and *it* with `co**de**` and [[Note|label]] #tag";
        let found = kinds(text);
        assert_eq!(get(&found, Kind::Heading(2)), Some(vec![text]));
        assert_eq!(get(&found, Kind::Bold), Some(vec!["**bold**"]));
        assert_eq!(get(&found, Kind::Italic), Some(vec!["*it*"]));
        assert_eq!(get(&found, Kind::Code), Some(vec!["`co**de**`"]));
        assert_eq!(get(&found, Kind::Link), Some(vec!["[[Note|label]]"]));
        assert_eq!(get(&found, Kind::Tag), Some(vec!["#tag"]));
        assert!(found[&Kind::Markup].iter().any(|m| m == "Note|"));
    }

    #[test]
    fn tasks_and_metadata() {
        let found = kinds("- [x] Done thing ✅ 2026-10-05\n    - [ ] Open ⏳ 2026-10-06");
        assert_eq!(found[&Kind::TaskDone][0], "[x]");
        assert_eq!(get(&found, Kind::TaskOpen), Some(vec!["[ ]"]));
        assert_eq!(get(&found, Kind::Metadata), Some(vec!["✅ 2026-10-05", "⏳ 2026-10-06"]));
        assert_eq!(get(&found, Kind::ListMarker), Some(vec!["- ", "    - "]));
    }

    #[test]
    fn blocks_carry_state() {
        let text = "---\ntags: [a]\n---\n```swift\nlet **x** = 1\n```\n%%\nhidden\n%%\nafter";
        let found = kinds(text);
        assert_eq!(get(&found, Kind::FrontMatter), Some(vec!["---", "tags: [a]", "---"]));
        assert_eq!(get(&found, Kind::CodeBlock), Some(vec!["let **x** = 1"]));
        assert_eq!(get(&found, Kind::Fence), Some(vec!["```swift", "```"]));
        assert_eq!(get(&found, Kind::Comment), Some(vec!["%%", "hidden", "%%"]));
        assert_eq!(get(&found, Kind::Bold), None);
        let at = |word: &str| text[..text.find(word).unwrap()].chars().count();
        assert_eq!(MarkdownHighlighter::block_state(at("let"), text), BlockState::Fence("```".into()));
        assert_eq!(MarkdownHighlighter::block_state(at("after"), text), BlockState::Normal);
    }

    #[test]
    fn callout_and_quote() {
        let found = kinds("> [!warning] Careful\n> body");
        assert_eq!(get(&found, Kind::CalloutTitle), Some(vec!["[!warning] Careful"]));
        assert_eq!(found[&Kind::Quote].len(), 2);
    }
}
