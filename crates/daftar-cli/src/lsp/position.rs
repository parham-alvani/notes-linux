//! Positions as the Language Server Protocol counts them: a line from 0,
//! split at `\n`, and a character that is a UTF-16 code unit, so a Persian
//! letter is one and an emoji past U+FFFF is two. The core counts
//! characters (Unicode scalar values) and Rust's strings bytes; these
//! convert between the three.

use serde_json::{Value, json};

/// Line [n] of [text], from 0, without its `\n`; empty past the end.
pub fn line(text: &str, n: usize) -> &str {
    text.split('\n').nth(n).unwrap_or("")
}

/// How many UTF-16 code units [text] takes.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The UTF-16 offset of byte [byte] of [line]; past the end is the end.
pub fn utf16_of_byte(line: &str, byte: usize) -> usize {
    let mut byte = byte.min(line.len());
    while !line.is_char_boundary(byte) {
        byte -= 1;
    }
    utf16_len(&line[..byte])
}

/// The UTF-16 offset of the [chars]th character of [line], as the core and
/// the lint's columns count them; past the end is the end.
pub fn utf16_of_chars(line: &str, chars: usize) -> usize {
    line.chars().take(chars).map(char::len_utf16).sum()
}

/// The byte of [line] at UTF-16 offset [unit]: past the end is the end, and
/// an offset inside a character (between the two halves of an emoji) is
/// that character's start.
pub fn byte_of_utf16(line: &str, unit: usize) -> usize {
    let mut seen = 0;
    for (byte, c) in line.char_indices() {
        let next = seen + c.len_utf16();
        if next > unit {
            return byte;
        }
        seen = next;
    }
    line.len()
}

/// `{line, character}`.
pub fn at(line: usize, character: usize) -> Value {
    json!({ "line": line, "character": character })
}

/// `{start, end}` on one line.
pub fn on_line(line: usize, start: usize, end: usize) -> Value {
    json!({ "start": at(line, start), "end": at(line, end) })
}

/// Where [text] ends: after its last `\n`, the line after it.
pub fn end(text: &str) -> Value {
    let last = text.split('\n').count() - 1;
    at(last, utf16_len(line(text, last)))
}

/// The one edit that makes [old] into [new]: the whole lines between what
/// the two share at the start and at the end, or none when they are the
/// same. Formatting a note changes a few lines, so this is what the editor
/// gets, not the whole note, and its cursor and marks elsewhere stay.
pub fn line_edit(old: &str, new: &str) -> Option<Value> {
    if old == new {
        return None;
    }
    let (a, b): (Vec<&str>, Vec<&str>) = (old.split_inclusive('\n').collect(), new.split_inclusive('\n').collect());
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // A last line with no `\n` is not the same line as one with it.
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y && x.ends_with('\n')).count();
    let (start, stop) = (prefix, a.len() - suffix);
    let end = if stop == a.len() { end(old) } else { at(stop, 0) };
    Some(json!({ "range": { "start": at(start, 0), "end": end }, "newText": b[prefix..b.len() - suffix].concat() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Persian is one unit a letter; an emoji past U+FFFF is two.
    #[test]
    fn a_character_is_counted_in_utf16_units() {
        let line = "سلام 😀 [[یادداشت]]";
        assert_eq!(utf16_len("سلام"), 4);
        assert_eq!(utf16_len("😀"), 2);
        // The `[[` starts after 4 letters, a space, the emoji's two units and a space.
        let byte = line.find("[[").unwrap();
        assert_eq!(utf16_of_byte(line, byte), 8);
        assert_eq!(byte_of_utf16(line, 8), byte);
        assert_eq!(utf16_of_chars(line, 7), 8);
        // Between the emoji's halves is the emoji's start.
        assert_eq!(byte_of_utf16(line, 6), line.find('😀').unwrap());
        assert_eq!(byte_of_utf16(line, 5), line.find('😀').unwrap());
        // Past the end is the end.
        assert_eq!(byte_of_utf16(line, 100), line.len());
        assert_eq!(utf16_of_byte(line, 1000), utf16_len(line));
        assert_eq!(utf16_of_chars("ab", 9), 2);
    }

    #[test]
    fn a_line_and_the_end_of_a_text() {
        assert_eq!(line("a\nب\n", 1), "ب");
        assert_eq!(line("a", 4), "");
        assert_eq!(end("a\n😀"), at(1, 2));
        assert_eq!(end("a\n"), at(1, 0));
        assert_eq!(end(""), at(0, 0));
    }

    fn applied(old: &str, edit: &Value) -> String {
        let offset = |p: &Value| {
            let (l, c) = (p["line"].as_u64().unwrap() as usize, p["character"].as_u64().unwrap() as usize);
            old.split_inclusive('\n').take(l).map(str::len).sum::<usize>() + byte_of_utf16(line(old, l), c)
        };
        let (start, stop) = (offset(&edit["range"]["start"]), offset(&edit["range"]["end"]));
        format!("{}{}{}", &old[..start], edit["newText"].as_str().unwrap(), &old[stop..])
    }

    #[test]
    fn an_edit_replaces_only_the_lines_that_changed() {
        assert_eq!(line_edit("a\n", "a\n"), None);
        let edit = line_edit("# T\n*  item\ntext\n", "# T\n- item\ntext\n").unwrap();
        assert_eq!(edit["range"], json!({ "start": at(1, 0), "end": at(2, 0) }));
        assert_eq!(edit["newText"], "- item\n");
        for (old, new) in [
            ("a\nb", "a\nb\n"),
            ("a\nb\n", "a\nb"),
            ("", "# A\n"),
            ("x\n", ""),
            ("😀 a\nسلام\n", "😀 a\n\nسلام\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nb\nc", "z\nb\nc"),
        ] {
            assert_eq!(applied(old, &line_edit(old, new).unwrap()), new, "{old:?} → {new:?}");
        }
    }
}
