//! The places where a note's name, or one of its aliases, is written in
//! another note without a link, as Obsidian's "Unlinked mentions" lists
//! them. A mention is the name as a whole word, in any case and with or
//! without accents, outside links, code, web addresses, tags and the front
//! matter. Columns and lengths are in characters.

use std::collections::HashMap;
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
        MentionFinder::new(names).find(text).into_iter().map(|(_, m)| m).collect()
    }

    /// [name] as [find] compares it, without its case and its accents: two
    /// names with the same folded text are one name to it.
    pub fn folded(name: &str) -> String {
        name.chars().map(fold).collect()
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
        // In a table row the `|` is escaped, as Obsidian writes it there.
        let bar = if line.trim_start().starts_with('|') { "\\|" } else { "|" };
        let link = if mention.text == name { format!("[[{name}]]") } else { format!("[[{name}{bar}{}]]", mention.text) };
        lines[mention.line] =
            format!("{}{link}{}", chars[..mention.column].iter().collect::<String>(), chars[end..].iter().collect::<String>());
        Ok(lines.join("\n"))
    }
}

/// The names of [UnlinkedMentions::find] made ready once, for a text or
/// for every note of a vault: a tree of their folded characters, walked
/// from each place a word can start, so a line is read once whatever the
/// number of names.
pub struct MentionFinder {
    /// Each node's children by folded character; the root is node 0.
    next: Vec<HashMap<char, usize>>,
    /// The name a node ends, by its index in the names given.
    ends: Vec<Option<usize>>,
    /// Each name's length in characters, as given.
    lengths: Vec<usize>,
}

impl MentionFinder {
    pub fn new(names: &[String]) -> MentionFinder {
        let mut finder = MentionFinder { next: vec![HashMap::new()], ends: vec![None], lengths: Vec::with_capacity(names.len()) };
        for (at, name) in names.iter().enumerate() {
            let length = name.chars().count();
            finder.lengths.push(length);
            if length < UnlinkedMentions::SHORTEST {
                continue;
            }
            let mut node = 0;
            for c in UnlinkedMentions::folded(name).chars() {
                node = if let Some(&child) = finder.next[node].get(&c) {
                    child
                } else {
                    finder.next.push(HashMap::new());
                    finder.ends.push(None);
                    let child = finder.next.len() - 1;
                    finder.next[node].insert(c, child);
                    child
                };
            }
            // The first of two names that fold the same stands for both.
            if node != 0 && finder.ends[node].is_none() {
                finder.ends[node] = Some(at);
            }
        }
        finder
    }

    /// The mentions in [text], in order, each with the index of its name
    /// in the names given: each place once, the longest name that matches
    /// there.
    pub fn find(&self, text: &str) -> Vec<(usize, Mention)> {
        let mut found = Vec::new();
        if self.next[0].is_empty() {
            return found;
        }
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
            let mut candidates = self.candidates(&chars, &folded);
            if candidates.is_empty() {
                continue;
            }
            candidates.sort_by_key(|(name, range)| (std::cmp::Reverse(self.lengths[*name]), range.start));
            let mut taken = protected(line);
            let mut kept = Vec::new();
            for (name, range) in candidates {
                if taken.iter().any(|t| t.start < range.end && range.start < t.end) {
                    continue;
                }
                kept.push((name, mention(&range, number, &chars)));
                taken.push(range);
            }
            kept.sort_by_key(|(_, m)| m.column);
            found.extend(kept);
        }
        found
    }

    /// Every name that matches [chars] as a whole word, with where.
    fn candidates(&self, chars: &[char], folded: &[String]) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        for start in 0..chars.len() {
            if start > 0 && is_letter(chars[start - 1]) {
                continue;
            }
            let mut node = 0;
            'walk: for (end, piece) in folded.iter().enumerate().skip(start) {
                for c in piece.chars() {
                    match self.next[node].get(&c) {
                        Some(&child) => node = child,
                        None => break 'walk,
                    }
                }
                if let Some(name) = self.ends[node]
                    && !piece.is_empty()
                    && !chars.get(end + 1).is_some_and(|c| is_letter(*c))
                {
                    out.push((name, start..end + 1));
                }
            }
        }
        out
    }
}

/// A character without its case and its accents, as a search that ignores
/// both compares it: `É` is `e`.
fn fold(c: char) -> String {
    if c.is_ascii() {
        return c.to_ascii_lowercase().to_string();
    }
    c.to_string().nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
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

/// A letter, digit or mark: a name is a whole word with none right
/// before or after it, "Acme" in "Acme's", not in "Acmes".
fn is_letter(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
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
        UnlinkedMentions::find(&names.iter().map(ToString::to_string).collect::<Vec<_>>(), text)
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

    #[test]
    fn linking_in_a_table_row_escapes_the_bar() {
        let text = "| Who | What |\n| --- | --- |\n| acme | rent |";
        let found = find(&["Acme"], text);
        assert_eq!(UnlinkedMentions::link(&found[0], "Acme", text).unwrap().split('\n').nth(2), Some("| [[Acme\\|acme]] | rent |"));
    }

    #[test]
    fn a_finder_names_the_name_it_found_for_every_text() {
        let names: Vec<String> = ["Acme", "Globex Bank", "Initech"].map(String::from).to_vec();
        let finder = MentionFinder::new(&names);
        let found = finder.find("Globex bank paid acme.\nInitech, and Initech again.");
        let named: Vec<(usize, usize, &str)> = found.iter().map(|(n, m)| (*n, m.line, m.text.as_str())).collect();
        assert_eq!(named, [(1, 0, "Globex bank"), (0, 0, "acme"), (2, 1, "Initech"), (2, 1, "Initech")]);
        assert!(finder.find("Acmes and Initechs are other words.").is_empty());
        assert_eq!(UnlinkedMentions::folded("Café Acme"), "cafe acme");
    }
}
