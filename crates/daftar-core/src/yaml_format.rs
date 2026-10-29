//! A note's YAML front matter, formatted as Prettier's YAML printer formats
//! it, for the shapes front matter takes: a mapping of scalars, lists and
//! mappings, block scalars, comments of their own lines. Anything else (an
//! anchor, a flow mapping, a scalar over lines, a comment after a value) is
//! not formatted: `None`, and the front matter is kept as written.

struct Line<'a> {
    indent: usize,
    text: &'a str,
}

pub fn format(value: &str, tab_width: usize, single_quote: bool, _print_width: usize) -> Option<String> {
    let lines: Vec<Option<Line>> = value
        .split('\n')
        .map(|l| {
            let l = l.trim_end();
            if l.trim().is_empty() {
                None
            } else {
                let text = l.trim_start_matches(' ');
                if text.starts_with('\t') {
                    return Some(Line { indent: usize::MAX, text });
                }
                Some(Line { indent: l.len() - text.len(), text })
            }
        })
        .collect();
    if lines.iter().flatten().any(|l| l.indent == usize::MAX) {
        return None;
    }
    let mut out = Vec::new();
    let mut at = 0;
    mapping(&lines, &mut at, 0, 0, &mut out, &Style { tab_width, single_quote })?;
    if at < lines.len() {
        return None;
    }
    Some(out.join("\n"))
}

struct Style {
    tab_width: usize,
    single_quote: bool,
}

fn skip_blank(lines: &[Option<Line>], at: &mut usize) -> bool {
    let start = *at;
    while *at < lines.len() && lines[*at].is_none() {
        *at += 1;
    }
    *at > start
}

/// The entries of a mapping whose keys are at `indent`, printed at `level`.
fn mapping(lines: &[Option<Line>], at: &mut usize, indent: usize, level: usize, out: &mut Vec<String>, style: &Style) -> Option<()> {
    let pad = " ".repeat(level * style.tab_width);
    let mut first = true;
    loop {
        let before = *at;
        let blank = skip_blank(lines, at);
        let Some(Some(line)) = lines.get(*at) else {
            *at = if *at >= lines.len() { *at } else { before };
            return Some(());
        };
        if line.indent < indent {
            *at = before;
            return Some(());
        }
        if line.indent > indent {
            return None;
        }
        if blank && !first {
            out.push(String::new());
        }
        first = false;
        if line.text.starts_with('#') {
            out.push(format!("{pad}{}", line.text));
            *at += 1;
            continue;
        }
        let (key, rest) = split_key(line.text)?;
        *at += 1;
        let rest = rest.trim();
        if rest.is_empty() {
            out.push(format!("{pad}{key}:"));
            let mut probe = *at;
            skip_blank(lines, &mut probe);
            match lines.get(probe) {
                Some(Some(next)) if next.text.starts_with("- ") || next.text == "-" => {
                    if next.indent < indent {
                        continue;
                    }
                    *at = probe;
                    sequence(lines, at, next.indent, level + 1, out, style)?;
                }
                Some(Some(next)) if next.indent > indent && !next.text.starts_with('#') => {
                    *at = probe;
                    mapping(lines, at, next.indent, level + 1, out, style)?;
                }
                _ => {}
            }
            continue;
        }
        if let Some(header) = block_scalar_header(rest) {
            let body = block_scalar(lines, at, indent, header, level + 1, style)?;
            out.push(format!("{pad}{key}: {header}"));
            out.extend(body);
            continue;
        }
        out.push(format!("{pad}{key}: {}", scalar(rest, style)?));
    }
}

fn sequence(lines: &[Option<Line>], at: &mut usize, indent: usize, level: usize, out: &mut Vec<String>, style: &Style) -> Option<()> {
    let pad = " ".repeat(level * style.tab_width);
    let mut first = true;
    loop {
        let before = *at;
        let blank = skip_blank(lines, at);
        let Some(Some(line)) = lines.get(*at) else {
            *at = if *at >= lines.len() { *at } else { before };
            return Some(());
        };
        if line.indent != indent || !(line.text.starts_with("- ") || line.text == "-") {
            if line.indent > indent {
                return None;
            }
            *at = before;
            return Some(());
        }
        if blank && !first {
            out.push(String::new());
        }
        first = false;
        let item = line.text[1..].trim();
        *at += 1;
        if item.is_empty() || item.starts_with("- ") || split_key(item).is_some() || block_scalar_header(item).is_some() {
            return None;
        }
        out.push(format!("{pad}- {}", scalar(item, style)?));
    }
}

/// `key: rest`, for a plain key.
fn split_key(text: &str) -> Option<(&str, &str)> {
    if text.starts_with(['"', '\'', '-', '[', '{', '?', '&', '*', '!', '|', '>', '%', '@', '`', '#']) {
        return None;
    }
    let colon = text.find(':')?;
    let (key, rest) = (&text[..colon], &text[colon + 1..]);
    if !(rest.is_empty() || rest.starts_with(' ')) || key.contains(" #") || key.trim() != key || key.is_empty() {
        return None;
    }
    Some((key, rest))
}

fn block_scalar_header(rest: &str) -> Option<&str> {
    matches!(rest, "|" | "|-" | "|+" | ">" | ">-" | ">+").then_some(rest)
}

/// A block scalar's lines, moved to its new indent; a folded one's lines
/// joined, as Prettier does with `proseWrap` never.
fn block_scalar(lines: &[Option<Line>], at: &mut usize, parent: usize, header: &str, level: usize, style: &Style) -> Option<Vec<String>> {
    let start = *at;
    let mut end = start;
    while end < lines.len() && lines[end].as_ref().is_none_or(|l| l.indent > parent) {
        end += 1;
    }
    while end > start && lines[end - 1].is_none() {
        end -= 1;
    }
    *at = end;
    let body = &lines[start..end];
    let base = body.iter().flatten().map(|l| l.indent).min()?;
    let pad = " ".repeat(level * style.tab_width);
    if header.starts_with('>') {
        if body.iter().any(|l| l.as_ref().is_none_or(|l| l.indent != base)) {
            return None;
        }
        let joined: Vec<&str> = body.iter().flatten().map(|l| l.text).collect();
        return Some(vec![format!("{pad}{}", joined.join(" "))]);
    }
    Some(body.iter().map(|l| l.as_ref().map_or(String::new(), |l| format!("{pad}{}{}", " ".repeat(l.indent - base), l.text))).collect())
}

/// A scalar on one line: quotes as Prettier prefers them, a flow list
/// spaced.
fn scalar(text: &str, style: &Style) -> Option<String> {
    if text.starts_with(['&', '*', '!', '{', '|', '>', '%', '@', '`']) || text.contains(" #") {
        return None;
    }
    if let Some(inner) = text.strip_prefix('[') {
        let inner = inner.strip_suffix(']')?;
        if inner.contains(['[', ']', '{', '}', '"', '\'']) {
            return None;
        }
        if inner.trim().is_empty() {
            return Some("[]".to_string());
        }
        let items: Vec<&str> = inner.split(',').map(str::trim).collect();
        if items.iter().any(|i| i.is_empty()) {
            return None;
        }
        return Some(format!("[{}]", items.join(", ")));
    }
    if let Some(inner) = text.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'')?;
        if inner.replace("''", "").contains('\'') {
            return None;
        }
        let content = inner.replace("''", "'");
        return Some(quote(&content, style, inner.contains('\\')));
    }
    if let Some(inner) = text.strip_prefix('"') {
        let inner = inner.strip_suffix('"')?;
        let unescaped_quotes = inner.replace("\\\\", "").replace("\\\"", "");
        if unescaped_quotes.contains('"') {
            return None;
        }
        if inner.replace("\\\\", "").replace("\\\"", "").contains('\\') {
            return Some(text.to_string());
        }
        let content = inner.replace("\\\"", "\"").replace("\\\\", "\\");
        return Some(quote(&content, style, content.contains('\\')));
    }
    if text.contains(": ") || text.ends_with(':') {
        return None;
    }
    Some(text.to_string())
}

/// The content in the quotes that need the fewest escapes, double ones when
/// it is even (single with `singleQuote`). A backslash can only be single
/// quoted.
fn quote(content: &str, style: &Style, backslash: bool) -> String {
    let doubles = content.matches('"').count();
    let singles = content.matches('\'').count();
    let single = if backslash {
        !content.contains('\'') || doubles > 0
    } else if style.single_quote {
        singles <= doubles
    } else {
        doubles > singles
    };
    if single && !(backslash && content.contains('\'') && doubles == 0) {
        format!("'{}'", content.replace('\'', "''"))
    } else {
        format!("\"{}\"", content.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_as_prettier_writes_it() {
        let text = "a:   1\nb: 'it''s'\nc: 'say \"hi\"'\nd: \"x \\\"q\\\" y\"\ne:\n  - x\n  -   'y'\nf:\n   g: 1\n   h:\n     - z\n\n\ni: |\n  line one\n    indented\n  line two\nj: >-\n      folded\n      text\nl: [a,   b]\nm: ~\n# comment\nn: 'a\\b'\no:\n- p\n- q";
        let want = "a: 1\nb: \"it's\"\nc: 'say \"hi\"'\nd: 'x \"q\" y'\ne:\n    - x\n    - \"y\"\nf:\n    g: 1\n    h:\n        - z\n\ni: |\n    line one\n      indented\n    line two\nj: >-\n    folded text\nl: [a, b]\nm: ~\n# comment\nn: 'a\\b'\no:\n    - p\n    - q";
        assert_eq!(format(text, 4, false, 100).as_deref(), Some(want));
    }

    #[test]
    fn what_it_does_not_know_is_kept() {
        assert_eq!(format("a: &x 1", 4, false, 100), None);
        assert_eq!(format("a: b: c", 4, false, 100), None);
        assert_eq!(format("a: 1 # note", 4, false, 100), None);
    }
}
