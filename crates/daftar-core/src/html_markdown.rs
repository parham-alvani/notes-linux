//! Rich text pasted from a web page or a document, as Markdown, the way
//! Obsidian converts a paste: headings, emphasis, links, pictures, lists,
//! quotes, code and tables keep their meaning; colours and fonts do not.

use std::sync::LazyLock;

use ego_tree::NodeRef;
use regex::Regex;
use scraper::{Html, Node};

/// The tags that make a paste worth converting. Code editors copy coloured
/// `div` and `span` lines, which stay plain text.
const MEANINGFUL: [&str; 24] = [
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "a",
    "strong",
    "b",
    "em",
    "i",
    "ul",
    "ol",
    "li",
    "table",
    "blockquote",
    "pre",
    "code",
    "img",
    "p",
    "del",
    "s",
    "strike",
    "hr",
];
const BLOCK_NAMES: [&str; 15] = ["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "table", "blockquote", "pre", "hr", "section"];

static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}]+").unwrap());
static LANGUAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:language|lang)-([\w+#-]+)").unwrap());

pub struct HtmlMarkdown;

impl HtmlMarkdown {
    /// [html] as Markdown, or `None` when it holds nothing a plain paste would lose.
    pub fn convert(html: &str) -> Option<String> {
        let document = Html::parse_document(html);
        let root = document.tree.root();
        let body = root.descendants().find(|n| name(*n) == "body").unwrap_or(root);
        if !has_meaning(body) {
            return None;
        }
        let mut writer = Writer::default();
        writer.block(body);
        let text = writer.finish();
        (!text.is_empty()).then_some(text)
    }
}

type Ref<'a> = NodeRef<'a, Node>;

/// An element's tag name in lower case; empty for anything else.
fn name(node: Ref) -> String {
    match node.value() {
        Node::Element(element) => element.name().to_lowercase(),
        _ => String::new(),
    }
}

fn attr<'a>(node: Ref<'a>, key: &str) -> Option<&'a str> {
    match node.value() {
        Node::Element(element) => element.attr(key),
        _ => None,
    }
}

/// All the text under [node], as the DOM's `textContent`.
fn text_of(node: Ref) -> String {
    node.descendants()
        .filter_map(|n| match n.value() {
            Node::Text(text) => Some(&**text),
            _ => None,
        })
        .collect()
}

fn style(node: Ref) -> String {
    attr(node, "style").unwrap_or("").to_lowercase()
}

fn has_meaning(node: Ref) -> bool {
    if MEANINGFUL.contains(&name(node).as_str()) {
        return true;
    }
    let style = style(node);
    if ["font-weight:700", "font-weight: 700", "font-weight:bold", "font-style:italic", "font-style: italic"]
        .iter()
        .any(|s| style.contains(s))
    {
        return true;
    }
    node.children().any(has_meaning)
}

fn holds_blocks(node: Ref) -> bool {
    node.children().any(|c| BLOCK_NAMES.contains(&name(c).as_str()))
}

/// Runs of spaces as one, and no space at the ends of a line.
fn tidy(text: &str) -> String {
    let spaced = SPACES.replace_all(text, " ");
    let lines: Vec<&str> = spaced.split('\n').map(|l| l.trim_matches([' ', '\t'])).collect();
    lines.join("\n").trim().to_string()
}

#[derive(Default)]
struct Writer {
    blocks: Vec<String>,
}

impl Writer {
    fn finish(self) -> String {
        self.blocks
            .iter()
            .map(|b| b.trim_matches(['\n', '\r']).to_string())
            .filter(|b| !b.trim_matches([' ', '\t']).is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The block elements of [node], each a paragraph of its own.
    fn block(&mut self, node: Ref) {
        let mut inline = String::new();
        let flush = |inline: &mut String, blocks: &mut Vec<String>| {
            let text = tidy(inline);
            if !text.is_empty() {
                blocks.push(text);
            }
            inline.clear();
        };
        for child in node.children() {
            let tag = name(child);
            match tag.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush(&mut inline, &mut self.blocks);
                    let level: usize = tag[1..].parse().unwrap_or(1);
                    self.blocks.push(format!("{} {}", "#".repeat(level), tidy(&inline_text(child))));
                }
                "p" => {
                    flush(&mut inline, &mut self.blocks);
                    self.blocks.push(tidy(&inline_text(child)));
                }
                "ul" | "ol" => {
                    flush(&mut inline, &mut self.blocks);
                    self.blocks.push(list(child, tag == "ol", 0));
                }
                "blockquote" => {
                    flush(&mut inline, &mut self.blocks);
                    let mut inner = Writer::default();
                    inner.block(child);
                    let quoted: Vec<String> =
                        inner.finish().split('\n').map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") }).collect();
                    self.blocks.push(quoted.join("\n"));
                }
                "pre" => {
                    flush(&mut inline, &mut self.blocks);
                    let code = text_of(child);
                    self.blocks.push(format!("```{}\n{}\n```", language(child), code.trim_matches(['\n', '\r'])));
                }
                "hr" => {
                    flush(&mut inline, &mut self.blocks);
                    self.blocks.push("---".into());
                }
                "table" => {
                    flush(&mut inline, &mut self.blocks);
                    if let Some(table) = table(child) {
                        self.blocks.push(table);
                    }
                }
                "div" | "section" | "article" | "main" | "header" | "footer" | "figure" | "body" | "center" => {
                    flush(&mut inline, &mut self.blocks);
                    self.block(child);
                }
                "head" | "style" | "script" | "meta" | "title" | "link" => {}
                _ if holds_blocks(child) => {
                    // Google Docs wraps the whole paste in a `<b>` of normal weight.
                    flush(&mut inline, &mut self.blocks);
                    self.block(child);
                }
                _ => inline.push_str(&inline_text(child)),
            }
        }
        flush(&mut inline, &mut self.blocks);
    }
}

/// The text of an inline node, with its marks.
fn inline_text(node: Ref) -> String {
    let tag = match node.value() {
        Node::Text(text) => return text.replace('\n', " "),
        Node::Element(_) => name(node),
        _ => return String::new(),
    };
    let inner: String = node.children().map(inline_text).collect();
    let mark = |marker: &str| -> String {
        let trimmed = inner.trim_matches(' ');
        if trimmed.is_empty() {
            return inner.clone();
        }
        // The mark hugs the words, so `** bold**` does not happen.
        let lead = &inner[..inner.len() - inner.trim_start_matches(' ').len()];
        let trail = &inner[inner.trim_end_matches(' ').len()..];
        format!("{lead}{marker}{trimmed}{marker}{trail}")
    };
    let style = style(node).replace(' ', "");
    match tag.as_str() {
        "strong" | "b" => {
            if style.contains("font-weight:normal") || style.contains("font-weight:400") {
                inner
            } else {
                mark("**")
            }
        }
        "em" | "i" => mark("*"),
        "del" | "s" | "strike" => mark("~~"),
        "mark" => mark("=="),
        "code" => format!("`{}`", text_of(node)),
        "br" => "\n".into(),
        "a" => {
            let Some(href) = attr(node, "href").filter(|h| !h.is_empty() && !h.starts_with("javascript:")) else { return inner };
            let label = inner.trim_matches(' ');
            if label.is_empty() || label == href { format!("<{href}>") } else { format!("[{label}]({})", href.replace(' ', "%20")) }
        }
        "img" => match attr(node, "src").filter(|s| !s.starts_with("data:")) {
            Some(src) => format!("![{}]({src})", attr(node, "alt").unwrap_or("")),
            None => String::new(),
        },
        "span" => {
            let mut text = inner.clone();
            if style.contains("font-weight:700") || style.contains("font-weight:bold") {
                text = mark("**");
            }
            if style.contains("font-style:italic") {
                text = if text == inner { mark("*") } else { format!("*{text}*") };
            }
            text
        }
        "script" | "style" | "head" => String::new(),
        _ => inner,
    }
}

fn list(node: Ref, ordered: bool, depth: usize) -> String {
    let mut lines = Vec::new();
    let first: i64 = attr(node, "start").and_then(|s| s.parse().ok()).unwrap_or(1);
    for (number, item) in (first..).zip(node.children().filter(|c| name(*c) == "li")) {
        let mut text = String::new();
        let mut nested = Vec::new();
        for child in item.children() {
            match name(child).as_str() {
                tag @ ("ul" | "ol") => nested.push(list(child, tag == "ol", depth + 1)),
                "p" => {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&inline_text(child));
                }
                _ => text.push_str(&inline_text(child)),
            }
        }
        let indent = "    ".repeat(depth);
        let mut marker = if ordered { format!("{number}. ") } else { "- ".into() };
        // A checkbox from a task list.
        if let Some(box_) = item.children().find(|c| name(*c) == "input" && attr(*c, "type") == Some("checkbox")) {
            marker = format!("- [{}] ", if attr(box_, "checked").is_some() { "x" } else { " " });
        }
        lines.push(format!("{indent}{marker}{}", tidy(&text)));
        lines.extend(nested);
    }
    lines.join("\n")
}

fn table(node: Ref) -> Option<String> {
    let rows: Vec<Vec<String>> = node
        .descendants()
        .filter(|n| name(*n) == "tr")
        .map(|row| {
            row.children()
                .filter(|c| matches!(name(*c).as_str(), "td" | "th"))
                .map(|cell| tidy(&inline_text(cell)).replace('|', "\\|").replace('\n', " "))
                .collect::<Vec<_>>()
        })
        .filter(|r| !r.is_empty())
        .collect();
    let first = rows.first()?;
    let width = rows.iter().map(Vec::len).max().unwrap_or(first.len());
    let line = |cells: &[String]| {
        let mut cells = cells.to_vec();
        cells.resize(width, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![line(first), format!("|{}", " --- |".repeat(width))];
    out.extend(rows[1..].iter().map(|r| line(r)));
    Some(out.join("\n"))
}

fn language(pre: Ref) -> String {
    let classes: Vec<&str> = std::iter::once(pre).chain(pre.children()).filter_map(|n| attr(n, "class")).collect();
    LANGUAGE.captures(&classes.join(" ")).map(|c| c[1].to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_page_becomes_markdown() {
        let html = r#"<html><head><style>p{}</style></head><body>
<h2>Getting <em>there</em></h2>
<p>Take the <strong>red</strong> line to <a href="https://example.com/a b">the park</a>.<br>Then walk.</p>
<ul><li>One</li><li>Two<ul><li>Two and a half</li></ul></li></ul>
<ol start="3"><li><p>Third</p></li></ol>
<blockquote><p>Quoted</p></blockquote>
<pre><code class="language-swift">let a = 1
let b = 2</code></pre>
<table><tr><th>Name</th><th>Age</th></tr><tr><td>Ana</td><td>30</td></tr></table>
<p><img src="https://example.com/x.png" alt="X"> and <code>inline</code></p>
</body></html>"#;
        assert_eq!(
            HtmlMarkdown::convert(html).as_deref(),
            Some(
                "## Getting *there*\n\nTake the **red** line to [the park](https://example.com/a%20b).\nThen walk.\n\n- One\n- Two\n    - Two and a half\n\n3. Third\n\n> Quoted\n\n```swift\nlet a = 1\nlet b = 2\n```\n\n| Name | Age |\n| --- | --- |\n| Ana | 30 |\n\n![X](https://example.com/x.png) and `inline`"
            )
        );
    }

    #[test]
    fn code_editors_and_plain_text_stay_plain() {
        assert_eq!(
            HtmlMarkdown::convert(
                r#"<meta charset="utf-8"><div style="color: #d4d4d4;"><div><span style="color: #569cd6;">let</span> a = 1</div></div>"#
            ),
            None
        );
        assert_eq!(HtmlMarkdown::convert("just words"), None);
    }

    #[test]
    fn google_docs_bold_spans_count() {
        let html = r#"<b style="font-weight:normal;" id="docs"><p><span style="font-weight:700;">Bold</span> and <span style="font-style:italic;">slanted</span></p></b>"#;
        assert_eq!(HtmlMarkdown::convert(html).as_deref(), Some("**Bold** and *slanted*"));
    }

    #[test]
    fn a_task_list_keeps_its_boxes() {
        let html = r#"<ul><li><input type="checkbox" checked> Done</li><li><input type="checkbox"> Open</li></ul>"#;
        assert_eq!(HtmlMarkdown::convert(html).as_deref(), Some("- [x] Done\n- [ ] Open"));
    }
}
