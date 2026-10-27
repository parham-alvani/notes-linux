//! The places where a note's name, or one of its aliases, is written in
//! another note without a link, as Obsidian's "Unlinked mentions" lists
//! them. A mention is the name as a whole word, in any case and with or
//! without accents, outside links, code, web addresses, tags and the front
//! matter. Columns and lengths are in characters.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::vault_edits::Failure;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Mention {
    /// Zero-based line in the file.
    pub line: usize,
    /// Where the name starts in the line, and its length, in characters.
    pub column: usize,
    pub length: usize,
    /// The name as the note writes it.
    pub text: String,
    /// The line around the name, and where the name is in it (characters).
    pub excerpt: String,
    pub excerpt_range: Range<usize>,
}

/// Links, code, web addresses and tags: text a mention cannot be in. The
/// tag's group 1 is the tag; the character before it is not protected.
static PATTERNS: LazyLock<[Regex; 5]> = LazyLock::new(|| {
    [r"!?\[\[[^\]]*\]\]", r"!?\[[^\]]*\]\([^)]*\)", r"`[^`]*`", r"[a-zA-Z][a-zA-Z0-9+.-]*://\S+", r"(?:^|[^\p{L}\p{N}])(#[\p{L}\p{N}_/-]+)"]
        .map(|p| Regex::new(p).unwrap())
});

pub struct UnlinkedMentions;

impl UnlinkedMentions {
    /// Names shorter than this match too many words to be worth a list.
    pub const SHORTEST: usize = 3;

    /// The mentions of [names] in [text]: each place once, the longest name
    /// that matches there.
    pub fn find(names: &[String], text: &str) -> Vec<Mention> {
        let mut names: Vec<&String> =
            names.iter().filter(|n| n.chars().count() >= Self::SHORTEST).collect::<BTreeSet<_>>().into_iter().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));
        if names.is_empty() {
            return vec![];
        }
        let folded_names: Vec<String> = names.iter().map(|n| n.chars().map(fold).collect()).collect();
        let mut found = Vec::new();
        let (mut fenced, mut front) = (false, false);
        for (number, line) in text.split('\n').enumerate() {
            let trimmed = line.trim_matches([' ', '\t']);
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
            if fenced {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let folded: Vec<String> = chars.iter().map(|c| fold(*c)).collect();
            let mut taken = protected(line);
            for name in &folded_names {
                let mut from = 0;
                while let Some(range) = search(&folded, name, from) {
                    from = range.end;
                    if !is_word(&range, &chars) || taken.iter().any(|t| t.start < range.end && range.start < t.end) {
                        continue;
                    }
                    found.push(mention(&range, number, &chars));
                    taken.push(range);
                }
            }
        }
        found.sort_by_key(|m| (m.line, m.column));
        found
    }

    /// [text] with the mention a link: `[[Name]]` when it is written as the
    /// name is, `[[Name|as written]]` when not. Refuses when the line no
    /// longer holds it where it was found.
    pub fn link(mention: &Mention, name: &str, text: &str) -> Result<String, Failure> {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        let line = lines.get(mention.line).ok_or(Failure::TextMoved)?;
        let chars: Vec<char> = line.chars().collect();
        let end = mention.column + mention.length;
        if end > chars.len() || chars[mention.column..end].iter().collect::<String>() != mention.text {
            return Err(Failure::TextMoved);
        }
        let link = if mention.text == name { format!("[[{name}]]") } else { format!("[[{name}|{}]]", mention.text) };
        lines[mention.line] =
            format!("{}{link}{}", chars[..mention.column].iter().collect::<String>(), chars[end..].iter().collect::<String>());
        Ok(lines.join("\n"))
    }
}

/// A character without its case and its accents, as a search that ignores
/// both compares it: `É` is `e`.
fn fold(c: char) -> String {
    c.to_string().nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// The first run of characters from [from] whose folded text is [name].
fn search(folded: &[String], name: &str, from: usize) -> Option<Range<usize>> {
    (from..folded.len()).find_map(|start| {
        let mut joined = String::new();
        for (end, piece) in folded.iter().enumerate().skip(start) {
            joined.push_str(piece);
            if joined.len() >= name.len() {
                return (joined == name && !piece.is_empty()).then_some(start..end + 1);
            }
            if !name.starts_with(&joined) {
                return None;
            }
        }
        None
    })
}

/// Character ranges of the protected text in [line].
pub(crate) fn protected(line: &str) -> Vec<Range<usize>> {
    let chars = |bytes: Range<usize>| line[..bytes.start].chars().count()..line[..bytes.end].chars().count();
    let mut out = Vec::new();
    for (at, pattern) in PATTERNS.iter().enumerate() {
        for caps in pattern.captures_iter(line) {
            let found = if at == 4 { caps.get(1) } else { caps.get(0) };
            if let Some(found) = found {
                out.push(chars(found.range()));
            }
        }
    }
    out
}

/// No letter, digit or mark right before or after: "Acme" in "Acme's", not in "Acmes".
fn is_word(range: &Range<usize>, chars: &[char]) -> bool {
    let letter = |at: Option<usize>| at.and_then(|i| chars.get(i)).is_some_and(|c| c.is_alphanumeric() || is_combining_mark(*c));
    !letter(range.start.checked_sub(1)) && !letter(Some(range.end))
}

/// The mention, with up to 60 characters of the line on each side.
fn mention(range: &Range<usize>, line: usize, chars: &[char]) -> Mention {
    let start = range.start.saturating_sub(60);
    let end = (range.end + 60).min(chars.len());
    let around = &chars[start..end];
    let lead = around.iter().take_while(|c| **c == ' ' || **c == '\t').count();
    let mut excerpt: String = around[lead..].iter().collect();
    let mut offset = range.start - start - lead;
    if start > 0 {
        excerpt.insert(0, '…');
        offset += 1;
    }
    if end < chars.len() {
        excerpt.push('…');
    }
    Mention {
        line,
        column: range.start,
        length: range.len(),
        text: chars[range.clone()].iter().collect(),
        excerpt,
        excerpt_range: offset..offset + range.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn find(names: &[&str], text: &str) -> Vec<Mention> {
        UnlinkedMentions::find(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>(), text)
    }

    fn excerpt_part(m: &Mention) -> String {
        m.excerpt.chars().skip(m.excerpt_range.start).take(m.excerpt_range.len()).collect()
    }

    #[test]
    fn a_name_is_found_as_a_whole_word_in_any_case() {
        let found = find(&["Acme"], "Lunch with acme today.\nAcme's offer came.\nAcmes and Acmeville are other words.");
        assert_eq!(found.iter().map(|m| m.line).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(found.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["acme", "Acme"]);
        assert_eq!((found[0].column, found[0].length), (11, 4));
    }

    #[test]
    fn links_code_and_tags_are_not_mentions() {
        let text = "---
company: Acme
---
See [[Acme]] and [the site](https://acme.example/Acme) and `Acme` and #Acme.
```
Acme in code
```
Only this Acme counts.";
        let found = find(&["Acme"], text);
        assert_eq!(found.iter().map(|m| m.line).collect::<Vec<_>>(), [7]);
        assert_eq!(found[0].excerpt, "Only this Acme counts.");
        assert_eq!(excerpt_part(&found[0]), "Acme");
    }

    #[test]
    fn the_longest_name_wins_and_short_names_are_skipped() {
        let found = find(&["Acme", "Acme Bank", "AB"], "Paid Acme Bank, AB said.");
        assert_eq!(found.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Acme Bank"]);
        assert!(find(&["Go"], "Go is short").is_empty());
    }

    #[test]
    fn persian_names_are_found_too() {
        let found = find(&["تهران"], "سفر به تهران در پاییز");
        assert_eq!(found.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["تهران"]);
    }

    #[test]
    fn accents_do_not_matter() {
        let found = find(&["Café"], "We met at the cafe.");
        assert_eq!(found.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["cafe"]);
    }

    #[test]
    fn a_long_line_is_cut_around_the_name() {
        let line = format!("{}Acme{}", "word ".repeat(30), " word".repeat(30));
        let found = find(&["Acme"], &line);
        assert!(found[0].excerpt.starts_with('…') && found[0].excerpt.ends_with('…'));
        assert_eq!(excerpt_part(&found[0]), "Acme");
    }

    #[test]
    fn linking_keeps_the_text_as_written() {
        let text = "Lunch with acme today.\nAcme called.";
        let found = find(&["Acme"], text);
        assert_eq!(UnlinkedMentions::link(&found[0], "Acme", text).unwrap(), "Lunch with [[Acme|acme]] today.\nAcme called.");
        assert_eq!(UnlinkedMentions::link(&found[1], "Acme", text).unwrap(), "Lunch with acme today.\n[[Acme]] called.");
        // The file changed since: nothing is written.
        assert_eq!(UnlinkedMentions::link(&found[1], "Acme", "Other text.\nNo name."), Err(Failure::TextMoved));
    }
}
