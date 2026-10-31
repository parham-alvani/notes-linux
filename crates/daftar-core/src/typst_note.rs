//! A note as Typst markup, for `daftar-cli export`, which sets it in a PDF.
//! Obsidian's syntax is read as the reader reads it: a wiki link is its
//! text, a picture embed is the picture, a callout is a box, `==x==` is
//! marked and `%%x%%` is left out. Text goes in as Typst strings, so no
//! character of a note can be taken for Typst's own syntax. A paragraph whose
//! first strong letter is Persian or Arabic is set right to left.
//!
//! The pictures are named `/pictures/<n>.<ext>` in the markup, in the order
//! of [`TypstNote::pictures`], which holds their vault paths.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::LazyLock;

use markdown::mdast::{AlignKind, Node};
use markdown::{Constructs, ParseOptions};
use regex::Regex;

/// The kinds of picture Typst draws.
const PICTURES: [&str; 7] = ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp"];

/// The Tasks plugin's signs, as words: no font of the PDF has emoji.
const TASK_SIGNS: [(&str, &str); 14] = [
    ("📅", "due"),
    ("⏳", "scheduled"),
    ("🛫", "starts"),
    ("➕", "created"),
    ("✅", "done"),
    ("❌", "cancelled"),
    ("🔁", "every"),
    ("🆔", "id"),
    ("⛔", "after"),
    ("🔺", "highest"),
    ("⏫", "high"),
    ("🔼", "medium"),
    ("🔽", "low"),
    ("⏬", "lowest"),
];

static WIKI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\]\n]+?)\]\]").unwrap());
static HIGHLIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==([^=\n]+)==").unwrap());
static EMOJI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{Extended_Pictographic}\u{FE0F}\u{200D}]").unwrap());
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[!([A-Za-z-]+)\][+-]?[ \t]*").unwrap());
static STATUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(.)\] ").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypstNote {
    /// The note's body as Typst markup.
    pub markup: String,
    /// The vault paths of the pictures the markup draws, by number.
    pub pictures: Vec<String>,
}

impl TypstNote {
    /// [text], the note at [path], as Typst. [resolve] finds the vault path
    /// a link names from this note, as `VaultIndex::resolve` does.
    pub fn new(text: &str, path: &str, resolve: impl Fn(&str, &str) -> Option<String>) -> TypstNote {
        let text = without_comments(text);
        let mut constructs = Constructs::gfm();
        constructs.frontmatter = true;
        constructs.math_flow = true;
        constructs.math_text = true;
        let options =
            ParseOptions { constructs, gfm_strikethrough_single_tilde: true, math_text_single_dollar: true, ..ParseOptions::default() };
        let mut writer = Writer { path, resolve: &resolve, pictures: Vec::new(), footnotes: HashMap::new() };
        let mut markup = String::new();
        // markdown-rs has no syntax that fails: a note is always a tree.
        if let Ok(root) = markdown::to_mdast(&text, &options) {
            writer.collect_footnotes(&root);
            writer.blocks(root.children().map_or(&[][..], Vec::as_slice), &mut markup);
        }
        TypstNote { markup, pictures: writer.pictures }
    }

    /// The folder nearest [path] that holds a `logo.svg` or `logo.png`
    /// among [files], with that logo's path: a company's folder in a vault
    /// of companies. The vault's own top is never one.
    pub fn folder_logo<'a>(path: &str, files: impl IntoIterator<Item = &'a String> + Clone) -> Option<(String, String)> {
        let mut folder = path.rsplit_once('/').map(|(folder, _)| folder);
        while let Some(at) = folder {
            for name in ["logo.svg", "logo.png"] {
                let logo = format!("{at}/{name}");
                if files.clone().into_iter().any(|file| *file == logo) {
                    return Some((at.to_string(), logo));
                }
            }
            folder = at.rsplit_once('/').map(|(up, _)| up);
        }
        None
    }

    /// [text] as a Typst string literal.
    pub fn string(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 2);
        out.push('"');
        for c in text.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => {}
                '\t' => out.push_str("\\t"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

struct Writer<'a, F: Fn(&str, &str) -> Option<String>> {
    path: &'a str,
    resolve: &'a F,
    pictures: Vec<String>,
    footnotes: HashMap<String, Vec<Node>>,
}

impl<F: Fn(&str, &str) -> Option<String>> Writer<'_, F> {
    fn collect_footnotes(&mut self, node: &Node) {
        if let Node::FootnoteDefinition(definition) = node {
            self.footnotes.insert(definition.identifier.clone(), definition.children.clone());
        }
        for child in node.children().map_or(&[][..], Vec::as_slice) {
            self.collect_footnotes(child);
        }
    }

    fn blocks(&mut self, nodes: &[Node], out: &mut String) {
        for node in nodes {
            self.block(node, out);
        }
    }

    fn block(&mut self, node: &Node, out: &mut String) {
        match node {
            Node::Paragraph(paragraph) => {
                let mut inner = String::new();
                self.inlines(&paragraph.children, &mut inner);
                if !inner.trim().is_empty() {
                    directed(&plain(&paragraph.children), &inner, out);
                    out.push_str("\n\n");
                }
            }
            Node::Heading(heading) => {
                let mut inner = String::new();
                self.inlines(&heading.children, &mut inner);
                let body = format!("#heading(level: {})[{inner}]", heading.depth);
                directed(&plain(&heading.children), &body, out);
                out.push_str("\n\n");
            }
            Node::Code(code) => {
                let lang = code.lang.as_deref().map_or_else(|| "none".to_string(), TypstNote::string);
                let _ = writeln!(out, "#raw(block: true, lang: {lang}, {})\n", TypstNote::string(&code.value));
            }
            Node::Math(math) => {
                let _ = writeln!(out, "#raw(block: true, lang: \"latex\", {})\n", TypstNote::string(&math.value));
            }
            Node::ThematicBreak(_) => out.push_str("#line(length: 100%, stroke: 0.5pt + rule)\n\n"),
            Node::Blockquote(quote) => self.quote(&quote.children, out),
            Node::List(list) => self.list(list, out),
            Node::Table(table) => self.table(table, out),
            Node::Html(html) => {
                if html.value.trim().eq_ignore_ascii_case("<br>") {
                    out.push_str("#v(0.6em)\n\n");
                }
            }
            // Front matter, link definitions and footnotes (each set where it is named).
            Node::Yaml(_) | Node::Toml(_) | Node::Definition(_) | Node::FootnoteDefinition(_) => {}
            other => {
                if let Some(children) = other.children() {
                    self.blocks(children, out);
                }
            }
        }
    }

    /// A quote, or a callout when it opens with `[!kind]`.
    fn quote(&mut self, children: &[Node], out: &mut String) {
        let opening = match children.first() {
            Some(Node::Paragraph(paragraph)) => match paragraph.children.first() {
                Some(Node::Text(text)) => CALLOUT.captures(&text.value).map(|c| (c[1].to_lowercase(), c[0].len())),
                _ => None,
            },
            _ => None,
        };
        let Some((kind, cut)) = opening else {
            let mut inner = String::new();
            self.blocks(children, &mut inner);
            let _ = writeln!(out, "#quote(block: true)[{inner}]\n");
            return;
        };
        let Some(Node::Paragraph(first)) = children.first() else { return };
        // The title is the rest of the first line; what follows it is the body.
        let mut rest = first.children.clone();
        if let Some(Node::Text(text)) = rest.first_mut() {
            text.value = text.value[cut..].to_string();
        }
        let mut title_nodes = Vec::new();
        let mut body_nodes = Vec::new();
        let mut in_title = true;
        for node in rest {
            if in_title {
                if let Node::Text(text) = &node
                    && let Some((head, tail)) = text.value.split_once('\n')
                {
                    let mut head_node = text.clone();
                    head_node.value = head.to_string();
                    title_nodes.push(Node::Text(head_node));
                    let mut tail_node = text.clone();
                    tail_node.value = tail.to_string();
                    body_nodes.push(Node::Text(tail_node));
                    in_title = false;
                    continue;
                }
                if matches!(node, Node::Break(_)) {
                    in_title = false;
                    continue;
                }
                title_nodes.push(node);
            } else {
                body_nodes.push(node);
            }
        }
        let mut title = String::new();
        self.inlines(&title_nodes, &mut title);
        if title.trim().is_empty() {
            title = TypstNote::string(&capitalized(&kind));
            title.insert(0, '#');
        }
        let mut body = String::new();
        if !plain(&body_nodes).trim().is_empty() {
            let mut inner = String::new();
            self.inlines(&body_nodes, &mut inner);
            directed(&plain(&body_nodes), &inner, &mut body);
            body.push_str("\n\n");
        }
        self.blocks(&children[1..], &mut body);
        let _ = writeln!(out, "#callout({})[{title}][{body}]\n", TypstNote::string(&kind));
    }

    fn list(&mut self, list: &markdown::mdast::List, out: &mut String) {
        let mut items = Vec::new();
        let mut all_tasks = true;
        for item in &list.children {
            let Node::ListItem(item) = item else { continue };
            let mut children = item.children.clone();
            // `[x]` is GFM's; Obsidian's other statuses (`[/]`, `[-]`, …) are text.
            let status = match item.checked {
                Some(true) => Some('x'),
                Some(false) => Some(' '),
                None => match children.first_mut() {
                    Some(Node::Paragraph(paragraph)) => match paragraph.children.first_mut() {
                        Some(Node::Text(text)) => {
                            let found = STATUS.captures(&text.value).and_then(|c| c[1].chars().next());
                            if let Some(mark) = found {
                                text.value = text.value[3 + mark.len_utf8()..].to_string();
                            }
                            found
                        }
                        _ => None,
                    },
                    _ => None,
                },
            };
            let mut inner = String::new();
            self.blocks(&children, &mut inner);
            let inner = inner.trim_end().to_string();
            all_tasks &= status.is_some();
            items.push(match status {
                Some(mark) => format!("[#task({})[{inner}]]", TypstNote::string(&mark.to_string())),
                None => format!("[{inner}]"),
            });
        }
        let first = list.children.first().map(|item| plain(std::slice::from_ref(item))).unwrap_or_default();
        let body = if list.ordered {
            format!("#enum(start: {}, tight: {}, {})", list.start.unwrap_or(1), !list.spread, items.join(", "))
        } else {
            // A list of tasks has their boxes for marks.
            let marker = if all_tasks { "marker: [], " } else { "" };
            format!("#list({marker}tight: {}, {})", !list.spread, items.join(", "))
        };
        directed(&first, &body, out);
        out.push_str("\n\n");
    }

    fn table(&mut self, table: &markdown::mdast::Table, out: &mut String) {
        let columns = table.align.len().max(1);
        let align: Vec<&str> = table
            .align
            .iter()
            .map(|a| match a {
                AlignKind::Left => "left",
                AlignKind::Right => "right",
                AlignKind::Center => "center",
                AlignKind::None => "start",
            })
            .collect();
        let mut cells = Vec::new();
        let mut all = Vec::new();
        for (index, row) in table.children.iter().enumerate() {
            let Node::TableRow(row) = row else { continue };
            let mut row_cells = Vec::new();
            for cell in row.children.iter().take(columns) {
                let Node::TableCell(cell) = cell else { continue };
                let mut inner = String::new();
                self.inlines(&cell.children, &mut inner);
                all.push(plain(&cell.children));
                row_cells.push(format!("[{inner}]"));
            }
            while row_cells.len() < columns {
                row_cells.push("[]".to_string());
            }
            if index == 0 {
                cells.push(format!("table.header({})", row_cells.join(", ")));
            } else {
                cells.extend(row_cells);
            }
        }
        let body = format!("#table(columns: {columns}, align: ({},), {})", align.join(", "), cells.join(", "));
        directed(&all.join(" "), &body, out);
        out.push_str("\n\n");
    }

    fn inlines(&mut self, nodes: &[Node], out: &mut String) {
        // Text nodes side by side are one run: a wiki link can span them.
        let mut run = String::new();
        for node in nodes {
            if let Node::Text(text) = node {
                run.push_str(&text.value);
                continue;
            }
            self.text(&std::mem::take(&mut run), out);
            self.inline(node, out);
        }
        self.text(&run, out);
    }

    fn inline(&mut self, node: &Node, out: &mut String) {
        match node {
            Node::Emphasis(e) => self.wrapped("emph", &e.children, out),
            Node::Strong(s) => self.wrapped("strong", &s.children, out),
            Node::Delete(d) => self.wrapped("strike", &d.children, out),
            Node::InlineCode(code) => {
                let _ = write!(out, "#raw({})", TypstNote::string(&code.value));
            }
            Node::InlineMath(math) => {
                let _ = write!(out, "#raw({})", TypstNote::string(&math.value));
            }
            Node::Break(_) => out.push_str("#linebreak()"),
            Node::Link(link) => {
                let mut inner = String::new();
                self.inlines(&link.children, &mut inner);
                if is_web(&link.url) {
                    let _ = write!(out, "#link({})[{inner}]", TypstNote::string(&link.url));
                } else {
                    let _ = write!(out, "#internal[{inner}]");
                }
            }
            Node::Image(image) => {
                let target = percent_encoding::percent_decode_str(&image.url).decode_utf8_lossy().to_string();
                if !self.picture(&target, None, out) {
                    self.text(&image.alt, out);
                }
            }
            Node::FootnoteReference(reference) => {
                let children = self.footnotes.get(&reference.identifier).cloned().unwrap_or_default();
                let mut inner = String::new();
                for child in &children {
                    if let Node::Paragraph(paragraph) = child {
                        self.inlines(&paragraph.children, &mut inner);
                    }
                }
                let _ = write!(out, "#footnote[{inner}]");
            }
            Node::Html(html) => {
                if html.value.trim().eq_ignore_ascii_case("<br>") {
                    out.push_str("#linebreak()");
                }
            }
            other => {
                if let Some(children) = other.children() {
                    self.inlines(children, out);
                }
            }
        }
    }

    fn wrapped(&mut self, function: &str, children: &[Node], out: &mut String) {
        let mut inner = String::new();
        self.inlines(children, &mut inner);
        let _ = write!(out, "#{function}[{inner}]");
    }

    /// A run of text: wiki links and embeds, `==marks==`, the rest as strings.
    fn text(&mut self, text: &str, out: &mut String) {
        let mut at = 0;
        for found in WIKI.captures_iter(text) {
            let whole = found.get(0).expect("a match has its whole");
            self.marked(&text[at..whole.start()], out);
            at = whole.end();
            let inside = &found[2];
            let (target, alias) = match inside.split_once('|') {
                Some((target, alias)) => (target.trim(), Some(alias.trim())),
                None => (inside.trim(), None),
            };
            if !found[1].is_empty() {
                // `![[pic.png|300]]`: the number is a width in pixels.
                let width = alias.and_then(|a| a.split('x').next()).and_then(|w| w.trim().parse::<u32>().ok());
                if !self.picture(target, width, out) {
                    let name = target.split('#').next().unwrap_or(target);
                    let _ = write!(out, "#embedded[#{}]", TypstNote::string(&crate::Vault::title(name)));
                }
                continue;
            }
            let shown = alias.map_or_else(
                || {
                    let (note, heading) = target.split_once('#').unwrap_or((target, ""));
                    let title = crate::Vault::title(note);
                    let heading = heading.trim_start_matches('^').replace('#', " › ");
                    match (title.is_empty(), heading.is_empty()) {
                        (true, _) => heading,
                        (false, true) => title,
                        (false, false) => format!("{title} › {heading}"),
                    }
                },
                str::to_string,
            );
            let _ = write!(out, "#internal[#{}]", TypstNote::string(&shown));
        }
        self.marked(&text[at..], out);
    }

    fn marked(&mut self, text: &str, out: &mut String) {
        let mut at = 0;
        for found in HIGHLIGHT.captures_iter(text) {
            let whole = found.get(0).expect("a match has its whole");
            plain_text(&text[at..whole.start()], out);
            let _ = write!(out, "#highlight[");
            plain_text(&found[1], out);
            out.push(']');
            at = whole.end();
        }
        plain_text(&text[at..], out);
    }

    /// Draws the picture [target] names, if it is one in the vault.
    fn picture(&mut self, target: &str, width: Option<u32>, out: &mut String) -> bool {
        let target = target.split('#').next().unwrap_or(target);
        let Some(extension) = target.rsplit_once('.').map(|(_, e)| e.to_lowercase()) else { return false };
        if !PICTURES.contains(&extension.as_str()) || is_web(target) {
            return false;
        }
        let Some(path) = (self.resolve)(target, self.path) else { return false };
        let number = self.pictures.iter().position(|p| *p == path).unwrap_or_else(|| {
            self.pictures.push(path.clone());
            self.pictures.len() - 1
        });
        let width = width.map_or_else(|| "auto".to_string(), |w| format!("{}pt", f64::from(w) * 0.75));
        let _ = write!(out, "#picture(\"/pictures/{number}.{extension}\", width: {width})");
        true
    }
}

/// [text] with Typst's strings, the Tasks signs as words and other emoji out.
fn plain_text(text: &str, out: &mut String) {
    if text.is_empty() {
        return;
    }
    let mut text = text.replace('\n', " ");
    for (sign, word) in TASK_SIGNS {
        text = text.replace(sign, word);
    }
    let text = EMOJI.replace_all(&text, "");
    out.push('#');
    out.push_str(&TypstNote::string(&text));
}

/// [body], set right to left when [text]'s first strong letter is.
fn directed(text: &str, body: &str, out: &mut String) {
    if right_to_left(text) {
        let _ = write!(out, "#right-to-left[{body}]");
    } else {
        out.push_str(body);
    }
}

/// Is the first strong letter of [text] a right-to-left one?
pub fn right_to_left(text: &str) -> bool {
    for c in text.chars() {
        if matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}') {
            return true;
        }
        if c.is_alphabetic() {
            return false;
        }
    }
    false
}

/// The words of [nodes], for their direction.
fn plain(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(&text.value),
            Node::InlineCode(code) => out.push_str(&code.value),
            other => {
                if let Some(children) = other.children() {
                    out.push_str(&plain(children));
                }
            }
        }
        out.push(' ');
    }
    out
}

fn is_web(url: &str) -> bool {
    let lower = url.to_lowercase();
    ["http://", "https://", "mailto:", "tel:"].iter().any(|scheme| lower.starts_with(scheme))
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// [text] without Obsidian's `%%comments%%`, but in code.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    let mut in_comment = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !in_comment {
            if let Some(open) = &fence {
                if trimmed.starts_with(open.as_str()) {
                    fence = None;
                }
                out.push_str(line);
                continue;
            }
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                let mark: String = trimmed.chars().take_while(|&c| c == '`' || c == '~').collect();
                fence = Some(mark);
                out.push_str(line);
                continue;
            }
        }
        let mut rest = line;
        let mut kept = String::new();
        while !rest.is_empty() {
            if let Some(at) = rest.find("%%") {
                if !in_comment {
                    kept.push_str(&rest[..at]);
                }
                in_comment = !in_comment;
                rest = &rest[at + 2..];
            } else {
                if !in_comment {
                    kept.push_str(rest);
                }
                rest = "";
            }
        }
        // A line that was only a comment leaves no blank line behind.
        if !line.trim().is_empty() && kept.trim().is_empty() {
            continue;
        }
        if line.ends_with('\n') && !kept.ends_with('\n') {
            kept.push('\n');
        }
        out.push_str(&kept);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typst(text: &str) -> TypstNote {
        TypstNote::new(text, "Notes/Note.md", |target, _| match target {
            "beach.png" => Some("uploads/beach.png".to_string()),
            "uploads/sky blue.webp" => Some("uploads/sky blue.webp".to_string()),
            _ => None,
        })
    }

    #[test]
    fn text_goes_in_as_strings() {
        assert_eq!(typst("A #hash, \"quotes\" and [x] = y").markup, "#\"A #hash, \\\"quotes\\\" and [x] = y\"\n\n");
    }

    #[test]
    fn a_string_escapes_what_typst_reads() {
        assert_eq!(TypstNote::string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn headings_emphasis_and_code() {
        let note = typst("# Title\n\nSome *soft* and **bold** with `code`.\n\n```rust\nfn main() {}\n```\n");
        assert_eq!(
            note.markup,
            "#heading(level: 1)[#\"Title\"]\n\n#\"Some \"#emph[#\"soft\"]#\" and \"#strong[#\"bold\"]#\" with \"#raw(\"code\")#\".\"\n\n#raw(block: true, lang: \"rust\", \"fn main() {}\")\n\n"
        );
    }

    #[test]
    fn wiki_links_are_their_text() {
        let note = typst("See [[Notes/Plan]], [[Plan|the plan]] and [[Plan#Steps]].");
        assert_eq!(
            note.markup,
            "#\"See \"#internal[#\"Plan\"]#\", \"#internal[#\"the plan\"]#\" and \"#internal[#\"Plan › Steps\"]#\".\"\n\n"
        );
    }

    #[test]
    fn pictures_are_drawn_and_numbered_once() {
        let note = typst("![[beach.png|300]]\n\n![sky](uploads/sky%20blue.webp)\n\n![[beach.png]]\n\n![[missing.png]]");
        assert_eq!(note.pictures, ["uploads/beach.png", "uploads/sky blue.webp"]);
        assert!(note.markup.contains("#picture(\"/pictures/0.png\", width: 225pt)"));
        assert!(note.markup.contains("#picture(\"/pictures/1.webp\", width: auto)"));
        assert!(note.markup.contains("#picture(\"/pictures/0.png\", width: auto)"));
        assert!(note.markup.contains("#embedded[#\"missing.png\"]"));
    }

    #[test]
    fn tasks_keep_their_status_and_signs_become_words() {
        let note = typst("- [ ] Call 📅 2026-11-02\n- [x] Done ✅ 2026-10-30\n- [/] Halfway\n");
        assert_eq!(
            note.markup,
            "#list(marker: [], tight: true, [#task(\" \")[#\"Call due 2026-11-02\"]], [#task(\"x\")[#\"Done done 2026-10-30\"]], [#task(\"/\")[#\"Halfway\"]])\n\n"
        );
    }

    #[test]
    fn a_callout_is_a_box_with_its_title() {
        let note = typst("> [!warning] Mind the gap\n> It is wide.\n");
        assert_eq!(note.markup, "#callout(\"warning\")[#\"Mind the gap\"][#\"It is wide.\"\n\n]\n\n");
        let untitled = typst("> [!tip]\n> Short.\n");
        assert!(untitled.markup.starts_with("#callout(\"tip\")[#\"Tip\"]"));
    }

    #[test]
    fn persian_is_set_right_to_left() {
        let note = typst("سلام دنیا\n\nHello سلام\n\n## «سرآغاز»\n");
        assert_eq!(note.markup, "#right-to-left[#\"سلام دنیا\"]\n\n#\"Hello سلام\"\n\n#right-to-left[#heading(level: 2)[#\"«سرآغاز»\"]]\n\n");
    }

    #[test]
    fn front_matter_and_comments_are_left_out() {
        let note = typst("---\ntags: [a]\n---\nKept %%hidden%% text.\n\n%%\nAll of this.\n%%\n\n```\n%% in code %%\n```\n");
        assert_eq!(note.markup, "#\"Kept  text.\"\n\n#raw(block: true, lang: none, \"%% in code %%\")\n\n");
    }

    #[test]
    fn highlights_tables_and_footnotes() {
        let note = typst("A ==marked== word.[^1]\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n[^1]: The note.\n");
        assert!(note.markup.starts_with("#\"A \"#highlight[#\"marked\"]#\" word.\"#footnote[#\"The note.\"]"));
        assert!(note.markup.contains("#table(columns: 2, align: (left, right,), table.header([#\"a\"], [#\"b\"]), [#\"1\"], [#\"2\"])"));
    }

    #[test]
    fn the_nearest_folder_with_a_logo() {
        let files: Vec<String> =
            ["Companies/Snapp/logo.png", "Companies/Snapp/Snapp.md", "Companies/Snapp/Platform/NATS/NATS.md", "logo.svg"]
                .iter()
                .map(ToString::to_string)
                .collect();
        assert_eq!(
            TypstNote::folder_logo("Companies/Snapp/Platform/NATS/NATS.md", &files),
            Some(("Companies/Snapp".to_string(), "Companies/Snapp/logo.png".to_string()))
        );
        assert_eq!(TypstNote::folder_logo("Notes/Inbox.md", &files), None);
        assert_eq!(TypstNote::folder_logo("Top.md", &files), None);
    }
}
