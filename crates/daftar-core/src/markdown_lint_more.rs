//! The rules of `markdown_lint` that read a note as a whole: links against
//! the note's headings and definitions, tables cell by cell, footnotes,
//! comments and what git or a paste leaves behind. markdownlint's keep its
//! numbers and options; MD066 on are rumdl's, with rumdl's numbers.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

use crate::markdown_lint::LintConfig;

/// A link as the parser found it.
pub struct LinkView<'a> {
    pub range: Range<usize>,
    pub wiki: bool,
    pub auto: bool,
    pub destination: &'a str,
    pub image: bool,
}

/// What these rules read of a parsed note; lines from 0.
pub struct View<'a> {
    pub lines: &'a [String],
    pub starts: &'a [usize],
    /// Code, HTML and math blocks and the front matter.
    pub opaque: &'a [bool],
    pub fenced: Vec<(usize, usize)>,
    pub headings: Vec<(usize, &'a str)>,
    /// Every line a list takes.
    pub lists: HashSet<usize>,
    pub tables: &'a [(usize, usize)],
    pub links: Vec<LinkView<'a>>,
    pub code_spans: &'a [Range<usize>],
    pub html: &'a [(usize, String)],
    pub source: &'a str,
}

impl View<'_> {
    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(at) => at,
            Err(at) => at.saturating_sub(1),
        }
    }

    fn column(&self, offset: usize) -> usize {
        let line = self.line_of(offset);
        self.lines[line].get(..offset - self.starts[line]).map_or(1, |s| s.chars().count() + 1)
    }

    /// Line [n] with its code spans blanked, so what is in one is not read.
    fn prose(&self, n: usize) -> String {
        let line = &self.lines[n];
        let start = self.starts[n];
        let mut out: Vec<u8> = line.as_bytes().to_vec();
        for span in self.code_spans.iter().filter(|r| r.start < start + line.len() && r.end > start) {
            for i in span.start.max(start)..span.end.min(start + line.len()) {
                out[i - start] = b' ';
            }
        }
        String::from_utf8(out).unwrap_or_else(|_| line.clone())
    }

    /// The lines the text rules read: not code, HTML, math or the front matter.
    fn text_lines(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.lines.len()).filter(|&n| !self.opaque[n])
    }
}

type Push<'a> = dyn FnMut(usize, Option<usize>, &'static str, String) + 'a;

pub fn check(view: &View, config: &LintConfig, push: &mut Push) {
    blockquotes(view, push);
    emphasis_headings(view, config, push);
    html(view, config, push);
    fragments(view, config, push);
    references(view, config, push);
    tables(view, config, push);
    link_text(view, config, push);
    footnotes(view, push);
    pasted_markers(view, push);
    comments(view, push);
    conflicts(view, push);
    invisible(view, push);
}

/// Strips the `>` marks a line opens with.
fn unquoted(line: &str) -> &str {
    let mut rest = line.trim_start();
    while let Some(after) = rest.strip_prefix('>') {
        rest = after.trim_start();
    }
    rest
}

/// MD028: a blank line between two blockquotes, which some parsers join
/// and others keep apart.
fn blockquotes(view: &View, push: &mut Push) {
    let quote = |n: usize| view.lines[n].trim_start().starts_with('>');
    for n in view.text_lines() {
        if !view.lines[n].trim().is_empty() || n == 0 || !quote(n - 1) {
            continue;
        }
        let next = (n + 1..view.lines.len()).find(|&m| !view.lines[m].trim().is_empty());
        if next.is_some_and(|m| quote(m) && !view.opaque[m]) {
            push(n, None, "MD028", String::new());
        }
    }
}

/// MD036: a paragraph of one line that is all emphasis, standing for a heading.
fn emphasis_headings(view: &View, config: &LintConfig, push: &mut Push) {
    static WHOLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\*\*|__|\*|_)([^*_].*?)(\*\*|__|\*|_)$").unwrap());
    let punctuation = config.text("MD036", "punctuation", ".,;:!?。，；：！？");
    for n in view.text_lines() {
        let line = view.lines[n].trim();
        let alone = (n == 0 || view.lines[n - 1].trim().is_empty()) && view.lines.get(n + 1).is_none_or(|l| l.trim().is_empty());
        if !alone || view.lists.contains(&n) || line.starts_with('>') {
            continue;
        }
        if let Some(m) = WHOLE.captures(line)
            && m[1] == m[3]
            && !m[2].ends_with(|c: char| punctuation.contains(c))
            && !m[2].contains(['*', '_'])
        {
            push(n, Some(view.lines[n].find(line).unwrap_or(0) + 1), "MD036", format!("[Context: \"{}\"]", m[2].trim()));
        }
    }
}

/// MD033: HTML in the text, but the elements `allowed_elements` names.
fn html(view: &View, config: &LintConfig, push: &mut Push) {
    static OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([A-Za-z][A-Za-z0-9-]*)[\s/>]").unwrap());
    let allowed: HashSet<String> = config
        .option("MD033", "allowed_elements")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str()).map(str::to_lowercase).collect())
        .unwrap_or_default();
    for (start, text) in view.html {
        for m in OPEN.captures_iter(text) {
            let element = m[1].to_lowercase();
            if !allowed.contains(&element) {
                let at = start + m.get(0).map_or(0, |g| g.start());
                push(view.line_of(at), Some(view.column(at)), "MD033", format!("[Element: {element}]"));
            }
        }
    }
}

/// A heading's fragment as GitHub makes it: lower case, marks left out, a
/// dash for each space.
fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// MD051: `[x](#fragment)` to no heading of the note, nor an `id` in its HTML.
fn fragments(view: &View, config: &LintConfig, push: &mut Push) {
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b(?:id|name)\s*=\s*["']?([^"'\s>]+)"#).unwrap());
    let ignore_case = config.flag("MD051", "ignore_case", false);
    let mut known: HashSet<String> = HashSet::from(["top".to_string()]);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (_, text) in &view.headings {
        let base = slug(text);
        let count = seen.entry(base.clone()).or_default();
        known.insert(if *count == 0 { base.clone() } else { format!("{base}-{count}") });
        *count += 1;
    }
    for (_, html) in view.html {
        known.extend(ID.captures_iter(html).map(|m| m[1].to_string()));
    }
    for link in view.links.iter().filter(|l| !l.wiki && !l.auto) {
        let Some(fragment) = link.destination.strip_prefix('#') else { continue };
        let fragment = percent_decode_str(fragment).decode_utf8_lossy();
        // `#L12` and `#L12C3-L14` are lines of a file on GitHub.
        if fragment.is_empty() || fragment.starts_with('^') {
            continue;
        }
        let found = known.contains(fragment.as_ref()) || (ignore_case && known.iter().any(|k| k.eq_ignore_ascii_case(&fragment)));
        if !found {
            let at = link.range.start;
            push(view.line_of(at), Some(view.column(at)), "MD051", format!("[Context: \"#{fragment}\"]"));
        }
    }
}

/// A label as CommonMark matches it: case folded, spaces made one.
fn label(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// MD052 and MD053: `[text][label]` with no `[label]: …`, and a
/// `[label]: …` no link uses.
fn references(view: &View, config: &LintConfig, push: &mut Push) {
    static DEFINITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ {0,3}\[([^\]^][^\]]*)\]:\s*\S").unwrap());
    static FULL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[([^\[\]]*)\]\[([^\[\]]*)\]").unwrap());
    static SHORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]]+)\]").unwrap());
    let mut defined: Vec<(usize, String)> = Vec::new();
    for n in view.text_lines() {
        if let Some(m) = DEFINITION.captures(&view.lines[n]) {
            defined.push((n, label(&m[1])));
        }
    }
    let labels: HashSet<&str> = defined.iter().map(|(_, l)| l.as_str()).collect();
    let mut used: HashSet<String> = HashSet::new();
    let definition_lines: HashSet<usize> = defined.iter().map(|(n, _)| *n).collect();
    for n in view.text_lines().filter(|n| !definition_lines.contains(n)) {
        let prose = view.prose(n);
        for m in FULL.captures_iter(&prose) {
            let wanted = if m[3].trim().is_empty() { label(&m[2]) } else { label(&m[3]) };
            // `[[note]]` is a wikilink; `[ ][x]` a task's box.
            let at = m.get(0).map_or(0, |g| g.start());
            if prose[..at].ends_with('[') || prose[at + m[0].len()..].starts_with(']') || wanted.is_empty() {
                continue;
            }
            used.insert(wanted.clone());
            if !labels.contains(wanted.as_str()) {
                push(
                    n,
                    Some(prose[..at].chars().count() + 1),
                    "MD052",
                    format!("[Missing link or image reference definition: \"{wanted}\"]"),
                );
            }
        }
        used.extend(SHORT.captures_iter(&prose).map(|m| label(&m[1])));
    }
    let ignored: Vec<String> = config
        .option("MD053", "ignored_definitions")
        .and_then(|v| v.as_array())
        .map_or_else(|| vec!["//".to_string()], |a| a.iter().filter_map(|e| e.as_str()).map(label).collect());
    let mut first: HashSet<&str> = HashSet::new();
    for (n, name) in &defined {
        if ignored.contains(name) {
            continue;
        }
        if !first.insert(name) {
            push(*n, Some(1), "MD053", format!("[Duplicate link or image reference definition: \"{name}\"]"));
        } else if !used.contains(name) {
            push(*n, Some(1), "MD053", format!("[Unused link or image reference definition: \"{name}\"]"));
        }
    }
}

/// The cells of a table row, split at each `|` not escaped as `\|`: as GFM
/// splits them, before code spans, so a code span does not keep one.
fn cells(row: &str) -> (Vec<String>, bool, bool) {
    let row = unquoted(row).trim();
    let (mut cells, mut cell, mut pipes) = (Vec::new(), String::new(), Vec::new());
    let mut previous = None;
    for (i, c) in row.char_indices() {
        if c == '|' && previous != Some('\\') {
            pipes.push(i);
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(c);
        }
        previous = Some(c);
    }
    cells.push(cell);
    let leading = pipes.first() == Some(&0);
    let trailing = pipes.last().is_some_and(|&i| i + 1 == row.len());
    if leading {
        cells.remove(0);
    }
    if trailing {
        cells.pop();
    }
    (cells, leading, trailing)
}

/// MD055 and MD056: rows whose outer pipes differ from the style, and rows
/// with more or fewer cells than the heading row.
fn tables(view: &View, config: &LintConfig, push: &mut Push) {
    let style = config.text("MD055", "style", "consistent");
    for &(first, last) in view.tables {
        let (header, lead, trail) = cells(&view.lines[first]);
        let (want_lead, want_trail) = match style.as_str() {
            "leading_and_trailing" => (true, true),
            "leading_only" => (true, false),
            "trailing_only" => (false, true),
            "no_leading_or_trailing" => (false, false),
            _ => (lead, trail),
        };
        for n in first..=last.min(view.lines.len() - 1) {
            let (row, leading, trailing) = cells(&view.lines[n]);
            let column = view.lines[n].len() - view.lines[n].trim_start().len() + 1;
            if leading != want_lead {
                let expected = if want_lead { "leading_and_trailing" } else { "no_leading_or_trailing" };
                push(
                    n,
                    Some(column),
                    "MD055",
                    format!("[Expected: {expected}; Actual: {}; Missing leading pipe]", if leading { "leading" } else { "none" }),
                );
            }
            if trailing != want_trail {
                push(n, Some(view.lines[n].trim_end().chars().count()), "MD055", "[Trailing pipe]".to_string());
            }
            if n != first + 1 && row.len() != header.len() {
                let (expected, actual) = (header.len(), row.len());
                let which = if actual < expected {
                    "Too few cells, row will be missing data"
                } else {
                    "Too many cells, extra data will be missing"
                };
                push(n, Some(column), "MD056", format!("[Expected: {expected}; Actual: {actual}; {which}]"));
            }
        }
    }
}

/// MD059: a link whose text says nothing of where it goes.
fn link_text(view: &View, config: &LintConfig, push: &mut Push) {
    let prohibited: Vec<String> = config.option("MD059", "prohibited_texts").and_then(|v| v.as_array()).map_or_else(
        || ["click here", "here", "link", "more"].iter().map(ToString::to_string).collect(),
        |a| a.iter().filter_map(|e| e.as_str()).map(str::to_lowercase).collect(),
    );
    let plain = |t: &str| {
        t.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    for link in view.links.iter().filter(|l| !l.wiki && !l.auto && !l.image) {
        let source = view.source.get(link.range.clone()).unwrap_or("");
        let Some(text) = source.strip_prefix('[').and_then(|s| s.rfind("](").map(|end| &s[..end])) else { continue };
        if prohibited.contains(&plain(text)) {
            let at = link.range.start;
            push(view.line_of(at), Some(view.column(at)), "MD059", format!("[Context: \"{text}\"]"));
        }
    }
}

/// MD066 and MD068: a `[^note]` with no definition, a definition nothing
/// cites, and a definition with nothing in it.
fn footnotes(view: &View, push: &mut Push) {
    static DEFINITION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ {0,3}\[\^([^\]\s]+)\]:(.*)$").unwrap());
    static REFERENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\^([^\]\s]+)\]").unwrap());
    let mut defined: HashMap<String, usize> = HashMap::new();
    let mut cited: Vec<(usize, usize, String)> = Vec::new();
    for n in view.text_lines() {
        let prose = view.prose(n);
        let definition = DEFINITION.captures(&prose);
        if let Some(m) = &definition {
            defined.entry(m[1].to_string()).or_insert(n);
            let continued = view.lines.get(n + 1).is_some_and(|l| l.starts_with("    ") || l.starts_with('\t'));
            if m[2].trim().is_empty() && !continued {
                push(n, Some(1), "MD068", format!("[Footnote: \"{}\"]", &m[1]));
            }
        }
        let skip = definition.as_ref().and_then(|m| m.get(0)).map_or(0, |g| g.as_str().find(':').unwrap_or(0) + 1);
        for m in REFERENCE.captures_iter(&prose[skip..]) {
            let at = skip + m.get(0).map_or(0, |g| g.start());
            cited.push((n, prose[..at].chars().count() + 1, m[1].to_string()));
        }
    }
    let names: HashSet<&str> = cited.iter().map(|(_, _, name)| name.as_str()).collect();
    for (n, column, name) in &cited {
        if !defined.contains_key(name) {
            push(*n, Some(*column), "MD066", format!("[Footnote: \"{name}\"; Missing definition]"));
        }
    }
    for (name, n) in &defined {
        if !names.contains(name.as_str()) {
            push(*n, Some(1), "MD066", format!("[Footnote: \"{name}\"; Never cited]"));
        }
    }
}

/// MD069: `- - text`, the marker a paste brings along before the list's own.
fn pasted_markers(view: &View, push: &mut Push) {
    static TWICE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*(?:>\s*)*)([-*+]) +([-*+]) +(\S.*)$").unwrap());
    for n in view.text_lines() {
        let Some(m) = TWICE.captures(&view.lines[n]) else { continue };
        // `- - -` is a rule, and `- - [ ]` a task in a nested list on purpose.
        if m[4].chars().all(|c| c == '-' || c == '*' || c == '+' || c == ' ') || m[4].starts_with('[') {
            continue;
        }
        push(n, Some(m[1].chars().count() + 1), "MD069", format!("[Context: \"{} {} …\"]", &m[2], &m[3]));
    }
}

/// MD086: `<!--` with no `-->` after it, or a `%%` comment of Obsidian's
/// never closed: everything after it is hidden.
fn comments(view: &View, push: &mut Push) {
    let outside: Vec<(usize, String)> =
        (0..view.lines.len()).filter(|&n| !view.fenced.iter().any(|&(a, b)| (a..=b).contains(&n))).map(|n| (n, view.prose(n))).collect();
    let mut html: Option<(usize, usize)> = None;
    let mut percent: Option<(usize, usize)> = None;
    for (n, line) in &outside {
        let mut rest = 0;
        loop {
            let slice = &line[rest..];
            let next_html = if html.is_some() { slice.find("-->") } else { slice.find("<!--") };
            let next_percent = if html.is_none() { slice.find("%%") } else { None };
            match (next_html, next_percent) {
                (Some(h), p) if p.is_none_or(|p| h < p) => {
                    html = if html.is_some() { None } else { Some((*n, rest + h)) };
                    rest += h + if html.is_some() { 4 } else { 3 };
                }
                (_, Some(p)) => {
                    percent = if percent.is_some() { None } else { Some((*n, rest + p)) };
                    rest += p + 2;
                }
                _ => break,
            }
        }
    }
    for (n, at, what) in [html.map(|(n, at)| (n, at, "<!--")), percent.map(|(n, at)| (n, at, "%%"))].into_iter().flatten() {
        push(n, Some(view.lines[n][..at].chars().count() + 1), "MD086", format!("[Comment: \"{what}\" never closed]"));
    }
}

/// MD092: what git writes into a file it could not merge.
fn conflicts(view: &View, push: &mut Push) {
    let ours = |l: &str| l.starts_with("<<<<<<<") && l[7..].chars().next().is_none_or(char::is_whitespace);
    let theirs = |l: &str| l.starts_with(">>>>>>>") && l[7..].chars().next().is_none_or(char::is_whitespace);
    let mut open: Option<usize> = None;
    for (n, line) in view.lines.iter().enumerate() {
        if ours(line) {
            open = Some(n);
            push(n, Some(1), "MD092", "[Marker: \"<<<<<<<\"]".into());
        } else if let Some(start) = open {
            if line.trim_end() == "=======" || line.starts_with("|||||||") {
                push(n, Some(1), "MD092", format!("[Marker: \"{}\"; Conflict from line {}]", &line[..7], start + 1));
            } else if theirs(line) {
                push(n, Some(1), "MD092", format!("[Marker: \">>>>>>>\"; Conflict from line {}]", start + 1));
                open = None;
            }
        } else if theirs(line) {
            push(n, Some(1), "MD092", "[Marker: \">>>>>>>\"]".into());
        }
    }
}

/// MD084 (off unless turned on): characters that show nothing but change
/// how text matches or breaks. The joiners Persian is written with (ZWNJ,
/// ZWJ) and the direction marks are left alone.
fn invisible(view: &View, push: &mut Push) {
    for (n, line) in view.lines.iter().enumerate() {
        for (column, c) in line.chars().enumerate() {
            let name = match c {
                '\u{200B}' => "ZERO WIDTH SPACE",
                '\u{2060}' => "WORD JOINER",
                '\u{FEFF}' if n > 0 || column > 0 => "ZERO WIDTH NO-BREAK SPACE",
                '\u{00AD}' => "SOFT HYPHEN",
                '\u{2028}' | '\u{2029}' => "LINE OR PARAGRAPH SEPARATOR",
                '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => "BIDI CONTROL",
                _ => continue,
            };
            push(n, Some(column + 1), "MD084", format!("[U+{:04X} {name}]", c as u32));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::markdown_lint::{LintConfig, MarkdownLint};

    fn found(text: &str, rules: &[&str]) -> Vec<String> {
        let on: Vec<String> = rules.iter().map(|r| format!("\"{r}\": true")).collect();
        let config = LintConfig::parse(&format!("{{\"default\": false, {}}}", on.join(", "))).unwrap();
        MarkdownLint::lint(text, &config).iter().map(|p| format!("{} {}", p.line, p.rule)).collect()
    }

    #[test]
    fn quotes_emphasis_and_html() {
        assert_eq!(found("> a\n\n> b\n\n> [!note]\n>\n> c\n", &["MD028"]), ["2 MD028", "4 MD028"]);
        assert_eq!(found("Text.\n\n**Not a heading**\n\n**A sentence.**\n\n- **in a list**\n", &["MD036"]), ["3 MD036"]);
        let allowed = LintConfig::parse(r#"{"default": false, "MD033": {"allowed_elements": ["br"]}}"#).unwrap();
        let problems = MarkdownLint::lint("a<br>b <kbd>x</kbd>\n", &allowed);
        assert_eq!(problems.iter().map(|p| p.detail.as_str()).collect::<Vec<_>>(), ["[Element: kbd]"]);
    }

    #[test]
    fn links_against_headings_and_definitions() {
        let text = "# Top\n\n## My Heading\n\n## My Heading\n\n[a](#my-heading) [b](#my-heading-1) [c](#gone) [d](#top)\n\n\
                    [e][used] [f][missing] [g][] [[wiki]] [shortcut]\n\n[used]: https://e.org\n[g]: https://e.org\n\
                    [shortcut]: https://e.org\n[lonely]: https://e.org\n[used]: https://e.org/2\n\n[here](https://e.org) [the docs](https://e.org)\n";
        assert_eq!(found(text, &["MD051", "MD052", "MD053", "MD059"]), ["7 MD051", "9 MD052", "14 MD053", "15 MD053", "17 MD059"]);
    }

    #[test]
    fn tables_cell_by_cell() {
        let text = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| [[x \\| y]] | `\\` |\n| [[x | y]] | 3 |\n| 1 | 2\n";
        assert_eq!(found(text, &["MD055", "MD056"]), ["5 MD056", "6 MD055"]);
    }

    #[test]
    fn footnotes_pastes_comments_and_conflicts() {
        let text = "A[^1] and B[^2].\n\n[^1]: one\n[^3]: never cited\n[^4]:\n\n- - pasted\n- - -\n\n%% open\n";
        assert_eq!(found(text, &["MD066", "MD068", "MD069", "MD086"]), ["1 MD066", "4 MD066", "5 MD066", "5 MD068", "7 MD069", "10 MD086"]);
        assert!(found("%% a %%\n<!-- b -->\n`%%`\n\n```\n%%\n```\n", &["MD086"]).is_empty());
        let conflict = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> origin/main\n\nTitle\n=======\n";
        assert_eq!(found(conflict, &["MD092"]), ["2 MD092", "4 MD092", "6 MD092"]);
    }

    #[test]
    fn invisible_characters_but_the_persian_joiners() {
        assert_eq!(found("a\u{200B}b می\u{200C}خواهم \u{200F}\n", &["MD084"]), ["1 MD084"]);
        assert!(found("a\u{200B}b\n", &[]).is_empty(), "MD084 is off unless turned on");
        let config = LintConfig::all();
        assert_eq!(
            MarkdownLint::lint_bytes(b"ok\nbad \xff\n", &config).iter().map(|p| (p.line, p.rule)).collect::<Vec<_>>(),
            [(2, "MD094")]
        );
    }
}
