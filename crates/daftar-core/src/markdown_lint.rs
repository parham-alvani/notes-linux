//! Markdown lint, as markdownlint (`markdownlint-cli2`) checks a vault, with
//! the vault's own `.markdownlint.jsonc`, and Obsidian's syntax known:
//! wikilinks and embeds, callouts, `%%` comments, block ids, math and the
//! front matter are what they are in Obsidian, not lint. Each rule keeps
//! markdownlint's number, name and options, so a config written for it means
//! the same here, and `fix` mends what markdownlint's own `--fix` mends.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;

/// One problem: the line (from 1), the column when it is known, the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: usize,
    pub column: Option<usize>,
    pub rule: &'static str,
    pub detail: String,
}

impl Problem {
    pub fn name(&self) -> &'static str {
        RULES.iter().find(|(id, _, _)| *id == self.rule).map_or("", |(_, name, _)| name)
    }

    pub fn description(&self) -> &'static str {
        RULES.iter().find(|(id, _, _)| *id == self.rule).map_or("", |(_, _, text)| text)
    }
}

/// markdownlint's rules this checks: number, name and what it says.
pub const RULES: [(&str, &str, &str); 38] = [
    ("MD001", "heading-increment", "Heading levels should only increment by one level at a time"),
    ("MD003", "heading-style", "Heading style"),
    ("MD004", "ul-style", "Unordered list style"),
    ("MD005", "list-indent", "Inconsistent indentation for list items at the same level"),
    ("MD007", "ul-indent", "Unordered list indentation"),
    ("MD009", "no-trailing-spaces", "Trailing spaces"),
    ("MD010", "no-hard-tabs", "Hard tabs"),
    ("MD011", "no-reversed-links", "Reversed link syntax"),
    ("MD012", "no-multiple-blanks", "Multiple consecutive blank lines"),
    ("MD014", "commands-show-output", "Dollar signs used before commands without showing output"),
    ("MD018", "no-missing-space-atx", "No space after hash on atx style heading"),
    ("MD019", "no-multiple-space-atx", "Multiple spaces after hash on atx style heading"),
    ("MD020", "no-missing-space-closed-atx", "No space inside hashes on closed atx style heading"),
    ("MD021", "no-multiple-space-closed-atx", "Multiple spaces inside hashes on closed atx style heading"),
    ("MD022", "blanks-around-headings", "Headings should be surrounded by blank lines"),
    ("MD023", "heading-start-left", "Headings must start at the beginning of the line"),
    ("MD024", "no-duplicate-heading", "Multiple headings with the same content"),
    ("MD025", "single-title", "Multiple top-level headings in the same document"),
    ("MD026", "no-trailing-punctuation", "Trailing punctuation in heading"),
    ("MD027", "no-multiple-space-blockquote", "Multiple spaces after blockquote symbol"),
    ("MD029", "ol-prefix", "Ordered list item prefix"),
    ("MD030", "list-marker-space", "Spaces after list markers"),
    ("MD031", "blanks-around-fences", "Fenced code blocks should be surrounded by blank lines"),
    ("MD032", "blanks-around-lists", "Lists should be surrounded by blank lines"),
    ("MD034", "no-bare-urls", "Bare URL used"),
    ("MD035", "hr-style", "Horizontal rule style"),
    ("MD037", "no-space-in-emphasis", "Spaces inside emphasis markers"),
    ("MD038", "no-space-in-code", "Spaces inside code span elements"),
    ("MD039", "no-space-in-links", "Spaces inside link text"),
    ("MD040", "fenced-code-language", "Fenced code blocks should have a language specified"),
    ("MD042", "no-empty-links", "No empty links"),
    ("MD045", "no-alt-text", "Images should have alternate text (alt text)"),
    ("MD046", "code-block-style", "Code block style"),
    ("MD047", "single-trailing-newline", "Files should end with a single newline character"),
    ("MD048", "code-fence-style", "Code fence style"),
    ("MD049", "emphasis-style", "Emphasis style"),
    ("MD050", "strong-style", "Strong style"),
    ("MD058", "blanks-around-tables", "Tables should be surrounded by blank lines"),
];

/// Rules off unless the config turns them on, as markdownlint ships them.
const OFF_BY_DEFAULT: [&str; 0] = [];

/// Which rules run, with their options, from a `.markdownlint.json(c)`.
#[derive(Debug, Clone, Default)]
pub struct LintConfig {
    default: bool,
    rules: HashMap<String, serde_json::Value>,
}

impl LintConfig {
    /// Every rule on, with markdownlint's defaults.
    pub fn all() -> LintConfig {
        LintConfig { default: true, rules: HashMap::new() }
    }

    /// A `.markdownlint.jsonc`: JSON with comments and trailing commas. A rule
    /// is named by its number or its name.
    pub fn parse(text: &str) -> Result<LintConfig, String> {
        let json: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).map_err(|e| e.to_string())?;
        let object = json.as_object().ok_or("the config is not an object")?;
        let mut config = LintConfig { default: true, rules: HashMap::new() };
        for (key, value) in object {
            if key == "default" {
                config.default = value.as_bool().unwrap_or(true);
                continue;
            }
            let id =
                RULES.iter().find(|(id, name, _)| id.eq_ignore_ascii_case(key) || name.eq_ignore_ascii_case(key)).map(|(id, _, _)| *id);
            if let Some(id) = id {
                config.rules.insert(id.to_string(), value.clone());
            }
        }
        Ok(config)
    }

    pub fn on(&self, rule: &str) -> bool {
        if !RULES.iter().any(|(id, _, _)| *id == rule) {
            return false;
        }
        match self.rules.get(rule) {
            Some(serde_json::Value::Bool(on)) => *on,
            Some(serde_json::Value::Object(_)) => true,
            Some(_) => self.default,
            None => self.default && !OFF_BY_DEFAULT.contains(&rule),
        }
    }

    fn option(&self, rule: &str, key: &str) -> Option<&serde_json::Value> {
        self.rules.get(rule)?.as_object()?.get(key)
    }

    fn text(&self, rule: &str, key: &str, default: &str) -> String {
        self.option(rule, key).and_then(|v| v.as_str()).unwrap_or(default).to_string()
    }

    fn number(&self, rule: &str, key: &str, default: i64) -> i64 {
        self.option(rule, key).and_then(|v| v.as_i64()).unwrap_or(default)
    }

    fn flag(&self, rule: &str, key: &str, default: bool) -> bool {
        self.option(rule, key).and_then(|v| v.as_bool()).unwrap_or(default)
    }
}

/// JSON from JSONC: `//` and `/* */` comments and trailing commas out,
/// strings left as they are.
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '"' {
            out.push(c);
            at += 1;
            while at < chars.len() {
                out.push(chars[at]);
                if chars[at] == '\\' && at + 1 < chars.len() {
                    out.push(chars[at + 1]);
                    at += 2;
                    continue;
                }
                at += 1;
                if chars[at - 1] == '"' {
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.get(at + 1) == Some(&'/') {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            continue;
        }
        if c == '/' && chars.get(at + 1) == Some(&'*') {
            at += 2;
            while at + 1 < chars.len() && !(chars[at] == '*' && chars[at + 1] == '/') {
                at += 1;
            }
            at += 2;
            continue;
        }
        out.push(c);
        at += 1;
    }
    static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",(\s*[}\]])").unwrap());
    TRAILING.replace_all(&out, "$1").to_string()
}

#[derive(Debug, Clone)]
struct Heading {
    level: usize,
    line: usize,
    /// The heading's last line: the underline of a setext one.
    end: usize,
    setext: bool,
    text: String,
}

#[derive(Debug, Clone)]
struct List {
    ordered: bool,
    first: usize,
    last: usize,
    /// Lists it is in, outermost first: whether each is ordered.
    parents: Vec<bool>,
    items: Vec<usize>,
    blockquote: usize,
}

#[derive(Debug, Clone)]
struct CodeBlock {
    fenced: bool,
    info: String,
    first: usize,
    last: usize,
}

/// What the parser found, by line (from 0).
#[derive(Default)]
struct Document {
    source: String,
    lines: Vec<String>,
    starts: Vec<usize>,
    front_matter: usize,
    headings: Vec<Heading>,
    lists: Vec<List>,
    code: Vec<CodeBlock>,
    tables: Vec<(usize, usize)>,
    /// Lines that are code, html blocks, math blocks or the front matter:
    /// where the text rules do not look.
    opaque: Vec<bool>,
    /// Byte ranges inline rules skip: code spans, inline html, math, links' destinations.
    skip: Vec<Range<usize>>,
    /// `$...$` as markdownlint's parser (micromark's math) takes them: runs
    /// of `$` paired like code spans, whatever is around them. Text in one is
    /// not markdown to markdownlint, so its rules do not look there.
    dollars: Vec<Range<usize>>,
    emphasis: Vec<(Range<usize>, bool)>,
    links: Vec<(Range<usize>, LinkType, String, bool)>,
    code_spans: Vec<Range<usize>>,
    rules: Vec<usize>,
    texts: Vec<(Range<usize>, String)>,
}

impl Document {
    fn parse(text: &str) -> Document {
        let mut doc = Document {
            source: text.to_string(),
            lines: text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect(),
            ..Document::default()
        };
        let mut at = 0;
        for line in text.split('\n') {
            doc.starts.push(at);
            at += line.len() + 1;
        }
        doc.opaque = vec![false; doc.lines.len()];
        doc.front_matter = front_matter(&doc.lines);
        for line in 0..doc.front_matter {
            doc.opaque[line] = true;
        }
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_MATH
            | Options::ENABLE_WIKILINKS;
        let mut lists: Vec<List> = Vec::new();
        let mut quotes = 0usize;
        let mut heading: Option<(usize, Range<usize>)> = None;
        let mut code: Option<(bool, String, usize)> = None;
        let mut emphasis: Vec<(usize, bool)> = Vec::new();
        let mut links: Vec<(usize, LinkType, String, bool)> = Vec::new();
        let mut table: Option<usize> = None;
        for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
            let line = doc.line_of(range.start);
            match event {
                Event::Start(Tag::Heading { level, .. }) => heading = Some((level as usize, range.clone())),
                Event::End(TagEnd::Heading(_)) => {
                    if let Some((level, start)) = heading.take() {
                        let first = doc.line_of(start.start);
                        let last = doc.line_of(start.end.saturating_sub(1).max(start.start));
                        let setext = !doc.lines[first].trim_start().starts_with('#');
                        let text = if setext { doc.lines[first].trim().to_string() } else { atx_text(&doc.lines[first]) };
                        doc.headings.push(Heading { level, line: first, end: last, setext, text });
                    }
                }
                Event::Start(Tag::BlockQuote(_)) => quotes += 1,
                Event::End(TagEnd::BlockQuote(_)) => quotes = quotes.saturating_sub(1),
                Event::Start(Tag::List(start)) => {
                    lists.push(List {
                        ordered: start.is_some(),
                        first: line,
                        last: line,
                        parents: lists.iter().map(|l| l.ordered).collect(),
                        items: Vec::new(),
                        blockquote: quotes,
                    });
                }
                Event::End(TagEnd::List(_)) => {
                    if let Some(mut list) = lists.pop() {
                        list.last = doc.content_end(range.end);
                        doc.lists.push(list);
                    }
                }
                Event::Start(Tag::Item) => {
                    // A tab-indented item's range can start at the newline before it.
                    let line = if text.as_bytes().get(range.start) == Some(&b'\n') { line + 1 } else { line };
                    if let Some(list) = lists.last_mut() {
                        list.items.push(line);
                    }
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let (fenced, info) = match kind {
                        CodeBlockKind::Fenced(info) => (true, info.to_string()),
                        CodeBlockKind::Indented => (false, String::new()),
                    };
                    code = Some((fenced, info, line));
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some((fenced, info, first)) = code.take() {
                        let last = doc.content_end(range.end);
                        for l in first..=last.min(doc.lines.len() - 1) {
                            doc.opaque[l] = true;
                        }
                        doc.code.push(CodeBlock { fenced, info, first, last });
                    }
                }
                Event::Start(Tag::HtmlBlock) | Event::Start(Tag::MetadataBlock(_)) => {
                    let last = doc.content_end(range.end);
                    for l in line..=last.min(doc.lines.len() - 1) {
                        doc.opaque[l] = true;
                    }
                }
                Event::DisplayMath(_) => {
                    let last = doc.content_end(range.end);
                    for l in line..=last.min(doc.lines.len() - 1) {
                        doc.opaque[l] = true;
                    }
                    doc.skip.push(range.clone());
                }
                Event::Start(Tag::Table(_)) => table = Some(line),
                Event::End(TagEnd::Table) => {
                    if let Some(first) = table.take() {
                        doc.tables.push((first, doc.content_end(range.end)));
                    }
                }
                Event::Start(Tag::Emphasis) => emphasis.push((range.start, false)),
                Event::Start(Tag::Strong) => emphasis.push((range.start, true)),
                Event::End(TagEnd::Emphasis) | Event::End(TagEnd::Strong) => {
                    if let Some((start, strong)) = emphasis.pop() {
                        doc.emphasis.push((start..range.end, strong));
                    }
                }
                Event::Start(Tag::Link { link_type, dest_url, .. }) => links.push((range.start, link_type, dest_url.to_string(), false)),
                Event::Start(Tag::Image { link_type, dest_url, .. }) => links.push((range.start, link_type, dest_url.to_string(), true)),
                Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {
                    if let Some((start, kind, dest, image)) = links.pop() {
                        doc.links.push((start..range.end, kind, dest, image));
                        doc.skip.push(start..range.end);
                    }
                }
                Event::Code(_) => {
                    doc.code_spans.push(range.clone());
                    doc.skip.push(range);
                }
                Event::InlineMath(_) | Event::InlineHtml(_) | Event::Html(_) => doc.skip.push(range),
                Event::Rule => doc.rules.push(line),
                Event::Text(text) => doc.texts.push((range, text.to_string())),
                _ => {}
            }
        }
        doc.dollars = dollar_spans(&doc);
        doc
    }

    fn in_dollars(&self, offset: usize) -> bool {
        self.dollars.iter().any(|r| r.contains(&offset))
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(at) => at,
            Err(at) => at.saturating_sub(1),
        }
    }

    /// The last line holding text of a block that ends at byte [end]: a
    /// block's range takes in the newline that ends it, and blank lines a
    /// list ends on.
    fn content_end(&self, end: usize) -> usize {
        let mut line = self.line_of(end.saturating_sub(1));
        while line > 0 && self.lines[line].trim().is_empty() && self.starts[line] < end {
            line -= 1;
        }
        line
    }

    fn column(&self, offset: usize) -> usize {
        let line = self.line_of(offset);
        self.lines[line].get(..offset - self.starts[line]).map_or(1, |s| s.chars().count() + 1)
    }

    fn skipped(&self, offset: usize) -> bool {
        self.skip.iter().any(|r| r.contains(&offset))
    }

    fn blank(&self, line: usize) -> bool {
        self.lines.get(line).is_none_or(|l| strip_quotes(l).trim().is_empty())
    }
}

/// `$` runs paired as micromark pairs them, a paragraph (lines up to a
/// blank one) at a time, outside code and escapes.
fn dollar_spans(doc: &Document) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (n, line) in doc.lines.iter().enumerate() {
        if doc.opaque[n] || line.trim().is_empty() {
            open = None;
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let offset = doc.starts[n] + i;
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'$' && !doc.code_spans.iter().any(|r| r.contains(&offset)) {
                let run = bytes[i..].iter().take_while(|b| **b == b'$').count();
                match open {
                    Some((start, length)) if length == run => {
                        out.push(start..offset + run);
                        open = None;
                    }
                    None => open = Some((offset, run)),
                    _ => {}
                }
                i += run;
                continue;
            }
            i += 1;
        }
    }
    out
}

/// The lines the front matter takes, its fences and the blank lines after
/// it included, as markdownlint takes them; 0 with none.
fn front_matter(lines: &[String]) -> usize {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return 0;
    }
    let Some(close) = lines.iter().skip(1).position(|l| l.trim_end() == "---") else {
        return 0;
    };
    let mut end = close + 2;
    while end < lines.len() && lines[end].trim().is_empty() {
        end += 1;
    }
    end
}

/// A line without the `>` marks of the quotes it is in.
fn strip_quotes(line: &str) -> &str {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        match trimmed.strip_prefix('>') {
            // the space after `>` is the quote's own
            Some(after) => rest = after.strip_prefix(' ').unwrap_or(after),
            None => return rest,
        }
    }
}

/// An atx heading's text, without its marks.
fn atx_text(line: &str) -> String {
    let t = line.trim().trim_start_matches('#').trim();
    let t = t.trim_end_matches('#');
    t.trim().to_string()
}

pub struct MarkdownLint;

impl MarkdownLint {
    /// The problems of [text], in line order.
    pub fn lint(text: &str, config: &LintConfig) -> Vec<Problem> {
        let doc = Document::parse(text);
        let mut out = Vec::new();
        let mut push = |line: usize, column: Option<usize>, rule: &'static str, detail: String| {
            if config.on(rule) {
                out.push(Problem { line: line + 1, column, rule, detail });
            }
        };
        headings(&doc, config, &mut push);
        lines(&doc, config, &mut push);
        lists(&doc, config, &mut push);
        code(&doc, config, &mut push);
        inline(&doc, config, &mut push);
        if config.on("MD047") && !ends_well(text) {
            let last = doc.lines.len() - 1;
            push(last, Some(doc.lines[last].chars().count() + 1), "MD047", String::new());
        }
        if config.on("MD058") {
            for &(first, last) in &doc.tables {
                if first > doc.front_matter && !doc.blank(first - 1) {
                    push(first, None, "MD058", "[Expected: 1; Actual: 0; Above]".into());
                }
                if last + 1 < doc.lines.len() && !doc.blank(last + 1) {
                    push(last, None, "MD058", "[Expected: 1; Actual: 0; Below]".into());
                }
            }
        }
        out.sort_by_key(|p| (p.line, p.column.unwrap_or(0), p.rule));
        out
    }

    /// [text] with what can be mended mended, as `markdownlint --fix`:
    /// trailing spaces, blank lines, list marks, heading spaces, fences,
    /// emphasis marks, bare URLs and the final newline.
    pub fn fix(text: &str, config: &LintConfig) -> String {
        let mut text = text.to_string();
        // A fix can make or reveal another: a few rounds settle it.
        for _ in 0..4 {
            let fixed = fix_once(&text, config);
            if fixed == text {
                break;
            }
            text = fixed;
        }
        text
    }
}

type Push<'a> = dyn FnMut(usize, Option<usize>, &'static str, String) + 'a;

fn headings(doc: &Document, config: &LintConfig, push: &mut Push) {
    let style = config.text("MD003", "style", "consistent");
    let mut first_style: Option<&str> = None;
    let mut previous: Option<usize> = None;
    let front_title = doc.lines[..doc.front_matter].iter().any(|l| {
        static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*title\s*[:=]").unwrap());
        TITLE.is_match(l)
    });
    let level = config.number("MD025", "level", 1) as usize;
    let mut top = usize::from(front_title);
    let punctuation = config.text("MD026", "punctuation", ".,;:!。，；：！");
    let siblings = config.flag("MD024", "siblings_only", false);
    let mut seen: Vec<(Vec<String>, String)> = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    for h in &doc.headings {
        let line = &doc.lines[h.line];
        // MD001
        if let Some(previous) = previous
            && h.level > previous + 1
        {
            push(h.line, None, "MD001", format!("[Expected: h{}; Actual: h{}]", previous + 1, h.level));
        }
        previous = Some(h.level);
        // MD003
        let kind = if h.setext {
            "setext"
        } else if is_closed_atx(line) {
            "atx_closed"
        } else {
            "atx"
        };
        let expected = match style.as_str() {
            "consistent" => *first_style.get_or_insert(kind),
            "setext_with_atx" => {
                if h.level <= 2 {
                    "setext"
                } else {
                    "atx"
                }
            }
            other => other,
        };
        if kind != expected {
            push(h.line, None, "MD003", format!("[Expected: {expected}; Actual: {kind}]"));
        }
        if !h.setext {
            let body = line.trim_start();
            let hashes = body.chars().take_while(|c| *c == '#').count();
            let indent = line.len() - body.len();
            let after = &body[hashes..];
            // MD019
            if after.starts_with([' ', '\t']) && after[1..].starts_with([' ', '\t']) && !is_closed_atx(line) {
                push(h.line, Some(indent + hashes + 2), "MD019", String::new());
            }
            if is_closed_atx(line) {
                let inner = body.trim_end().trim_end_matches('#');
                let inner_after = &inner[hashes..];
                // MD020 is found on lines that are not headings too: below.
                // MD021
                if inner_after.starts_with([' ', '\t']) && inner_after[1..].starts_with([' ', '\t'])
                    || inner.ends_with("  ")
                    || inner.ends_with(" \t")
                {
                    push(h.line, Some(indent + inner.chars().count()), "MD021", String::new());
                }
            }
            // MD023
            if indent > 0 {
                push(h.line, Some(1), "MD023", String::new());
            }
        } else if line.starts_with([' ', '\t']) {
            push(h.line, Some(1), "MD023", String::new());
        }
        // MD022
        let above = config.number("MD022", "lines_above", 1).max(0) as usize;
        let below = config.number("MD022", "lines_below", 1).max(0) as usize;
        if h.line > doc.front_matter && above > 0 {
            let blanks = (0..above).take_while(|k| h.line > k + doc.front_matter && doc.blank(h.line - k - 1)).count();
            if blanks < above && !first_in_container(doc, h.line) {
                push(h.line, None, "MD022", format!("[Expected: {above}; Actual: {blanks}; Above]"));
            }
        }
        if h.end + 1 < doc.lines.len() && below > 0 {
            let blanks = (0..below).take_while(|k| doc.blank(h.end + k + 1)).count();
            let last_line = h.end + blanks + 1 >= doc.lines.len();
            if blanks < below && !last_line {
                push(h.line, None, "MD022", format!("[Expected: {below}; Actual: {blanks}; Below]"));
            }
        }
        // MD024
        let text = h.text.clone();
        while path.last().is_some_and(|(level, _)| *level >= h.level) {
            path.pop();
        }
        let parents: Vec<String> = if siblings { path.iter().map(|(_, t)| t.clone()).collect() } else { Vec::new() };
        if seen.iter().any(|(p, t)| *p == parents && *t == text) {
            push(h.line, None, "MD024", String::new());
        } else {
            seen.push((parents, text.clone()));
        }
        path.push((h.level, text.clone()));
        // MD025
        if h.level == level {
            top += 1;
            if top > 1 {
                push(h.line, None, "MD025", String::new());
            }
        }
        // MD026, but for an entity's `;`
        if let Some(last) = h.text.chars().last()
            && punctuation.contains(last)
            && !(last == ';' && is_entity_end(&h.text))
        {
            push(h.line, None, "MD026", format!("[Punctuation: '{last}']"));
        }
    }
}

/// `&amp;` and the like end in `;` without it being punctuation.
fn is_entity_end(text: &str) -> bool {
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#?[0-9a-zA-Z]+;$").unwrap());
    ENTITY.is_match(text)
}

fn is_closed_atx(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('#') && t.ends_with('#') && t.trim_start_matches('#').trim_end_matches('#').trim() != "" && {
        let inner = t.trim_end_matches('#');
        inner.ends_with([' ', '\t']) || !inner.trim_start_matches('#').contains(' ')
    }
}

/// The first line of a list item or a quote: a heading there needs no blank above.
fn first_in_container(doc: &Document, line: usize) -> bool {
    let text = &doc.lines[line];
    let stripped = strip_quotes(text);
    stripped.len() != text.len() && line > 0 && !doc.lines[line - 1].trim_start().starts_with('>')
}

fn lines(doc: &Document, config: &LintConfig, push: &mut Push) {
    let br = config.number("MD009", "br_spaces", 2).max(0) as usize;
    let maximum = config.number("MD012", "maximum", 1).max(0) as usize;
    let tabs_in_code = config.flag("MD010", "code_blocks", true);
    let mut blanks = 0usize;
    static ATX_NO_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#+[^#\s]").unwrap());
    static CLOSED_NO_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#+)([^#\s].*?|.*?[^#\s\\])(#+)\s*$").unwrap());
    static REVERSED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|[^\\])\(([^()]+)\)\[([^\]^][^\]]*)\]").unwrap());
    let in_code = |line: usize| doc.code.iter().any(|c| c.first <= line && line <= c.last);
    // MD009 looks at the fences of a fenced block, as markdownlint does.
    let in_code_body = |line: usize| {
        doc.code.iter().any(|c| if c.fenced { c.first < line && line < closing_fence(doc, c) } else { c.first <= line && line <= c.last })
    };
    for (n, line) in doc.lines.iter().enumerate() {
        if n < doc.front_matter {
            continue;
        }
        // MD012
        if line.trim().is_empty() && !in_code(n) {
            blanks += 1;
            if blanks > maximum {
                push(n, None, "MD012", format!("[Expected: {maximum}; Actual: {blanks}]"));
            }
        } else {
            blanks = 0;
        }
        // MD009: not in code, nor the two spaces of a line break.
        let trailing = line.len() - line.trim_end_matches(' ').len();
        if trailing > 0 && !in_code_body(n) && !(trailing == br && br >= 2 && !line.trim().is_empty()) {
            push(n, Some(line.trim_end_matches(' ').chars().count() + 1), "MD009", format!("[Expected: 0 or {br}; Actual: {trailing}]"));
        }
        // MD010
        if line.contains('\t') && (tabs_in_code || !in_code(n)) {
            let mut col = 0;
            let mut previous_tab = false;
            for c in line.chars() {
                col += 1;
                if c == '\t' && !previous_tab {
                    push(n, Some(col), "MD010", format!("[Column: {col}]"));
                }
                previous_tab = c == '\t';
            }
        }
        if doc.opaque[n] {
            continue;
        }
        let body = strip_quotes(line);
        let is_heading = doc.headings.iter().any(|h| h.line == n);
        // MD018: a line that wanted to be a heading.
        // (a closed one, `##Title##`, is MD020's)
        if !is_heading
            && ATX_NO_SPACE.is_match(line)
            && !CLOSED_NO_SPACE.is_match(line.trim())
            && !line.starts_with("#️⃣")
            && !in_paragraph_continuation(doc, n)
        {
            push(n, Some(1), "MD018", String::new());
        }
        // MD020
        if CLOSED_NO_SPACE.is_match(line.trim()) && !line.trim_start().starts_with("#️⃣") {
            let t = line.trim();
            let open = t.chars().take_while(|c| *c == '#').count();
            let after_open = t[open..].chars().next();
            let before_close = t.trim_end_matches('#').chars().last();
            if after_open.is_some_and(|c| !c.is_whitespace()) || before_close.is_some_and(|c| !c.is_whitespace()) {
                push(n, Some(1), "MD020", String::new());
            }
        }
        // MD027: more than one space after a quote's `>`.
        if line.trim_start().starts_with('>') {
            let t = line.trim_start();
            let after = &t[1..];
            if after.starts_with("  ") && !after.trim_start().is_empty() {
                let col = line.len() - t.len() + 3;
                push(n, Some(col), "MD027", String::new());
            }
        }
        // MD011
        if !in_code(n) {
            for caps in REVERSED.captures_iter(body) {
                let whole = caps.get(0).unwrap();
                let start = whole.start() + caps.get(1).unwrap().len();
                let offset = doc.starts[n] + (line.len() - body.len()) + start;
                if !doc.skipped(offset) {
                    push(n, Some(doc.column(offset)), "MD011", String::new());
                }
            }
        }
    }
    // MD035
    let style = config.text("MD035", "style", "consistent");
    let mut first: Option<String> = None;
    for &n in &doc.rules {
        let rule = doc.lines[n].trim().to_string();
        let expected = if style == "consistent" { first.get_or_insert(rule.clone()).clone() } else { style.clone() };
        if rule != expected {
            push(n, None, "MD035", format!("[Expected: {expected}; Actual: {rule}]"));
        }
    }
}

/// A `#word` line under a paragraph's line is a tag in that paragraph, and
/// markdownlint does not take it for a heading either.
fn in_paragraph_continuation(doc: &Document, n: usize) -> bool {
    n > doc.front_matter && !doc.blank(n - 1) && !doc.headings.iter().any(|h| h.end == n - 1)
}

fn lists(doc: &Document, config: &LintConfig, push: &mut Push) {
    let ul_style = config.text("MD004", "style", "consistent");
    let mut first_marker: Option<char> = None;
    let indent = config.number("MD007", "indent", 2).max(1) as usize;
    let start_indented = config.flag("MD007", "start_indented", false);
    let ol_style = config.text("MD029", "style", "one_or_ordered");
    for list in &doc.lists {
        let mut ordered_values: Vec<(usize, u64)> = Vec::new();
        for &line in &list.items {
            let text = &doc.lines[line];
            let body = strip_quotes(text);
            let lead = body.len() - body.trim_start_matches([' ', '\t']).len();
            let rest = &body[lead..];
            let (marker_len, marker) = if list.ordered {
                let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
                (digits + 1, rest.chars().nth(digits).unwrap_or('.'))
            } else {
                (1, rest.chars().next().unwrap_or('-'))
            };
            let after = &rest[marker_len.min(rest.len())..];
            let spaces = after.len() - after.trim_start_matches(' ').len();
            let column = text.len() - body.len() + lead + 1;
            if !list.ordered {
                // MD004
                let name = |c: char| match c {
                    '*' => "asterisk",
                    '+' => "plus",
                    _ => "dash",
                };
                let expected = match ul_style.as_str() {
                    "consistent" => name(*first_marker.get_or_insert(marker)),
                    "sublist" => name(marker),
                    other => match other {
                        "asterisk" => "asterisk",
                        "plus" => "plus",
                        _ => "dash",
                    },
                };
                if name(marker) != expected {
                    push(line, Some(column), "MD004", format!("[Expected: {expected}; Actual: {}]", name(marker)));
                }
                // MD007: in unordered lists only, all the way up.
                if list.parents.iter().all(|ordered| !ordered) {
                    let depth = list.parents.len();
                    let expected = (depth + usize::from(start_indented)) * indent;
                    if lead != expected {
                        push(line, Some(1), "MD007", format!("[Expected: {expected}; Actual: {lead}]"));
                    }
                }
            } else {
                let value: u64 = rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                ordered_values.push((line, value));
            }
            // MD030: one space after the mark (a mark with nothing after is fine).
            if !after.trim().is_empty() && spaces != 1 && !after.starts_with('\t') {
                push(line, Some(column), "MD030", format!("[Expected: 1; Actual: {spaces}]"));
            }
        }
        // MD029
        // As markdownlint counts: from 1 (or 0), whatever the first number.
        if let Some(&(_, first)) = ordered_values.first() {
            let incrementing = ordered_values.get(1).is_some_and(|&(_, second)| second != 1 || first == 0);
            let style = match ol_style.as_str() {
                "one" | "ordered" | "zero" => ol_style.as_str(),
                _ if incrementing => "ordered",
                _ => "one",
            };
            let mut expected = match style {
                "zero" => 0,
                "ordered" if incrementing && first == 0 => 0,
                _ => 1,
            };
            for &(line, value) in &ordered_values {
                if value != expected {
                    push(line, Some(1), "MD029", format!("[Expected: {expected}; Actual: {value}]"));
                }
                if style == "ordered" {
                    expected += 1;
                }
            }
        }
        // MD032: only a list that is not in another.
        if list.parents.is_empty() {
            if list.first > doc.front_matter
                && !doc.blank(list.first - 1)
                && !first_in_container(doc, list.first)
                && !quote_opens(doc, list)
            {
                push(list.first, None, "MD032", String::new());
            }
            if list.last + 1 < doc.lines.len() && !doc.blank(list.last + 1) && !quote_closes(doc, list) {
                push(list.last, None, "MD032", String::new());
            }
        }
    }
}

/// A list on a quote's first line has the quote's edge above it.
fn quote_opens(doc: &Document, list: &List) -> bool {
    list.blockquote > 0 && (list.first == 0 || !doc.lines[list.first - 1].trim_start().starts_with('>'))
}

/// A list on a quote's last line has the quote's edge under it.
fn quote_closes(doc: &Document, list: &List) -> bool {
    list.blockquote > 0 && !doc.lines.get(list.last + 1).is_some_and(|l| l.trim_start().starts_with('>'))
}

fn code(doc: &Document, config: &LintConfig, push: &mut Push) {
    let block_style = config.text("MD046", "style", "consistent");
    let mut first_block: Option<&str> = None;
    let fence_style = config.text("MD048", "style", "consistent");
    let mut first_fence: Option<&str> = None;
    for block in &doc.code {
        let kind = if block.fenced { "fenced" } else { "indented" };
        let expected = if block_style == "consistent" { *first_block.get_or_insert(kind) } else { block_style.as_str() };
        if kind != expected {
            push(block.first, None, "MD046", format!("[Expected: {expected}; Actual: {kind}]"));
        }
        if !block.fenced {
            continue;
        }
        let fence_line = &doc.lines[block.first];
        let fence = if strip_quotes(fence_line).trim_start().starts_with('~') { "tilde" } else { "backtick" };
        let expected = if fence_style == "consistent" { *first_fence.get_or_insert(fence) } else { fence_style.as_str() };
        if fence != expected {
            push(block.first, None, "MD048", format!("[Expected: {expected}; Actual: {fence}]"));
        }
        // MD040
        if block.info.trim().is_empty() {
            push(block.first, None, "MD040", String::new());
        }
        // MD031: blank lines around, unless the fence opens or closes its container.
        if block.first > doc.front_matter
            && !doc.blank(block.first - 1)
            && !first_in_container(doc, block.first)
            && !list_item_line(doc, block.first)
        {
            push(block.first, None, "MD031", String::new());
        }
        let closes = closing_fence(doc, block);
        if closes + 1 < doc.lines.len() && !doc.blank(closes + 1) && !container_ends(doc, closes) {
            push(closes, None, "MD031", String::new());
        }
        // MD014: every line a `$ ` command, and no output.
        let body: Vec<&String> = doc.lines[block.first + 1..closes.max(block.first + 1)].iter().filter(|l| !l.trim().is_empty()).collect();
        if !body.is_empty() && body.iter().all(|l| strip_quotes(l).trim_start().starts_with("$ ") || strip_quotes(l).trim_start() == "$") {
            for (k, l) in doc.lines[block.first + 1..closes].iter().enumerate() {
                if !l.trim().is_empty() {
                    push(block.first + 1 + k, Some(1), "MD014", String::new());
                }
            }
        }
    }
}

/// The line of a fenced block's closing fence (its last line when it never closes).
fn closing_fence(doc: &Document, block: &CodeBlock) -> usize {
    block.last.min(doc.lines.len() - 1)
}

/// MD047: the text ends with a newline, or its last line is blank.
fn ends_well(text: &str) -> bool {
    text.is_empty() || text.ends_with('\n') || text.rsplit('\n').next().is_some_and(|l| l.trim().is_empty())
}

fn list_item_line(doc: &Document, line: usize) -> bool {
    doc.lists.iter().any(|l| l.items.contains(&line))
}

/// The line after [line] leaves the list item or quote [line] is in.
fn container_ends(doc: &Document, line: usize) -> bool {
    let this_quoted = doc.lines[line].trim_start().starts_with('>');
    let next = &doc.lines[line + 1];
    (this_quoted && !next.trim_start().starts_with('>')) || {
        let indent = |l: &str| l.len() - l.trim_start().len();
        doc.lists.iter().any(|list| list.first <= line && line <= list.last)
            && indent(next) < indent(&doc.lines[line])
            && !doc.blank(line + 1)
    }
}

fn inline(doc: &Document, config: &LintConfig, push: &mut Push) {
    // MD034: a URL in text that no link holds.
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:https?|ftp)://[^\s<>\[\]()'`]*[^\s<>\[\]()'`.,;:!?]|\b[\w.+-]+@[\w-]+\.[\w.-]*\w").unwrap());
    for (range, text) in &doc.texts {
        let line = doc.line_of(range.start);
        if doc.opaque[line] || doc.skipped(range.start) {
            continue;
        }
        let source = &doc.lines[line];
        for m in URL.find_iter(text) {
            // In the text as the source has it: an escaped or entity URL is left alone.
            let Some(at) = source.find(m.as_str()) else { continue };
            let before = source[..at].chars().last();
            if matches!(before, Some('<') | Some('(') | Some('[') | Some('"') | Some('\'') | Some('/')) || in_wikilink(source, at) {
                continue;
            }
            if m.as_str().contains('@') && !m.as_str().contains("://") {
                // markdownlint takes an email for a bare URL too.
                if m.as_str().starts_with('@') || !m.as_str().split('@').nth(1).is_some_and(|d| d.contains('.')) {
                    continue;
                }
            }
            if doc.in_dollars(doc.starts[line] + at) {
                continue;
            }
            push(line, Some(source[..at].chars().count() + 1), "MD034", String::new());
        }
    }
    // MD037: emphasis marks that spaces kept from being emphasis.
    static LOOSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\*{1,3}|_{1,3})( +)\S[^*_\n]*?\S( +)?(\*{1,3}|_{1,3})").unwrap());
    for (range, text) in &doc.texts {
        let line = doc.line_of(range.start);
        if doc.opaque[line] || !(text.contains('*') || text.contains('_')) {
            continue;
        }
        for caps in LOOSE.captures_iter(text) {
            let (open, close) = (caps.get(1).unwrap().as_str(), caps.get(4).unwrap().as_str());
            if open != close {
                continue;
            }
            let start = caps.get(0).unwrap().start();
            // a word's own underscore, or a list's star, is not a mark
            if start > 0 && text[..start].chars().last().is_some_and(|c| c.is_alphanumeric()) {
                continue;
            }
            let offset = range.start + start;
            if doc.in_dollars(offset) {
                continue;
            }
            push(line, Some(doc.column(offset)), "MD037", String::new());
            if caps.get(3).is_some() {
                let end = caps.get(4).unwrap().start();
                push(line, Some(doc.column(range.start + end)), "MD037", String::new());
            }
        }
    }
    // MD038
    for span in &doc.code_spans {
        let source = &doc_text(doc, span);
        let ticks = source.chars().take_while(|c| *c == '`').count();
        if source.len() < ticks * 2 {
            continue;
        }
        let inner = &source[ticks..source.len() - ticks];
        if inner.trim().is_empty() {
            continue;
        }
        let lead = inner.len() - inner.trim_start_matches(' ').len();
        let trail = inner.len() - inner.trim_end_matches(' ').len();
        // One space on each side is CommonMark's padding; one space holds a
        // backtick at that end off the fence.
        let held = |spaces: usize, backtick: bool| spaces == 0 || (spaces == 1 && backtick);
        if (lead == 1 && trail == 1) || (held(lead, inner.trim_start().starts_with('`')) && held(trail, inner.trim_end().ends_with('`'))) {
            continue;
        }
        let line = doc.line_of(span.start);
        push(line, Some(doc.column(span.start)), "MD038", String::new());
    }
    // MD039, MD042, MD045
    for (range, kind, dest, image) in &doc.links {
        if matches!(kind, LinkType::WikiLink { .. } | LinkType::Autolink | LinkType::Email) {
            continue;
        }
        let line = doc.line_of(range.start);
        let source = doc_text(doc, range);
        let open = if *image { 2 } else { 1 };
        let Some(close) = source.find("](").or_else(|| source.find("][")).or_else(|| source.rfind(']')) else { continue };
        let label = &source[open.min(close)..close];
        if *image {
            if label.trim().is_empty() && config.on("MD045") {
                push(line, Some(doc.column(range.start)), "MD045", String::new());
            }
            continue;
        }
        if !label.trim().is_empty() && (label.starts_with([' ', '\t']) || label.ends_with([' ', '\t'])) {
            if label.starts_with([' ', '\t']) {
                push(line, Some(doc.column(range.start + 1)), "MD039", String::new());
            }
            if label.ends_with([' ', '\t']) {
                push(line, Some(doc.column(range.start + close)), "MD039", String::new());
            }
        }
        if dest.is_empty() || dest == "#" {
            push(line, Some(doc.column(range.start)), "MD042", String::new());
        }
    }
    // MD049, MD050
    let em_style = config.text("MD049", "style", "consistent");
    let strong_style = config.text("MD050", "style", "consistent");
    let mut first_em: Option<&str> = None;
    let mut first_strong: Option<&str> = None;
    let mut spans = doc.emphasis.clone();
    spans.sort_by_key(|(r, _)| r.start);
    for (range, strong) in spans {
        if doc.in_dollars(range.start) {
            continue;
        }
        // Emphasis that starts or ends inside a word (`Eigen*vectors*`) keeps
        // its mark: the other would not be emphasis there.
        let before = doc.source[..range.start].chars().last();
        let after = doc.source[range.end..].chars().next();
        if before.is_some_and(char::is_alphanumeric) || after.is_some_and(char::is_alphanumeric) {
            continue;
        }
        let source = doc_text(doc, &range);
        let mark = if source.starts_with('_') { "underscore" } else { "asterisk" };
        let (rule, style, first) = if strong { ("MD050", &strong_style, &mut first_strong) } else { ("MD049", &em_style, &mut first_em) };
        let expected = if style == "consistent" { *first.get_or_insert(mark) } else { style.as_str() };
        if mark != expected {
            let line = doc.line_of(range.start);
            let width = if strong { 2 } else { 1 };
            push(line, Some(doc.column(range.start)), rule, format!("[Expected: {expected}; Actual: {mark}]"));
            push(doc.line_of(range.end - 1), Some(doc.column(range.end - width)), rule, format!("[Expected: {expected}; Actual: {mark}]"));
        }
    }
}

fn doc_text(doc: &Document, range: &Range<usize>) -> String {
    doc.source.get(range.clone()).unwrap_or("").to_string()
}

/// Whether byte [at] of [line] is inside `[[...]]`.
fn in_wikilink(line: &str, at: usize) -> bool {
    let before = &line[..at];
    before.rfind("[[").is_some_and(|open| before[open..].find("]]").is_none())
}

/// One round of fixes.
fn fix_once(text: &str, config: &LintConfig) -> String {
    let problems = MarkdownLint::lint(text, config);
    if problems.is_empty() {
        return text.to_string();
    }
    let doc = Document::parse(text);
    let mut lines = doc.lines.clone();
    let mut insert_before: Vec<usize> = Vec::new();
    let mut insert_after: Vec<usize> = Vec::new();
    let mut remove: Vec<usize> = Vec::new();
    for p in &problems {
        let n = p.line - 1;
        let Some(line) = lines.get(n).cloned() else { continue };
        match p.rule {
            "MD009" => lines[n] = line.trim_end_matches(' ').to_string(),
            "MD010" if !doc.code.iter().any(|c| c.first <= n && n <= c.last) => lines[n] = line.replace('\t', "    "),
            "MD012" => remove.push(n),
            "MD029" => {
                let expected = p.detail.trim_start_matches("[Expected: ").split(';').next().unwrap_or("1");
                let body = strip_quotes(&line);
                let at = line.len() - body.len() + (body.len() - body.trim_start_matches([' ', '\t']).len());
                let digits = line[at..].chars().take_while(|c| c.is_ascii_digit()).count();
                lines[n] = format!("{}{expected}{}", &line[..at], &line[at + digits..]);
            }
            "MD004" => {
                if let Some(col) = p.column {
                    let mut chars: Vec<char> = line.chars().collect();
                    if let Some(c) = chars.get_mut(col - 1) {
                        *c = match config.text("MD004", "style", "dash").as_str() {
                            "asterisk" => '*',
                            "plus" => '+',
                            _ => '-',
                        };
                    }
                    lines[n] = chars.into_iter().collect();
                }
            }
            "MD030" => {
                static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*(?:>\s*)*(?:[-*+]|\d+[.)]))\s+").unwrap());
                lines[n] = MARK.replace(&line, "$1 ").to_string();
            }
            "MD018" => {
                static OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#+)").unwrap());
                lines[n] = OPEN.replace(&line, "$1 ").to_string();
            }
            "MD019" | "MD021" | "MD020" => {
                static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)(#+)\s*(.*?)\s*(#*)\s*$").unwrap());
                if let Some(c) = HEADING.captures(&line) {
                    let close = &c[4];
                    lines[n] = if close.is_empty() {
                        format!("{}{} {}", &c[1], &c[2], &c[3])
                    } else {
                        format!("{}{} {} {}", &c[1], &c[2], &c[3], close)
                    };
                }
            }
            "MD023" => lines[n] = line.trim_start().to_string(),
            "MD026" => lines[n] = line.trim_end().trim_end_matches(|c: char| ".,;:!。，；：！".contains(c)).to_string(),
            "MD027" => {
                static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*>) +").unwrap());
                lines[n] = QUOTE.replace(&line, "$1 ").to_string();
            }
            "MD035" => lines[n] = config.text("MD035", "style", "---").replace("consistent", "---"),
            "MD037" => {
                static LOOSE: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"(\*{1,3}|_{1,3}) +(\S[^*_\n]*?\S) *(\*{1,3}|_{1,3})").unwrap());
                lines[n] = LOOSE
                    .replace_all(
                        &line,
                        |c: &regex::Captures| if c[1] == c[3] { format!("{}{}{}", &c[1], &c[2], &c[3]) } else { c[0].to_string() },
                    )
                    .to_string();
            }
            "MD038" => {
                static SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`( +)([^`]*?\S)( *)`|`([^`]*?\S)( +)`").unwrap());
                lines[n] = SPAN
                    .replace_all(&line, |c: &regex::Captures| {
                        let inner = c.get(2).or(c.get(4)).map_or("", |m| m.as_str());
                        format!("`{inner}`")
                    })
                    .to_string();
            }
            "MD039" => {
                static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\s+([^\]]*?)\s*\]\(|\[([^\]]*?\S)\s+\]\(").unwrap());
                lines[n] = LABEL
                    .replace_all(&line, |c: &regex::Captures| format!("[{}](", c.get(1).or(c.get(2)).map_or("", |m| m.as_str())))
                    .to_string();
            }
            "MD034" => {
                if let Some(col) = p.column {
                    let chars: Vec<char> = line.chars().collect();
                    let start = col - 1;
                    let end = (start..chars.len()).find(|&i| chars[i].is_whitespace()).unwrap_or(chars.len());
                    let mut url: String = chars[start..end].iter().collect();
                    let tail: String =
                        url.chars().rev().take_while(|c| ".,;:!?)".contains(*c)).collect::<Vec<_>>().into_iter().rev().collect();
                    url.truncate(url.len() - tail.len());
                    let before: String = chars[..start].iter().collect();
                    let after: String = chars[end..].iter().collect();
                    lines[n] = format!("{before}<{url}>{tail}{after}");
                }
            }
            "MD048" => {
                let want = if config.text("MD048", "style", "backtick") == "tilde" { '~' } else { '`' };
                let from = if want == '`' { '~' } else { '`' };
                if let Some(block) = doc.code.iter().find(|c| c.first == n) {
                    let close = closing_fence(&doc, block);
                    for k in [n, close] {
                        let l = lines[k].clone();
                        let lead = l.len() - l.trim_start().len();
                        let run = l[lead..].chars().take_while(|c| *c == from).count();
                        lines[k] = format!("{}{}{}", &l[..lead], want.to_string().repeat(run), &l[lead + run..]);
                    }
                }
            }
            "MD049" | "MD050" => {
                if let Some(col) = p.column {
                    let width = if p.rule == "MD050" { 2 } else { 1 };
                    let want = if p.detail.contains("Expected: underscore") { '_' } else { '*' };
                    let mut chars: Vec<char> = line.chars().collect();
                    for c in chars.iter_mut().skip(col - 1).take(width) {
                        *c = want;
                    }
                    lines[n] = chars.into_iter().collect();
                }
            }
            "MD022" | "MD031" | "MD032" | "MD058" => {
                if p.detail.contains("Below") || (p.rule != "MD022" && p.rule != "MD058" && is_below(&doc, p)) {
                    insert_after.push(heading_end(&doc, n, p.rule));
                } else if p.detail.contains("Above") || p.rule != "MD022" {
                    insert_before.push(n);
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        if remove.contains(&k) {
            continue;
        }
        if insert_before.contains(&k) && out.last().is_some_and(|l: &String| !l.trim().is_empty()) {
            out.push(blank_like(line));
        }
        out.push(line.clone());
        if insert_after.contains(&k) && lines.get(k + 1).is_some_and(|l| !l.trim().is_empty()) {
            out.push(blank_like(line));
        }
    }
    let mut text = out.join("\n");
    if config.on("MD047") && !ends_well(&text) {
        text.push('\n');
    }
    text
}

/// A blank line inside the quote [line] is in.
fn blank_like(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with('>') { ">".to_string() } else { String::new() }
}

/// Whether a block problem on [p]'s line is for the line under the block.
fn is_below(doc: &Document, p: &Problem) -> bool {
    let n = p.line - 1;
    match p.rule {
        "MD031" => doc.code.iter().any(|c| closing_fence(doc, c) == n && c.first != n),
        "MD032" => doc.lists.iter().any(|l| l.last == n && (l.first != n || l.first == 0 || doc.blank(n.saturating_sub(1)))),
        _ => false,
    }
}

fn heading_end(doc: &Document, n: usize, rule: &str) -> usize {
    if rule == "MD022" { doc.headings.iter().find(|h| h.line == n).map_or(n, |h| h.end) } else { n }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LintConfig {
        LintConfig::parse(include_str!("../tests/lint/.markdownlint.jsonc")).unwrap()
    }

    #[test]
    fn the_config_is_read_with_its_comments() {
        let config = config();
        assert!(config.on("MD001"));
        assert!(!config.on("MD005"));
        assert!(!config.on("MD013"));
        assert!(config.on("MD058"));
        assert_eq!(config.number("MD007", "indent", 2), 4);
        assert_eq!(config.text("MD004", "style", ""), "dash");
    }

    #[test]
    fn obsidian_syntax_is_no_lint() {
        let text = include_str!("../tests/lint/obsidian.md");
        assert_eq!(MarkdownLint::lint(text, &config()), vec![]);
    }

    #[test]
    fn fixing_mends_what_it_can() {
        let text = "# Title\n## Two\n* a \n*  b\n\n\n\nVisit https://example.com today.\n~~~bash\nls\n~~~\n__strong__ and *em*";
        let fixed = MarkdownLint::fix(text, &config());
        assert_eq!(
            fixed,
            "# Title\n\n## Two\n\n- a\n- b\n\nVisit <https://example.com> today.\n\n```bash\nls\n```\n\n**strong** and *em*\n"
        );
        assert_eq!(MarkdownLint::lint(&fixed, &config()), vec![]);
    }
}
