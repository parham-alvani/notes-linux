//! A note for a terminal: Obsidian's markdown as styled blocks, one for each
//! line of the file that shows, then wrapped to the width of the pane.
//!
//! The reader in the app draws the same syntax with markdown-it; this is the
//! part of it a terminal can show. Each block remembers the line of the file
//! it came from, the links on it and the task it is, so a key on a row acts
//! on what the row shows.

use std::sync::LazyLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use regex::Regex;
use unicode_width::UnicodeWidthChar;

use daftar_core::text::direction;
use daftar_core::{TaskLine, TaskMetadata, TaskReason, TaskStatuses, TaskType};

use super::theme::Theme;

/// One line of the note as it shows, before it is wrapped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Block {
    pub spans: Vec<Span<'static>>,
    /// Drawn before the first row: a bullet, a quote's bar.
    pub prefix: Vec<Span<'static>>,
    /// Drawn before each row the text wraps to.
    pub hang: Vec<Span<'static>>,
    /// The line of the file, from 0.
    pub source: Option<usize>,
    /// The link targets on the line, as written, before `#` and `|`.
    pub links: Vec<String>,
    /// The status the task on this line has.
    pub task: Option<char>,
    pub rtl: bool,
    /// A rule across the pane.
    pub rule: bool,
    /// The row's background, for code.
    pub fill: Option<Color>,
}

/// One row on screen, and the block it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub line: Line<'static>,
    pub block: usize,
}

/// Front matter the reader does not print: the plugins' own keys.
const HIDDEN: [&str; 5] = ["cssclasses", "cssclass", "icon", "iconColor", "position"];

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$").unwrap());
static LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([ \t]*)([-*+]|\d+[.)])[ \t]+(.*)$").unwrap());
static CHECKBOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([^\]\n])\][ \t]*(.*)$").unwrap());
static CALLOUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[!([A-Za-z0-9_-]+)\][+-]?[ \t]*(.*)$").unwrap());
static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:-[ \t]*){3,}$|^(?:\*[ \t]*){3,}$|^(?:_[ \t]*){3,}$").unwrap());
static TABLE_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^:?-+:?$").unwrap());
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+\^[A-Za-z0-9-]+\s*$").unwrap());

/// The note's blocks. [properties] is its front matter as the parser read
/// it; [today] colours a due date that has passed.
pub fn blocks(text: &str, properties: &[(String, Vec<String>)], theme: &Theme, today: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let lines = daftar_core::text::lines(text);
    let mut at = front_matter_end(&lines).unwrap_or(0);
    let shown: Vec<_> = properties.iter().filter(|(key, _)| !HIDDEN.contains(&key.as_str())).collect();
    for (key, values) in &shown {
        let mut spans = vec![Span::styled(format!("{key:<12} "), theme.muted())];
        for (n, value) in values.iter().enumerate() {
            if n > 0 {
                spans.push(Span::styled(" · ", theme.faint()));
            }
            let style = if *key == "tags" { Style::new().fg(theme.accent) } else { Style::new().fg(theme.text) };
            let shown = if *key == "tags" && !value.starts_with('#') { format!("#{value}") } else { value.clone() };
            spans.extend(inline(&shown, style, theme).0);
        }
        out.push(Block { spans, source: Some(0), prefix: vec![Span::styled("│ ", theme.faint())], ..Block::default() });
    }
    if !shown.is_empty() {
        out.push(Block { rule: true, source: Some(at.saturating_sub(1)), ..Block::default() });
    }

    let mut fence: Option<(String, usize)> = None;
    let mut comment = false;
    let mut callout: Option<String> = None;
    let mut table: Vec<(usize, usize, String)> = Vec::new();
    while at < lines.len() {
        let raw = lines[at];
        let (depth, body) = unquote(raw);
        if depth == 0 {
            callout = None;
        }
        if !(table.is_empty() || body.trim_start().starts_with('|') && fence.is_none()) {
            out.extend(table_blocks(&std::mem::take(&mut table), callout.as_deref(), theme));
        }
        let quote = quote_prefix(depth, callout.as_deref(), theme);
        // Inside a fence: the code as written, until the fence closes.
        if let Some((mark, _)) = &fence {
            if body.trim_start().starts_with(mark.as_str()) && body.trim().trim_start_matches(mark.chars().next().unwrap()).is_empty() {
                fence = None;
                out.push(Block { prefix: quote, source: Some(at), fill: Some(theme.code_background), ..Block::default() });
            } else {
                out.push(Block {
                    spans: vec![Span::styled(format!(" {body}"), Style::new().fg(theme.text))],
                    prefix: quote.clone(),
                    hang: quote,
                    source: Some(at),
                    fill: Some(theme.code_background),
                    ..Block::default()
                });
            }
            at += 1;
            continue;
        }
        // An Obsidian comment hides everything until it closes.
        if comment {
            comment = !body.contains("%%");
            at += 1;
            continue;
        }
        let trimmed = body.trim();
        if trimmed.starts_with("%%") && trimmed.matches("%%").count() == 1 {
            comment = true;
            at += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let mark: String = trimmed.chars().take_while(|c| *c == '`' || *c == '~').collect();
            let language = trimmed[mark.len()..].trim().to_string();
            let label = match language.as_str() {
                "tasks" => "tasks query · t runs it".to_string(),
                "" => String::new(),
                other => other.to_string(),
            };
            out.push(Block {
                spans: vec![Span::styled(format!(" {label}"), theme.faint().add_modifier(Modifier::ITALIC))],
                prefix: quote,
                source: Some(at),
                fill: Some(theme.code_background),
                ..Block::default()
            });
            fence = Some((mark, at));
            at += 1;
            continue;
        }
        if trimmed == "$$" {
            fence = Some(("$$".into(), at));
            out.push(Block { prefix: quote, source: Some(at), fill: Some(theme.code_background), ..Block::default() });
            at += 1;
            continue;
        }
        if trimmed.starts_with('|') {
            table.push((at, depth, body.to_string()));
            at += 1;
            continue;
        }
        let mut block = line_block(body, depth, &mut callout, quote, theme, today);
        block.source = Some(at);
        // One blank row where the file has several.
        let blank = block.spans.is_empty() && !block.rule && block.prefix.iter().all(|s| s.content.trim().is_empty());
        if !(blank && out.last().is_some_and(|b: &Block| b.spans.is_empty() && !b.rule && b.fill.is_none())) {
            out.push(block);
        }
        at += 1;
    }
    if !table.is_empty() {
        out.extend(table_blocks(&table, callout.as_deref(), theme));
    }
    while out.last().is_some_and(|b| b.spans.is_empty() && !b.rule && b.fill.is_none()) {
        out.pop();
    }
    out
}

/// The line after the front matter, when the note has one.
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return None;
    }
    lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")).map(|end| end + 2)
}

/// How many `>` the line starts with, and the line without them.
fn unquote(raw: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = raw;
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        let Some(after) = trimmed.strip_prefix('>') else { break };
        depth += 1;
        rest = after.strip_prefix(' ').unwrap_or(after);
    }
    if depth == 0 { (0, raw) } else { (depth, rest) }
}

fn quote_prefix(depth: usize, callout: Option<&str>, theme: &Theme) -> Vec<Span<'static>> {
    let color = callout.map_or(theme.orange, |kind| theme.callout(kind));
    (0..depth).map(|_| Span::styled("▌ ", Style::new().fg(color))).collect()
}

/// A line that is not code and not a table.
fn line_block(body: &str, depth: usize, callout: &mut Option<String>, quote: Vec<Span<'static>>, theme: &Theme, today: &str) -> Block {
    let mut block = Block { prefix: quote.clone(), hang: quote.clone(), ..Block::default() };
    let trimmed = body.trim();
    if depth > 0
        && let Some(caps) = CALLOUT.captures(trimmed)
    {
        let kind = caps[1].to_lowercase();
        let color = theme.callout(&kind);
        *callout = Some(kind.clone());
        let title = if caps[2].trim().is_empty() {
            let mut name = kind.clone();
            name[..1].make_ascii_uppercase();
            name
        } else {
            caps[2].trim().to_string()
        };
        let (mut spans, links) = inline(&title, Style::new().fg(color).add_modifier(Modifier::BOLD), theme);
        spans.insert(0, Span::styled(format!("{} ", Theme::callout_icon(&kind)), Style::new().fg(color)));
        block.prefix = quote_prefix(depth, Some(&kind), theme);
        block.hang.clone_from(&block.prefix);
        block.rtl = direction::starts_rtl(&title);
        block.spans = spans;
        block.links = links;
        return block;
    }
    let text_style = if depth > 0 && callout.is_none() {
        Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC)
    } else {
        Style::new().fg(theme.text)
    };
    if trimmed.is_empty() {
        return block;
    }
    if RULE.is_match(trimmed) {
        block.rule = true;
        return block;
    }
    if let Some(caps) = HEADING.captures(trimmed) {
        let level = caps[1].len();
        let style = theme.heading(level);
        let (mut spans, links) = inline(&caps[2], style, theme);
        if level <= 2 {
            spans.insert(0, Span::styled(if level == 1 { "█ " } else { "▍ " }, style));
        } else {
            spans.insert(
                0,
                Span::styled(format!("{} ", "#".repeat(level)), style.remove_modifier(Modifier::BOLD).add_modifier(Modifier::DIM)),
            );
        }
        block.rtl = direction::starts_rtl(&caps[2]);
        block.spans = spans;
        block.links = links;
        return block;
    }
    if let Some(caps) = LIST.captures(body) {
        let indent = caps[1].chars().map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>() / 2;
        let pad = "  ".repeat(indent);
        let marker = &caps[2];
        let rest = &caps[3];
        if let Some(task) = CHECKBOX.captures(rest).filter(|_| TaskLine::is_task(body)) {
            let status = task[1].chars().next().unwrap_or(' ');
            // A status the vault's settings add is drawn by its type.
            let (glyph, color) = match (status, TaskStatuses::current().kind(status)) {
                ('>', TaskType::Todo) => ("↪", theme.blue),
                ('!', TaskType::Todo) => ("!", theme.red),
                ('?', TaskType::Todo) => ("?", theme.yellow),
                (_, TaskType::Done) => ("☑", theme.green),
                (_, TaskType::Cancelled | TaskType::NonTask) => ("☒", theme.done),
                (_, TaskType::InProgress) => ("◐", theme.accent),
                (_, TaskType::Todo) => ("☐", theme.accent),
            };
            block.prefix.push(Span::raw(pad.clone()));
            block.prefix.push(Span::styled(format!("{glyph} "), Style::new().fg(color)));
            block.hang.push(Span::raw(format!("{pad}  ")));
            let closed = TaskLine::is_closed(status);
            let style = if closed { Style::new().fg(theme.done).add_modifier(Modifier::CROSSED_OUT) } else { text_style };
            let body = BLOCK_ID.replace(&task[2], "").to_string();
            let (sentence, metas) = TaskMetadata::split(&body);
            let (sentence, reason) = TaskReason::take(&sentence);
            let (mut spans, links) = inline(&sentence, style, theme);
            if let Some(reason) = reason {
                spans.push(Span::styled(format!("  💬 {reason}"), Style::new().fg(theme.muted).add_modifier(Modifier::ITALIC)));
            }
            for meta in metas {
                let kind = TaskMetadata::kind(&meta.emoji).unwrap_or("");
                let color = match kind {
                    "due" if !closed && meta.value.as_str() < today => theme.red,
                    "due" => theme.orange,
                    "scheduled" | "start" => theme.blue,
                    "done" => theme.green,
                    "cancelled" => theme.done,
                    "priority" => theme.pink,
                    _ => theme.muted,
                };
                let value = if meta.value.is_empty() { meta.emoji.clone() } else { format!("{} {}", meta.emoji, meta.value) };
                spans.push(Span::raw(" "));
                spans.push(Span::styled(value, Style::new().fg(color)));
            }
            block.task = Some(status);
            block.rtl = direction::starts_rtl(&sentence);
            block.spans = spans;
            block.links = links;
            return block;
        }
        let bullet = if marker.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            format!("{marker} ")
        } else if indent % 2 == 0 {
            "• ".to_string()
        } else {
            "◦ ".to_string()
        };
        let width = bullet.chars().count();
        block.prefix.push(Span::raw(pad.clone()));
        block.prefix.push(Span::styled(bullet, Style::new().fg(theme.accent)));
        block.hang.push(Span::raw(format!("{pad}{}", " ".repeat(width))));
        let (spans, links) = inline(rest, text_style, theme);
        block.rtl = direction::starts_rtl(rest);
        block.spans = spans;
        block.links = links;
        return block;
    }
    let (spans, links) = inline(trimmed, text_style, theme);
    block.rtl = direction::starts_rtl(trimmed);
    block.spans = spans;
    block.links = links;
    block
}

/// A table, its columns as wide as their widest cell.
fn table_blocks(rows: &[(usize, usize, String)], callout: Option<&str>, theme: &Theme) -> Vec<Block> {
    let cells = |line: &str| -> Vec<String> {
        let inner = line.trim().trim_start_matches('|');
        let inner = inner.strip_suffix('|').unwrap_or(inner);
        let mut out = Vec::new();
        let mut cell = String::new();
        let mut escaped = false;
        // A `|` in a wikilink's alias, `[[a\|b]]`, is escaped.
        for c in inner.chars() {
            if c == '|' && !escaped {
                out.push(cell.trim().to_string());
                cell.clear();
            } else {
                cell.push(c);
            }
            escaped = c == '\\';
        }
        out.push(cell.trim().to_string());
        out
    };
    let parsed: Vec<(usize, usize, Vec<String>)> = rows.iter().map(|(at, depth, line)| (*at, *depth, cells(line))).collect();
    let separator = |row: &[String]| row.iter().all(|c| TABLE_SEPARATOR.is_match(c.trim()));
    let columns = parsed.iter().map(|r| r.2.len()).max().unwrap_or(0);
    let mut widths = vec![0; columns];
    let rendered: Vec<Vec<(Vec<Span<'static>>, Vec<String>)>> = parsed
        .iter()
        .enumerate()
        .map(|(n, (_, _, row))| {
            let style = if n == 0 { Style::new().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::new().fg(theme.text) };
            row.iter().map(|cell| inline(&cell.replace("\\|", "|"), style, theme)).collect()
        })
        .collect();
    for (n, row) in rendered.iter().enumerate() {
        if separator(&parsed[n].2) {
            continue;
        }
        for (column, (spans, _)) in row.iter().enumerate() {
            widths[column] = widths[column].max(width(spans));
        }
    }
    let bar = Style::new().fg(theme.rule);
    let mut out = Vec::new();
    for (n, row) in rendered.into_iter().enumerate() {
        let (at, depth, raw) = &parsed[n];
        let prefix = quote_prefix(*depth, callout, theme);
        if separator(raw) {
            let line = widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┼");
            out.push(Block { spans: vec![Span::styled(line, bar)], prefix, source: Some(*at), ..Block::default() });
            continue;
        }
        let mut spans = Vec::new();
        let mut links = Vec::new();
        for (column, wanted) in widths.iter().enumerate() {
            if column > 0 {
                spans.push(Span::styled("│", bar));
            }
            let (cell, cell_links) = row.get(column).cloned().unwrap_or_default();
            let pad = wanted - width(&cell);
            spans.push(Span::raw(" "));
            spans.extend(cell);
            spans.push(Span::raw(" ".repeat(pad + 1)));
            links.extend(cell_links);
        }
        out.push(Block { spans, prefix, source: Some(*at), links, ..Block::default() });
    }
    out
}

fn width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>()).sum()
}

/// Obsidian's inline syntax as spans on [base]: code, bold, italics,
/// strikes, highlights, wikilinks, links, tags, math and comments. Returns
/// the link targets too, in order.
pub fn inline(text: &str, base: Style, theme: &Theme) -> (Vec<Span<'static>>, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut links = Vec::new();
    let mut plain = String::new();
    let (mut bold, mut italic, mut strike, mut mark) = (false, false, false, false);
    // Colours change only on body text: a heading keeps its own.
    let colored = base.fg == Some(theme.text);
    let style = |bold: bool, italic: bool, strike: bool, mark: bool| {
        let mut s = base;
        if bold {
            s = s.add_modifier(Modifier::BOLD);
            if colored {
                s = s.fg(theme.strong);
            }
        }
        if italic {
            s = s.add_modifier(Modifier::ITALIC);
            if colored && !bold {
                s = s.fg(theme.em);
            }
        }
        if strike {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if mark {
            s = s.fg(theme.highlight_fg).bg(theme.highlight_bg);
        }
        s
    };
    let find = |from: usize, pattern: &str| -> Option<usize> {
        let p: Vec<char> = pattern.chars().collect();
        (from..chars.len().saturating_sub(p.len() - 1)).find(|&i| chars[i..i + p.len()] == p[..])
    };
    let starts = |i: usize, pattern: &str| pattern.chars().enumerate().all(|(n, c)| chars.get(i + n) == Some(&c));
    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                out.push(Span::styled(std::mem::take(&mut plain), style(bold, italic, strike, mark)));
            }
        };
    }
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
            plain.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '`' {
            let ticks = chars[i..].iter().take_while(|&&t| t == '`').count();
            let marker = "`".repeat(ticks);
            if let Some(end) = find(i + ticks, &marker) {
                flush!();
                let code: String = chars[i + ticks..end].iter().collect();
                out.push(Span::styled(code.trim().to_string(), Style::new().fg(theme.code).bg(theme.code_background)));
                i = end + ticks;
                continue;
            }
        }
        if starts(i, "%%")
            && let Some(end) = find(i + 2, "%%")
        {
            i = end + 2;
            continue;
        }
        if starts(i, "[[") || starts(i, "![[") {
            let embed = c == '!';
            let open = if embed { i + 3 } else { i + 2 };
            if let Some(end) = find(open, "]]") {
                flush!();
                let inner: String = chars[open..end].iter().collect();
                let (target, alias) = match inner.split_once('|') {
                    Some((t, a)) => (t.trim_end_matches('\\').to_string(), Some(a.to_string())),
                    None => (inner.clone(), None),
                };
                let file = target.split('#').next().unwrap_or("").trim().to_string();
                let shown = alias.unwrap_or_else(|| target.replace('#', " › ").trim_start_matches(" › ").to_string());
                let shown = if embed { format!("⧉ {shown}") } else { shown };
                out.push(Span::styled(shown, Style::new().fg(theme.link).add_modifier(Modifier::UNDERLINED)));
                if !file.is_empty() {
                    links.push(file);
                }
                i = end + 2;
                continue;
            }
        }
        if (c == '[' || starts(i, "![")) && !starts(i, "[[") {
            let open = if c == '!' { i + 2 } else { i + 1 };
            if let Some(close) = find(open, "](")
                && let Some(end) = find(close + 2, ")")
                && !chars[open..close].contains(&'\n')
            {
                flush!();
                let label: String = chars[open..close].iter().collect();
                let url: String = chars[close + 2..end].iter().collect::<String>().trim_matches(['<', '>']).to_string();
                let shown = if c == '!' { format!("⧉ {}", if label.is_empty() { &url } else { &label }) } else { label };
                out.push(Span::styled(shown, Style::new().fg(theme.link).add_modifier(Modifier::UNDERLINED)));
                links.push(url);
                i = end + 1;
                continue;
            }
        }
        if c == '#' && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            let tag: String = chars[i + 1..].iter().take_while(|t| t.is_alphanumeric() || matches!(t, '_' | '-' | '/')).collect();
            if !tag.is_empty() && !tag.chars().all(|t| t.is_ascii_digit()) {
                flush!();
                let n = tag.chars().count();
                out.push(Span::styled(format!("#{tag}"), Style::new().fg(theme.accent)));
                i += 1 + n;
                continue;
            }
        }
        if c == '$'
            && !starts(i, "$$")
            && i + 1 < chars.len()
            && !chars[i + 1].is_whitespace()
            && let Some(end) = find(i + 1, "$")
            && !chars[end - 1].is_whitespace()
        {
            flush!();
            out.push(Span::styled(chars[i + 1..end].iter().collect::<String>(), Style::new().fg(theme.em)));
            i = end + 1;
            continue;
        }
        // A marker turns its style on only when it closes later on the line.
        if let Some(marker) = ["**", "__", "~~", "=="].into_iter().find(|m| starts(i, m)) {
            let on = match marker {
                "~~" => strike,
                "==" => mark,
                _ => bold,
            };
            if on || find(i + 2, marker).is_some() {
                flush!();
                match marker {
                    "~~" => strike = !strike,
                    "==" => mark = !mark,
                    _ => bold = !bold,
                }
                i += 2;
                continue;
            }
        }
        let word_edge = |at: usize| at == 0 || !chars[at - 1].is_alphanumeric();
        if (c == '*' || (c == '_' && (italic || word_edge(i))))
            && (italic || (i + 1 < chars.len() && !chars[i + 1].is_whitespace() && find(i + 1, &c.to_string()).is_some()))
        {
            flush!();
            italic = !italic;
            i += 1;
            continue;
        }
        if starts(i, "<br>") {
            plain.push(' ');
            i += 4;
            continue;
        }
        plain.push(c);
        i += 1;
    }
    flush!();
    (out, links)
}

/// The blocks as rows [width] cells wide. A block wraps at spaces, and
/// each row it wraps to starts with its hang.
pub fn wrap(blocks: &[Block], width: u16, theme: &Theme) -> Vec<Row> {
    let width = width.max(10) as usize;
    let mut rows = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.rule {
            let line = Line::from(Span::styled("─".repeat(width.saturating_sub(2)), Style::new().fg(theme.rule)));
            rows.push(Row { line, block: index });
            continue;
        }
        let first = self::width(&block.prefix);
        let rest = self::width(&block.hang);
        let cells: Vec<(char, Style)> = block.spans.iter().flat_map(|span| span.content.chars().map(move |c| (c, span.style))).collect();
        let mut pieces: Vec<Vec<(char, Style)>> = Vec::new();
        let mut current: Vec<(char, Style)> = Vec::new();
        let mut used = 0;
        let mut space: Option<usize> = None;
        for cell in cells {
            let room = width.saturating_sub(if pieces.is_empty() { first } else { rest }).max(4);
            let w = cell.0.width().unwrap_or(0);
            if used + w > room && !current.is_empty() {
                let carry = match space {
                    Some(at) if at > 0 => current.split_off(at),
                    _ => Vec::new(),
                };
                pieces.push(std::mem::take(&mut current));
                current = carry.into_iter().skip_while(|(c, _)| *c == ' ').collect();
                used = current.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
                space = None;
            }
            if cell.0 == ' ' {
                space = Some(current.len() + 1);
            }
            used += w;
            current.push(cell);
        }
        pieces.push(current);
        for (n, mut piece) in pieces.into_iter().enumerate() {
            while piece.last().is_some_and(|(c, _)| *c == ' ') && pieces_trim(block) {
                piece.pop();
            }
            let mut spans: Vec<Span<'static>> = if n == 0 { block.prefix.clone() } else { block.hang.clone() };
            let mut text = String::new();
            let mut style: Option<Style> = None;
            for (c, s) in piece {
                if style.is_some_and(|current| current != s) {
                    spans.push(Span::styled(std::mem::take(&mut text), style.unwrap()));
                }
                style = Some(s);
                text.push(c);
            }
            if let Some(style) = style {
                spans.push(Span::styled(text, style));
            }
            let mut line = Line::from(spans);
            if let Some(fill) = block.fill {
                line = line.style(Style::new().bg(fill));
            }
            if block.rtl {
                line = line.right_aligned();
            }
            rows.push(Row { line, block: index });
        }
    }
    rows
}

/// A wrapped row loses the space it broke at; code keeps every space.
fn pieces_trim(block: &Block) -> bool {
    block.fill.is_none()
}

/// The text of a line of spans, for the tests.
#[cfg(test)]
pub fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str, width: u16) -> Vec<String> {
        let theme = Theme::NAZ;
        let parsed = daftar_core::NoteParser::parse(text);
        wrap(&blocks(text, &parsed.properties, &theme, "2026-10-21"), width, &theme).iter().map(|r| plain(&r.line)).collect()
    }

    #[test]
    fn headings_lists_and_tasks() {
        let rows = render("# Title\n\n- one [[Home|home]]\n  - two\n- [ ] Ship it 📅 2026-10-20\n- [x] Done ✅ 2026-10-01\n", 60);
        assert_eq!(rows, ["█ Title", "", "• one home", "  ◦ two", "☐ Ship it 📅 2026-10-20", "☑ Done ✅ 2026-10-01"]);
    }

    #[test]
    fn a_late_due_date_is_red_and_links_are_kept() {
        let theme = Theme::NAZ;
        let text = "- [ ] Call [[People/Ana#Phone|Ana]] about [the plan](Plans/Plan.md) 📅 2026-10-20";
        let blocks = blocks(text, &[], &theme, "2026-10-21");
        assert_eq!(blocks[0].links, ["People/Ana", "Plans/Plan.md"]);
        assert_eq!(blocks[0].task, Some(' '));
        assert_eq!(blocks[0].spans.last().unwrap().style.fg, Some(theme.red));
    }

    #[test]
    fn callouts_quotes_and_code() {
        let text = "> [!warning] Careful\n> Mind the gap\n\n```rust\nfn main() {}\n```\n";
        let rows = render(text, 40);
        assert_eq!(rows, ["▌ ⚠ Careful", "▌ Mind the gap", "", " rust", " fn main() {}", ""]);
    }

    #[test]
    fn tables_line_up() {
        let rows = render("| Name | Age |\n| --- | --- |\n| Ana | 7 |\n| Bartholomew | 12 |\n", 60);
        assert_eq!(rows[0], " Name        │ Age");
        assert_eq!(rows[1], "─────────────┼─────");
        assert_eq!(rows[3], " Bartholomew │ 12");
    }

    #[test]
    fn front_matter_shows_as_properties() {
        let rows = render("---\ntags: [work, plan]\naliases: [P]\ncssclasses: wide\n---\n# Plan\n", 60);
        assert_eq!(rows[0], "│ tags         #work · #plan");
        assert_eq!(rows[1], "│ aliases      P");
        assert!(rows[2].starts_with("───"));
        assert_eq!(rows[3], "█ Plan");
    }

    #[test]
    fn long_lines_wrap_at_spaces_under_their_bullet() {
        let rows = render("- alpha beta gamma delta epsilon", 16);
        assert_eq!(rows, ["• alpha beta", "  gamma delta", "  epsilon"]);
    }

    #[test]
    fn persian_is_right_aligned_and_kept_in_order() {
        let theme = Theme::NAZ;
        let blocks = blocks("سلام دنیا\nhello", &[], &theme, "2026-10-21");
        let rows = wrap(&blocks, 40, &theme);
        assert_eq!(plain(&rows[0].line), "سلام دنیا");
        assert_eq!(rows[0].line.alignment, Some(ratatui::layout::HorizontalAlignment::Right));
        assert_eq!(rows[1].line.alignment, None);
    }

    #[test]
    fn inline_markup() {
        let theme = Theme::NAZ;
        let (spans, _) = inline("a **b** *c* `d` ==e== ~~f~~ #tag 5 * 3 snake_case_name", Style::new().fg(theme.text), &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "a b c d e f #tag 5 * 3 snake_case_name");
        let bold = spans.iter().find(|s| s.content == "b").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let tag = spans.iter().find(|s| s.content == "#tag").unwrap();
        assert_eq!(tag.style.fg, Some(theme.accent));
    }
}
