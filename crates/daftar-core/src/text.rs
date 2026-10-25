//! Text helpers shared by the parser, the resolver and the index.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;

/// Normalisation shared by the resolver and the index: NFC, and case folded
/// without a locale, so a Turkish dotted I in a file name still matches.
pub mod slugs {
    use super::*;

    pub fn normalize(text: &str) -> String {
        text.nfc().collect()
    }

    pub fn fold(text: &str) -> String {
        normalize(text).to_lowercase()
    }

    /// A heading as `[[Note#Heading]]` names it: Obsidian matches the text,
    /// so this only folds case and collapses white space.
    pub fn heading(text: &str) -> String {
        fold(text).split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// Spaces and tabs, as Foundation's `.whitespaces`: not line breaks.
pub fn is_blank(c: char) -> bool {
    c.is_whitespace() && c != '\n' && c != '\r'
}

/// [text] without spaces and tabs at either end.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_blank)
}

/// [text] without white space at its end.
pub fn trim_end(text: &str) -> &str {
    text.trim_end_matches(char::is_whitespace)
}

/// The lines of [text], split at `\n`, each without a trailing `\r`.
pub fn lines(text: &str) -> Vec<&str> {
    text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect()
}

/// Per-text direction, from the letters in it.
pub mod direction {
    pub fn is_rtl(c: char) -> bool {
        matches!(c as u32,
            0x0590..=0x05FF | 0x0600..=0x06FF | 0x0700..=0x074F | 0x0750..=0x077F | 0x08A0..=0x08FF
            | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
    }

    pub fn contains_rtl(text: &str) -> bool {
        text.chars().any(is_rtl)
    }

    /// From the first strong letter, as `dir="auto"` decides it.
    pub fn starts_rtl(text: &str) -> bool {
        for c in text.chars() {
            if is_rtl(c) {
                return true;
            }
            if c.is_alphabetic() {
                return false;
            }
        }
        false
    }
}

/// The order a file manager shows names in, as Foundation's
/// `localizedStandardCompare`: case is ignored and a run of digits is
/// compared as a number, so `Note 2` comes before `Note 10`.
pub fn standard_compare(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut m = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    m.push(c);
                    x.next();
                }
                let mut n = String::new();
                while let Some(c) = y.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    y.next();
                }
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(p), Some(q)) => {
                let order = p.to_lowercase().cmp(q.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_as_numbers() {
        let mut names = vec!["Note 10", "note 2", "Note 1", "Apple"];
        names.sort_by(|a, b| standard_compare(a, b));
        assert_eq!(names, ["Apple", "Note 1", "note 2", "Note 10"]);
    }

    #[test]
    fn persian_is_right_to_left() {
        assert!(direction::contains_rtl("سلام"));
        assert!(direction::starts_rtl("۱۲ سلام"));
        assert!(!direction::starts_rtl("Hello سلام"));
    }
}
