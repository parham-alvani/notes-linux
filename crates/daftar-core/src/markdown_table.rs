//! A Markdown table in the editor: Tab goes to the next cell and lines the
//! columns up, as Obsidian's table editor does, and Return goes to the row
//! below, with a new row after the last. Live preview draws a table with its
//! delimiter row as a grid (`docs/specs/look.md`, part 2). Offsets in a line
//! are in characters.

use std::ops::Range;

use std::sync::LazyLock;

use regex::Regex;

use crate::editor_commands::TextEdit;
use crate::markdown_highlighter::{BlockState, MarkdownHighlighter};

static SEPARATOR_CELL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^:?-+:?$").unwrap());

/// Where Tab leaves the table and the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTab {
    pub lines: Vec<String>,
    pub row: usize,
    pub column: usize,
}

/// What Return does in a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableReturn {
    /// The table formatted, and the cell the cursor goes to.
    Cell(TableTab),
    /// The table's last row was empty: it goes, the table ends, and these
    /// are the rows that stay.
    End(Vec<String>),
}

/// The table a caret is in, in a note's text. Offsets are characters of the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableBlock {
    /// The table's rows, from the start of the first to the end of the
    /// last, without the last one's line break.
    pub range: Range<usize>,
    /// The rows, each without its line break.
    pub lines: Vec<String>,
    /// Which row holds the caret, and where that row's line is.
    pub row: usize,
    pub line: Range<usize>,
    /// The cell of that row the caret is in.
    pub column: usize,
}

pub struct MarkdownTable;

impl MarkdownTable {
    /// What a header cell of a new table holds until the person types its
    /// name: the slash bar's Table item selects the first, and Tab the next.
    pub const PLACEHOLDER: &'static str = "Column";

    /// The table [selection] is in, in the note [text]: the run of lines
    /// that start with a pipe around the caret's line. A line with no pipe
    /// at its start is never a row, so the text beside a table is never
    /// taken into it. None on a line that is no row; in a code block, the
    /// front matter, math or a comment, where a `|` line is text; and with
    /// a selection that is not inside one cell, which is raw text.
    pub fn block(text: &str, selection: Range<usize>) -> Option<TableBlock> {
        let chars: Vec<char> = text.chars().collect();
        if selection.start > selection.end || selection.end > chars.len() {
            return None;
        }
        // The line that holds [at], without its line break.
        let line_of = |at: usize| {
            let start = chars[..at].iter().rposition(|&c| c == '\n').map_or(0, |found| found + 1);
            let end = chars[at..].iter().position(|&c| c == '\n').map_or(chars.len(), |found| at + found);
            start..end
        };
        let row_text = |line: &Range<usize>| chars[line.clone()].iter().collect::<String>();
        let line = line_of(selection.end);
        let current = row_text(&line);
        if !Self::is_row(&current) || selection.start < line.start {
            return None;
        }
        let inside = selection.start - line.start..selection.end - line.start;
        if !inside.is_empty() && Self::pipes(&current).iter().any(|pipe| inside.contains(pipe)) {
            return None;
        }
        if MarkdownHighlighter::block_state(line.start, text) != BlockState::Normal {
            return None;
        }
        let (mut first, mut row, mut lines) = (line.start, 0, vec![current.clone()]);
        while first > 0 {
            let above = line_of(first - 1);
            let above_text = row_text(&above);
            if !Self::is_row(&above_text) {
                break;
            }
            lines.insert(0, above_text);
            first = above.start;
            row += 1;
        }
        let mut last = line.end;
        while last < chars.len() {
            let below = line_of(last + 1);
            let below_text = row_text(&below);
            if !Self::is_row(&below_text) {
                break;
            }
            lines.push(below_text);
            last = below.end;
        }
        let column = Self::column(inside.start, &current);
        Some(TableBlock { range: first..last, lines, row, line, column })
    }

    /// Tab, or Shift+Tab with [back], with [selection] in the note [text]:
    /// the edit that formats the caret's table and goes to the next cell or
    /// the one before, with a new row past the last cell. None where Tab is
    /// not the table's key (`block`).
    pub fn tab_key(text: &str, selection: Range<usize>, back: bool) -> Option<TextEdit> {
        let table = Self::block(text, selection)?;
        Some(Self::placed(&table.range, &Self::tab(&table.lines, table.row, table.column, back)))
    }

    /// Return with [selection] in the note [text]: the edit that goes to
    /// the row below, with a new row after the last (`enter`). On an empty
    /// last row the row goes and a blank line follows the table, or the
    /// next line would be a row too. None where Return is not the table's key.
    pub fn return_key(text: &str, selection: Range<usize>) -> Option<TextEdit> {
        let table = Self::block(text, selection)?;
        Some(match Self::enter(&table.lines, table.row, table.column) {
            TableReturn::Cell(moved) => Self::placed(&table.range, &moved),
            TableReturn::End(_) => {
                TextEdit { range: table.line.clone(), text: "\n".into(), selection: table.line.start + 1..table.line.start + 1 }
            }
        })
    }

    /// The table at characters [range] becomes [moved]'s formatted rows,
    /// with the caret at the start of the text of the cell it names. A
    /// header cell that still holds the placeholder is selected whole, so
    /// what is typed takes its place.
    fn placed(range: &Range<usize>, moved: &TableTab) -> TextEdit {
        let line = &moved.lines[moved.row];
        let start = range.start + moved.lines[..moved.row].iter().map(|l| l.chars().count() + 1).sum::<usize>();
        let cell = start + Self::cell_start(line, moved.column);
        let placeholder = moved.row == 0 && Self::cells(line).get(moved.column).is_some_and(|text| text == Self::PLACEHOLDER);
        let selected = if placeholder { Self::PLACEHOLDER.chars().count() } else { 0 };
        TextEdit { range: range.clone(), text: moved.lines.join("\n"), selection: cell..cell + selected }
    }

    /// The most rows under the delimiter row that are still drawn as a grid.
    pub const GRID_ROWS: usize = 200;

    /// Whether [lines], every row of one table, are drawn as a grid: a
    /// header, then the delimiter row, then at most `GRID_ROWS` rows. Each
    /// starts with its pipe at the line's start or after spaces only, so a
    /// table in a quote, a callout or a list item is none.
    pub fn is_grid<S: AsRef<str>>(lines: &[S]) -> bool {
        lines.len() >= 2
            && lines.len() - 2 <= Self::GRID_ROWS
            && lines.iter().all(|line| Self::is_row(line.as_ref()))
            && !Self::is_delimiter(lines[0].as_ref())
            && Self::is_delimiter(lines[1].as_ref())
    }

    /// Where the pipes that part the cells of [line] are, in characters. `\|` is none.
    pub fn pipes(line: &str) -> Vec<usize> {
        let (mut pipes, mut escaped) = (Vec::new(), false);
        for (offset, c) in line.chars().enumerate() {
            if c == '|' && !escaped {
                pipes.push(offset);
            }
            escaped = c == '\\' && !escaped;
        }
        pipes
    }

    /// The text of the cell that holds character [offset] of [line],
    /// without the spaces around it. An empty cell's place is between its
    /// two padding spaces. None before the first pipe and after the last.
    pub fn cell_text(line: &str, offset: usize) -> Option<Range<usize>> {
        let chars: Vec<char> = line.chars().collect();
        let pipes = Self::pipes(line);
        let before = pipes.iter().rposition(|&pipe| pipe < offset)?;
        let (open, close) = (pipes[before], *pipes.get(before + 1)?);
        let inner = open + 1..close;
        let Some(first) = inner.clone().find(|&at| chars[at] != ' ') else {
            let place = (open + 2).min(close).max(open + 1).min(close);
            return Some(place..place);
        };
        let last = inner.rev().find(|&at| chars[at] != ' ').unwrap_or(first);
        Some(first..last + 1)
    }

    /// Right at the end of a cell's text: the start of the next cell's
    /// text, over the pipe and the padding. None when Right is an ordinary key.
    pub fn right(line: &str, offset: usize) -> Option<usize> {
        let cell = Self::cell_text(line, offset.max(1)).filter(|cell| offset >= cell.end)?;
        let pipe = Self::pipes(line).into_iter().find(|&pipe| pipe >= cell.end)?;
        Self::cell_text(line, pipe + 1).map(|next| next.start)
    }

    /// Left at the start of a cell's text: the end of the text of the cell before.
    pub fn left(line: &str, offset: usize) -> Option<usize> {
        let cell = Self::cell_text(line, offset).filter(|cell| offset <= cell.start)?;
        let pipe = Self::pipes(line).into_iter().rfind(|&pipe| pipe < cell.start.max(1))?;
        Self::cell_text(line, pipe).map(|before| before.end)
    }

    /// The nearest place to [offset] in the text of its cell: where a click
    /// in a cell puts the caret.
    pub fn nearest(line: &str, offset: usize) -> usize {
        let count = line.chars().count();
        let pipes = Self::pipes(line);
        let at = match (pipes.first(), pipes.last()) {
            (Some(&first), _) if offset <= first => first + 1,
            (_, Some(&last)) if offset > last && line.trim_end().chars().count() == last + 1 => last,
            _ => offset,
        };
        Self::cell_text(line, at).map_or_else(|| offset.min(count), |cell| offset.clamp(cell.start, cell.end))
    }

    /// Return (or Shift+Return) in [lines] with the cursor in [row],
    /// [column]: the same column of the row below, the first row under the
    /// delimiter row from the header, and a new empty row after the last.
    /// On a last row whose cells are all empty the table ends.
    pub fn enter(lines: &[String], row: usize, column: usize) -> TableReturn {
        let mut table = Self::format(lines);
        let separators: Vec<bool> = table.iter().map(|l| Self::is_separator(&Self::cells(l))).collect();
        let columns = Self::cells(&table[0]).len().max(1);
        let column = column.min(columns - 1);
        let last = row + 1 >= table.len();
        if last && row > 0 && !separators[row] && Self::cells(&table[row]).iter().all(String::is_empty) {
            table.truncate(row);
            return TableReturn::End(table);
        }
        let mut next = row + 1;
        while next < table.len() && separators[next] {
            next += 1;
        }
        let mut column = column;
        if next >= table.len() {
            // A header alone gets its delimiter row first, as Advanced Tables
            // makes it, and the cursor goes to the first cell under it.
            if table.len() == 1 && !separators[0] {
                table.push(format!("|{}", " --- |".repeat(columns)));
                column = 0;
            }
            table.push(Self::empty_row(columns));
            table = Self::format(&table);
            next = table.len() - 1;
        }
        TableReturn::Cell(TableTab { lines: table, row: next, column })
    }

    /// A row starts with a pipe, as every table in the vault is written.
    pub fn is_row(line: &str) -> bool {
        line.trim().starts_with('|')
    }

    /// The cells of a row, without the outer pipes. `\|` stays in its cell.
    pub fn cells(line: &str) -> Vec<String> {
        let mut body = line.trim();
        body = body.strip_prefix('|').unwrap_or(body);
        if body.ends_with('|') && !body.ends_with("\\|") {
            body = &body[..body.len() - 1];
        }
        let (mut cells, mut cell, mut escaped) = (Vec::new(), String::new(), false);
        for c in body.chars() {
            if c == '|' && !escaped {
                cells.push(cell.trim().to_string());
                cell.clear();
            } else {
                cell.push(c);
            }
            escaped = c == '\\' && !escaped;
        }
        cells.push(cell.trim().to_string());
        cells
    }

    fn is_separator(cells: &[String]) -> bool {
        !cells.is_empty() && cells.iter().all(|c| SEPARATOR_CELL.is_match(c))
    }

    /// The width a cell takes in a monospaced line: a wide letter, as in
    /// Chinese, counts two, and a combining mark none.
    pub fn width(text: &str) -> usize {
        text.chars()
            .map(|c| {
                let v = c as u32;
                if is_nonspacing_mark(v) {
                    0
                } else if matches!(v, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFF00..=0xFF60 | 0x1F300..=0x1FAFF) {
                    2
                } else {
                    1
                }
            })
            .sum()
    }

    /// The rows with every column as wide as its widest cell, and as many
    /// cells in each row as the widest row has. The indent of the first row
    /// is kept.
    pub fn format(lines: &[String]) -> Vec<String> {
        let indent: String = lines.first().map_or_default(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect());
        let mut rows: Vec<Vec<String>> = lines.iter().map(|l| Self::cells(l)).collect();
        // Known before the rows are filled out: a filled-in separator is one still.
        let separators: Vec<bool> = rows.iter().map(|r| Self::is_separator(r)).collect();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut rows {
            row.resize(columns, String::new());
        }
        let mut widths = vec![3; columns];
        for (row, cells) in rows.iter().enumerate() {
            if separators[row] {
                continue;
            }
            for (column, cell) in cells.iter().enumerate() {
                widths[column] = widths[column].max(Self::width(cell));
            }
        }
        rows.iter()
            .enumerate()
            .map(|(row, cells)| {
                let parts: Vec<String> = cells
                    .iter()
                    .enumerate()
                    .map(|(column, cell)| {
                        if separators[row] {
                            let (left, right) = (cell.starts_with(':'), cell.ends_with(':'));
                            let dashes = "-".repeat(widths[column] - usize::from(left) - usize::from(right));
                            format!("{}{dashes}{}", if left { ":" } else { "" }, if right { ":" } else { "" })
                        } else {
                            format!("{cell}{}", " ".repeat(widths[column] - Self::width(cell)))
                        }
                    })
                    .collect();
                format!("{indent}| {} |", parts.join(" | "))
            })
            .collect()
    }

    /// Where the text of cell [column] starts in a formatted [line], in characters.
    pub fn cell_start(line: &str, column: usize) -> usize {
        let count = line.chars().count();
        let (mut pipes, mut escaped) = (0, false);
        for (offset, c) in line.chars().enumerate() {
            if c == '|' && !escaped {
                if pipes == column {
                    return (offset + 2).min(count);
                }
                pipes += 1;
            }
            escaped = c == '\\' && !escaped;
        }
        count
    }

    /// The cell that holds character [offset] of [line].
    pub fn column(offset: usize, line: &str) -> usize {
        let (mut pipes, mut escaped) = (0usize, false);
        for c in line.chars().take(offset) {
            if c == '|' && !escaped {
                pipes += 1;
            }
            escaped = c == '\\' && !escaped;
        }
        pipes.saturating_sub(1)
    }

    /// Tab (or Shift+Tab, [back]) in [lines] with the cursor in [row],
    /// [column]: the table formatted, and the row and cell the cursor goes
    /// to. Past the last cell a new row is made.
    pub fn tab(lines: &[String], row: usize, column: usize, back: bool) -> TableTab {
        let mut table = Self::format(lines);
        let columns = Self::cells(&table[0]).len() as isize;
        let separators: Vec<bool> = table.iter().map(|l| Self::is_separator(&Self::cells(l))).collect();
        let (mut r, mut c) = (row as isize, column as isize);
        loop {
            if back {
                c -= 1;
                if c < 0 {
                    r -= 1;
                    c = columns - 1;
                }
            } else {
                c += 1;
                if c >= columns {
                    r += 1;
                    c = 0;
                }
            }
            if !(r >= 0 && (r as usize) < table.len() && separators[r as usize]) {
                break;
            }
        }
        if r < 0 {
            return TableTab { lines: table, row: 0, column: 0 };
        }
        if r as usize >= table.len() {
            table.push(Self::empty_row(columns as usize));
            table = Self::format(&table);
            return TableTab { row: table.len() - 1, lines: table, column: 0 };
        }
        TableTab { lines: table, row: r as usize, column: c as usize }
    }

    /// A new row under [lines]' last, the table formatted, the cursor in its
    /// first cell. Under a header with no delimiter row yet, the delimiter
    /// row comes first, as Advanced Tables makes it.
    pub fn new_row(lines: &[String]) -> Vec<String> {
        let mut all = lines.to_vec();
        let columns = Self::cells(&lines[0]).len();
        if lines.len() == 1 && !Self::is_delimiter(&lines[0]) {
            all.push(format!("|{}", " --- |".repeat(columns.max(1))));
        }
        all.push(Self::empty_row(columns));
        Self::format(&all)
    }

    /// `| --- | :--: |`: the row under the header.
    pub fn is_delimiter(line: &str) -> bool {
        Self::is_row(line)
            && Self::cells(line).iter().all(|cell| {
                let cell = cell.trim();
                cell.len() >= 3 && cell.trim_matches(':').chars().all(|c| c == '-') && !cell.trim_matches(':').is_empty()
            })
    }

    fn empty_row(columns: usize) -> String {
        format!("|{}", " |".repeat(columns.max(1)))
    }
}

/// The nonspacing marks a note is likely to hold: Latin, Arabic and Persian
/// diacritics, the combining blocks and variation selectors.
fn is_nonspacing_mark(v: u32) -> bool {
    matches!(
        v,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_under_a_header_makes_the_delimiter_row() {
        let lines = vec!["| a | b |".to_string()];
        assert_eq!(MarkdownTable::new_row(&lines), ["| a   | b   |", "| --- | --- |", "|     |     |"]);
        let table = vec!["| a | b |".to_string(), "| --- | --- |".to_string(), "| 1 | 2 |".to_string()];
        assert_eq!(MarkdownTable::new_row(&table).len(), 4);
        assert!(MarkdownTable::is_delimiter("| :--- | ---: |"));
        assert!(!MarkdownTable::is_delimiter("| a | --- |"));
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn format_lines_the_columns_up() {
        let table = lines(&["| Name | Age |", "|:-|-:|", "| Sara | 7 |", "| Alexander | 30 | extra |"]);
        assert_eq!(
            MarkdownTable::format(&table),
            ["| Name      | Age |       |", "| :-------- | --: | ----- |", "| Sara      | 7   |       |", "| Alexander | 30  | extra |"]
        );
        assert_eq!(MarkdownTable::cells(r"| a \| b | c |"), [r"a \| b", "c"]);
    }

    #[test]
    fn tab_goes_to_the_next_cell_and_makes_a_row_at_the_end() {
        let table = lines(&["| a | b |", "| - | - |", "| c | d |"]);
        let next = MarkdownTable::tab(&table, 0, 1, false);
        // Over the separator, to the first cell of the next row.
        assert_eq!((next.row, next.column), (2, 0));
        let end = MarkdownTable::tab(&table, 2, 1, false);
        assert_eq!((end.lines.len(), end.row, end.column), (4, 3, 0));
        assert_eq!(end.lines[3], "|     |     |");
        let back = MarkdownTable::tab(&table, 2, 0, true);
        assert_eq!((back.row, back.column), (0, 1));
        assert_eq!(MarkdownTable::cell_start("| abc | de  |", 1), 8);
        assert_eq!(MarkdownTable::column(9, "| abc | de  |"), 1);
    }

    fn grid() -> Vec<String> {
        lines(&["| A | B |", "| --- | --- |", "| 1 | 2 |", "| 3 | 4 |"])
    }

    /// Where the cursor is after a move, as the text of its cell.
    fn cell(moved: &TableTab) -> String {
        MarkdownTable::cells(&moved.lines[moved.row])[moved.column].clone()
    }

    // docs/specs/look.md, part 2, cases 8 to 11.

    #[test]
    fn a_table_is_a_grid_only_with_its_delimiter_row() {
        assert!(MarkdownTable::is_grid(&["| A | B |", "| --- | --- |", "| 1 | 2 |"]));
        assert!(MarkdownTable::is_grid(&["  | A | B |", "  | :-: | --: |"]));
        assert!(!MarkdownTable::is_grid(&["| A | B |"]));
        assert!(!MarkdownTable::is_grid(&["| A | B |", "| 1 | 2 |"]));
        // In a quote the lines are no rows: the pipe is not at the line's start.
        assert!(!MarkdownTable::is_grid(&["> | A | B |", "> | --- | --- |", "> | 1 | 2 |"]));
        assert!(!MarkdownTable::is_grid(&["- | A | B |", "  | --- | --- |"]));
        let mut long = lines(&["| A |", "| --- |"]);
        long.extend((0..200).map(|n| format!("| {n} |")));
        assert!(MarkdownTable::is_grid(&long));
        long.push("| 201 |".into());
        assert!(!MarkdownTable::is_grid(&long));
    }

    #[test]
    fn enter_goes_to_the_row_below() {
        let TableReturn::Cell(below) = MarkdownTable::enter(&grid(), 2, 1) else { panic!("the table goes on") };
        assert_eq!((below.row, below.column, cell(&below).as_str()), (3, 1, "4"));
        assert_eq!(below.lines, ["| A   | B   |", "| --- | --- |", "| 1   | 2   |", "| 3   | 4   |"]);
        let TableReturn::Cell(added) = MarkdownTable::enter(&grid(), 3, 1) else { panic!("the table goes on") };
        assert_eq!(added.lines, ["| A   | B   |", "| --- | --- |", "| 1   | 2   |", "| 3   | 4   |", "|     |     |"]);
        assert_eq!((added.row, added.column), (4, 1));
        let TableReturn::Cell(under) = MarkdownTable::enter(&grid(), 0, 1) else { panic!("the table goes on") };
        assert_eq!((under.row, cell(&under).as_str()), (2, "2"));
        // A header with no delimiter row yet gets one, and a row.
        let TableReturn::Cell(made) = MarkdownTable::enter(&lines(&["| a | b |"]), 0, 1) else { panic!("the table goes on") };
        assert_eq!(made.lines, ["| a   | b   |", "| --- | --- |", "|     |     |"]);
        assert_eq!((made.row, made.column), (2, 0));
    }

    #[test]
    fn enter_on_an_empty_last_row_ends_the_table() {
        let TableReturn::Cell(added) = MarkdownTable::enter(&grid(), 3, 1) else { panic!("the table goes on") };
        assert_eq!(
            MarkdownTable::enter(&added.lines, 4, 1),
            TableReturn::End(lines(&["| A   | B   |", "| --- | --- |", "| 1   | 2   |", "| 3   | 4   |"]))
        );
        // An empty row that is not the last is a row like any other.
        let TableReturn::Cell(on) = MarkdownTable::enter(&lines(&["| A |", "| --- |", "|  |", "| 1 |"]), 2, 0) else {
            panic!("the table goes on")
        };
        assert_eq!(on.row, 3);
    }

    #[test]
    fn tab_moves_between_cells_and_adds_a_row() {
        let to = |row: usize, column: usize, back: bool| MarkdownTable::tab(&grid(), row, column, back);
        assert_eq!(cell(&to(2, 0, false)), "2");
        assert_eq!(cell(&to(2, 1, false)), "3");
        // Over the delimiter row.
        assert_eq!(cell(&to(0, 1, false)), "1");
        let end = to(3, 1, false);
        assert_eq!((end.lines.len(), end.row, end.column, end.lines[4].as_str()), (5, 4, 0, "|     |     |"));
        assert_eq!(cell(&to(3, 0, true)), "2");
        let stays = to(0, 0, true);
        assert_eq!((stays.row, stays.column), (0, 0));
    }

    #[test]
    fn the_arrows_and_a_click_stay_in_the_cells_text() {
        let line = "| abc | de  |     |";
        assert_eq!(MarkdownTable::pipes(line), [0, 6, 12, 18]);
        assert_eq!(MarkdownTable::pipes(r"| a \| b | c |"), [0, 9, 13]);
        assert_eq!(MarkdownTable::cell_text(line, 3), Some(2..5));
        assert_eq!(MarkdownTable::cell_text(line, 11), Some(8..10));
        // An empty cell: between its two padding spaces.
        assert_eq!(MarkdownTable::cell_text(line, 15), Some(14..14));
        assert_eq!(MarkdownTable::cell_text(line, 0), None);
        // Right at the end of `abc` goes to `de`; inside `abc` it is an ordinary key.
        assert_eq!(MarkdownTable::right(line, 5), Some(8));
        assert_eq!(MarkdownTable::right(line, 4), None);
        assert_eq!(MarkdownTable::right(line, 10), Some(14));
        assert_eq!(MarkdownTable::right(line, 14), None);
        // Left at the start of `de` goes to the end of `abc`.
        assert_eq!(MarkdownTable::left(line, 8), Some(5));
        assert_eq!(MarkdownTable::left(line, 9), None);
        assert_eq!(MarkdownTable::left(line, 14), Some(10));
        assert_eq!(MarkdownTable::left(line, 2), None);
        // A click in the padding goes to the nearest place in the text.
        assert_eq!(MarkdownTable::nearest(line, 1), 2);
        assert_eq!(MarkdownTable::nearest(line, 6), 5);
        assert_eq!(MarkdownTable::nearest(line, 11), 10);
        assert_eq!(MarkdownTable::nearest(line, 17), 14);
        assert_eq!(MarkdownTable::nearest(line, 19), 14);
        assert_eq!(MarkdownTable::nearest(line, 0), 2);
        assert_eq!(MarkdownTable::nearest(line, 3), 3);
    }

    #[test]
    fn persian_and_wide_letters_keep_the_columns() {
        let table = MarkdownTable::format(&lines(&["| نام | x |", "| --- | --- |", "| 中文 | y |"]));
        // The column is as wide as 中文, two wide letters.
        assert_eq!(table[0], "| نام  | x   |");
        assert_eq!(table[2], "| 中文 | y   |");
    }
}
