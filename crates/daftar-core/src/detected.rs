//! Street addresses and phone numbers written as plain text in a note: the
//! reader makes them links, to a map and to the phone. Text in links, code,
//! web addresses, tags and the front matter is left alone, as for unlinked
//! mentions.
//!
//! The Mac finds them with the system's data detector, which Linux has no
//! counterpart of; here a phone number is any run of eight digits or more in
//! the usual separators, and an address is a house number, a few words and a
//! street word (`1 Infinite Loop, Cupertino, CA 95014`). Fewer addresses are
//! found than on the Mac, and none is found that is not one.

use std::sync::LazyLock;

use regex::Regex;

use crate::unlinked_mentions::protected;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetectedKind {
    Address,
    Phone,
}

impl DetectedKind {
    pub fn name(self) -> &'static str {
        match self {
            DetectedKind::Address => "address",
            DetectedKind::Phone => "phone",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DetectedItem {
    pub kind: DetectedKind,
    /// As the note writes it.
    pub text: String,
    /// What the link opens: the address on one line, or the number's digits.
    pub value: String,
    pub line: usize,
}

static PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:\+\d{1,3}[ .-]?)?(?:\(\d{1,5}\)[ .-]?)?\d{2,5}(?:[ .-]?\d{2,5}){1,5}").unwrap());
static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b\d{1,5}[a-zA-Z]?\s+(?:[A-Z][\w'.-]*\s+){0,4}",
        r"(?:Street|St\.?|Avenue|Ave\.?|Road|Rd\.?|Boulevard|Blvd\.?|Lane|Ln\.?|Drive|Dr\.?|Way|Court|Ct\.?|Place|Pl\.?|Square|Sq\.?|Loop|Terrace|Parkway|Pkwy|Highway|Hwy)\b",
        // Then the town, and a state with its ZIP code, before a town's words.
        r"(?:,\s*(?:[A-Z]{2}\s+\d{5}(?:-\d{4})?|[A-Z][\w'.-]*(?:\s+[A-Z][\w'.-]*)*)){0,3}",
    ))
    .unwrap()
});
static DATE_LIKE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}|\d{1,2}[/.]\d{1,2}[/.]\d{2,4}|\d{1,2}:\d{2}").unwrap());

pub struct Detected;

impl Detected {
    pub fn find(text: &str) -> Vec<DetectedItem> {
        let mut found = Vec::new();
        let (mut fenced, mut front) = (false, false);
        for (number, line) in text.split('\n').enumerate() {
            let trimmed = line.trim();
            if number == 0 && trimmed == "---" {
                front = true;
                continue;
            }
            if front {
                if trimmed == "---" || trimmed == "..." {
                    front = false;
                }
                continue;
            }
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                continue;
            }
            if fenced || !trimmed.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            let taken = protected(line);
            let free = |start: usize, end: usize| {
                let chars = line[..start].chars().count()..line[..end].chars().count();
                !taken.iter().any(|t| t.start < chars.end && chars.start < t.end)
            };
            let mut places: Vec<(usize, usize)> = Vec::new();
            for found_at in ADDRESS.find_iter(line).filter(|m| free(m.start(), m.end())) {
                let written = found_at.as_str().trim().to_string();
                places.push((found_at.start(), found_at.end()));
                found.push(DetectedItem { kind: DetectedKind::Address, value: written.clone(), text: written, line: number });
            }
            for found_at in PHONE.find_iter(line) {
                let (start, end) = (found_at.start(), found_at.end());
                // Digits run on: part of a longer number, or of a word.
                let touches = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
                if touches(line[..start].chars().last()) || touches(line[end..].chars().next()) {
                    continue;
                }
                if !free(start, end) || places.iter().any(|(a, b)| *a < end && start < *b) {
                    continue;
                }
                let written = found_at.as_str().trim();
                if !Self::is_phone(written) {
                    continue;
                }
                let value: String = written.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
                found.push(DetectedItem { kind: DetectedKind::Phone, text: written.to_string(), value, line: number });
            }
        }
        found
    }

    /// A phone number, not a date, a time or a short count: eight digits or more.
    pub fn is_phone(text: &str) -> bool {
        text.chars().filter(char::is_ascii_digit).count() >= 8 && !DATE_LIKE.is_match(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_phones_are_found() {
        let text = "---\nphone: +34 612 345 678\n---\nCall the office at +34 612 345 678 before noon.\nThe flat is at 1 Infinite Loop, Cupertino, CA 95014.\n- [ ] Pay rent 📅 2026-10-30 at 10:30\n`+34 699 999 999` and [[Note +34 655 555 555]]";
        let found = Detected::find(text);
        let phones: Vec<&DetectedItem> = found.iter().filter(|f| f.kind == DetectedKind::Phone).collect();
        assert_eq!(phones.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["+34 612 345 678"]);
        assert_eq!(phones[0].value, "+34612345678");
        assert_eq!(phones[0].line, 3);
        let places: Vec<&DetectedItem> = found.iter().filter(|f| f.kind == DetectedKind::Address).collect();
        assert_eq!(places.len(), 1);
        assert!(places[0].text.starts_with("1 Infinite Loop"));
        assert_eq!(places[0].text, "1 Infinite Loop, Cupertino, CA 95014");
    }

    #[test]
    fn dates_and_short_numbers_are_not_phones() {
        assert!(!Detected::is_phone("2026-10-30"));
        assert!(!Detected::is_phone("12345"));
        assert!(Detected::is_phone("(021) 8888 1234"));
        assert!(Detected::find("Order 12 of 2026 costs 30.").is_empty());
    }
}
