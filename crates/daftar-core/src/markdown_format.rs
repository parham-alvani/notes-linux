//! Markdown formatting, as Prettier 3.9 formats a note with the vault's
//! `.prettierrc`, with no Node. The note is parsed by `markdown-rs`, the Rust
//! port of micromark that Prettier parses with, with Prettier's own syntax on
//! top (`[[wiki links]]`, `{{ liquid }}`), and printed by the rules of
//! Prettier's `language-markdown` printer, each function by the name of the
//! one it follows. Checked against Prettier on the notes of a real vault and
//! on those notes unformatted in every way at random.
//!
//! Where it differs: code in a fenced block is never reformatted, except
//! Markdown's own (Prettier formats JSON, YAML, JavaScript and the rest of
//! its languages there); `[[Note:alias]]` keeps its alias, which Prettier
//! drops. Only `proseWrap` `never` and `preserve` are known.

use std::sync::LazyLock;

use markdown::mdast::{self, AlignKind, ReferenceKind};
use markdown::{Constructs, ParseOptions};
use regex::Regex;

use crate::string_width::string_width;
use crate::yaml_format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProseWrap {
    Never,
    Preserve,
}

/// The options of a `.prettierrc` that Markdown uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatConfig {
    pub print_width: usize,
    pub tab_width: usize,
    pub prose_wrap: ProseWrap,
    pub single_quote: bool,
}

impl Default for FormatConfig {
    /// Prettier's defaults.
    fn default() -> Self {
        FormatConfig { print_width: 80, tab_width: 2, prose_wrap: ProseWrap::Preserve, single_quote: false }
    }
}

impl FormatConfig {
    /// A `.prettierrc`, in JSON or as YAML's `key: value` lines.
    pub fn parse(text: &str) -> Result<FormatConfig, String> {
        let mut config = FormatConfig::default();
        let pairs: Vec<(String, String)> = match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(map)) => {
                map.into_iter().map(|(k, v)| (k, v.as_str().map_or_else(|| v.to_string(), str::to_string))).collect()
            }
            Ok(_) => return Err("not an object".to_string()),
            Err(_) => text
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .map(|l| {
                    let (k, v) = l.split_once(':').ok_or_else(|| format!("cannot read: {l}"))?;
                    Ok((k.trim().to_string(), v.trim().trim_matches(['"', '\'']).to_string()))
                })
                .collect::<Result<_, String>>()?,
        };
        for (key, value) in pairs {
            let number = || value.parse::<usize>().map_err(|_| format!("{key}: not a number: {value}"));
            match key.as_str() {
                "printWidth" => config.print_width = number()?,
                "tabWidth" => config.tab_width = number()?,
                "singleQuote" => config.single_quote = value == "true",
                "proseWrap" => {
                    config.prose_wrap = match value.as_str() {
                        "never" => ProseWrap::Never,
                        "preserve" => ProseWrap::Preserve,
                        other => return Err(format!("proseWrap {other} is not supported, only never and preserve")),
                    }
                }
                _ => {}
            }
        }
        Ok(config)
    }
}

pub struct MarkdownFormat;

impl MarkdownFormat {
    /// The note as Prettier would write it.
    pub fn format(text: &str, config: &FormatConfig) -> String {
        let (bom, text) = match text.strip_prefix('\u{FEFF}') {
            Some(rest) => ("\u{FEFF}", rest),
            None => ("", text),
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let Some(doc) = document(&text, config) else { return format!("{bom}{text}") };
        format!("{bom}{}", Printer::print(&doc, config.print_width))
    }
}

/// The printed root of a note, without its last line break, for a note in a
/// note (a ` ```markdown ` block).
fn document(text: &str, config: &FormatConfig) -> Option<Doc> {
    let root = parse(text)?;
    let f = Fmt { src: text, config };
    if root.children.is_empty() {
        return Some(Doc::Text(String::new()));
    }
    Some(Doc::Concat(vec![f.root(&root), Doc::Hard]))
}

// ---------------------------------------------------------------- the tree

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    start: usize,
    end: usize,
    start_line: usize,
    end_line: usize,
    children: Vec<Node>,
}

#[derive(Debug, Clone)]
enum Kind {
    Root,
    FrontMatter {
        start: String,
        end: String,
        language: String,
        explicit: String,
        value: String,
        raw: String,
    },
    Paragraph,
    Heading(u8),
    ThematicBreak,
    Blockquote,
    List {
        ordered: bool,
        start: u32,
        aligned: bool,
    },
    ListItem {
        spread: bool,
        checked: Option<bool>,
    },
    Html(String),
    Code {
        value: String,
        lang: Option<String>,
        meta: Option<String>,
        indented: bool,
    },
    Math {
        value: String,
        meta: Option<String>,
    },
    Definition {
        url: String,
        title: Option<String>,
        label: String,
    },
    FootnoteDefinition(String),
    Table(Vec<AlignKind>),
    TableRow,
    TableCell,
    /// The parser's text, before it is cut into words.
    Text(Option<String>),
    Sentence(Vec<Tok>),
    /// Text kept as written: the paragraph could make a wiki link of it.
    RawText(String),
    Emphasis,
    Strong,
    Delete,
    InlineCode(String),
    InlineMath,
    Break,
    Link {
        url: String,
        title: Option<String>,
    },
    Image {
        url: String,
        title: Option<String>,
        alt: String,
    },
    LinkReference {
        kind: ReferenceKind,
        label: String,
    },
    ImageReference {
        kind: ReferenceKind,
        label: String,
        alt: String,
    },
    FootnoteReference(String),
    WikiLink(String),
    Liquid(String),
}

impl Node {
    fn is(&self, name: &str) -> bool {
        self.name() == name
    }

    /// The node's mdast type.
    fn name(&self) -> &'static str {
        match self.kind {
            Kind::Root => "root",
            Kind::FrontMatter { .. } => "frontMatter",
            Kind::Paragraph => "paragraph",
            Kind::Heading(_) => "heading",
            Kind::ThematicBreak => "thematicBreak",
            Kind::Blockquote => "blockquote",
            Kind::List { .. } => "list",
            Kind::ListItem { .. } => "listItem",
            Kind::Html(_) => "html",
            Kind::Code { .. } => "code",
            Kind::Math { .. } => "math",
            Kind::Definition { .. } => "definition",
            Kind::FootnoteDefinition(_) => "footnoteDefinition",
            Kind::Table(_) => "table",
            Kind::TableRow => "tableRow",
            Kind::TableCell => "tableCell",
            Kind::Text(_) | Kind::RawText(_) => "text",
            Kind::Sentence(_) => "sentence",
            Kind::Emphasis => "emphasis",
            Kind::Strong => "strong",
            Kind::Delete => "delete",
            Kind::InlineCode(_) => "inlineCode",
            Kind::InlineMath => "inlineMath",
            Kind::Break => "break",
            Kind::Link { .. } => "link",
            Kind::Image { .. } => "image",
            Kind::LinkReference { .. } => "linkReference",
            Kind::ImageReference { .. } => "imageReference",
            Kind::FootnoteReference(_) => "footnoteReference",
            Kind::WikiLink(_) => "wikiLink",
            Kind::Liquid(_) => "liquidNode",
        }
    }

    fn is_inline(&self) -> bool {
        matches!(
            self.name(),
            "liquidNode"
                | "inlineCode"
                | "emphasis"
                | "strong"
                | "delete"
                | "wikiLink"
                | "link"
                | "linkReference"
                | "image"
                | "imageReference"
                | "footnoteReference"
                | "sentence"
                | "break"
                | "inlineMath"
        )
    }

    fn is_indented_code(&self) -> bool {
        matches!(self.kind, Kind::Code { indented: true, .. })
    }

    fn is_setext_heading(&self) -> bool {
        matches!(self.kind, Kind::Heading(_)) && self.start_line != self.end_line
    }
}

/// A word or the space between two, as Prettier's `splitText` cuts text.
#[derive(Debug, Clone)]
enum Tok {
    Word {
        value: String,
        kind: WordKind,
        leading_punctuation: bool,
        trailing_punctuation: bool,
    },
    /// `" "`, `"\n"`, or `""` between two CJK letters.
    Space(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordKind {
    NonCjk,
    CjLetter,
    KLetter,
    CjkPunctuation,
}

impl Tok {
    fn value(&self) -> &str {
        match self {
            Tok::Word { value, .. } => value,
            Tok::Space(s) => s,
        }
    }

    fn kind(&self) -> Option<WordKind> {
        match self {
            Tok::Word { kind, .. } => Some(*kind),
            Tok::Space(_) => None,
        }
    }
}

// ---------------------------------------------------------------- parsing

struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(text: &str) -> Lines {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { starts }
    }

    /// The line of a byte offset, from 1.
    fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset)
    }
}

fn parse_options() -> ParseOptions {
    let mut constructs = Constructs::gfm();
    constructs.math_flow = true;
    constructs.math_text = true;
    ParseOptions { constructs, gfm_strikethrough_single_tilde: true, math_text_single_dollar: true, ..ParseOptions::default() }
}

/// Prettier's `parseFrontMatter`.
fn front_matter(text: &str) -> Option<Kind> {
    let start = text.get(..3)?;
    if start != "---" && start != "+++" {
        return None;
    }
    let first_break = text[3..].find('\n')? + 3;
    let explicit = text[3..first_break].trim().to_string();
    let mut end = text[first_break..].find(&format!("\n{start}")).map(|i| i + first_break);
    let language = if !explicit.is_empty() {
        explicit.clone()
    } else if start == "+++" {
        "toml".to_string()
    } else {
        "yaml".to_string()
    };
    if end.is_none() && start == "---" && language == "yaml" {
        end = text[first_break..].find("\n...").map(|i| i + first_break);
    }
    let end = end?;
    let raw_end = end + 4;
    Some(Kind::FrontMatter {
        start: start.to_string(),
        end: text[raw_end - 3..raw_end].to_string(),
        language,
        explicit,
        value: text[first_break + 1..end].to_string(),
        raw: text[..raw_end].to_string(),
    })
}

/// A wiki link or a liquid tag found in the text, which the parser is not
/// to read inside.
struct Span {
    start: usize,
    end: usize,
    kind: SpanKind,
}

enum SpanKind {
    Wiki(String),
    Liquid,
}

fn parse(text: &str) -> Option<Node> {
    let lines = Lines::new(text);
    let front = front_matter(text);
    let mut content = text.to_string();
    let front_end = match &front {
        Some(Kind::FrontMatter { raw, .. }) => {
            let blank: String = raw.chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect::<String>();
            // Same bytes as the raw text: every character becomes spaces as long.
            let blank: String =
                raw.chars().zip(blank.chars()).map(|(c, b)| if b == '\n' { "\n".to_string() } else { " ".repeat(c.len_utf8()) }).collect();
            content.replace_range(..raw.len(), &blank);
            raw.len()
        }
        _ => 0,
    };
    let options = parse_options();
    let first = markdown::to_mdast(&content, &options).ok()?;
    let spans = find_spans(&content, &first);
    let symbols = symbols_by_delimiters(&content, &first);
    let tree = if spans.is_empty() && symbols.is_empty() {
        first
    } else {
        let mut bytes = content.clone().into_bytes();
        for span in &spans {
            mask(&mut bytes, span);
        }
        for (s, e) in symbols {
            bytes[s..e].fill(b'x');
        }
        let masked = String::from_utf8(bytes).ok()?;
        markdown::to_mdast(&masked, &options).ok()?
    };
    let mut root = convert(&tree, text, &lines)?;
    carve(&mut root, &spans, text, &lines);
    if let Some(kind) = front {
        root.children.insert(0, Node { kind, start: 0, end: front_end, start_line: 1, end_line: lines.line(front_end), children: vec![] });
    }
    Some(root)
}

/// The symbols (an emoji, `©`) beside a `*` or `_` outside code: markdown-rs
/// takes them for punctuation when it decides what a run of `*` or `_`
/// can open or close, as CommonMark 0.31 does; the micromark Prettier
/// parses with takes them for letters.
fn symbols_by_delimiters(content: &str, tree: &mdast::Node) -> Vec<(usize, usize)> {
    static SYMBOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{S}&&[^\x00-\x7F]]").unwrap());
    if !content.contains(['*', '_']) {
        return Vec::new();
    }
    let mut containers = Vec::new();
    let mut shut = Vec::new();
    collect_ranges(tree, &mut containers, &mut shut);
    SYMBOL
        .find_iter(content)
        .filter(|m| content[..m.start()].ends_with(['*', '_']) || content[m.end()..].starts_with(['*', '_']))
        .filter(|m| !shut.iter().any(|&(s, e)| s <= m.start() && m.start() < e))
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Where a wiki link or a liquid tag cannot start: in code, math, HTML, a
/// link's destination, an image.
fn find_spans(content: &str, tree: &mdast::Node) -> Vec<Span> {
    let mut containers = Vec::new();
    let mut shut = Vec::new();
    collect_ranges(tree, &mut containers, &mut shut);
    let bytes = content.as_bytes();
    let escaped = |i: usize| bytes[..i].iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1;
    let inside = |start: usize, end: usize| {
        containers.iter().any(|&(s, e)| s <= start && end <= e) && !shut.iter().any(|&(s, e)| s <= start && start < e)
    };
    let mut spans = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let found = match (bytes[i], bytes[i + 1]) {
            (b'[', b'[') if !escaped(i) && !(i > 0 && bytes[i - 1] == b'!' && !escaped(i - 1)) => wiki_link(content, i),
            (b'{', b'{' | b'%') if !escaped(i) => liquid(content, i).map(|end| (end, None)),
            _ => None,
        };
        match found {
            Some((end, value)) if inside(i, end) => {
                spans.push(Span { start: i, end, kind: value.map_or(SpanKind::Liquid, SpanKind::Wiki) });
                i = end;
            }
            _ => i += 1,
        }
    }
    spans
}

fn collect_ranges(node: &mdast::Node, containers: &mut Vec<(usize, usize)>, shut: &mut Vec<(usize, usize)>) {
    let Some(p) = node.position() else { return };
    let (s, e) = (p.start.offset, p.end.offset);
    match node {
        mdast::Node::Paragraph(_) | mdast::Node::Heading(_) | mdast::Node::TableCell(_) => containers.push((s, e)),
        mdast::Node::InlineCode(_)
        | mdast::Node::InlineMath(_)
        | mdast::Node::Html(_)
        | mdast::Node::Code(_)
        | mdast::Node::Math(_)
        | mdast::Node::Image(_)
        | mdast::Node::ImageReference(_)
        | mdast::Node::Definition(_) => shut.push((s, e)),
        mdast::Node::Link(_) | mdast::Node::LinkReference(_) => {
            let children = node.children().map_or(&[][..], Vec::as_slice);
            match children.last().and_then(|c| c.position()) {
                Some(last) if children.first().and_then(|c| c.position()).is_some_and(|f| f.start.offset > s) => {
                    shut.push((last.end.offset, e))
                }
                _ => shut.push((s, e)),
            }
        }
        _ => {}
    }
    for child in node.children().into_iter().flatten() {
        collect_ranges(child, containers, shut);
    }
}

/// micromark-extension-wiki-link: `[[target]]` or `[[target:alias]]`, on one
/// line, the target and the alias not blank. The end, and the text inside.
fn wiki_link(text: &str, start: usize) -> Option<(usize, Option<String>)> {
    let chars = text[start + 2..].char_indices().map(|(i, c)| (i + start + 2, c));
    let mut seen = false;
    let mut alias: Option<bool> = None;
    let mut chars = chars.peekable();
    loop {
        let (i, c) = chars.next()?;
        match c {
            '\n' => return None,
            ']' => {
                if !seen || alias == Some(false) {
                    return None;
                }
                if text[i + 1..].starts_with(']') {
                    return Some((i + 2, Some(text[start + 2..i].to_string())));
                }
                return None;
            }
            ':' if alias.is_none() => {
                if !seen {
                    return None;
                }
                alias = Some(false);
            }
            ' ' | '\t' => {}
            _ => {
                if alias.is_some() {
                    alias = Some(true);
                }
                seen = true;
            }
        }
    }
}

/// Prettier's liquid syntax: `{% … %}` or `{{ … }}`, over lines if need be.
fn liquid(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let close = if bytes[start + 1] == b'%' { b'%' } else { b'}' };
    let mut i = start + 2;
    while i < bytes.len() {
        if bytes[i] == close && bytes.get(i + 1) == Some(&b'}') {
            return Some(i + 2);
        }
        i += 1;
    }
    None
}

/// Hides a span from the parser: braces at its ends, `x` inside, the same
/// bytes long, with `|` and `\` kept for a table's cells and a line's
/// container marks kept on the lines after the first.
fn mask(bytes: &mut [u8], span: &Span) {
    let (s, e) = (span.start, span.end);
    bytes[s] = b'{';
    bytes[s + 1] = b'{';
    bytes[e - 1] = b'}';
    bytes[e - 2] = b'}';
    let mut line_start = false;
    for b in &mut bytes[s + 2..e - 2] {
        match *b {
            b'\n' => line_start = true,
            b' ' | b'\t' | b'>' if line_start => {}
            b'|' | b'\\' => line_start = false,
            _ => {
                *b = b'x';
                line_start = false;
            }
        }
    }
}

fn convert(node: &mdast::Node, src: &str, lines: &Lines) -> Option<Node> {
    use mdast::Node as M;
    let position = node.position()?;
    let (start, end) = (position.start.offset, position.end.offset);
    let kind = match node {
        M::Root(_) => Kind::Root,
        M::Paragraph(_) => Kind::Paragraph,
        M::Heading(h) => Kind::Heading(h.depth),
        M::ThematicBreak(_) => Kind::ThematicBreak,
        M::Blockquote(_) => Kind::Blockquote,
        M::List(l) => Kind::List { ordered: l.ordered, start: l.start.unwrap_or(1), aligned: false },
        M::ListItem(i) => Kind::ListItem { spread: i.spread, checked: i.checked },
        M::Html(h) => Kind::Html(h.value.clone()),
        M::Code(c) => {
            static INDENTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\n?(?: {4,}|\t)").unwrap());
            Kind::Code { value: c.value.clone(), lang: c.lang.clone(), meta: c.meta.clone(), indented: INDENTED.is_match(&src[start..end]) }
        }
        M::Math(m) => Kind::Math { value: m.value.clone(), meta: m.meta.clone() },
        M::Definition(d) => {
            Kind::Definition { url: d.url.clone(), title: d.title.clone(), label: d.label.clone().unwrap_or_else(|| d.identifier.clone()) }
        }
        M::FootnoteDefinition(f) => Kind::FootnoteDefinition(f.label.clone().unwrap_or_else(|| f.identifier.clone())),
        M::Table(t) => Kind::Table(t.align.clone()),
        M::TableRow(_) => Kind::TableRow,
        M::TableCell(_) => Kind::TableCell,
        M::Text(t) => Kind::Text(Some(t.value.clone())),
        M::Emphasis(_) => Kind::Emphasis,
        M::Strong(_) => Kind::Strong,
        M::Delete(_) => Kind::Delete,
        M::InlineCode(c) => {
            // micromark takes a line ending at either end for padding as it
            // does a space; markdown-rs only a space.
            let v = &c.value;
            let strip = v.len() >= 2
                && (v.starts_with('\n') || v.ends_with('\n'))
                && v.starts_with([' ', '\n'])
                && v.ends_with([' ', '\n'])
                && v.chars().any(|ch| ch != ' ' && ch != '\n');
            Kind::InlineCode(if strip { v[1..v.len() - 1].to_string() } else { v.clone() })
        }
        M::InlineMath(_) => Kind::InlineMath,
        M::Break(_) => Kind::Break,
        M::Link(l) => Kind::Link { url: l.url.clone(), title: l.title.clone() },
        M::Image(i) => Kind::Image { url: i.url.clone(), title: i.title.clone(), alt: i.alt.clone() },
        M::LinkReference(r) => {
            Kind::LinkReference { kind: r.reference_kind, label: r.label.clone().unwrap_or_else(|| r.identifier.clone()) }
        }
        M::ImageReference(r) => Kind::ImageReference {
            kind: r.reference_kind,
            label: r.label.clone().unwrap_or_else(|| r.identifier.clone()),
            alt: r.alt.clone(),
        },
        M::FootnoteReference(f) => Kind::FootnoteReference(f.label.clone().unwrap_or_else(|| f.identifier.clone())),
        _ => return None,
    };
    let mut children: Vec<Node> = node.children().into_iter().flatten().filter_map(|c| convert(c, src, lines)).collect();
    // markdown-rs starts a text that opens with a character escape after the
    // backslash, where micromark starts it at the backslash.
    let bytes = src.as_bytes();
    for i in 0..children.len() {
        let c = &children[i];
        let floor = if i == 0 { start } else { children[i - 1].end };
        if matches!(c.kind, Kind::Text(_)) && c.start > floor && bytes[c.start - 1] == b'\\' && bytes[c.start].is_ascii_punctuation() {
            let s = c.start - 1;
            children[i].start = s;
            children[i].start_line = lines.line(s);
        }
    }
    setext_after_paragraph(&mut children, src, lines);
    // And it ends a list item, and so its list, after the blank line that
    // follows, where micromark ends it with its content.
    let end = match (&kind, children.last()) {
        (Kind::ListItem { .. } | Kind::List { .. }, Some(last)) => last.end,
        _ => end,
    };
    Some(Node { kind, start, end, start_line: lines.line(start), end_line: lines.line(end), children })
}

/// markdown-rs lets an empty list item interrupt a paragraph; CommonMark
/// does not, and a lone `-` under a paragraph is a setext heading's line.
fn setext_after_paragraph(children: &mut Vec<Node>, src: &str, lines: &Lines) {
    let mut i = 0;
    while i + 1 < children.len() {
        let fits = children[i].is("paragraph")
            && matches!(children[i + 1].kind, Kind::List { ordered: false, .. })
            && children[i + 1].start_line == children[i].end_line + 1
            && children[i + 1].children.first().is_some_and(|item| item.children.is_empty() && src[item.start..item.end].trim() == "-");
        if !fits {
            i += 1;
            continue;
        }
        let mut list = children.remove(i + 1);
        let item = list.children.remove(0);
        let paragraph = &mut children[i];
        paragraph.kind = Kind::Heading(2);
        paragraph.end = item.end;
        paragraph.end_line = lines.line(item.end);
        if let Some(next) = list.children.first() {
            list.start = next.start;
            list.start_line = next.start_line;
            children.insert(i + 1, list);
        }
        i += 1;
    }
}

/// Cuts the wiki links and liquid tags out of the text they were hidden in.
fn carve(node: &mut Node, spans: &[Span], src: &str, lines: &Lines) {
    if spans.is_empty() {
        return;
    }
    let mut children = Vec::with_capacity(node.children.len());
    for mut child in std::mem::take(&mut node.children) {
        if !matches!(child.kind, Kind::Text(_)) {
            carve(&mut child, spans, src, lines);
            children.push(child);
            continue;
        }
        let inside: Vec<&Span> = spans.iter().filter(|s| child.start <= s.start && s.end <= child.end).collect();
        if inside.is_empty() {
            children.push(child);
            continue;
        }
        let piece = |kind: Kind, start: usize, end: usize| Node {
            kind,
            start,
            end,
            start_line: lines.line(start),
            end_line: lines.line(end),
            children: vec![],
        };
        let mut at = child.start;
        for span in inside {
            if span.start > at {
                children.push(piece(Kind::Text(None), at, span.start));
            }
            let kind = match &span.kind {
                SpanKind::Wiki(value) => Kind::WikiLink(value.clone()),
                SpanKind::Liquid => Kind::Liquid(liquid_value(&src[span.start..span.end])),
            };
            children.push(piece(kind, span.start, span.end));
            at = span.end;
        }
        if at < child.end {
            children.push(piece(Kind::Text(None), at, child.end));
        }
    }
    node.children = children;
}

/// A liquid tag's text without the container marks of the lines after its
/// first.
fn liquid_value(raw: &str) -> String {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*(?:>[ \t]?[ \t]*)*").unwrap());
    raw.split('\n')
        .enumerate()
        .map(|(i, l)| if i == 0 { l } else { &l[PREFIX.find(l).map_or(0, |m| m.end())..] })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prettier's `getOrderedListItemInfo`: the number and the spaces after it.
fn ordered_item_info(item: &Node, src: &str) -> (u64, String) {
    static INFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(\d+)[.)](\s*)").unwrap());
    let end = item.children.first().map_or(item.end, |c| c.start);
    match INFO.captures(&src[item.start..end]) {
        Some(c) => (c[1].parse().unwrap_or(0), c[2].to_string()),
        None => (0, String::new()),
    }
}

fn column(src: &str, offset: usize) -> usize {
    let line_start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    src[line_start..offset].chars().count()
}

fn align_lists(node: &mut Node, parents_aligned: bool, src: &str, tab_width: usize) {
    let len = node.children.len();
    for i in 0..len {
        let next_indented = node.children.get(i + 1).is_some_and(Node::is_indented_code);
        let child = &mut node.children[i];
        let mut aligned_here = parents_aligned;
        if let Kind::List { ordered, .. } = child.kind {
            let aligned =
                parents_aligned && !child.children.is_empty() && !next_indented && list_is_aligned(child, ordered, src, tab_width);
            if let Kind::List { aligned: a, .. } = &mut child.kind {
                *a = aligned;
            }
            aligned_here = if child.children.is_empty() { parents_aligned } else { aligned };
        }
        align_lists(child, aligned_here, src, tab_width);
    }
}

fn list_is_aligned(list: &Node, ordered: bool, src: &str, tab_width: usize) -> bool {
    if !ordered {
        return true;
    }
    let first = &list.children[0];
    if ordered_item_info(first, src).1.len() > 1 {
        return true;
    }
    let start = |item: &Node| item.children.first().map(|c| column(src, c.start) as i64).unwrap_or(-1);
    let first_start = start(first);
    if first_start == -1 {
        return false;
    }
    let Some(second) = list.children.get(1) else { return first_start % tab_width as i64 == 0 };
    if first_start != start(second) {
        return false;
    }
    if first_start % tab_width as i64 == 0 {
        return true;
    }
    ordered_item_info(second, src).1.len() > 1
}

// ---------------------------------------------------------------- text

static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[!-/:-@\[-`{-~\u{3000}\u{ff5e}]|\p{P})").unwrap());
static CJK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:[[\p{scx=Han}\p{scx=Katakana}\p{scx=Hiragana}\p{scx=Hangul}\p{scx=Bopomofo}]&&[\p{Lo}\p{Nl}\p{So}\p{Lm}\p{Sk}\p{Mn}]]|[\u{2E80}-\u{2FDF}\u{3000}-\u{303F}\u{3190}-\u{33FF}\u{FE30}-\u{FE4F}\u{FF00}-\u{FFEF}])[\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]?",
    )
    .unwrap()
});
static HANGUL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{scx=Hangul}").unwrap());

fn is_punctuation(text: &str) -> bool {
    PUNCTUATION.is_match(text)
}

/// Prettier's `splitText`.
fn split_text(text: &str) -> Vec<Tok> {
    let mut nodes: Vec<Tok> = Vec::new();
    let mut tokens: Vec<(bool, &str)> = Vec::new();
    let mut last = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if matches!(bytes[i], b' ' | b'\t' | b'\n') {
            let s = i;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n') {
                i += 1;
            }
            tokens.push((false, &text[last..s]));
            tokens.push((true, &text[s..i]));
            last = i;
        } else {
            i += 1;
        }
    }
    tokens.push((false, &text[last..]));
    let count = tokens.len();
    for (index, (space, token)) in tokens.into_iter().enumerate() {
        if space {
            nodes.push(Tok::Space(if token.contains('\n') { "\n" } else { " " }));
            continue;
        }
        if (index == 0 || index == count - 1) && token.is_empty() {
            continue;
        }
        let mut parts: Vec<(bool, &str)> = Vec::new();
        let mut at = 0;
        for m in CJK.find_iter(token) {
            parts.push((false, &token[at..m.start()]));
            parts.push((true, m.as_str()));
            at = m.end();
        }
        parts.push((false, &token[at..]));
        let n = parts.len();
        for (inner, (cjk, part)) in parts.into_iter().enumerate() {
            if (inner == 0 || inner == n - 1) && part.is_empty() {
                continue;
            }
            if !cjk {
                if !part.is_empty() {
                    let last_char = part.chars().last().map(String::from).unwrap_or_default();
                    append(
                        &mut nodes,
                        Tok::Word {
                            value: part.to_string(),
                            kind: WordKind::NonCjk,
                            leading_punctuation: is_punctuation(part),
                            trailing_punctuation: is_punctuation(&last_char),
                        },
                    );
                }
                continue;
            }
            let (kind, punctuation) = if is_punctuation(part) {
                (WordKind::CjkPunctuation, true)
            } else if HANGUL.is_match(part) {
                (WordKind::KLetter, false)
            } else {
                (WordKind::CjLetter, false)
            };
            append(
                &mut nodes,
                Tok::Word { value: part.to_string(), kind, leading_punctuation: punctuation, trailing_punctuation: punctuation },
            );
        }
    }
    nodes
}

fn append(nodes: &mut Vec<Tok>, node: Tok) {
    if let Some(Tok::Word { value, kind, .. }) = nodes.last() {
        let new_kind = node.kind().unwrap();
        let between = (*kind == WordKind::NonCjk && new_kind == WordKind::CjkPunctuation)
            || (*kind == WordKind::CjkPunctuation && new_kind == WordKind::NonCjk);
        if !between && !value.contains('\u{3000}') && !node.value().contains('\u{3000}') {
            nodes.push(Tok::Space(""));
        }
    }
    nodes.push(node);
}

fn html_trim_start(s: &str) -> &str {
    s.trim_start_matches(['\t', '\n', '\x0C', '\r', ' '])
}

fn html_trim_end(s: &str) -> &str {
    s.trim_end_matches(['\t', '\n', '\x0C', '\r', ' '])
}

/// Prettier's `getBlockquoteRawText`: the `>` of the lines after the first
/// taken off, as the parser's value has them.
fn blockquote_raw_text(raw: &str, value: Option<&str>) -> String {
    static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[ \t]*>[ \t]*)*").unwrap());
    let values: Vec<&str> = value.map(|v| v.split('\n').collect()).unwrap_or_default();
    raw.split('\n')
        .enumerate()
        .map(|(i, line)| {
            let leading = values.get(i).and_then(|v| BRACKETS.find(v)).map_or("", |m| m.as_str());
            let cut = BRACKETS.find(line).map_or(0, |m| m.end());
            format!("{leading}{}", &line[cut..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prettier's `splitTextIntoSentences`, with the wiki-link rule: a
/// paragraph whose text opens `[[` before a wiki link or a `]]` keeps its
/// text as written.
fn sentences(node: &mut Node, src: &str, paragraph: bool, risky: bool, quoted: bool) {
    let (paragraph, risky) = if node.is("paragraph") { (true, is_risky(node, src, &mut false)) } else { (paragraph, risky) };
    let is_paragraph = node.is("paragraph");
    let count = node.children.len();
    for (index, child) in node.children.iter_mut().enumerate() {
        if let Kind::Text(value) = &child.kind {
            let mut text = src[child.start..child.end].to_string();
            if paragraph {
                if quoted {
                    text = blockquote_raw_text(&text, value.as_deref());
                }
                if is_paragraph {
                    if index == 0 {
                        text = html_trim_start(&text).to_string();
                    }
                    if index == count - 1 {
                        text = html_trim_end(&text).to_string();
                    }
                }
            }
            child.kind = if paragraph && risky { Kind::RawText(text) } else { Kind::Sentence(split_text(&text)) };
            continue;
        }
        let quoted = quoted || (!paragraph && child.is("blockquote"));
        sentences(child, src, paragraph, risky, quoted);
    }
}

fn is_risky(node: &Node, src: &str, open: &mut bool) -> bool {
    for child in &node.children {
        match child.kind {
            Kind::WikiLink(_) if *open => return true,
            Kind::Text(_) => {
                let raw = &src[child.start..child.end];
                if raw.contains("[[") {
                    *open = true;
                }
                if *open && raw.contains("]]") {
                    return true;
                }
            }
            _ => {
                if is_risky(child, src, open) {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------- the doc

/// Prettier's document, as much of it as Markdown with `proseWrap` never
/// uses.
#[derive(Debug, Clone)]
enum Doc {
    Text(String),
    Concat(Vec<Doc>),
    /// A line break: the line's trailing spaces go, the indent comes.
    Hard,
    /// A line break that keeps the trailing spaces: `markAsRoot(literalline)`.
    Literal,
    Align(String, Box<Doc>),
    /// The aligned table if its first line fits, else the compact one.
    Table {
        aligned: Vec<String>,
        compact: Vec<String>,
        header: usize,
    },
    /// `group([softline, doc])`: on this line if the line fits it.
    SoftGroup(Box<Doc>),
}

fn text(s: impl Into<String>) -> Doc {
    Doc::Text(s.into())
}

fn concat(docs: Vec<Doc>) -> Doc {
    Doc::Concat(docs)
}

fn align(n: usize, doc: Doc) -> Doc {
    if n == 0 { doc } else { Doc::Align(" ".repeat(n), Box::new(doc)) }
}

/// Prettier's `replaceEndOfLine`.
fn lines_of(value: &str, line: Doc) -> Doc {
    let mut parts = Vec::new();
    for (i, l) in value.split('\n').enumerate() {
        if i > 0 {
            parts.push(line.clone());
        }
        parts.push(text(l));
    }
    concat(parts)
}

struct Printer {
    out: String,
    indent: Vec<String>,
    pos: usize,
    width: usize,
}

impl Printer {
    fn print(doc: &Doc, width: usize) -> String {
        let mut p = Printer { out: String::new(), indent: Vec::new(), pos: 0, width };
        p.doc(doc);
        p.out
    }

    fn indent(&self) -> String {
        self.indent.concat()
    }

    fn newline(&mut self, trim: bool) {
        if trim {
            let kept = self.out.trim_end_matches([' ', '\t']).len();
            self.out.truncate(kept);
        }
        let indent = self.indent();
        self.out.push('\n');
        self.out.push_str(&indent);
        self.pos = indent.len();
    }

    fn doc(&mut self, doc: &Doc) {
        match doc {
            Doc::Text(s) => {
                self.out.push_str(s);
                match s.rfind('\n') {
                    Some(i) => self.pos = string_width(&s[i + 1..]),
                    None => self.pos += string_width(s),
                }
            }
            Doc::Concat(docs) => docs.iter().for_each(|d| self.doc(d)),
            Doc::Hard => self.newline(true),
            Doc::Literal => self.newline(false),
            Doc::Align(s, d) => {
                self.indent.push(s.clone());
                self.doc(d);
                self.indent.pop();
            }
            Doc::Table { aligned, compact, header } => {
                let rows =
                    if *header <= self.width.saturating_sub(self.pos) && *header + self.pos <= self.width { aligned } else { compact };
                for (i, row) in rows.iter().enumerate() {
                    if i > 0 {
                        self.newline(true);
                    }
                    self.doc(&text(row.clone()));
                }
            }
            Doc::SoftGroup(d) => {
                if flat_width(d).is_none_or(|w| self.pos + w > self.width) {
                    self.newline(true);
                }
                self.doc(d);
            }
        }
    }
}

/// The width of a doc up to its first line break; `None` when it cannot be
/// measured flat.
fn flat_width(doc: &Doc) -> Option<usize> {
    fn walk(doc: &Doc, width: &mut usize) -> bool {
        match doc {
            Doc::Text(s) => {
                if let Some(i) = s.find('\n') {
                    *width += string_width(&s[..i]);
                    return false;
                }
                *width += string_width(s);
                true
            }
            Doc::Concat(docs) => docs.iter().all(|d| walk(d, width)),
            Doc::Hard | Doc::Literal => false,
            Doc::Align(_, d) | Doc::SoftGroup(d) => walk(d, width),
            Doc::Table { aligned, .. } => {
                *width += aligned.first().map_or(0, |r| string_width(r));
                false
            }
        }
    }
    let mut width = 0;
    walk(doc, &mut width);
    Some(width)
}

// ---------------------------------------------------------------- printing

struct Fmt<'a> {
    src: &'a str,
    config: &'a FormatConfig,
}

/// What a node's ancestors say about how to print it.
#[derive(Clone, Copy, Default)]
struct Cx {
    /// In a shortcut or collapsed link reference: printed as written.
    same_content: bool,
    in_emphasis: bool,
    in_emphasis_or_strong: bool,
    in_table_cell: bool,
    /// The nth list in a row, of the nearest list around.
    list_nth: Option<usize>,
    /// The parent is a strong next to a word.
    strong_by_word: bool,
    /// The parent is an emphasis or a strong.
    parent_emphasis_or_strong: bool,
    first_in_parent: bool,
    in_blockquote: bool,
}

impl Fmt<'_> {
    fn never(&self) -> bool {
        self.config.prose_wrap == ProseWrap::Never
    }

    fn root(&self, root: &Node) -> Doc {
        let mut root = root.clone();
        align_lists(&mut root, true, self.src, self.config.tab_width);
        sentences(&mut root, self.src, false, false, false);
        // printRoot's ignore ranges: `<!-- prettier-ignore-start -->` to `-end`.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut open: Option<usize> = None;
        for (i, child) in root.children.iter().enumerate() {
            match prettier_ignore(child) {
                Some("start") if open.is_none() => open = Some(i),
                Some("end") => {
                    if let Some(s) = open.take() {
                        ranges.push((s, i));
                    }
                }
                _ => {}
            }
        }
        let cx = Cx::default();
        self.children(&root, &cx, false, |i, child| {
            if let Some(&(s, e)) = ranges.iter().find(|&&(s, e)| s <= i && i <= e) {
                if i == s {
                    let start = &root.children[s];
                    let end = &root.children[e];
                    let value = |n: &Node| if let Kind::Html(v) = &n.kind { v.clone() } else { String::new() };
                    return Some(concat(vec![text(value(start)), text(&self.src[start.end..end.start]), text(value(end))]));
                }
                return None;
            }
            Some(self.node(child, &root, i, &cx))
        })
    }

    /// Prettier's `printChildren`: each child, with the line breaks between
    /// blocks.
    fn children(&self, parent: &Node, cx: &Cx, parent_loose: bool, mut each: impl FnMut(usize, &Node) -> Option<Doc>) -> Doc {
        let _ = cx;
        let mut parts = Vec::new();
        let mut previous: Option<&Node> = None;
        for (i, child) in parent.children.iter().enumerate() {
            let Some(doc) = each(i, child) else { continue };
            if !parts.is_empty() && pre_hardline(child, parent) {
                parts.push(Doc::Hard);
                if let Some(prev) = previous.or_else(|| i.checked_sub(1).map(|p| &parent.children[p]))
                    && self.double_hardline(child, prev, parent, parent_loose)
                {
                    parts.push(Doc::Hard);
                }
            }
            parts.push(doc);
            previous = Some(child);
        }
        concat(parts)
    }

    fn double_hardline(&self, node: &Node, previous: &Node, parent: &Node, parent_loose: bool) -> bool {
        if node.is_setext_heading() && node.start_line < previous.end_line {
            return false;
        }
        if loose_item(previous, Some(node))
            || (node.is("list")
                && parent.is("listItem")
                && (previous.is("code") || previous.is("paragraph"))
                && previous.end_line + 1 < node.start_line)
        {
            return true;
        }
        let sequence = previous.name() == node.name();
        let sibling = sequence && (node.is("listItem") || node.is("definition"));
        let tight_item = parent.is("listItem") && (node.is("list") || !parent_loose);
        let after_ignore = prettier_ignore(previous) == Some("next");
        let html_after_html = node.is("html") && previous.is("html") && previous.end_line + 1 == node.start_line;
        let html_after_paragraph = node.is("html") && previous.is("paragraph") && previous.end_line + 1 == node.start_line;
        let html_after_item =
            node.is("html") && parent.is("listItem") && previous.is("paragraph") && previous.end_line + 1 == node.start_line;
        !(sibling || tight_item || after_ignore || html_after_html || html_after_paragraph || html_after_item)
    }

    fn node(&self, node: &Node, parent: &Node, index: usize, cx: &Cx) -> Doc {
        if index > 0 && prettier_ignore(&parent.children[index - 1]) == Some("next") {
            let mut raw = self.src[node.start..node.end].to_string();
            // printPrettierIgnored: a list in a blockquote drops its last `>`.
            if node.is("list") && cx.in_blockquote {
                static LAST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n>\s*$").unwrap());
                raw = LAST.replace(&raw, "").to_string();
            }
            return text(raw);
        }
        if cx.same_content {
            return self.same_content(node);
        }
        let mut cx = *cx;
        cx.first_in_parent = index == 0;
        match &node.kind {
            Kind::Root => unreachable!(),
            Kind::FrontMatter { start, end, language, explicit, value, raw } => {
                self.front_matter(start, end, language, explicit, value, raw)
            }
            Kind::Paragraph => {
                concat(node.children.iter().enumerate().map(|(i, c)| self.node(c, node, i, &self.child_cx(&cx, node))).collect())
            }
            Kind::Sentence(toks) => self.sentence(toks, &cx),
            Kind::RawText(value) => lines_of(value, Doc::Hard),
            Kind::Emphasis => {
                let style = if node.children.first().is_some_and(|c| self.is_autolink(c)) {
                    self.src[node.start..].chars().next().map(String::from).unwrap_or_default()
                } else if prev_or_next_word(parent, index) || cx.strong_by_word || cx.in_emphasis {
                    "*".to_string()
                } else {
                    "_".to_string()
                };
                let mut inner = self.child_cx(&cx, node);
                inner.in_emphasis = true;
                inner.in_emphasis_or_strong = true;
                inner.parent_emphasis_or_strong = true;
                inner.strong_by_word = false;
                concat(vec![text(&style), self.inline_children(node, &inner), text(&style)])
            }
            Kind::Strong => {
                let mut inner = self.child_cx(&cx, node);
                inner.in_emphasis_or_strong = true;
                inner.parent_emphasis_or_strong = true;
                inner.strong_by_word = prev_or_next_word(parent, index);
                concat(vec![text("**"), self.inline_children(node, &inner), text("**")])
            }
            Kind::Delete => {
                let inner = self.child_cx(&cx, node);
                concat(vec![text("~~"), self.inline_children(node, &inner), text("~~")])
            }
            Kind::InlineCode(value) => {
                let mut code = if self.never() { value.replace('\n', " ") } else { value.clone() };
                if cx.in_table_cell {
                    code = code.replace('|', "\\|");
                }
                let ticks = "`".repeat(min_not_present_run(&code, '`'));
                let padded = code.starts_with('`')
                    || code.ends_with('`')
                    || (code.starts_with([' ', '\n']) && code.ends_with([' ', '\n']) && code.chars().any(|c| c != ' ' && c != '\n'));
                let pad = if padded { " " } else { "" };
                text(format!("{ticks}{pad}{code}{pad}{ticks}"))
            }
            Kind::WikiLink(value) => {
                static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\t\n]+").unwrap());
                let value = if self.never() { SPACES.replace_all(value, " ").to_string() } else { value.clone() };
                // micromark-extension-wiki-link reads `[[a:b]]` as a link to `a`
                // named `b`, and Prettier writes `[[a]]`; this keeps the name.
                let raw = &self.src[node.start + 2..node.end - 2];
                let rest = if raw.len() > value.len() && raw.starts_with(value.as_str()) { &raw[value.len()..] } else { "" };
                let rest = if self.never() { SPACES.replace_all(rest, " ").to_string() } else { rest.to_string() };
                text(format!("[[{value}{rest}]]"))
            }
            Kind::Liquid(value) => lines_of(value, Doc::Hard),
            Kind::Link { url, title } => match self.src.as_bytes()[node.start] {
                b'<' => {
                    let raw = &self.src[node.start + 1..];
                    let url =
                        if url.starts_with("mailto:") && !raw.starts_with("mailto:") { &url["mailto:".len()..] } else { url.as_str() };
                    text(format!("<{url}>"))
                }
                b'[' => {
                    let inner = self.child_cx(&cx, node);
                    concat(vec![
                        text("["),
                        self.inline_children(node, &inner),
                        text(format!("]({}{})", print_url(url, &[" ", ")"]), self.print_title(title.as_deref(), true))),
                    ])
                }
                _ => text(&self.src[node.start..node.end]),
            },
            Kind::Image { url, title, alt } => {
                let alt = bracket_content(self.src, node.start, node.end).filter(|a| !a.is_empty()).unwrap_or(alt.as_str());
                text(format!("![{alt}]({}{})", print_url(url, &[" ", ")"]), self.print_title(title.as_deref(), true)))
            }
            Kind::Blockquote => {
                let mut inner = self.child_cx(&cx, node);
                inner.in_blockquote = true;
                concat(vec![text("> "), Doc::Align("> ".to_string(), Box::new(self.block_children(node, &inner, false)))])
            }
            Kind::Heading(depth) => {
                let inner = self.child_cx(&cx, node);
                if node.is_setext_heading() {
                    let end = node.end;
                    let line_start = self.src[..end].rfind('\n').map_or(0, |i| i + 1);
                    let last = &self.src[line_start..end];
                    let at = last.find('=').map(|i| i as i64).unwrap_or(-1).max(last.find('-').map(|i| i as i64).unwrap_or(-1));
                    let underline = if at < 0 { last } else { &last[at as usize..] };
                    concat(vec![self.inline_children(node, &inner), Doc::Hard, text(underline)])
                } else {
                    concat(vec![text(format!("{} ", "#".repeat(*depth as usize))), self.inline_children(node, &inner)])
                }
            }
            Kind::Code { value, lang, meta, indented } => {
                if *indented {
                    return align(4, concat(vec![text("    "), lines_of(value, Doc::Hard)]));
                }
                let style = "`".repeat(3.max(max_run(value, '`') + 1));
                let head =
                    text(format!("{style}{}{}", lang.as_deref().unwrap_or(""), meta.as_ref().map(|m| format!(" {m}")).unwrap_or_default()));
                let body = match lang.as_deref() {
                    Some(l) if is_markdown_language(l) => match document(value, self.config) {
                        Some(Doc::Concat(mut parts)) => {
                            parts.pop();
                            concat(parts)
                        }
                        _ => lines_of(value, Doc::Hard),
                    },
                    _ => lines_of(value, Doc::Hard),
                };
                concat(vec![head, Doc::Hard, body, Doc::Hard, text(style)])
            }
            Kind::Html(value) => {
                let value = if parent.is("root") && index + 1 == parent.children.len() { value.trim_end() } else { value.as_str() };
                static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^<!--.*-->$").unwrap());
                lines_of(value, if COMMENT.is_match(value) { Doc::Hard } else { Doc::Literal })
            }
            Kind::List { .. } => self.list(node, parent, index, &cx),
            Kind::ListItem { .. } => unreachable!(),
            Kind::ThematicBreak => match cx.list_nth {
                None => text("---"),
                Some(n) => text(if n % 2 == 0 { "***" } else { "---" }),
            },
            Kind::LinkReference { kind, label } => {
                let mut inner = self.child_cx(&cx, node);
                inner.same_content = *kind != ReferenceKind::Full;
                let tail = match kind {
                    ReferenceKind::Full => print_link_reference(label),
                    ReferenceKind::Collapsed => "[]".to_string(),
                    ReferenceKind::Shortcut => String::new(),
                };
                concat(vec![text("["), self.inline_children(node, &inner), text("]"), text(tail)])
            }
            Kind::ImageReference { kind, label, alt } => {
                let alt = bracket_content(self.src, node.start, node.end).filter(|a| !a.is_empty()).unwrap_or(alt.as_str());
                match kind {
                    ReferenceKind::Full => text(format!("![{alt}]{}", print_link_reference(label))),
                    _ => text(format!("!{}{}", print_link_reference(label), if *kind == ReferenceKind::Collapsed { "[]" } else { "" })),
                }
            }
            Kind::Definition { url, title, label } => {
                let url = if url.is_empty() { "<>".to_string() } else { print_url(url, &[" "]) };
                let title = title.as_deref().map(|t| format!(" {}", self.print_title(Some(t), false))).unwrap_or_default();
                text(format!("{}: {url}{title}", print_link_reference(label)))
            }
            Kind::FootnoteReference(label) => text(format!("[^{label}]")),
            Kind::FootnoteDefinition(label) => {
                let inner = self.child_cx(&cx, node);
                let inline = node.children.len() == 1
                    && node.children[0].is("paragraph")
                    && (self.never() || node.children[0].start_line == node.children[0].end_line);
                let head = text(format!("[^{label}]: "));
                if inline {
                    return concat(vec![head, self.block_children(node, &inner, false)]);
                }
                let body = self.children(node, &inner, false, |i, c| {
                    let d = self.node(c, node, i, &inner);
                    Some(if i == 0 { Doc::SoftGroup(Box::new(d)) } else { d })
                });
                concat(vec![head, align(4, body)])
            }
            Kind::Table(aligns) => self.table(node, aligns, &cx),
            Kind::TableRow | Kind::TableCell => unreachable!(),
            Kind::Break => {
                if self.src[node.start..].starts_with(|c: char| c.is_whitespace()) {
                    concat(vec![text("  "), Doc::Literal])
                } else {
                    concat(vec![text("\\"), Doc::Hard])
                }
            }
            Kind::Math { value, meta } => {
                let mut parts = vec![text(format!("$${}", meta.as_ref().map(|m| format!(" {m}")).unwrap_or_default())), Doc::Hard];
                if !value.is_empty() {
                    parts.push(lines_of(value, Doc::Hard));
                    parts.push(Doc::Hard);
                }
                parts.push(text("$$"));
                concat(parts)
            }
            Kind::InlineMath => text(&self.src[node.start..node.end]),
            Kind::Text(_) => text(&self.src[node.start..node.end]),
        }
    }

    fn child_cx(&self, cx: &Cx, _node: &Node) -> Cx {
        let mut inner = *cx;
        inner.parent_emphasis_or_strong = false;
        inner.strong_by_word = false;
        inner
    }

    fn inline_children(&self, node: &Node, cx: &Cx) -> Doc {
        self.children(node, cx, false, |i, c| {
            let mut inner = *cx;
            if !matches!(c.kind, Kind::Emphasis) {
                inner.strong_by_word = false;
            }
            Some(self.node(c, node, i, &inner))
        })
    }

    fn block_children(&self, node: &Node, cx: &Cx, parent_loose: bool) -> Doc {
        self.children(node, cx, parent_loose, |i, c| Some(self.node(c, node, i, cx)))
    }

    /// `shouldRemainTheSameContent`: the text as written, its spaces made one.
    fn same_content(&self, node: &Node) -> Doc {
        let mut out = String::new();
        for tok in split_text(&self.src[node.start..node.end]) {
            match tok {
                Tok::Word { value, .. } => out.push_str(&value),
                Tok::Space(s) => out.push_str(if s.is_empty() { "" } else { " " }),
            }
        }
        text(out)
    }

    fn sentence(&self, toks: &[Tok], cx: &Cx) -> Doc {
        let mut parts = Vec::new();
        for (i, tok) in toks.iter().enumerate() {
            match tok {
                Tok::Word { value, .. } => parts.push(text(self.word(value, toks, i, cx))),
                Tok::Space(value) => {
                    if !self.never() && *value == "\n" {
                        parts.push(Doc::Hard);
                        continue;
                    }
                    let can_be_space = *value == " " || (*value == "\n" && line_break_can_be_space(toks, i));
                    parts.push(text(if can_be_space { " " } else { "" }));
                }
            }
        }
        concat(parts)
    }

    /// Prettier's `printWord`.
    fn word(&self, value: &str, toks: &[Tok], index: usize, cx: &Cx) -> String {
        if !cx.in_emphasis_or_strong {
            return value.to_string();
        }
        let mut text = value.to_string();
        if index == 0 && (text.starts_with('*') || text.starts_with('_')) && cx.first_in_parent && cx.parent_emphasis_or_strong {
            text = format!("\\{text}");
        }
        static RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\\+|^|.)(\*+|_+)($|.)").unwrap());
        let previous_last = index.checked_sub(1).and_then(|i| toks[i].value().chars().last());
        let next_first = toks.get(index + 1).and_then(|t| t.value().chars().next());
        RUN.replace_all(&text, |c: &regex::Captures| {
            let (preceding, run, following) = (&c[1], &c[2], &c[3]);
            if !preceding.is_empty() && preceding.chars().all(|ch| ch == '\\') && preceding.len() % 2 == 1 {
                return c[0].to_string();
            }
            let before = preceding.chars().last().or(previous_last);
            let after = following.chars().next().or(next_first);
            if can_open_or_close(before, run, after) { format!("{preceding}\\{run}{following}") } else { c[0].to_string() }
        })
        .to_string()
    }

    fn list(&self, list: &Node, parent: &Node, index: usize, cx: &Cx) -> Doc {
        let Kind::List { ordered, start, aligned } = list.kind else { unreachable!() };
        let nth = nth_sibling_index(parent, index);
        let git_friendly = ordered && list.children.len() >= 2 && {
            let second = ordered_item_info(&list.children[1], self.src).0;
            second == 1 && {
                let first = ordered_item_info(&list.children[0], self.src).0;
                first != 0 || list.children.get(2).is_some_and(|c| ordered_item_info(c, self.src).0 == 1)
            }
        };
        let min_indent = match parent.children.get(index + 1) {
            Some(next) if next.is_indented_code() => {
                let Kind::Code { value, .. } = &next.kind else { unreachable!() };
                4 + value.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>() + 1
            }
            _ => 0,
        };
        let mut inner = self.child_cx(cx, list);
        inner.list_nth = Some(nth);
        let count = list.children.len();
        self.children(list, &inner, false, |i, item| {
            let raw = if ordered {
                let number = if i == 0 {
                    start as u64
                } else if git_friendly {
                    1
                } else {
                    start as u64 + i as u64
                };
                format!("{number}{}", if nth.is_multiple_of(2) { ". " } else { ") " })
            } else if nth.is_multiple_of(2) {
                "- ".to_string()
            } else {
                "* ".to_string()
            };
            let mut prefix = if aligned && ordered { self.align_list_prefix(&raw) } else { raw };
            if prefix.len() < min_indent {
                prefix = prefix.trim_end().to_string();
                let trailing = (min_indent - prefix.len()).min(4);
                prefix.push_str(&" ".repeat(trailing));
                let leading = min_indent.saturating_sub(prefix.len()).min(3);
                prefix = format!("{}{prefix}", " ".repeat(leading));
            }
            let loose = loose_item(item, list.children.get(i + 1));
            let _ = count;
            let body = self.list_item(item, &prefix, &inner, loose);
            let html_pair = item.children.len() == 2
                && item.children[1].is("html")
                && column(self.src, item.children[0].start) != column(self.src, item.children[1].start);
            if html_pair { Some(concat(vec![text(&prefix), body])) } else { Some(concat(vec![text(&prefix), align(prefix.len(), body)])) }
        })
    }

    fn align_list_prefix(&self, prefix: &str) -> String {
        let rest = prefix.len() % self.config.tab_width;
        let extra = if rest == 0 { 0 } else { self.config.tab_width - rest };
        format!("{prefix}{}", " ".repeat(if extra >= 4 { 0 } else { extra }))
    }

    fn list_item(&self, item: &Node, list_prefix: &str, cx: &Cx, loose: bool) -> Doc {
        let Kind::ListItem { checked, .. } = item.kind else { unreachable!() };
        let checkbox = match checked {
            None => "",
            Some(true) => "[x] ",
            Some(false) => "[ ] ",
        };
        let mut inner = *cx;
        inner.list_nth = cx.list_nth;
        let body = self.children(item, &inner, loose, |i, child| {
            let doc = self.node(child, item, i, &inner);
            if (i == 0 && !child.is("list")) || child.is_indented_code() {
                return Some(align(checkbox.len(), doc));
            }
            let alignment = self.config.tab_width.saturating_sub(list_prefix.len()).min(3);
            Some(concat(vec![text(" ".repeat(alignment)), align(alignment, doc)]))
        });
        concat(vec![text(checkbox), body])
    }

    fn table(&self, table: &Node, aligns: &[AlignKind], cx: &Cx) -> Doc {
        let mut inner = self.child_cx(cx, table);
        inner.in_table_cell = true;
        let mut widths: Vec<usize> = Vec::new();
        let contents: Vec<Vec<(String, usize)>> = table
            .children
            .iter()
            .map(|row| {
                row.children
                    .iter()
                    .enumerate()
                    .map(|(column, cell)| {
                        let doc = self.inline_children(cell, &inner);
                        let text = Printer::print(&doc, self.config.print_width);
                        let width = string_width(&text);
                        if widths.len() <= column {
                            widths.resize(column + 1, 3);
                        }
                        widths[column] = widths[column].max(width);
                        (text, width)
                    })
                    .collect()
            })
            .collect();
        let header_cells = contents.first().map_or(0, Vec::len);
        let align_row = |compact: bool| -> Vec<String> {
            widths
                .iter()
                .enumerate()
                .filter(|(i, _)| *i < header_cells)
                .map(|(i, &w)| {
                    let a = aligns.get(i).copied().unwrap_or(AlignKind::None);
                    let first = if matches!(a, AlignKind::Center | AlignKind::Left) { ":" } else { "-" };
                    let last = if matches!(a, AlignKind::Center | AlignKind::Right) { ":" } else { "-" };
                    let middle = if compact { "-".to_string() } else { "-".repeat(w - 2) };
                    format!("{first}{middle}{last}")
                })
                .collect()
        };
        let row = |cells: &Vec<(String, usize)>, compact: bool| -> Vec<String> {
            cells
                .iter()
                .enumerate()
                .map(|(i, (text, width))| {
                    if compact {
                        return text.clone();
                    }
                    let spaces = widths[i] - width;
                    let before = match aligns.get(i) {
                        Some(AlignKind::Right) => spaces,
                        Some(AlignKind::Center) => spaces / 2,
                        _ => 0,
                    };
                    format!("{}{text}{}", " ".repeat(before), " ".repeat(spaces - before))
                })
                .collect()
        };
        let lines = |compact: bool| -> Vec<String> {
            let mut rows = Vec::new();
            if let Some(first) = contents.first() {
                rows.push(row(first, compact));
            }
            rows.push(align_row(compact));
            for r in contents.iter().skip(1) {
                rows.push(row(r, compact));
            }
            rows.into_iter().map(|cells| format!("| {} |", cells.join(" | "))).collect()
        };
        let aligned = lines(false);
        if !self.never() {
            return Doc::Table { header: 0, compact: aligned.clone(), aligned };
        }
        let header = aligned.first().map_or(0, |r| string_width(r));
        Doc::Table { aligned, compact: lines(true), header }
    }

    fn front_matter(&self, start: &str, end: &str, language: &str, explicit: &str, value: &str, raw: &str) -> Doc {
        if language != "yaml" {
            return text(raw);
        }
        let value = value.trim();
        let formatted = if value.is_empty() {
            Some(String::new())
        } else {
            yaml_format::format(value, self.config.tab_width, self.config.single_quote, self.config.print_width)
        };
        let Some(formatted) = formatted else { return text(raw) };
        let mut parts = vec![text(format!("{start}{explicit}")), Doc::Hard];
        if !formatted.is_empty() {
            parts.push(lines_of(&formatted, Doc::Hard));
            parts.push(Doc::Hard);
        }
        parts.push(text(end));
        concat(parts)
    }

    fn print_title(&self, title: Option<&str>, space: bool) -> String {
        let Some(title) = title.filter(|t| !t.is_empty()) else { return String::new() };
        if space {
            return format!(" {}", self.print_title(Some(title), false));
        }
        if title.contains('"') && title.contains('\'') && !title.contains(')') {
            return format!("({title})");
        }
        let (preferred, alternate) = if self.config.single_quote { ('\'', '"') } else { ('"', '\'') };
        let p = title.chars().filter(|&c| c == preferred).count();
        let a = title.chars().filter(|&c| c == alternate).count();
        let quote = if p > a { alternate } else { preferred };
        let title = title.replace('\\', "\\\\").replace(quote, &format!("\\{quote}"));
        format!("{quote}{title}{quote}")
    }

    /// Prettier's `isAutolink`: a link whose one child is all of it.
    fn is_autolink(&self, node: &Node) -> bool {
        node.is("link") && node.children.len() == 1 && node.children[0].start == node.start && node.children[0].end == node.end
    }
}

fn pre_hardline(node: &Node, parent: &Node) -> bool {
    let inline_html = node.is("html") && (parent.is_inline() || matches!(parent.name(), "tableCell" | "paragraph" | "heading"));
    !node.is_inline() && !inline_html
}

/// Prettier's `isLooseListItem`.
fn loose_item(node: &Node, next: Option<&Node>) -> bool {
    let Kind::ListItem { spread, .. } = node.kind else { return false };
    spread || next.is_some_and(|n| n.is("listItem") && node.end_line + 1 < n.start_line)
}

fn prettier_ignore(node: &Node) -> Option<&'static str> {
    static IGNORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<!--\s*prettier-ignore(?:-(start|end))?\s*-->$").unwrap());
    let Kind::Html(value) = &node.kind else { return None };
    let c = IGNORE.captures(value)?;
    Some(match c.get(1).map(|m| m.as_str()) {
        Some("start") => "start",
        Some("end") => "end",
        _ => "next",
    })
}

fn nth_sibling_index(parent: &Node, index: usize) -> usize {
    let Kind::List { ordered, .. } = parent.children[index].kind else { return 0 };
    let mut nth: i64 = -1;
    for child in &parent.children[..=index] {
        match child.kind {
            Kind::List { ordered: o, .. } if o == ordered => nth += 1,
            _ => nth = -1,
        }
    }
    nth.max(0) as usize
}

/// Prettier's `prevOrNextWord`, for an emphasis or a strong among its
/// siblings.
fn prev_or_next_word(parent: &Node, index: usize) -> bool {
    let space = |c: char| c.is_whitespace() && (c == '\t' || c == '\n' || c == '\x0C' || c == '\r' || is_space_separator(c));
    let previous = index.checked_sub(1).map(|i| &parent.children[i]);
    let next = parent.children.get(index + 1);
    let before = matches!(previous.map(|p| &p.kind), Some(Kind::Sentence(toks)) if matches!(toks.last(), Some(Tok::Word { value, trailing_punctuation: false, .. }) if !value.ends_with(space)));
    let after = matches!(next.map(|n| &n.kind), Some(Kind::Sentence(toks)) if matches!(toks.first(), Some(Tok::Word { value, leading_punctuation: false, .. }) if !value.starts_with(space)));
    before || after
}

fn is_space_separator(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// Prettier's `canOpenOrCloseStrongOrEmphasis`.
fn can_open_or_close(preceding: Option<char>, run: &str, following: Option<char>) -> bool {
    let (Some(preceding), Some(following)) = (preceding, following) else { return false };
    let ws = |c: char| matches!(c, '\t' | '\n' | '\x0C' | '\r') || is_space_separator(c);
    let punct = |c: char| is_punctuation(c.encode_utf8(&mut [0; 4]));
    let (followed_ws, preceded_ws) = (ws(following), ws(preceding));
    let (followed_p, preceded_p) = (punct(following), punct(preceding));
    let left = !followed_ws && (!followed_p || preceded_ws || preceded_p);
    let right = !preceded_ws && (!preceded_p || followed_ws || followed_p);
    if run.starts_with('*') {
        return left || right;
    }
    if left {
        return !right || preceded_p;
    }
    if right {
        return !left || followed_p;
    }
    false
}

/// Prettier's `lineBreakCanBeConvertedToSpace`, for a `"\n"` in a sentence.
fn line_break_can_be_space(toks: &[Tok], index: usize) -> bool {
    let (Some(previous), Some(next)) = (index.checked_sub(1).map(|i| &toks[i]), toks.get(index + 1)) else { return true };
    let (Some(pk), Some(nk)) = (previous.kind(), next.kind()) else { return true };
    let latin = |k: WordKind| k == WordKind::NonCjk || k == WordKind::KLetter;
    if (latin(pk) && latin(nk))
        || (pk == WordKind::KLetter && nk == WordKind::CjLetter)
        || (nk == WordKind::KLetter && pk == WordKind::CjLetter)
    {
        return true;
    }
    if pk == WordKind::CjkPunctuation || nk == WordKind::CjkPunctuation || (pk == WordKind::CjLetter && nk == WordKind::CjLetter) {
        return false;
    }
    const ASCII: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
    if next.value().chars().next().is_some_and(|c| ASCII.contains(c)) || previous.value().chars().last().is_some_and(|c| ASCII.contains(c))
    {
        return true;
    }
    let (Tok::Word { trailing_punctuation, .. }, Tok::Word { leading_punctuation, .. }) = (previous, next) else { return true };
    if *trailing_punctuation || *leading_punctuation {
        return false;
    }
    // isInSentenceWithCJSpaces
    let (mut spaced, mut tight) = (0, 0);
    for i in 1..toks.len().saturating_sub(1) {
        if let Tok::Space(v @ (" " | "")) = toks[i] {
            let (a, b) = (toks[i - 1].kind(), toks[i + 1].kind());
            if (a == Some(WordKind::CjLetter) && b == Some(WordKind::NonCjk))
                || (a == Some(WordKind::NonCjk) && b == Some(WordKind::CjLetter))
            {
                if v == " " { spaced += 1 } else { tight += 1 }
            }
        }
    }
    spaced > tight
}

fn max_run(text: &str, c: char) -> usize {
    let (mut max, mut run) = (0, 0);
    for ch in text.chars() {
        if ch == c {
            run += 1;
            max = max.max(run);
        } else {
            run = 0;
        }
    }
    max
}

/// Prettier's `getMinNotPresentContinuousCount`.
fn min_not_present_run(text: &str, c: char) -> usize {
    let mut present = std::collections::HashSet::new();
    let mut run = 0;
    for ch in text.chars().chain(std::iter::once('\0')) {
        if ch == c {
            run += 1;
        } else {
            if run > 0 {
                present.insert(run);
            }
            run = 0;
        }
    }
    let max = present.iter().copied().max().unwrap_or(0);
    (1..max).find(|n| !present.contains(n)).unwrap_or(max + 1)
}

fn print_url(url: &str, dangerous: &[&str]) -> String {
    if dangerous.iter().any(|d| url.contains(d)) { format!("<{}>", url.replace('<', "%3C").replace('>', "%3E")) } else { url.to_string() }
}

fn print_link_reference(label: &str) -> String {
    static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\t\n\x0B\x0C\r ]+").unwrap());
    let label = SPACES.replace_all(label, " ");
    let mut out = String::from("[");
    for c in label.chars() {
        if matches!(c, '\\' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(']');
    out
}

/// Prettier's `getBracketContent`: an image's alt text as written.
fn bracket_content(src: &str, start: usize, end: usize) -> Option<&str> {
    let first = start + src[start..end].find('[')?;
    let bytes = src.as_bytes();
    let mut depth = 1;
    let mut i = first + 1;
    while i < end {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[first + 1..i]);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A fenced block Prettier formats as Markdown: its `inferParser`.
fn is_markdown_language(lang: &str) -> bool {
    matches!(lang, "markdown" | "pandoc" | "md" | "livemd" | "mdown" | "mdwn" | "mkd" | "mkdn" | "mkdown" | "ronn" | "scd" | "workbook")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> FormatConfig {
        FormatConfig { print_width: 100, tab_width: 4, prose_wrap: ProseWrap::Never, single_quote: false }
    }

    fn format(text: &str) -> String {
        MarkdownFormat::format(text, &vault())
    }

    #[test]
    fn paragraphs_become_one_line() {
        assert_eq!(
            format("Text with *emph* and __strong__ and a  double space\ttab.\nA second line joined.\n"),
            "Text with _emph_ and **strong** and a double space tab. A second line joined.\n"
        );
    }

    #[test]
    fn lists_take_dashes_and_four_spaces() {
        assert_eq!(
            format("* [X] done\n* [ ] todo\n  * nested\n\n1) one\n2) two\n"),
            "- [x] done\n- [ ] todo\n    - nested\n\n1. one\n2. two\n"
        );
    }

    #[test]
    fn tables_line_up() {
        assert_eq!(format("| a | bb |\n|:-|-:|\n| ccc | d |\n"), "| a   |  bb |\n| :-- | --: |\n| ccc |   d |\n");
    }

    #[test]
    fn wiki_links_are_not_markdown() {
        assert_eq!(format("See [[a_b_c]] and [[x*y*z]] and [[A:b]].\n"), "See [[a_b_c]] and [[x*y*z]] and [[A:b]].\n");
        assert_eq!(format("see [[a]]  x\ny\n"), "see [[a]] x y\n");
    }

    #[test]
    fn an_embed_keeps_its_paragraph_as_written() {
        assert_eq!(format("![[a b.png]]  x\ny\n"), "![[a b.png]]  x\ny\n");
    }

    #[test]
    fn config_reads_a_prettierrc() {
        let config = FormatConfig::parse(r#"{"printWidth": 100, "proseWrap": "never", "tabWidth": 4, "semi": false}"#).unwrap();
        assert_eq!(config, vault());
        assert!(FormatConfig::parse(r#"{"proseWrap": "always"}"#).is_err());
    }
}
