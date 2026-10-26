//! A YAML document as a `.base` file writes it: block maps and lists, plain
//! and quoted scalars, flow lists and block scalars. Not anchors, tags or
//! several documents in one file; Obsidian writes none of those.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlNode {
    Null,
    Scalar(String),
    List(Vec<YamlNode>),
    /// In the file's order.
    Map(Vec<(String, YamlNode)>),
}

impl YamlNode {
    pub fn get(&self, key: &str) -> Option<&YamlNode> {
        match self {
            YamlNode::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn string(&self) -> Option<&str> {
        match self {
            YamlNode::Scalar(text) => Some(text),
            _ => None,
        }
    }

    pub fn items(&self) -> Vec<&YamlNode> {
        match self {
            YamlNode::List(items) => items.iter().collect(),
            YamlNode::Null => vec![],
            _ => vec![self],
        }
    }

    pub fn entries(&self) -> &[(String, YamlNode)] {
        match self {
            YamlNode::Map(entries) => entries,
            _ => &[],
        }
    }
}

struct Line {
    indent: usize,
    text: String,
}

pub struct Yaml;

impl Yaml {
    pub fn parse(source: &str) -> YamlNode {
        let mut lines: Vec<Line> = source
            .split('\n')
            .filter_map(|raw| {
                let line = raw.strip_suffix('\r').unwrap_or(raw);
                let text = line.trim_matches([' ', '\t']);
                if text.is_empty() || text.starts_with('#') || text == "---" || text == "..." {
                    return None;
                }
                Some(Line { indent: line.chars().take_while(|&c| c == ' ').count(), text: text.to_string() })
            })
            .collect();
        let Some(first) = lines.first() else { return YamlNode::Null };
        let indent = first.indent;
        let mut at = 0;
        block(&mut lines, &mut at, indent)
    }

    /// `key: value`, where the key is plain or quoted; an expression with a
    /// colon in a string is not a key.
    pub fn split_key(text: &str) -> Option<(String, String)> {
        if let Some(quote) = text.chars().next().filter(|&c| c == '"' || c == '\'') {
            let close = 1 + text[1..].find(quote)?;
            let after = &text[close + 1..];
            let rest = after.strip_prefix(':')?;
            if !(rest.is_empty() || rest.starts_with(' ')) {
                return None;
            }
            return Some((text[1..close].to_string(), trim(rest).to_string()));
        }
        let mut from = 0;
        while let Some(offset) = text[from..].find(':') {
            let colon = from + offset;
            let next = colon + 1;
            if next == text.len() || text[next..].starts_with(' ') {
                let key = &text[..colon];
                if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || "_-. $/".contains(c)) {
                    return None;
                }
                return Some((trim(key).to_string(), trim(&text[next..]).to_string()));
            }
            from = next;
        }
        None
    }

    /// One value on one line: quoted, a flow list, or plain.
    pub fn inline(value: &str) -> YamlNode {
        let text = strip_comment(value);
        if text == "~" || text == "null" || text.is_empty() {
            return YamlNode::Null;
        }
        if text == "{}" {
            return YamlNode::Map(vec![]);
        }
        if text.starts_with('[') && text.ends_with(']') && text.len() >= 2 {
            let inner = trim(&text[1..text.len() - 1]);
            return YamlNode::List(if inner.is_empty() { vec![] } else { split_flow(inner).iter().map(|i| Yaml::inline(i)).collect() });
        }
        let quoted = |q: char| text.chars().count() >= 2 && text.starts_with(q) && text.ends_with(q);
        if quoted('\'') {
            return YamlNode::Scalar(text[1..text.len() - 1].replace("''", "'"));
        }
        if quoted('"') {
            let mut out = String::new();
            let mut escaped = false;
            for c in text[1..text.len() - 1].chars() {
                if escaped {
                    out.push(match c {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else {
                    out.push(c);
                }
            }
            return YamlNode::Scalar(out);
        }
        YamlNode::Scalar(text)
    }
}

fn trim(text: &str) -> &str {
    text.trim_matches([' ', '\t'])
}

fn is_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

fn block(lines: &mut [Line], at: &mut usize, indent: usize) -> YamlNode {
    if *at >= lines.len() {
        return YamlNode::Null;
    }
    if is_item(&lines[*at].text) { list(lines, at, indent) } else { map(lines, at, indent) }
}

fn list(lines: &mut [Line], at: &mut usize, indent: usize) -> YamlNode {
    let mut items = Vec::new();
    while *at < lines.len() && lines[*at].indent == indent && is_item(&lines[*at].text) {
        let text = lines[*at].text.clone();
        let rest = trim(&text[1..]).to_string();
        if rest.is_empty() {
            *at += 1;
            if *at < lines.len() && lines[*at].indent > indent {
                let inner = lines[*at].indent;
                items.push(block(lines, at, inner));
            } else {
                items.push(YamlNode::Null);
            }
        } else if Yaml::split_key(&rest).is_some() || is_item(&rest) {
            // `- key: value` starts a map whose keys line up after the dash.
            let inner = indent + text.chars().count() - rest.chars().count();
            lines[*at] = Line { indent: inner, text: rest };
            items.push(block(lines, at, inner));
        } else {
            items.push(scalar(&rest, lines, at, indent));
        }
    }
    YamlNode::List(items)
}

fn map(lines: &mut [Line], at: &mut usize, indent: usize) -> YamlNode {
    let mut entries = Vec::new();
    while *at < lines.len() && lines[*at].indent == indent && !is_item(&lines[*at].text) {
        let Some((key, value)) = Yaml::split_key(&lines[*at].text) else {
            *at += 1;
            continue;
        };
        if value.is_empty() {
            *at += 1;
            // A list under a key may sit at the key's own indent.
            if *at < lines.len() && (lines[*at].indent > indent || (lines[*at].indent == indent && is_item(&lines[*at].text))) {
                let inner = lines[*at].indent;
                entries.push((key, block(lines, at, inner)));
            } else {
                entries.push((key, YamlNode::Null));
            }
        } else {
            entries.push((key, scalar(&value, lines, at, indent)));
        }
    }
    YamlNode::Map(entries)
}

fn scalar(value: &str, lines: &mut [Line], at: &mut usize, parent: usize) -> YamlNode {
    *at += 1;
    if let Some(style) = value.chars().next().filter(|&c| c == '|' || c == '>')
        && value.chars().count() <= 2
    {
        let mut parts = Vec::new();
        while *at < lines.len() && lines[*at].indent > parent {
            parts.push(lines[*at].text.clone());
            *at += 1;
        }
        return YamlNode::Scalar(parts.join(if style == '|' { "\n" } else { " " }));
    }
    Yaml::inline(value)
}

/// A ` #` outside quotes starts a comment.
fn strip_comment(text: &str) -> String {
    if !text.contains(" #") {
        return text.to_string();
    }
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (offset, c) in text.char_indices() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '#' && previous == ' ' {
            return trim(&text[..offset]).to_string();
        }
        previous = c;
    }
    text.to_string()
}

/// The items of a flow list, split at the commas outside quotes and brackets.
fn split_flow(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '[' || c == '{' {
            depth += 1;
        } else if c == ']' || c == '}' {
            depth -= 1;
        } else if c == ',' && depth == 0 {
            items.push(trim(&current).to_string());
            current.clear();
            continue;
        }
        current.push(c);
    }
    items.push(trim(&current).to_string());
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(node: Option<&YamlNode>) -> Vec<String> {
        node.map(|n| n.items().iter().filter_map(|i| i.string().map(String::from)).collect()).unwrap_or_default()
    }

    #[test]
    fn block_maps_lists_and_scalars() {
        let node = Yaml::parse(
            "filters:\n  and:\n    - file.inFolder(\"Shops\")\n    - 'tags.contains(\"x: y\")'\nformulas:\n  shop: file.folder.split(\"/\")[1]\n\
             views:\n  - type: table\n    name: All # a comment\n    order:\n      - file.name\n    limit: 5\nempty:\nflow: [a, \"b, c\"]\n\
             text: |\n  one\n  two\n",
        );
        assert_eq!(strings(node.get("filters").and_then(|f| f.get("and"))), ["file.inFolder(\"Shops\")", "tags.contains(\"x: y\")"]);
        assert_eq!(node.get("formulas").and_then(|f| f.get("shop")).and_then(YamlNode::string), Some("file.folder.split(\"/\")[1]"));
        let views = node.get("views").unwrap().items();
        let view = views[0];
        assert_eq!(view.get("name").and_then(YamlNode::string), Some("All"));
        assert_eq!(strings(view.get("order")), ["file.name"]);
        assert_eq!(view.get("limit").and_then(YamlNode::string), Some("5"));
        assert_eq!(node.get("empty"), Some(&YamlNode::Null));
        assert_eq!(strings(node.get("flow")), ["a", "b, c"]);
        assert_eq!(node.get("text").and_then(YamlNode::string), Some("one\ntwo"));
    }

    #[test]
    fn list_under_a_key_at_the_same_indent() {
        let node = Yaml::parse("order:\n- a\n- b\nnext: 1\n");
        assert_eq!(strings(node.get("order")), ["a", "b"]);
        assert_eq!(node.get("next").and_then(YamlNode::string), Some("1"));
    }
}
