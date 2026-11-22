//! Focus mode's paragraph (`docs/specs/look.md`, part 2): the editor dims
//! every paragraph but the caret's. A paragraph is the run of lines around
//! the caret with no blank line in it, so a list, a table, a callout and a
//! quote are each one; on a blank line it is that line alone. Every
//! paragraph a selection touches stays bright. Focus mode changes how the
//! text is drawn and nothing else.

use std::ops::Range;

pub struct FocusParagraph;

impl FocusParagraph {
    /// The lines that stay bright, first and last, for a selection that
    /// starts on line [from] and ends on line [to] of a note of [count]
    /// lines; [blank] says whether a line holds nothing but spaces. An
    /// editor asks its buffer for the lines, so the note is never copied.
    pub fn lines(from: usize, to: usize, count: usize, blank: impl Fn(usize) -> bool) -> (usize, usize) {
        let (mut first, mut last) = (from.min(to), from.max(to).min(count.saturating_sub(1)));
        first = first.min(last);
        if !blank(first) {
            while first > 0 && !blank(first - 1) {
                first -= 1;
            }
        }
        if !blank(last) {
            while last + 1 < count && !blank(last + 1) {
                last += 1;
            }
        }
        (first, last)
    }

    /// The characters of [text] that stay bright for [selection], a caret
    /// when it is empty: from the start of the first line to the end of
    /// the last, without its line ending.
    pub fn range(text: &str, selection: Range<usize>) -> Range<usize> {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut starts = Vec::with_capacity(lines.len());
        let mut at = 0;
        for line in &lines {
            starts.push(at);
            at += line.chars().count() + 1;
        }
        let line_of = |offset: usize| starts.partition_point(|start| *start <= offset).saturating_sub(1);
        let (first, last) =
            Self::lines(line_of(selection.start), line_of(selection.end), lines.len(), |line| lines[line].trim().is_empty());
        starts[first]..starts[last] + lines[last].chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // docs/specs/look.md, part 2, case 17.
    #[test]
    fn focus_keeps_the_carets_paragraph() {
        let text = "a\n\nb\nc\n\nd";
        let bright = |selection: Range<usize>| {
            let range = FocusParagraph::range(text, selection);
            text.chars().skip(range.start).take(range.len()).collect::<String>()
        };
        // The caret in `c`: `b` and `c`.
        assert_eq!(bright(5..5), "b\nc");
        assert_eq!(bright(3..3), "b\nc");
        // On a blank line, that line alone.
        assert_eq!(FocusParagraph::range(text, 2..2), 2..2);
        assert_eq!(FocusParagraph::range(text, 7..7), 7..7);
        // A selection from `a` to `b`: both paragraphs, with the line between them.
        assert_eq!(bright(0..3), "a\n\nb\nc");
        assert_eq!(bright(0..1), "a");
        assert_eq!(bright(8..9), "d");
        // A list, a table and a callout with no blank line are each one paragraph.
        let list = "x\n\n- one\n- two\n  - three\n\n| a |\n| - |\n\n> [!tip]\n> body";
        let of = |selection: Range<usize>| {
            let range = FocusParagraph::range(list, selection);
            list.chars().skip(range.start).take(range.len()).collect::<String>()
        };
        assert_eq!(of(12..12), "- one\n- two\n  - three");
        assert_eq!(of(28..28), "| a |\n| - |");
        assert_eq!(of(50..50), "> [!tip]\n> body");
        // By lines, as an editor asks its buffer.
        let blank = |line: usize| [false, true, false, false, true, false][line];
        assert_eq!(FocusParagraph::lines(3, 3, 6, blank), (2, 3));
        assert_eq!(FocusParagraph::lines(4, 4, 6, blank), (4, 4));
        assert_eq!(FocusParagraph::lines(2, 0, 6, blank), (0, 3));
        assert_eq!(FocusParagraph::lines(9, 9, 6, blank), (5, 5));
    }
}
