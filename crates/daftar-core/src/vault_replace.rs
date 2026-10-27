//! Find and replace in the text of a note, for "Replace in Vault": the
//! matches with their lines and a preview, and the text with a chosen part
//! of them replaced. A pattern is plain text, or a regular expression whose
//! replacement can name its groups as `$1`. Code blocks and inline code can
//! be left out, as `TagRename` leaves them.
//!
//! Unlike the Mac, whose ranges are UTF-16 units, a match's range here is
//! in bytes of the text.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;
use std::sync::LazyLock;

use fancy_regex::Regex;

use crate::tag_rename::FENCE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReplaceOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regular_expression: bool,
    /// Leave fenced code blocks and inline code as they are.
    pub skip_code: bool,
}

impl Default for ReplaceOptions {
    fn default() -> ReplaceOptions {
        ReplaceOptions { match_case: false, whole_word: false, regular_expression: false, skip_code: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceFailure {
    /// The regular expression does not compile; the library's message.
    BadPattern(String),
    /// The note's matches are not the ones the preview showed.
    Changed,
}

impl fmt::Display for ReplaceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplaceFailure::BadPattern(message) => write!(f, "The regular expression is not valid: {message}"),
            ReplaceFailure::Changed => write!(f, "The note changed after the preview."),
        }
    }
}

impl std::error::Error for ReplaceFailure {}

/// One match in a text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReplaceMatch {
    /// In bytes of the whole text.
    pub location: usize,
    pub length: usize,
    /// The match's first line, from 0.
    pub line: usize,
    pub found: String,
    pub replacement: String,
    /// The rest of the line before and after the match, cut short when long.
    pub prefix: String,
    pub suffix: String,
}

impl ReplaceMatch {
    pub fn id(&self) -> usize {
        self.location
    }

    /// The match's line as it is; a line break shows as ↵.
    pub fn before(&self) -> String {
        format!("{}{}{}", self.prefix, flat(&self.found), self.suffix)
    }

    /// The match's line as it will be.
    pub fn after(&self) -> String {
        format!("{}{}{}", self.prefix, flat(&self.replacement), self.suffix)
    }
}

/// A find pattern and its replacement, compiled once for many notes.
#[derive(Debug, Clone)]
pub struct ReplacePattern {
    regex: Regex,
    template: String,
    skip_code: bool,
}

impl ReplacePattern {
    /// `None` for an empty find text; an error for a regular expression that does not compile.
    pub fn new(find: &str, replace: &str, options: ReplaceOptions) -> Result<Option<ReplacePattern>, ReplaceFailure> {
        if find.is_empty() {
            return Ok(None);
        }
        let mut pattern = if options.regular_expression { find.to_string() } else { fancy_regex::escape(find).into_owned() };
        // A word is letters, digits and `_` in every script; `\b` would split at Persian joiners.
        if options.whole_word {
            pattern = format!(r"(?<![\p{{L}}\p{{N}}_])(?:{pattern})(?![\p{{L}}\p{{N}}_])");
        }
        let flags = if options.match_case { "(?m)" } else { "(?mi)" };
        let regex = Regex::new(&format!("{flags}{pattern}")).map_err(|e| ReplaceFailure::BadPattern(e.to_string()))?;
        let template = if options.regular_expression { template(replace) } else { replace.replace('$', "$$") };
        Ok(Some(ReplacePattern { regex, template, skip_code: options.skip_code }))
    }
}

/// A replacement as the Mac writes it, `$1` and `\$`, in the library's
/// terms: `${1}`, so `$1a` is group 1 and an `a`, and `$$`.
fn template(replace: &str) -> String {
    let mut out = String::new();
    let mut chars = replace.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('$') => out.push_str("$$"),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            '$' if chars.peek().is_some_and(char::is_ascii_digit) => {
                out.push_str("${");
                while let Some(d) = chars.next_if(char::is_ascii_digit) {
                    out.push(d);
                }
                out.push('}');
            }
            '$' => out.push_str("$$"),
            _ => out.push(c),
        }
    }
    out
}

pub struct VaultReplace;

impl VaultReplace {
    /// The matches of [find] in [text], in order.
    pub fn matches(text: &str, find: &str, replace: &str, options: ReplaceOptions) -> Result<Vec<ReplaceMatch>, ReplaceFailure> {
        Ok(ReplacePattern::new(find, replace, options)?.map(|p| Self::matches_of(text, &p)).unwrap_or_default())
    }

    pub fn matches_of(text: &str, pattern: &ReplacePattern) -> Vec<ReplaceMatch> {
        let code = if pattern.skip_code { code_ranges(text) } else { vec![] };
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let line = |location: usize| starts.partition_point(|&s| s <= location) - 1;
        let line_end = |index: usize| if index + 1 < starts.len() { starts[index + 1] - 1 } else { text.len() };

        let mut out = Vec::new();
        for captures in pattern.regex.captures_iter(text) {
            // A pattern that runs too long on a note stops there.
            let Ok(captures) = captures else { break };
            let Some(whole) = captures.get(0) else { continue };
            let range = whole.start()..whole.end();
            if code.iter().any(|c| (c.start < range.end && range.start < c.end) || (range.is_empty() && c.contains(&range.start))) {
                continue;
            }
            let (first, last) = (line(range.start), line(range.end));
            let prefix = &text[starts[first]..range.start];
            let suffix = &text[range.end..line_end(last).max(range.end)];
            let mut replacement = String::new();
            captures.expand(&pattern.template, &mut replacement);
            out.push(ReplaceMatch {
                location: range.start,
                length: range.len(),
                line: first,
                found: whole.as_str().to_string(),
                replacement,
                prefix: flat(&shortened(prefix, true)),
                suffix: flat(&shortened(suffix, false)),
            });
        }
        out
    }

    /// [text] with the [chosen] matches replaced. A match whose range is
    /// not in the text, or overlaps one after it, is left out.
    pub fn apply(chosen: &[ReplaceMatch], text: &str) -> String {
        let mut sorted: Vec<&ReplaceMatch> = chosen.iter().collect();
        sorted.sort_by_key(|m| std::cmp::Reverse(m.location));
        let mut out = text.to_string();
        let mut end = usize::MAX;
        for m in sorted {
            let stop = m.location + m.length;
            if stop > end.min(out.len()) || !out.is_char_boundary(m.location) || !out.is_char_boundary(stop) {
                continue;
            }
            out.replace_range(m.location..stop, &m.replacement);
            end = m.location;
        }
        out
    }

    /// The text of a note read again before a write: its matches must be
    /// the [shown] ones, or the note is refused; then the [chosen] ids are replaced.
    pub fn replace(
        text: &str,
        pattern: &ReplacePattern,
        shown: &[ReplaceMatch],
        chosen: &BTreeSet<usize>,
    ) -> Result<String, ReplaceFailure> {
        let now = Self::matches_of(text, pattern);
        if now != shown {
            return Err(ReplaceFailure::Changed);
        }
        let picked: Vec<ReplaceMatch> = now.into_iter().filter(|m| chosen.contains(&m.id())).collect();
        Ok(Self::apply(&picked, text))
    }
}

static INLINE_CODE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"`[^`\n]*`").unwrap());

/// Fenced code blocks, with their fence lines, and inline code spans.
fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fence: Option<String> = None;
    let mut block_start = 0;
    for line in text.split('\n') {
        if let Some(c) = FENCE.captures(line) {
            let marker = c[1].to_string();
            match &fence {
                None => {
                    fence = Some(marker);
                    block_start = offset;
                }
                Some(open) if marker.starts_with(&open[..1]) && marker.len() >= open.len() => {
                    fence = None;
                    out.push(block_start..offset + line.len());
                }
                _ => {}
            }
        } else if fence.is_none() && line.contains('`') {
            out.extend(INLINE_CODE.find_iter(line).map(|m| offset + m.start()..offset + m.end()));
        }
        offset += line.len() + 1;
    }
    // A block left open runs to the end of the note, as Obsidian shows it.
    if fence.is_some() {
        out.push(block_start..text.len());
    }
    out
}

/// A preview is one line: a line break in it shows as ↵.
fn flat(text: &str) -> String {
    text.replace('\r', "").replace('\n', "↵")
}

fn shortened(text: &str, keeping_end: bool) -> String {
    const LIMIT: usize = 60;
    let count = text.chars().count();
    if count <= LIMIT {
        return text.to_string();
    }
    if keeping_end {
        format!("…{}", text.chars().skip(count - LIMIT).collect::<String>())
    } else {
        format!("{}…", text.chars().take(LIMIT).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ReplaceOptions {
        ReplaceOptions::default()
    }

    #[test]
    fn plain_text_is_found_on_its_lines() {
        let text = "Call Alice\nalice and ALICE\nMalice";
        let found = VaultReplace::matches(text, "alice", "Bob", options()).unwrap();
        assert_eq!(found.iter().map(|m| m.line).collect::<Vec<_>>(), [0, 1, 1, 2]);
        assert_eq!(found.iter().map(|m| m.found.as_str()).collect::<Vec<_>>(), ["Alice", "alice", "ALICE", "alice"]);
        assert_eq!(found[1].before(), "alice and ALICE");
        assert_eq!(found[1].after(), "Bob and ALICE");
        assert_eq!(VaultReplace::apply(&found, text), "Call Bob\nBob and Bob\nMBob");
        // The replacement of plain text is plain: `$1` is `$1`.
        let dollars = VaultReplace::matches("a.b", ".", "$1", options()).unwrap();
        assert_eq!(VaultReplace::apply(&dollars, "a.b"), "a$1b");
    }

    #[test]
    fn case_and_whole_word_narrow_the_matches() {
        let text = "Alice alice Malice alice_x کارها کار";
        let with = |o: ReplaceOptions, find: &str| VaultReplace::matches(text, find, "", o).unwrap();
        assert_eq!(with(ReplaceOptions { match_case: true, ..options() }, "alice").len(), 3);
        let words = with(ReplaceOptions { whole_word: true, ..options() }, "alice");
        assert_eq!(words.iter().map(|m| m.found.as_str()).collect::<Vec<_>>(), ["Alice", "alice"]);
        let persian = with(ReplaceOptions { whole_word: true, ..options() }, "کار");
        assert_eq!(persian.iter().map(|m| text[..m.location].chars().count()).collect::<Vec<_>>(), [33]);
    }

    #[test]
    fn a_regular_expression_names_its_groups() {
        let text = "2026-10-26 and 2025-01-02";
        let regex = ReplaceOptions { regular_expression: true, ..options() };
        let found = VaultReplace::matches(text, r"(\d{4})-(\d\d)-(\d\d)", "$3.$2.$1", regex).unwrap();
        assert_eq!(VaultReplace::apply(&found, text), "26.10.2026 and 02.01.2025");
        // Anchors match at each line.
        let lines = VaultReplace::matches("a\nb", "^", "- ", regex).unwrap();
        assert_eq!(VaultReplace::apply(&lines, "a\nb"), "- a\n- b");
        assert!(matches!(VaultReplace::matches(text, "(", "", regex), Err(ReplaceFailure::BadPattern(_))));
        assert!(VaultReplace::matches(text, "", "x", regex).unwrap().is_empty());
        // `$1a` is group 1 and an `a`, as the Mac's template reads it.
        let group = VaultReplace::matches("xy", "(x)", "$1a\\$", regex).unwrap();
        assert_eq!(VaultReplace::apply(&group, "xy"), "xa$y");
    }

    #[test]
    fn code_is_skipped_when_asked() {
        let text = "word `word` word\n```\nword\n```\n> ~~~\n> word\n> ~~~\nword";
        let skipped = VaultReplace::matches(text, "word", "X", options()).unwrap();
        assert_eq!(skipped.iter().map(|m| m.line).collect::<Vec<_>>(), [0, 0, 7]);
        assert!(VaultReplace::apply(&skipped, text).starts_with("X `word` X\n```\nword"));
        assert_eq!(VaultReplace::matches(text, "word", "X", ReplaceOptions { skip_code: false, ..options() }).unwrap().len(), 6);
        // A block left open runs to the end.
        assert_eq!(VaultReplace::matches("word\n```\nword", "word", "X", options()).unwrap().len(), 1);
    }

    #[test]
    fn only_the_chosen_matches_are_replaced() {
        let text = "one one one";
        let found = VaultReplace::matches(text, "one", "two", options()).unwrap();
        assert_eq!(VaultReplace::apply(std::slice::from_ref(&found[1]), text), "one two one");
        assert_eq!(VaultReplace::apply(&[], text), text);
        let pattern = ReplacePattern::new("one", "two", options()).unwrap().unwrap();
        let chosen = BTreeSet::from([found[0].id(), found[2].id()]);
        assert_eq!(VaultReplace::replace(text, &pattern, &found, &chosen).unwrap(), "two one two");
        // The note changed after the preview: refused.
        assert_eq!(VaultReplace::replace("one one", &pattern, &found, &BTreeSet::from([found[0].id()])), Err(ReplaceFailure::Changed));
    }

    #[test]
    fn a_long_line_is_cut_in_the_preview() {
        let text = format!("{} needle {}\nnext", "a".repeat(200), "b".repeat(200));
        let found = VaultReplace::matches(&text, "needle", "pin", options()).unwrap();
        let before = found[0].before();
        assert!(before.starts_with('…') && before.ends_with('…') && before.chars().count() < 140);
        assert!(found[0].after().contains(" pin "));
        let across = VaultReplace::matches("a\nb", r"a\nb", "c", ReplaceOptions { regular_expression: true, ..options() }).unwrap();
        assert_eq!(across[0].before(), "a↵b");
    }
}
