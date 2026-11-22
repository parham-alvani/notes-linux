//! What live preview draws over a note's text in the editor, part 2 of
//! `docs/specs/look.md`: a task's box, the date chips, a callout's panel, a
//! code block's panel and a table's grid. Every rule here is over text only, in **character**
//! ranges, so an app with any toolkit draws the same parts. A drawn part is a
//! way to see the text, never the text: nothing here changes a line but
//! `click`, which is the task toggle.
//!
//! Line based, as the highlighter is: `parts` takes whole lines and the block
//! state the first begins in, so a paragraph is read again on its own after a
//! key. What it cannot see from those lines (the callout the first line
//! continues, whether a quote goes on under the last) is given by the caller.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::day::Day;
use crate::markdown_highlighter::{BlockState, MarkdownHighlighter, inline_code};
use crate::markdown_table::MarkdownTable;
use crate::task_line::TaskLine;
use crate::task_status::{TaskStatuses, TaskType};

/// A task's box: the characters it is drawn over, and the status in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBox {
    pub range: Range<usize>,
    pub symbol: char,
}

/// How a status is drawn: a Lucide icon in one colour of the palette, named
/// as the reader's CSS names it (`list-marker`, `done-color`, `accent`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxLook {
    pub icon: &'static str,
    pub color: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChipKind {
    Due,
    Scheduled,
    Start,
    Created,
    Done,
    Cancelled,
    Repeat,
}

impl ChipKind {
    fn of(emoji: char) -> Option<ChipKind> {
        Some(match emoji {
            '📅' => ChipKind::Due,
            '⏳' => ChipKind::Scheduled,
            '🛫' => ChipKind::Start,
            '➕' => ChipKind::Created,
            '✅' => ChipKind::Done,
            '❌' => ChipKind::Cancelled,
            '🔁' => ChipKind::Repeat,
            _ => return None,
        })
    }

    /// The chip's colour, by the reader's names.
    pub fn color(self, late: bool) -> &'static str {
        match self {
            ChipKind::Due if late => "red",
            ChipKind::Due => "orange",
            ChipKind::Scheduled => "blue",
            ChipKind::Start => "purple",
            ChipKind::Created => "text-muted",
            ChipKind::Done => "spring-green",
            ChipKind::Cancelled => "red",
            ChipKind::Repeat => "pink",
        }
    }
}

/// A date on a task line, from its emoji to its last character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub kind: ChipKind,
    pub range: Range<usize>,
    /// A due date before today on an open task.
    pub late: bool,
    /// On a closed task, where the chips are drawn weaker.
    pub closed: bool,
}

/// The first line of a callout: `> [!kind]`, with `+` or `-` and a title or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalloutHead {
    /// The `> ` at the line's start.
    pub quote: Range<usize>,
    /// `[!kind]`, its `+` or `-`, and the spaces before the title.
    pub mark: Range<usize>,
    /// The kind it is drawn as: the one written, or `note` for one the reader does not know.
    pub kind: String,
    /// The title drawn: the one written, else the kind's name with a capital.
    pub title: String,
    /// False when the title is the kind's name, which is drawn only.
    pub written: bool,
}

/// A fence line: its fence characters with the spaces after them, and the language word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    pub marks: Range<usize>,
    pub word: Option<Range<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRole {
    Open,
    Body,
    Close,
}

/// What the line above the first line given is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Above {
    /// No quote: a blank line, a paragraph, the start of the note.
    #[default]
    Nothing,
    /// A `>` line of a quote with no `[!kind]`.
    Quote,
    /// A `>` line of a callout of this kind (as drawn).
    Callout(String),
}

/// A line that is in a panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Callout { kind: String, head: Option<CalloutHead>, quote: Range<usize>, last: bool },
    Code { role: CodeRole, fence: Option<Fence> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLine {
    /// The line's characters, without its line ending.
    pub range: Range<usize>,
    pub block: Block,
}

/// The drawn parts of some lines, every range in characters of the note.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parts {
    pub boxes: Vec<TaskBox>,
    pub chips: Vec<Chip>,
    pub lines: Vec<BlockLine>,
    /// The lines that are a table's rows in Markdown, without their line
    /// endings: outside a quote, a list item and a code block. `grid_lines`
    /// says which of them are drawn as a grid.
    pub rows: Vec<Range<usize>>,
}

/// What a line of a grid is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridRole {
    Header,
    /// `| --- | :-: |`: not drawn as text, it is the hairline under the header.
    Delimiter,
    Row,
}

/// One line of a table drawn as a grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridLine {
    /// The line's characters, without its line ending.
    pub range: Range<usize>,
    pub role: GridRole,
    /// The table's last line: the box ends under it.
    pub last: bool,
    /// The pipes between its cells: each is drawn as a hairline, not as a character.
    pub pipes: Vec<usize>,
    /// The table's widest line, in the columns of a monospaced line: an
    /// app that cannot scroll a table sideways draws one wider than its
    /// measure as before.
    pub widest: usize,
}

/// One panel of a note: its lines, from 0, and what it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub lines: Range<usize>,
    pub kind: PanelKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKind {
    Callout { kind: String, title: String },
    Code { word: Option<String> },
}

/// The editor's line and the room at each side of it, in px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    pub line: f64,
    pub room: f64,
}

// Any indent, `>` marks each with its space, a list marker, one space, `[`,
// one character, `]`, one space.
static TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([ \t]*(?:>[ \t]?)*[ \t]*)([-*+]|\d+[.)]) \[([^\]\n])\] ").unwrap());
static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[ \t]*>)+[ \t]?").unwrap());
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[!([\w-]+)\][+-]?[ \t]*").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*((?:`{3,}|~{3,})[ \t]*)([^\s`]+)?").unwrap());
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\^[A-Za-z0-9-]+\s*$").unwrap());

/// The reader's table (`web/build.mjs`): a status's icon and colour.
const STATUSES: [(char, &str, &str); 23] = [
    (' ', "square", "list-marker"),
    ('x', "square-check-big", "done-color"),
    ('X', "square-check-big", "done-color"),
    ('-', "square-x", "done-color"),
    ('/', "square-dot", "accent"),
    ('>', "square-arrow-right", "blue"),
    ('<', "calendar-clock", "blue"),
    ('!', "circle-alert", "red"),
    ('?', "circle-help", "yellow"),
    ('*', "star", "vivid-yellow"),
    ('"', "quote", "warm-grey"),
    ('i', "info", "blue"),
    ('b', "bookmark", "orange"),
    ('l', "map-pin", "red"),
    ('I', "lightbulb", "vivid-yellow"),
    ('S', "piggy-bank", "spring-green"),
    ('f', "flame", "red"),
    ('k', "key", "vivid-yellow"),
    ('w', "trophy", "vivid-yellow"),
    ('u', "thumbs-up", "spring-green"),
    ('p', "thumbs-up", "spring-green"),
    ('d', "thumbs-down", "red"),
    ('c', "thumbs-down", "red"),
];

/// Each callout kind's Lucide icon, as the reader has them (`web/build.mjs`).
const CALLOUTS: [(&[&str], &str); 15] = [
    (&["note"], "pencil"),
    (&["info"], "info"),
    (&["abstract", "summary", "tldr"], "clipboard-list"),
    (&["todo"], "circle-check"),
    (&["tip", "hint", "important"], "flame"),
    (&["success", "check", "done"], "check"),
    (&["question", "help", "faq"], "circle-help"),
    (&["warning", "caution", "attention"], "triangle-alert"),
    (&["failure", "fail", "missing"], "x"),
    (&["danger", "error"], "zap"),
    (&["bug"], "bug"),
    (&["example"], "list"),
    (&["quote", "cite"], "quote"),
    (&["figure"], "image"),
    (&["read"], "book-open"),
];

const PRIORITIES: [char; 5] = ['🔺', '⏫', '🔼', '🔽', '⏬'];

/// The room at each side of the editor's line is never less than this.
const LEAST_ROOM: f64 = 32.0;

pub struct LivePreview;

impl LivePreview {
    /// The box of [line], when it is a task with its mark complete: over
    /// the marker and the brackets (`- [ ]`), or over the brackets alone
    /// after a number, which stays the list marker.
    pub fn task_box(line: &str) -> Option<TaskBox> {
        let caps = TASK.captures(line)?;
        let marker = caps.get(2)?;
        let symbol = caps[3].chars().next()?;
        let chars = |bytes: usize| line[..bytes].chars().count();
        let end = chars(caps.get(3)?.end()) + 1;
        let numbered = marker.as_str().starts_with(|c: char| c.is_ascii_digit());
        let start = if numbered { end - 3 } else { chars(marker.start()) };
        Some(TaskBox { range: start..end, symbol })
    }

    /// Whether a caret or a selection is inside the characters in [range]:
    /// a caret from before the first to after the last, both ends counted,
    /// or a selection that reaches into it. Inside, a part is shown raw.
    pub fn inside(range: &Range<usize>, selection: (usize, usize)) -> bool {
        let (from, to) = (selection.0.min(selection.1), selection.0.max(selection.1));
        if from == to { from >= range.start && from <= range.end } else { from < range.end && range.start < to }
    }

    /// How the status [symbol] is drawn: by the reader's table first, else
    /// by the type the vault's statuses give it; a symbol the vault does not
    /// have is a dashed square.
    pub fn box_look(symbol: char, statuses: &TaskStatuses) -> BoxLook {
        if let Some((_, icon, color)) = STATUSES.iter().find(|(status, _, _)| *status == symbol) {
            return BoxLook { icon, color };
        }
        if !statuses.all.iter().any(|status| status.symbol == symbol) {
            return BoxLook { icon: "square-dashed", color: "list-marker" };
        }
        match statuses.kind(symbol) {
            TaskType::Todo => BoxLook { icon: "square", color: "list-marker" },
            TaskType::InProgress => BoxLook { icon: "square-dot", color: "accent" },
            TaskType::Done => BoxLook { icon: "square-check-big", color: "done-color" },
            TaskType::Cancelled => BoxLook { icon: "square-x", color: "done-color" },
            TaskType::NonTask => BoxLook { icon: "square-dashed", color: "list-marker" },
        }
    }

    /// A click on the box of [line]: the lines it becomes, which are
    /// `TaskLine::toggled`'s. None when the line is no task.
    pub fn click(line: &str, today: Day) -> Option<Vec<String>> {
        Self::task_box(line)?;
        TaskLine::toggled(line, &today.to_string())
    }

    /// Where a caret at [caret] is after the characters [line] (one line,
    /// without its ending) became [lines]: before the line it stays; in it,
    /// it keeps its offset in the task's own line, the last one (a repeat
    /// leaves its next occurrence above), no further than that line's end;
    /// after it, it stays on the same text.
    pub fn caret_after_click(caret: usize, line: &Range<usize>, lines: &[String]) -> usize {
        let Some(last) = lines.last() else { return caret };
        let above: usize = lines[..lines.len() - 1].iter().map(|l| l.chars().count() + 1).sum();
        let last = last.chars().count();
        if caret < line.start {
            caret
        } else if caret <= line.end {
            line.start + above + (caret - line.start).min(last)
        } else {
            caret + above + last - line.len()
        }
    }

    /// The date chips of [line], a task: each of `➕ ⏳ 🛫 📅 ✅ ❌` with one
    /// space and a real day, and `🔁` with its rule. A date that is no day
    /// stays as written, and so does every date on a line that is no task.
    pub fn chips(line: &str, today: Day, statuses: &TaskStatuses) -> Vec<Chip> {
        let Some(task) = Self::task_box(line) else { return Vec::new() };
        let kind = statuses.kind(task.symbol);
        let (open, closed) = (matches!(kind, TaskType::Todo | TaskType::InProgress), kind.is_closed());
        let chars: Vec<char> = line.chars().collect();
        // A `^block` id ends the line's text: a rule stops before it.
        let end = BLOCK_ID.find(line).map_or(chars.len(), |id| line[..id.start()].chars().count());
        let mut chips = Vec::new();
        let mut at = task.range.end;
        while at < end {
            let Some(kind) = ChipKind::of(chars[at]) else {
                at += 1;
                continue;
            };
            if kind == ChipKind::Repeat {
                let stop = (at + 1..end).find(|&i| ChipKind::of(chars[i]).is_some() || PRIORITIES.contains(&chars[i])).unwrap_or(end);
                let mut last = stop;
                while last > at + 1 && chars[last - 1].is_whitespace() {
                    last -= 1;
                }
                if last > at + 1 {
                    chips.push(Chip { kind, range: at..last, late: false, closed });
                }
                at = stop;
                continue;
            }
            let date: String = chars.get(at + 2..at + 12).unwrap_or_default().iter().collect();
            let ends = chars.get(at + 12).is_none_or(|c| c.is_whitespace());
            match Day::parse(&date) {
                Some(day) if chars.get(at + 1) == Some(&' ') && date.len() == 10 && ends => {
                    let late = kind == ChipKind::Due && open && day < today;
                    chips.push(Chip { kind, range: at..at + 12, late, closed });
                    at += 12;
                }
                _ => at += 1,
            }
        }
        chips
    }

    /// The `> ` marks at the start of [line], when it is a quote's line.
    pub fn quote_mark(line: &str) -> Option<Range<usize>> {
        QUOTE.find(line).map(|found| 0..line[..found.end()].chars().count())
    }

    /// [line] as the first line of a callout.
    pub fn callout_head(line: &str) -> Option<CalloutHead> {
        let quote = QUOTE.find(line)?;
        let rest = &line[quote.end()..];
        let caps = CALLOUT.captures(rest)?;
        let name = caps[1].to_lowercase();
        let known = CALLOUTS.iter().any(|(kinds, _)| kinds.contains(&name.as_str()));
        let written = rest[caps[0].len()..].trim();
        let title = if written.is_empty() {
            let mut letters = name.chars();
            letters.next().map_or_default(|first| first.to_uppercase().chain(letters).collect())
        } else {
            written.to_string()
        };
        let start = line[..quote.end()].chars().count();
        Some(CalloutHead {
            quote: 0..start,
            mark: start..start + caps[0].chars().count(),
            kind: if known { name } else { "note".into() },
            title,
            written: !written.is_empty(),
        })
    }

    /// The Lucide icon of a callout [kind]; `note`'s for one the reader does not know.
    pub fn callout_icon(kind: &str) -> &'static str {
        let kind = kind.to_lowercase();
        CALLOUTS.iter().find(|(kinds, _)| kinds.contains(&kind.as_str())).map_or("pencil", |(_, icon)| icon)
    }

    /// [line] as a fence line: three or more backticks or `~`, and the language after them.
    pub fn fence(line: &str) -> Option<Fence> {
        let caps = FENCE.captures(line)?;
        let chars = |bytes: usize| line[..bytes].chars().count();
        let marks = caps.get(1)?;
        Some(Fence {
            marks: chars(marks.start())..chars(marks.end()),
            word: caps.get(2).map(|word| chars(word.start())..chars(word.end())),
        })
    }

    /// The drawn parts of [lines], whole lines of a note that start at
    /// character [offset] in the block [state]. [above] is what the line
    /// before them is, and [quote_below] whether the line after them is a
    /// `>` line, so a callout cut by the lines given is still one panel.
    /// Code blocks, inline code, the front matter, math and comments hold
    /// no box and no chip.
    pub fn parts(lines: &str, offset: usize, state: &BlockState, above: &Above, quote_below: bool, today: Day) -> Parts {
        let statuses = TaskStatuses::current();
        let mut parts = Parts::default();
        let mut state = state.clone();
        let mut run = above.clone();
        // A fence opened on a `>` line is the quote's: its lines are no code panel.
        let mut quoted_fence = matches!(state, BlockState::Fence(_)) && run != Above::Nothing;
        let mut start = offset;
        for raw in lines.split_inclusive('\n') {
            let line = raw.trim_end_matches(['\n', '\r']);
            let length = line.chars().count();
            let range = start..start + length;
            let before = state.clone();
            state = MarkdownHighlighter::next(line, &state, start == 0);
            start += raw.chars().count();
            let shift = |r: Range<usize>| range.start + r.start..range.start + r.end;
            let quote = Self::quote_mark(line);
            let (fenced, markdown) = (matches!(before, BlockState::Fence(_)), before == BlockState::Normal);
            if !(fenced || markdown && matches!(state, BlockState::Normal | BlockState::Fence(_))) {
                run = Above::Nothing;
                continue;
            }
            if fenced && !quoted_fence {
                run = Above::Nothing;
                let role = if state == BlockState::Normal { CodeRole::Close } else { CodeRole::Body };
                let fence =
                    (role == CodeRole::Close).then(|| Self::fence(line)).flatten().map(|f| Fence { marks: shift(f.marks), word: None });
                parts.lines.push(BlockLine { range, block: Block::Code { role, fence } });
                continue;
            }
            let opens = markdown && matches!(state, BlockState::Fence(_));
            let Some(quote) = quote else {
                run = Above::Nothing;
                if opens {
                    quoted_fence = false;
                    let fence = Self::fence(line).map(|f| Fence { marks: shift(f.marks), word: f.word.map(shift) });
                    parts.lines.push(BlockLine { range, block: Block::Code { role: CodeRole::Open, fence } });
                } else if markdown {
                    if MarkdownTable::is_row(line) {
                        parts.rows.push(range.clone());
                    }
                    Self::inline(line, range.start, today, &statuses, &mut parts);
                }
                continue;
            };
            if opens {
                quoted_fence = true;
            }
            let head = if run == Above::Nothing && markdown { Self::callout_head(line) } else { None };
            if run == Above::Nothing {
                run = head.as_ref().map_or(Above::Quote, |head| Above::Callout(head.kind.clone()));
            }
            if let Above::Callout(kind) = &run {
                let head = head.map(|h| CalloutHead { quote: shift(h.quote), mark: shift(h.mark), ..h });
                parts.lines.push(BlockLine {
                    range: range.clone(),
                    block: Block::Callout { kind: kind.clone(), head, quote: shift(quote), last: false },
                });
            }
            if markdown && !opens {
                Self::inline(line, range.start, today, &statuses, &mut parts);
            }
        }
        // A callout's last line: the next line given is not of it, or no quote follows.
        let quote_follows = run != Above::Nothing && quote_below;
        for at in 0..parts.lines.len() {
            let next = parts.lines.get(at + 1).map(|next| (next.range.start, matches!(&next.block, Block::Callout { head: None, .. })));
            let end = parts.lines[at].range.end;
            if let Block::Callout { last, .. } = &mut parts.lines[at].block {
                *last = match next {
                    Some((next_start, continues)) => !(continues && next_start == end + 1),
                    None => !(quote_follows && end + 1 >= start),
                };
            }
        }
        parts
    }

    /// The lines of the tables drawn as a grid among [rows], the row lines
    /// `parts` found in [lines], which start at character [offset]. A table
    /// is a grid by all of its rows (`MarkdownTable::is_grid`), so the rows
    /// right [above] the first line given and right [below] the last are
    /// given too, when those lines are rows: a row read again on its own
    /// after a key still knows its table.
    pub fn grid_lines(lines: &str, offset: usize, rows: &[Range<usize>], above: &[String], below: &[String]) -> Vec<GridLine> {
        let chars: Vec<char> = lines.chars().collect();
        let end = offset + chars.len() - usize::from(chars.last() == Some(&'\n'));
        let text = |row: &Range<usize>| chars[row.start - offset..row.end - offset].iter().collect::<String>();
        let mut out = Vec::new();
        let mut at = 0;
        while at < rows.len() {
            // One table: the rows with no other line between them.
            let mut next = at + 1;
            while next < rows.len() && rows[next].start == rows[next - 1].end + 1 {
                next += 1;
            }
            let run = &rows[at..next];
            let before: &[String] = if run[0].start == offset { above } else { &[] };
            let after: &[String] = if run[run.len() - 1].end >= end { below } else { &[] };
            let table: Vec<String> = before.iter().cloned().chain(run.iter().map(text)).chain(after.iter().cloned()).collect();
            if MarkdownTable::is_grid(&table) {
                let widest = table.iter().map(|line| MarkdownTable::width(line)).max().unwrap_or(0);
                for (index, row) in run.iter().enumerate() {
                    let place = before.len() + index;
                    out.push(GridLine {
                        range: row.clone(),
                        role: match place {
                            0 => GridRole::Header,
                            1 => GridRole::Delimiter,
                            _ => GridRole::Row,
                        },
                        last: place + 1 == table.len(),
                        pipes: MarkdownTable::pipes(&table[place]).into_iter().map(|pipe| row.start + pipe).collect(),
                        widest,
                    });
                }
            }
            at = next;
        }
        out
    }

    /// The box and the chips of one line of Markdown, outside its inline code.
    fn inline(line: &str, start: usize, today: Day, statuses: &TaskStatuses, parts: &mut Parts) {
        let Some(task) = Self::task_box(line) else { return };
        parts.boxes.push(TaskBox { range: start + task.range.start..start + task.range.end, symbol: task.symbol });
        let code = inline_code(line);
        for chip in Self::chips(line, today, statuses) {
            if !code.iter().any(|c| c.start < chip.range.end && chip.range.start < c.end) {
                parts.chips.push(Chip { range: start + chip.range.start..start + chip.range.end, ..chip });
            }
        }
    }

    /// The panels of a whole note: each callout and each code block, by its lines.
    pub fn panels(text: &str) -> Vec<Panel> {
        let chars: Vec<char> = text.chars().collect();
        let line_of = |at: usize| chars[..at.min(chars.len())].iter().filter(|c| **c == '\n').count();
        let word = |range: &Range<usize>| chars[range.clone()].iter().collect::<String>().to_lowercase();
        let mut panels: Vec<Panel> = Vec::new();
        for line in Self::parts(text, 0, &BlockState::Normal, &Above::Nothing, false, Day::today()).lines {
            let at = line_of(line.range.start);
            let opens = match &line.block {
                Block::Callout { head, .. } => head.is_some(),
                Block::Code { role, .. } => *role == CodeRole::Open,
            };
            match (panels.last_mut(), opens) {
                (Some(panel), false) if panel.lines.end == at => panel.lines.end = at + 1,
                _ => {
                    let kind = match &line.block {
                        Block::Callout { kind, head, .. } => {
                            PanelKind::Callout { kind: kind.clone(), title: head.as_ref().map_or_default(|head| head.title.clone()) }
                        }
                        Block::Code { fence, .. } => PanelKind::Code { word: fence.as_ref().and_then(|f| f.word.as_ref()).map(word) },
                    };
                    panels.push(Panel { lines: at..at + 1, kind });
                }
            }
        }
        panels
    }

    /// The editor's line in a pane [pane] px wide: [width] em of the text
    /// size, in the middle, with never less than 32 px at each side. A
    /// width of 0 is no limit: the pane's width less that room.
    pub fn measure(pane: f64, text_size: f64, width: f64) -> Measure {
        let most = (pane - 2.0 * LEAST_ROOM).max(0.0);
        let line = if width > 0.0 { (width * text_size).min(most) } else { most };
        Measure { line, room: ((pane - line) / 2.0).max(0.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> Day {
        Day::new(2026, 11, 22).unwrap()
    }

    /// The vault's statuses, with `r` In Review.
    fn vault() -> TaskStatuses {
        TaskStatuses::from_json(
            r#"{"statusSettings": {"customStatuses": [{"symbol": "r", "name": "In Review", "nextStatusSymbol": "x", "type": "IN_PROGRESS"}]}}"#,
        )
    }

    fn parts(text: &str) -> Parts {
        LivePreview::parts(text, 0, &BlockState::Normal, &Above::Nothing, false, today())
    }

    #[test]
    fn a_task_box_is_drawn_when_the_mark_is_complete() {
        assert!(LivePreview::task_box("- [ ] ").is_some());
        assert!(LivePreview::task_box("- [ ]").is_none());
        for line in ["  * [x] a", "1. [/] a", "> - [ ] a", "- [r] a"] {
            assert!(LivePreview::task_box(line).is_some(), "{line}");
        }
        for line in ["- [] a", "- [xx] a", "-[ ] a", "[ ] a"] {
            assert!(LivePreview::task_box(line).is_none(), "{line}");
        }
        assert!(parts("```\n- [ ] a\n```\n").boxes.is_empty());
        assert_eq!(parts("text\n- [ ] a\n").boxes, [TaskBox { range: 5..10, symbol: ' ' }]);
    }

    #[test]
    fn the_box_covers_the_mark_and_the_brackets() {
        assert_eq!(LivePreview::task_box("- [ ] Buy").unwrap().range, 0..5);
        assert_eq!(LivePreview::task_box("1. [x] a").unwrap().range, 3..6);
        assert_eq!(LivePreview::task_box("> - [ ] a").unwrap().range, 2..7);
        let range = LivePreview::task_box("- [ ] Buy").unwrap().range;
        for caret in [0, 3, 5] {
            assert!(LivePreview::inside(&range, (caret, caret)), "{caret}");
        }
        assert!(!LivePreview::inside(&range, (6, 6)));
        // A selection that touches the box shows it raw too.
        assert!(LivePreview::inside(&range, (4, 8)) && !LivePreview::inside(&range, (5, 8)));
    }

    #[test]
    fn a_click_on_the_box_is_the_task_toggle() {
        let click = |line: &str| LivePreview::click(line, today()).unwrap();
        assert_eq!(click("- [ ] Call ➕ 2026-11-20"), ["- [x] Call ➕ 2026-11-20 ✅ 2026-11-22"]);
        assert_eq!(click("- [x] Call ➕ 2026-11-20 ✅ 2026-11-22"), ["- [ ] Call ➕ 2026-11-20"]);
        assert_eq!(click("- [/] a"), ["- [x] a ✅ 2026-11-22"]);
        assert_eq!(click("- [-] a ❌ 2026-11-01"), ["- [ ] a"]);
        assert_eq!(click("- [/] a [reason:: waiting]"), ["- [x] a ✅ 2026-11-22"]);
        for line in ["- [ ] Call ➕ 2026-11-20", "- [-] a ❌ 2026-11-01", "- [ ] Pay 📅 2026-11-20 🔁 every week"] {
            assert_eq!(LivePreview::click(line, today()), TaskLine::toggled(line, "2026-11-22"), "{line}");
        }
        assert_eq!(LivePreview::click("- [ ]", today()), None);
        assert_eq!(LivePreview::click("Call", today()), None);
        // The caret keeps its offset in the line's text; past the new end, it is at the end.
        let line = 10..33;
        let done = click("- [ ] Call ➕ 2026-11-20");
        assert_eq!(LivePreview::caret_after_click(18, &line, &done), 18);
        assert_eq!(LivePreview::caret_after_click(4, &line, &done), 4);
        assert_eq!(LivePreview::caret_after_click(40, &line, &done), 53);
        assert_eq!(LivePreview::caret_after_click(30, &(10..30), &click("- [-] a ❌ 2026-11-01")), 17);
        // A repeating task leaves its next occurrence above: the caret stays on the same text.
        let repeated = click("- [ ] Pay 📅 2026-11-20 🔁 every week");
        assert_eq!(repeated.len(), 2);
        assert_eq!(LivePreview::caret_after_click(8, &(0..35), &repeated), repeated[0].chars().count() + 1 + 8);
    }

    #[test]
    fn each_status_has_its_drawing() {
        let look = |symbol: char| {
            let look = LivePreview::box_look(symbol, &vault());
            (look.icon, look.color)
        };
        assert_eq!(look(' '), ("square", "list-marker"));
        assert_eq!(look('x'), ("square-check-big", "done-color"));
        assert_eq!(look('X'), ("square-check-big", "done-color"));
        assert_eq!(look('/'), ("square-dot", "accent"));
        assert_eq!(look('-'), ("square-x", "done-color"));
        assert_eq!(look('>'), ("square-arrow-right", "blue"));
        assert_eq!(look('r'), ("square-dot", "accent"));
        assert_eq!(look('~'), ("square-dashed", "list-marker"));
    }

    #[test]
    fn dates_on_a_task_are_chips() {
        let line = "- [ ] Pay 📅 2026-11-20 ⏳ 2026-11-18 🔁 every week ➕ 2026-11-01";
        let chars: Vec<char> = line.chars().collect();
        let chips = LivePreview::chips(line, today(), &vault());
        let written: Vec<(ChipKind, String)> = chips.iter().map(|chip| (chip.kind, chars[chip.range.clone()].iter().collect())).collect();
        assert_eq!(
            written,
            [
                (ChipKind::Due, "📅 2026-11-20".to_string()),
                (ChipKind::Scheduled, "⏳ 2026-11-18".to_string()),
                (ChipKind::Repeat, "🔁 every week".to_string()),
                (ChipKind::Created, "➕ 2026-11-01".to_string()),
            ]
        );
        assert!(LivePreview::chips("- [ ] a 📅 2026-02-30", today(), &vault()).is_empty());
        assert!(LivePreview::chips("Meeting 📅 2026-11-20", today(), &vault()).is_empty());
        // A rule ends before a priority and before a block id; inline code holds no chip.
        let rule = LivePreview::chips("- [ ] a 🔁 every day ⏫ ^id", today(), &vault());
        assert_eq!(rule[0].range, 8..19);
        assert_eq!(LivePreview::chips("- [ ] a 🔁 every day ^id", today(), &vault())[0].range, 8..19);
        assert!(parts("- [ ] a `📅 2026-11-20`\n").chips.is_empty());
        assert_eq!(parts("x\n- [ ] a 📅 2026-11-20\n").chips[0].range, 10..22);
    }

    #[test]
    fn a_late_due_date_is_red() {
        let chip = |line: &str| LivePreview::chips(line, today(), &vault()).remove(0);
        assert!(chip("- [ ] a 📅 2026-11-21").late);
        assert!(!chip("- [ ] a 📅 2026-11-22").late);
        assert!(!chip("- [x] a 📅 2026-11-21 ✅ 2026-11-22").late);
        assert!(!chip("- [-] a 📅 2026-11-21").late);
        assert!(chip("- [/] a 📅 2026-11-21").late);
        let scheduled = chip("- [ ] a ⏳ 2026-11-01");
        assert!(!scheduled.late);
        assert_eq!(scheduled.kind.color(scheduled.late), "blue");
        assert_eq!(ChipKind::Due.color(true), "red");
        assert_eq!(ChipKind::Due.color(false), "orange");
        // On a closed task the chips are drawn weaker.
        assert!(chip("- [x] a 📅 2026-11-21 ✅ 2026-11-22").closed && !chip("- [ ] a 📅 2026-11-21").closed);
    }

    #[test]
    fn a_chip_is_raw_with_the_caret_in_it() {
        let line = "- [ ] a 📅 2026-11-20";
        let range = LivePreview::chips(line, today(), &vault()).remove(0).range;
        assert_eq!(range, 8..20);
        // Just before the emoji, in the date, just after the last 0.
        for caret in [8, 14, 20] {
            assert!(LivePreview::inside(&range, (caret, caret)), "{caret}");
        }
        // In `a`.
        assert!(!LivePreview::inside(&range, (7, 7)));
    }

    #[test]
    fn a_callout_has_its_kinds_look() {
        let panel = |text: &str| LivePreview::panels(text);
        let callout = |kind: &str, title: &str| PanelKind::Callout { kind: kind.into(), title: title.into() };
        assert_eq!(panel("> [!warning] Mind\n> the gap\n"), [Panel { lines: 0..2, kind: callout("warning", "Mind") }]);
        assert_eq!(panel("> [!tip]"), [Panel { lines: 0..1, kind: callout("tip", "Tip") }]);
        assert_eq!(panel("> [!nonsense] x"), [Panel { lines: 0..1, kind: callout("note", "x") }]);
        assert_eq!(panel("> plain"), []);
        assert_eq!(panel("> [!note]- x"), panel("> [!note] x"));
        // The marks drawn at no width, and the name that is drawn only.
        let head = LivePreview::callout_head("> [!tip]+ ").unwrap();
        assert_eq!((head.quote, head.mark, head.written), (0..2, 2..10, false));
        assert_eq!(LivePreview::callout_icon("warning"), "triangle-alert");
        assert_eq!(LivePreview::callout_icon("nonsense"), "pencil");
        // One panel for the run, an empty `>` line in it, a task in it with its box.
        let run = parts("a\n> [!todo] T\n>\n> - [ ] b\n\n> [!note]\n");
        let lasts: Vec<bool> = run.lines.iter().map(|line| matches!(line.block, Block::Callout { last: true, .. })).collect();
        assert_eq!(lasts, [false, false, true, true]);
        assert_eq!(run.boxes.len(), 1);
        // A callout cut by the lines given is still one panel.
        let below = LivePreview::parts("> b\n", 20, &BlockState::Normal, &Above::Callout("tip".into()), true, today());
        assert_eq!(
            below.lines,
            [BlockLine { range: 20..23, block: Block::Callout { kind: "tip".into(), head: None, quote: 20..22, last: false } }]
        );
        let end = LivePreview::parts("> b\n", 20, &BlockState::Normal, &Above::Callout("tip".into()), false, today());
        assert!(matches!(end.lines[0].block, Block::Callout { last: true, .. }));
        assert!(LivePreview::parts("> b\n", 20, &BlockState::Normal, &Above::Quote, false, today()).lines.is_empty());
    }

    #[test]
    fn a_code_block_has_its_language_word() {
        let code = |word: Option<&str>| PanelKind::Code { word: word.map(str::to_string) };
        assert_eq!(LivePreview::panels("```Rust\nlet a = 1;\n```\n"), [Panel { lines: 0..3, kind: code(Some("rust")) }]);
        assert_eq!(LivePreview::panels("```\na\n```\n"), [Panel { lines: 0..3, kind: code(None) }]);
        assert_eq!(LivePreview::panels("a\n~~~py\nb\n\nc"), [Panel { lines: 1..5, kind: code(Some("py")) }]);
        // Two blocks with nothing between them are two panels; a `>` line in one is code.
        let two = LivePreview::panels("```\n> a\n```\n```sh\nb\n```\n");
        assert_eq!(two, [Panel { lines: 0..3, kind: code(None) }, Panel { lines: 3..6, kind: code(Some("sh")) }]);
        // The fence characters are drawn at no width; a code block in a callout is the callout's.
        assert_eq!(LivePreview::fence("```rust title"), Some(Fence { marks: 0..3, word: Some(3..7) }));
        assert_eq!(LivePreview::fence("~~~~ py"), Some(Fence { marks: 0..5, word: Some(5..7) }));
        let quoted = LivePreview::panels("> [!note]\n> ```\n> a\n> ```\n");
        assert_eq!(quoted, [Panel { lines: 0..4, kind: PanelKind::Callout { kind: "note".into(), title: "Note".into() } }]);
    }

    /// The grid lines of a whole note, as (line's start, role, last).
    fn grid(text: &str, above: &[&str], below: &[&str]) -> Vec<(usize, GridRole, bool)> {
        let parts = LivePreview::parts(text, 0, &BlockState::Normal, &Above::Nothing, false, today());
        let own = |lines: &[&str]| lines.iter().map(ToString::to_string).collect::<Vec<_>>();
        LivePreview::grid_lines(text, 0, &parts.rows, &own(above), &own(below))
            .into_iter()
            .map(|l| (l.range.start, l.role, l.last))
            .collect()
    }

    #[test]
    fn a_table_is_a_grid_only_with_its_delimiter_row() {
        use GridRole::{Delimiter, Header, Row};
        let table = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        assert_eq!(grid(table, &[], &[]), [(0, Header, false), (10, Delimiter, false), (24, Row, true)]);
        assert_eq!(grid("| A | B |\n", &[], &[]), []);
        assert_eq!(grid("> | A | B |\n> | --- | --- |\n> | 1 | 2 |\n", &[], &[]), []);
        assert_eq!(grid("- | A | B |\n  | --- | --- |\n", &[], &[]), []);
        assert_eq!(grid(&format!("```\n{table}```\n"), &[], &[]), []);
        let long = format!("{table}{}", "| 3 | 4 |\n".repeat(200));
        assert_eq!(grid(&long, &[], &[]), []);
        assert_eq!(grid(&long[..long.len() - 10], &[], &[]).len(), 202);
        // Two tables with text between them, and the pipes of a line.
        let two = grid("| A |\n| - |\n\nx\n| B |\n| --- |\n| 1 |", &[], &[]);
        assert_eq!(two, [(15, Header, false), (21, Delimiter, false), (29, Row, true)]);
        let parts = LivePreview::parts(table, 100, &BlockState::Normal, &Above::Nothing, false, today());
        let lines = LivePreview::grid_lines(table, 100, &parts.rows, &[], &[]);
        assert_eq!((lines[0].pipes.as_slice(), lines[0].widest, lines[2].widest), ([100, 104, 108].as_slice(), 13, 13));
        // One row read on its own knows its table by the rows above and below it.
        assert_eq!(grid("| 1 | 2 |\n", &["| A | B |", "| --- | --- |"], &[]), [(0, Row, true)]);
        assert_eq!(grid("| 1 | 2 |\n", &["| A | B |", "| --- | --- |"], &["| 3 | 4 |"]), [(0, Row, false)]);
        assert_eq!(grid("| A | B |\n", &[], &["| --- | --- |"]), [(0, Header, false)]);
        assert_eq!(grid("| --- | --- |\n", &["| A | B |"], &[]), [(0, Delimiter, true)]);
        assert_eq!(grid("| 1 | 2 |\n", &["| A | B |"], &[]), []);
    }

    #[test]
    fn the_editors_line_is_the_measure() {
        assert_eq!(LivePreview::measure(2000.0, 16.0, 35.0).line, 560.0);
        assert_eq!(LivePreview::measure(1000.0, 16.0, 35.0), Measure { line: 560.0, room: 220.0 });
        assert_eq!(LivePreview::measure(500.0, 16.0, 35.0), Measure { line: 436.0, room: 32.0 });
        assert_eq!(LivePreview::measure(1000.0, 16.0, 0.0), Measure { line: 936.0, room: 32.0 });
        assert_eq!(LivePreview::measure(40.0, 16.0, 35.0), Measure { line: 0.0, room: 20.0 });
    }
}
