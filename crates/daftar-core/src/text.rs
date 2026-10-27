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

    /// [fold], and Persian as people type it on different keyboards made
    /// one: Arabic Yeh and Kaf as the Persian letters, the hamza forms of
    /// Alef, Waw and Yeh as the letters under them, Persian and Arabic
    /// digits as ASCII ones, and the half space (ZWNJ) as a space. Each
    /// character stays one, so offsets into the folded text are offsets
    /// into the text. For matching only: a link still resolves by [fold].
    pub fn search(text: &str) -> String {
        fold(text).chars().map(|c| if c == '\u{200c}' { ' ' } else { persian_letter(c) }).collect()
    }

    /// The letter a Persian search finds [c] as: see [search].
    pub fn persian_letter(c: char) -> char {
        match c {
            'ي' | 'ى' | 'ئ' => 'ی',
            'ك' => 'ک',
            'ة' => 'ه',
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ؤ' => 'و',
            '۰'..='۹' => char::from(b'0' + (c as u32 - '۰' as u32) as u8),
            '٠'..='٩' => char::from(b'0' + (c as u32 - '٠' as u32) as u8),
            _ => c,
        }
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
    /// The direction of a markdown line as it reads, from its first strong
    /// letter after the syntax a reader does not show: the list mark, the
    /// task box, the heading and quote marks, a callout's type and a
    /// wikilink's target before its alias. `- [x] سلام` is right to left,
    /// though `x` is a Latin letter. `None` when it has no letter at all.
    pub fn of_line(line: &str) -> Option<bool> {
        let mut rest = line.trim_start();
        loop {
            let before = rest;
            rest = rest.trim_start_matches(['>', '#', ' ', '\t']);
            for mark in ["- ", "* ", "+ "] {
                rest = rest.strip_prefix(mark).unwrap_or(rest);
            }
            let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
            if digits > 0 && rest[digits..].starts_with(['.', ')']) {
                rest = &rest[digits + 1..];
            }
            if let Some(after) = rest.strip_prefix('[') {
                let mut chars = after.chars();
                if let (Some(_), Some(']')) = (chars.next(), chars.next()) {
                    rest = chars.as_str();
                } else if let Some(kind) = after.strip_prefix('!') {
                    rest = kind.find(']').map_or(rest, |end| &kind[end + 1..]);
                }
            }
            if rest == before {
                break;
            }
        }
        let mut chars = rest.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            // `[[target|alias]]` reads as its alias.
            if rest[at..].starts_with("[[") {
                let inner = &rest[at + 2..];
                let close = inner.find("]]").unwrap_or(inner.len());
                if let Some(bar) = inner[..close].find('|') {
                    let shown = &inner[bar + 1..close];
                    if let Some(found) = first_strong(shown) {
                        return Some(found);
                    }
                    let skip = at + 2 + close;
                    while chars.peek().is_some_and(|(next, _)| *next < skip) {
                        chars.next();
                    }
                    continue;
                }
            }
            if is_rtl(c) {
                return Some(true);
            }
            if c.is_alphabetic() {
                return Some(false);
            }
        }
        None
    }

    /// Whether the first letter of [text] is right to left, as a text
    /// layout finds a paragraph's direction; `None` with no letter.
    pub fn first_strong(text: &str) -> Option<bool> {
        text.chars().find(|&c| is_rtl(c) || c.is_alphabetic()).map(is_rtl)
    }

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

    #[test]
    fn persian_is_searched_as_it_is_meant() {
        assert_eq!(slugs::search("علي كتاب ۱۲"), "علی کتاب 12");
        assert_eq!(slugs::search("می\u{200c}خواهم"), "می خواهم");
        assert_eq!(slugs::search("پائیز آب").chars().count(), "پائیز آب".chars().count());
        assert_eq!(slugs::search("پائیز آب"), "پاییز اب");
        assert_eq!(slugs::search("Hello"), "hello");
    }

    #[test]
    fn a_line_reads_in_the_direction_of_its_text_not_its_syntax() {
        assert_eq!(direction::of_line("- [x] تماس با علی"), Some(true));
        assert_eq!(direction::of_line("  * [ ] buy milk"), Some(false));
        assert_eq!(direction::of_line("12. مورد"), Some(true));
        assert_eq!(direction::of_line("## کارها"), Some(true));
        assert_eq!(direction::of_line("> [!note] یادداشت"), Some(true));
        assert_eq!(direction::of_line("- [[Start|پیوند]] و"), Some(true));
        assert_eq!(direction::of_line("[[Start]] سلام"), Some(false));
        assert_eq!(direction::of_line("Mixed فارسی"), Some(false));
        assert_eq!(direction::of_line("- [ ] 12:30"), None);
        assert_eq!(direction::of_line(""), None);
    }
}
