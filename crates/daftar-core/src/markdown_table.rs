//! A Markdown table in the editor: Tab goes to the next cell and lines the
//! columns up, as Obsidian's table editor does, and Return at the end of a
//! row starts a new one. Offsets in a line are in characters.

use std::sync::LazyLock;

use regex::Regex;

static SEPARATOR_CELL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^:?-+:?$").unwrap());

/// Where Tab leaves the table and the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTab {
    pub lines: Vec<String>,
    pub row: usize,
    pub column: usize,
}

pub struct MarkdownTable;

impl MarkdownTable {
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
        let indent: String = lines.first().map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect()).unwrap_or_default();
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

    #[test]
    fn persian_and_wide_letters_keep_the_columns() {
        let table = MarkdownTable::format(&lines(&["| نام | x |", "| --- | --- |", "| 中文 | y |"]));
        // The column is as wide as 中文, two wide letters.
        assert_eq!(table[0], "| نام  | x   |");
        assert_eq!(table[2], "| 中文 | y   |");
    }
}
