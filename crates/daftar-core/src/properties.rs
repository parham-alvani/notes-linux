//! A note's front matter as Obsidian's Properties view shows it: a row for
//! each property, with a type. The port of `Properties.swift`. A property the
//! form did not change prints as it was, byte for byte; a changed one is
//! written again in its own style, and the body is never touched. YAML the
//! form cannot show without losing something stays text, with the reason.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Local, Timelike};
use regex::Regex;
use std::sync::LazyLock;

/// A property's value as the form edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    /// `key:` with nothing after it.
    Empty,
    Text(String),
    List(Vec<String>),
}

impl PropertyValue {
    pub fn items(&self) -> Vec<String> {
        match self {
            PropertyValue::Empty => Vec::new(),
            PropertyValue::Text(text) => vec![text.clone()],
            PropertyValue::List(items) => items.clone(),
        }
    }

    fn normal(self) -> PropertyValue {
        if self == PropertyValue::Text(String::new()) { PropertyValue::Empty } else { self }
    }
}

/// Obsidian's property types, as `.obsidian/types.json` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyType {
    Text,
    Multitext,
    Number,
    Checkbox,
    Date,
    Datetime,
    Aliases,
    Tags,
}

static NUMERAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$").unwrap());
static DAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());
static MOMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?Z?$").unwrap());

impl PropertyType {
    pub const ALL: [PropertyType; 8] = [
        PropertyType::Text,
        PropertyType::Multitext,
        PropertyType::Number,
        PropertyType::Checkbox,
        PropertyType::Date,
        PropertyType::Datetime,
        PropertyType::Aliases,
        PropertyType::Tags,
    ];

    /// The name in `types.json`.
    pub fn name(self) -> &'static str {
        match self {
            PropertyType::Text => "text",
            PropertyType::Multitext => "multitext",
            PropertyType::Number => "number",
            PropertyType::Checkbox => "checkbox",
            PropertyType::Date => "date",
            PropertyType::Datetime => "datetime",
            PropertyType::Aliases => "aliases",
            PropertyType::Tags => "tags",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            PropertyType::Text => "Text",
            PropertyType::Multitext => "List",
            PropertyType::Number => "Number",
            PropertyType::Checkbox => "Checkbox",
            PropertyType::Date => "Date",
            PropertyType::Datetime => "Date & time",
            PropertyType::Aliases => "Aliases",
            PropertyType::Tags => "Tags",
        }
    }

    pub fn from_name(name: &str) -> Option<PropertyType> {
        PropertyType::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn is_list(self) -> bool {
        matches!(self, PropertyType::Multitext | PropertyType::Aliases | PropertyType::Tags)
    }

    /// The type Obsidian gives [key]: the one in `types.json`, the type of
    /// `tags`, `aliases` and `cssclasses`, or else the shape of the value.
    pub fn infer(key: &str, value: &PropertyValue, types: &HashMap<String, String>) -> PropertyType {
        let named = types.get(key).or_else(|| types.iter().find(|(k, _)| k.to_lowercase() == key.to_lowercase()).map(|(_, v)| v));
        if let Some(found) = named.and_then(|n| PropertyType::from_name(n)) {
            return found;
        }
        match key.to_lowercase().as_str() {
            "tags" | "tag" => return PropertyType::Tags,
            "aliases" | "alias" => return PropertyType::Aliases,
            "cssclasses" | "cssclass" => return PropertyType::Multitext,
            _ => {}
        }
        match value {
            PropertyValue::Empty => PropertyType::Text,
            PropertyValue::List(_) => PropertyType::Multitext,
            PropertyValue::Text(text) => {
                let lower = text.to_lowercase();
                if lower == "true" || lower == "false" {
                    PropertyType::Checkbox
                } else if NUMERAL.is_match(text) {
                    PropertyType::Number
                } else if DAY.is_match(text) && Self::date(text).is_some() {
                    PropertyType::Date
                } else if MOMENT.is_match(text) && Self::date(text).is_some() {
                    PropertyType::Datetime
                } else {
                    PropertyType::Text
                }
            }
        }
    }

    /// `2026-10-25` or `2026-10-25T09:30`, in the local time zone.
    pub fn date(text: &str) -> Option<DateTime<Local>> {
        crate::bases::parse_date(text).map(|(date, _)| date)
    }

    /// How Obsidian writes a date (`2026-10-25`) or a date and time
    /// (`2026-10-25T09:30`) in the local time zone.
    pub fn format(date: DateTime<Local>, time: bool) -> String {
        let day = format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day());
        if time { format!("{day}T{:02}:{:02}", date.hour(), date.minute()) } else { day }
    }
}

/// What the editor shows for a note's front matter.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontMatterForm {
    /// The note has no front matter; a property added makes one.
    Absent(PropertySheet),
    Form(PropertySheet),
    /// YAML the form cannot show without losing something: the raw text
    /// stays, and the reason says why.
    Source(String),
}

impl FrontMatterForm {
    pub fn sheet(&self) -> Option<&PropertySheet> {
        match self {
            FrontMatterForm::Absent(sheet) | FrontMatterForm::Form(sheet) => Some(sheet),
            FrontMatterForm::Source(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    Plain,
    Single,
    Double,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Style {
    /// Of the scalar, or of the list's first item.
    quote: Quote,
    /// `[a, b]`, not a block of `- a` lines.
    flow: bool,
    /// What goes before a block item's value: `  - `.
    item_prefix: String,
    /// Each item as written, to reuse for an item that did not change.
    items: Vec<(String, String)>,
}

impl Default for Style {
    fn default() -> Style {
        Style { quote: Quote::Plain, flow: false, item_prefix: "  - ".into(), items: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub value: PropertyValue,
    /// The key as written, quotes and all.
    key_text: String,
    /// The lines as written; none once the value changed.
    lines: Option<Vec<String>>,
    /// Blank lines after it, kept where they are.
    trailing: Vec<String>,
    style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyFailure {
    Exists(String),
    BadName,
}

impl std::fmt::Display for PropertyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyFailure::Exists(key) => write!(f, "The note has a property “{key}” already."),
            PropertyFailure::BadName => write!(f, "A property needs a name."),
        }
    }
}

/// The front matter as a list of properties, each with the lines it was
/// written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertySheet {
    properties: Vec<Property>,
    /// The front matter as read, fences included: an edit is applied only
    /// to a note that still starts with it.
    pub original: String,
    open: String,
    close: String,
    close_newline: bool,
    leading: Vec<String>,
    eol: &'static str,
    edited: bool,
}

/// YAML the form cannot show, and why.
struct Unrepresentable(String);

fn unrepresentable<T>(reason: impl Into<String>) -> Result<T, Unrepresentable> {
    Err(Unrepresentable(reason.into()))
}

const INDICATORS: [char; 19] = ['-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`'];

impl PropertySheet {
    // Reading

    /// The front matter of [note], as a form when every line of it fits one.
    pub fn read(note: &str) -> FrontMatterForm {
        let Some(block) = FrontMatterBlock::new(note) else {
            let mut sheet = PropertySheet::empty(String::new());
            if note.contains("\r\n") {
                sheet.eol = "\r";
                sheet.open = "---\r".into();
                sheet.close = "---\r".into();
            }
            return FrontMatterForm::Absent(sheet);
        };
        match PropertySheet::from_block(&block) {
            Ok(sheet) => FrontMatterForm::Form(sheet),
            Err(Unrepresentable(reason)) => FrontMatterForm::Source(reason),
        }
    }

    /// Where the front matter ends, in characters, as the editor counts:
    /// the fences and the newline after the closing one. 0 with none.
    pub fn length(note: &str) -> usize {
        FrontMatterBlock::new(note).map_or(0, |block| block.text.chars().count())
    }

    fn empty(original: String) -> PropertySheet {
        PropertySheet {
            properties: Vec::new(),
            original,
            open: "---".into(),
            close: "---".into(),
            close_newline: true,
            leading: Vec::new(),
            eol: "",
            edited: false,
        }
    }

    pub fn properties(&self) -> &[Property] {
        &self.properties
    }

    fn from_block(block: &FrontMatterBlock) -> Result<PropertySheet, Unrepresentable> {
        let mut sheet = PropertySheet::empty(block.text.clone());
        sheet.open.clone_from(&block.open);
        sheet.close.clone_from(&block.close);
        sheet.close_newline = block.close_newline;
        sheet.eol = if sheet.open.ends_with('\r') { "\r" } else { "" };
        let lines = &block.lines;
        let mut at = 0;
        while at < lines.len() && is_blank(&lines[at]) {
            sheet.leading.push(lines[at].clone());
            at += 1;
        }
        while at < lines.len() {
            let line = &lines[at];
            if is_blank(line) {
                if let Some(last) = sheet.properties.last_mut() {
                    last.trailing.push(line.clone());
                }
                at += 1;
                continue;
            }
            let text = content(line);
            if text.starts_with([' ', '\t']) {
                return unrepresentable(if split_key(text.trim_matches([' ', '\t'])).is_some() {
                    "It has a map inside a map."
                } else {
                    "A value goes on over more than one line."
                });
            }
            if text.starts_with('#') {
                return unrepresentable("It has a comment.");
            }
            if text.starts_with("- ") || text == "-" {
                return unrepresentable("It is a list, not a list of properties.");
            }
            let Some((key_text, key, rest)) = split_key(text) else {
                return unrepresentable(format!("The line “{text}” is not a property."));
            };
            if sheet.properties.iter().any(|p| p.key == key) {
                return unrepresentable(format!("The property “{key}” is there twice."));
            }
            let mut property = Property {
                key: key.clone(),
                value: PropertyValue::Empty,
                key_text,
                lines: Some(vec![line.clone()]),
                trailing: Vec::new(),
                style: Style::default(),
            };
            at += 1;
            if rest.is_empty() {
                // A block list, under the key or at the key's own indent.
                let mut items: Vec<(String, String)> = Vec::new();
                let mut prefix: Option<String> = None;
                while at < lines.len() && !is_blank(&lines[at]) {
                    let item = content(&lines[at]);
                    let indent: String = item.chars().take_while(|&c| c == ' ').collect();
                    let after = &item[indent.len()..];
                    if !(after.starts_with("- ") || after == "-") {
                        if indent.is_empty() {
                            break;
                        }
                        return unrepresentable(if split_key(after).is_some() {
                            format!("“{key}” holds a map inside a map.")
                        } else {
                            format!("“{key}” goes on over more than one line.")
                        });
                    }
                    let spaces: String = after[1..].chars().take_while(|&c| c == ' ').collect();
                    let item_prefix = format!("{indent}-{spaces}");
                    if let Some(prefix) = &prefix
                        && prefix.chars().take_while(|&c| c == ' ').count() != indent.len()
                    {
                        return unrepresentable(format!("The list “{key}” has items at different indents."));
                    }
                    if prefix.is_none() {
                        prefix = Some(if spaces.is_empty() { format!("{indent}- ") } else { item_prefix });
                    }
                    let text = &after[1 + spaces.len()..];
                    if text.starts_with("- ") || text == "-" {
                        return unrepresentable(format!("“{key}” holds a list inside a list."));
                    }
                    if split_key(text).is_some() {
                        return unrepresentable(format!("“{key}” holds a map inside a list."));
                    }
                    let (value, quote) = scalar(text, &key)?;
                    if items.is_empty() {
                        property.style.quote = quote;
                    }
                    items.push((value, text.to_string()));
                    if let Some(lines_of) = property.lines.as_mut() {
                        lines_of.push(lines[at].clone());
                    }
                    at += 1;
                }
                // The item's own value may go on, more indented, on the next line.
                if at < lines.len() && !is_blank(&lines[at]) && content(&lines[at]).starts_with(' ') {
                    return unrepresentable(format!("An item of “{key}” goes on over more than one line."));
                }
                if let Some(prefix) = prefix {
                    property.value = PropertyValue::List(items.iter().map(|(v, _)| v.clone()).collect());
                    property.style.item_prefix = prefix;
                    property.style.items = items;
                }
            } else if rest.starts_with('[') {
                if !rest.ends_with(']') {
                    return unrepresentable(if rest.contains(" #") {
                        "It has a comment.".to_string()
                    } else {
                        format!("The list “{key}” does not close on its line.")
                    });
                }
                let inner = rest[1..rest.len() - 1].trim_matches([' ', '\t']);
                let mut items: Vec<(String, String)> = Vec::new();
                let texts = if inner.is_empty() { Vec::new() } else { split_flow(inner) };
                for text in texts {
                    if text.starts_with('[') || text.starts_with('{') {
                        return unrepresentable(format!("“{key}” holds a list inside a list; a link there needs quotes."));
                    }
                    let (value, quote) = scalar(&text, &key)?;
                    if items.is_empty() {
                        property.style.quote = quote;
                    }
                    items.push((value, text));
                }
                property.value = PropertyValue::List(items.iter().map(|(v, _)| v.clone()).collect());
                property.style.flow = true;
                property.style.items = items;
            } else {
                let (value, quote) = scalar(&rest, &key)?;
                property.value = PropertyValue::Text(value);
                property.style.quote = quote;
            }
            sheet.properties.push(property);
        }
        Ok(sheet)
    }

    // Edits

    pub fn value(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    /// A new value for [key]; nothing when it has that value already.
    pub fn set(&mut self, key: &str, value: PropertyValue) {
        let value = value.normal();
        let Some(property) = self.properties.iter_mut().find(|p| p.key == key) else { return };
        if property.value == value {
            return;
        }
        property.value = value;
        property.lines = None;
        self.edited = true;
    }

    /// A new property, after the others.
    pub fn add(&mut self, key: &str, value: PropertyValue) -> Result<(), PropertyFailure> {
        let name = key.trim_matches([' ', '\t']);
        if name.is_empty() || name.contains(['\n', '\r']) {
            return Err(PropertyFailure::BadName);
        }
        if self.properties.iter().any(|p| p.key == name) {
            return Err(PropertyFailure::Exists(name.to_string()));
        }
        let property = Property {
            key: name.to_string(),
            value: value.normal(),
            key_text: key_text(name)?,
            lines: None,
            trailing: Vec::new(),
            style: Style::default(),
        };
        self.properties.push(property);
        self.edited = true;
        Ok(())
    }

    pub fn remove(&mut self, key: &str) {
        let Some(at) = self.properties.iter().position(|p| p.key == key) else { return };
        let gone = self.properties.remove(at);
        // Its blank lines stay with the property before it.
        if at > 0 {
            self.properties[at - 1].trailing.extend(gone.trailing);
        } else {
            self.leading.extend(gone.trailing);
        }
        self.edited = true;
    }

    /// Renames [key] in place; its value and its lines stay as they are.
    pub fn rename(&mut self, key: &str, new: &str) -> Result<(), PropertyFailure> {
        let name = new.trim_matches([' ', '\t']);
        if name.is_empty() || name.contains(['\n', '\r']) {
            return Err(PropertyFailure::BadName);
        }
        let Some(at) = self.properties.iter().position(|p| p.key == key) else { return Ok(()) };
        if name == key {
            return Ok(());
        }
        if self.properties.iter().any(|p| p.key == name) {
            return Err(PropertyFailure::Exists(name.to_string()));
        }
        let text = key_text(name)?;
        let property = &mut self.properties[at];
        match property.lines.as_mut() {
            Some(lines) if lines.first().is_some_and(|first| first.starts_with(&property.key_text)) => {
                lines[0] = format!("{text}{}", &lines[0][property.key_text.len()..]);
            }
            _ => property.lines = None,
        }
        property.key_text = text;
        property.key = name.to_string();
        self.edited = true;
        Ok(())
    }

    // Writing

    /// The front matter as it prints now: [original] when nothing changed,
    /// nothing when the last property went.
    pub fn text(&self) -> String {
        if !self.edited {
            return self.original.clone();
        }
        if self.properties.is_empty() {
            return String::new();
        }
        let mut lines = vec![self.open.clone()];
        lines.extend(self.leading.iter().cloned());
        for property in &self.properties {
            match &property.lines {
                Some(written) => lines.extend(written.iter().cloned()),
                None => lines.extend(lines_of(property).into_iter().map(|l| l + self.eol)),
            }
            lines.extend(property.trailing.iter().cloned());
        }
        lines.push(self.close.clone());
        lines.join("\n") + if self.close_newline { "\n" } else { "" }
    }

    /// [note] with this front matter, or none when the note's front matter
    /// is no longer the one this sheet was read from.
    pub fn applied(&self, note: &str) -> Option<String> {
        let now = FrontMatterBlock::new(note).map(|b| b.text).unwrap_or_default();
        if now != self.original {
            return None;
        }
        Some(self.text() + &note[now.len()..])
    }
}

/// One change the form makes, as the app's `win.set-property` action
/// carries it from the inspector to the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyEdit {
    Set(String, PropertyValue),
    Add(String, PropertyValue),
    Remove(String),
    Rename(String, String),
}

/// Between the parts of an encoded edit, and between a list's items: no
/// property and no value holds them.
const PART: char = '\u{1e}';
const ITEM: char = '\u{1f}';

impl PropertyEdit {
    /// The edit as one string: the operation, the key, and the value.
    pub fn encode(&self) -> String {
        let value = |value: &PropertyValue| match value {
            PropertyValue::Empty => "empty".to_string(),
            PropertyValue::Text(text) => format!("text{PART}{text}"),
            PropertyValue::List(items) => format!("list{PART}{}", items.join(&ITEM.to_string())),
        };
        match self {
            PropertyEdit::Set(key, v) => format!("set{PART}{key}{PART}{}", value(v)),
            PropertyEdit::Add(key, v) => format!("add{PART}{key}{PART}{}", value(v)),
            PropertyEdit::Remove(key) => format!("remove{PART}{key}"),
            PropertyEdit::Rename(key, to) => format!("rename{PART}{key}{PART}{to}"),
        }
    }

    pub fn decode(text: &str) -> Option<PropertyEdit> {
        let parts: Vec<&str> = text.splitn(4, PART).collect();
        let value = || match parts.get(2..) {
            Some(["empty"]) => Some(PropertyValue::Empty),
            Some(["text", text]) => Some(PropertyValue::Text((*text).to_string())),
            Some(["list", ""]) => Some(PropertyValue::List(Vec::new())),
            Some(["list", items]) => Some(PropertyValue::List(items.split(ITEM).map(str::to_string).collect())),
            _ => None,
        };
        let key = (*parts.get(1)?).to_string();
        match parts[0] {
            "set" => Some(PropertyEdit::Set(key, value()?)),
            "add" => Some(PropertyEdit::Add(key, value()?)),
            "remove" if parts.len() == 2 => Some(PropertyEdit::Remove(key)),
            "rename" if parts.len() == 3 => Some(PropertyEdit::Rename(key, parts[2].to_string())),
            _ => None,
        }
    }
}

impl PropertySheet {
    /// [note] with [edit] made to its front matter, or why it cannot be:
    /// front matter the form cannot show, or a name that is taken.
    pub fn edit(note: &str, edit: &PropertyEdit) -> Result<String, String> {
        let mut sheet = match PropertySheet::read(note) {
            FrontMatterForm::Absent(sheet) | FrontMatterForm::Form(sheet) => sheet,
            FrontMatterForm::Source(reason) => return Err(reason),
        };
        match edit {
            PropertyEdit::Set(key, value) => sheet.set(key, value.clone()),
            PropertyEdit::Add(key, value) => sheet.add(key, value.clone()).map_err(|e| e.to_string())?,
            PropertyEdit::Remove(key) => sheet.remove(key),
            PropertyEdit::Rename(key, to) => sheet.rename(key, to).map_err(|e| e.to_string())?,
        }
        sheet.applied(note).ok_or_else(|| "The front matter changed meanwhile.".to_string())
    }
}

fn content(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn is_blank(line: &str) -> bool {
    line.chars().all(|c| matches!(c, ' ' | '\t' | '\r'))
}

/// `key: value`: the key as written, the key, and the rest of the line.
fn split_key(text: &str) -> Option<(String, String, String)> {
    let first = text.chars().next()?;
    if first == '"' || first == '\'' {
        let close = text[1..].find(first)? + 1;
        let name = &text[1..close];
        let after = &text[close + 1..];
        if !after.starts_with(':') || !(after.len() == 1 || after[1..].starts_with(' ')) || name.contains('\\') {
            return None;
        }
        return Some((text[..=close].to_string(), name.to_string(), after[1..].trim_matches([' ', '\t']).to_string()));
    }
    if INDICATORS.contains(&first) {
        return None;
    }
    let mut from = 0;
    while let Some(found) = text[from..].find(':') {
        let colon = from + found;
        let next = &text[colon + 1..];
        // A scalar, not a letter: a space and a combining mark after it are one letter.
        if next.is_empty() || next.starts_with([' ', '\t']) {
            let key = &text[..colon];
            if key != key.trim_matches([' ', '\t']) || key.is_empty() {
                return None;
            }
            return Some((key.to_string(), key.to_string(), next.trim_matches([' ', '\t']).to_string()));
        }
        from = colon + 1;
    }
    None
}

/// One scalar on one line, and how it was quoted.
fn scalar(text: &str, key: &str) -> Result<(String, Quote), Unrepresentable> {
    let Some(first) = text.chars().next() else { return Ok((String::new(), Quote::Plain)) };
    let open = || {
        if text.contains(" #") {
            Unrepresentable("It has a comment.".into())
        } else {
            Unrepresentable(format!("A quote in “{key}” does not close on its line."))
        }
    };
    match first {
        '\'' => {
            if text.chars().count() < 2 || !text.ends_with('\'') {
                return Err(open());
            }
            let inner = &text[1..text.len() - 1];
            // A lone quote inside ends the string early: what follows is not part of it.
            if inner.replace("''", "").contains('\'') {
                return Err(open());
            }
            Ok((inner.replace("''", "'"), Quote::Single))
        }
        '"' => {
            if text.chars().count() < 2 || !text.ends_with('"') {
                return Err(open());
            }
            let mut out = String::new();
            let mut escaped = false;
            for c in text[1..text.len() - 1].chars() {
                if escaped {
                    match c {
                        '\\' | '"' | '/' => out.push(c),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        _ => return unrepresentable(format!("“{key}” has an escape the form does not know.")),
                    }
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    return Err(open());
                } else {
                    out.push(c);
                }
            }
            if escaped {
                return Err(open());
            }
            Ok((out, Quote::Double))
        }
        '&' | '*' => unrepresentable(format!("It has an anchor or an alias (“{key}”).")),
        '!' => unrepresentable(format!("“{key}” has a YAML tag.")),
        '|' | '>' => unrepresentable(format!("“{key}” is text on more than one line.")),
        '{' => unrepresentable(format!("“{key}” holds a map inside a map.")),
        '%' | '@' | '`' => unrepresentable(format!("“{key}” starts with a character YAML keeps for itself.")),
        _ => {
            if text.contains(" #") || text.contains("\t#") {
                return unrepresentable("It has a comment.");
            }
            if text.contains(": ") || text.ends_with(':') {
                return unrepresentable(format!("“{key}” has a colon that YAML reads as a map."));
            }
            Ok((text.trim_matches([' ', '\t']).to_string(), Quote::Plain))
        }
    }
}

/// The items of a flow list, split at the commas outside quotes and brackets.
fn split_flow(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in text.chars() {
        if let Some(open) = quote {
            // `\"` does not close a double-quoted item.
            if escaped {
                escaped = false;
            } else if open == '"' && c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
        } else if (c == '"' || c == '\'') && current.chars().all(|c| c == ' ') {
            // Only at an item's start: `it's` is plain text.
            quote = Some(c);
        } else if c == '[' || c == '{' {
            depth += 1;
        } else if c == ']' || c == '}' {
            depth -= 1;
        } else if c == ',' && depth == 0 {
            items.push(current.trim_matches([' ', '\t']).to_string());
            current.clear();
            continue;
        }
        current.push(c);
    }
    items.push(current.trim_matches([' ', '\t']).to_string());
    items
}

fn lines_of(property: &Property) -> Vec<String> {
    let key = format!("{}:", property.key_text);
    let style = &property.style;
    match &property.value {
        PropertyValue::Empty => vec![key],
        PropertyValue::Text(text) if text.is_empty() => vec![key],
        PropertyValue::Text(text) => vec![format!("{key} {}", quoted(text, style.quote, false))],
        PropertyValue::List(items) => {
            let mut unused = style.items.clone();
            let texts: Vec<String> = items
                .iter()
                .map(|item| match unused.iter().position(|(value, _)| value == item) {
                    Some(at) => unused.remove(at).1,
                    None => quoted(item, style.quote, style.flow),
                })
                .collect();
            if style.flow {
                return vec![format!("{key} [{}]", texts.join(", "))];
            }
            if texts.is_empty() {
                return vec![key];
            }
            std::iter::once(key).chain(texts.into_iter().map(|t| format!("{}{t}", style.item_prefix))).collect()
        }
    }
}

/// A key as YAML reads it back: plain, or in quotes that hold no escape.
fn key_text(name: &str) -> Result<String, PropertyFailure> {
    if let Some(first) = name.chars().next()
        && !INDICATORS.contains(&first)
        && !name.contains(": ")
        && !name.ends_with(':')
        && !name.contains(" #")
        && !name.contains(['\n', '\r', '\t'])
    {
        return Ok(name.to_string());
    }
    if !name.contains('"') && !name.contains('\\') {
        return Ok(format!("\"{name}\""));
    }
    if !name.contains('\'') {
        return Ok(format!("'{name}'"));
    }
    Err(PropertyFailure::BadName)
}

/// [text] in the style asked for, or in double quotes when plain text would
/// read as something else.
pub fn quoted(text: &str, quote: Quote, flow: bool) -> String {
    match quote {
        Quote::Single if !text.contains(['\n', '\r', '\t']) => format!("'{}'", text.replace('\'', "''")),
        Quote::Plain if is_plain(text, flow) => text.to_string(),
        _ => format!("\"{}\"", escape(text)),
    }
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"").replace('\r', "\\r").replace('\n', "\\n").replace('\t', "\\t")
}

fn is_plain(text: &str, flow: bool) -> bool {
    let (Some(first), Some(last)) = (text.chars().next(), text.chars().last()) else { return false };
    if first == ' ' || last == ' ' {
        return false;
    }
    if INDICATORS.contains(&first) && !(first == '-' && text.chars().nth(1).is_some_and(|c| c != ' ')) {
        return false;
    }
    if ["null", "Null", "NULL", "~"].contains(&text) {
        return false;
    }
    if text.contains(": ") || text.ends_with(':') || text.contains(" #") {
        return false;
    }
    if text.chars().any(|c| c == '\n' || c == '\r' || c == '\t' || (c.is_ascii() && (c as u32) < 32)) {
        return false;
    }
    if flow && text.contains([',', '[', ']', '{', '}']) {
        return false;
    }
    true
}

/// The front matter's fences and lines in a note, as written: a first line
/// of `---` and the next `---` or `...` line.
struct FrontMatterBlock {
    /// From the start through the closing fence and its newline.
    text: String,
    open: String,
    close: String,
    close_newline: bool,
    /// The lines between the fences, each with its `\r` if it had one.
    lines: Vec<String>,
}

impl FrontMatterBlock {
    fn new(note: &str) -> Option<FrontMatterBlock> {
        let mut lines = Vec::new();
        let mut opening = "";
        let mut start = 0;
        let mut first = true;
        while start < note.len() {
            let end = note[start..].find('\n').map_or(note.len(), |n| start + n);
            let line = &note[start..end];
            let bare = content(line).trim_matches([' ', '\t']);
            if first {
                if bare != "---" {
                    return None;
                }
                opening = line;
                first = false;
            } else if bare == "---" || bare == "..." {
                let close_newline = end < note.len();
                return Some(FrontMatterBlock {
                    text: note[..if close_newline { end + 1 } else { end }].to_string(),
                    open: opening.to_string(),
                    close: line.to_string(),
                    close_newline,
                    lines,
                });
            } else {
                lines.push(line.to_string());
            }
            if end >= note.len() {
                break;
            }
            start = end + 1;
        }
        None
    }
}

/// The property names used in the vault, the most used first, for the
/// completion of a new property's name.
pub struct PropertyNames;

impl PropertyNames {
    pub fn ranked<'a>(keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for key in keys {
            *counts.entry(key).or_default() += 1;
        }
        let mut names: Vec<(&str, usize)> = counts.into_iter().collect();
        names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        names.into_iter().map(|(name, _)| name.to_string()).collect()
    }

    /// The names that start with [prefix], then those that hold it.
    pub fn complete(prefix: &str, names: &[String], limit: usize) -> Vec<String> {
        let query = prefix.trim_matches([' ', '\t']).to_lowercase();
        if query.is_empty() {
            return names.iter().take(limit).cloned().collect();
        }
        let starts = names.iter().filter(|n| n.to_lowercase().starts_with(&query) && n.to_lowercase() != query);
        let holds = names.iter().filter(|n| !n.to_lowercase().starts_with(&query) && n.to_lowercase().contains(&query));
        starts.chain(holds).take(limit).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(note: &str) -> PropertySheet {
        match PropertySheet::read(note) {
            FrontMatterForm::Form(sheet) => sheet,
            other => panic!("not a form: {other:?}"),
        }
    }

    const REPRESENTABLE: [&str; 16] = [
        "---\ntitle: Plain\n---\nBody",
        "---\ntitle: 'single ''quoted'''\nother: \"double \\\"quoted\\\" \\\\ x\"\n---\n",
        "---\ntags: [a, \"b c\", 'd']\naliases: []\n---\n# Heading\n",
        "---\ntags:\n  - a\n  - b\naliases:\n- one\n- 'two'\n---\n",
        "---\ntags:\n    -   spaced\n    -   items\n---\n",
        "---\nempty:\nnext: 1\n---\n",
        "---\n\ntitle: blank lines kept\n\n\nother: x\n\n---\n",
        "---\r\ntitle: crlf\r\ntags:\r\n  - a\r\n---\r\nBody\r\n",
        "---\ntitle: no newline after the fence\n---",
        "---\n---\nNo properties",
        "---\ndone: true\ncount: 12\nratio: -0.5\nday: 2026-10-25\nwhen: 2026-10-25T09:30\nup: \"[[Parent]]\"\n---\n",
        "---\n\"quoted key\": v\n'single key': w\ndue date: 2026-01-01\n---\n",
        "---\nurl: https://example.com/a?b=c#frag\ntime: 09:30\n---\n",
        "---\nعنوان: سلام دنیا\nتگ‌ها: [کار, خانه]\n---\nمتن\n",
        "---\ntitle: trailing spaces   \n...\nBody with --- in it\n---\n",
        "---\nlist:\n  -\n  - x\n---\n",
    ];

    #[test]
    fn a_form_reads_and_prints_the_same_bytes() {
        for note in REPRESENTABLE {
            let sheet = form(note);
            assert_eq!(sheet.text(), sheet.original, "{note:?}");
            assert_eq!(sheet.applied(note).as_deref(), Some(note));
            // A value set to what it is changes nothing.
            let mut same = sheet.clone();
            for property in sheet.properties() {
                same.set(&property.key, property.value.clone());
            }
            assert_eq!(same.applied(note).as_deref(), Some(note));
        }
    }

    #[test]
    fn yaml_the_form_cannot_show_stays_raw_and_says_why() {
        let raw = [
            ("---\n# a comment\ntitle: x\n---\n", "comment"),
            ("---\ntitle: x # a comment\n---\n", "comment"),
            ("---\nbase: &anchor x\nother: *anchor\n---\n", "anchor"),
            ("---\nmap:\n  inner: 1\n---\n", "map inside a map"),
            ("---\ntext: |\n  line one\n  line two\n---\n", "more than one line"),
            ("---\ntext: >\n  folded\n---\n", "more than one line"),
            ("---\ntext: plain\n  goes on\n---\n", "more than one line"),
            ("---\ntext: \"open\n  quote\"\n---\n", "does not close"),
            ("---\nlist:\n  - a: 1\n---\n", "map inside a list"),
            ("---\nlist: [[Link]]\n---\n", "list inside a list"),
            ("---\nflow: {a: 1}\n---\n", "map inside a map"),
            ("---\ntitle: x\ntitle: y\n---\n", "twice"),
            ("---\n- a\n- b\n---\n", "list"),
            ("---\ntagged: !!str x\n---\n", "YAML tag"),
            ("---\njust text\n---\n", "not a property"),
            ("---\nlist:\n  - a\n    more\n---\n", "more than one line"),
        ];
        for (note, why) in raw {
            match PropertySheet::read(note) {
                FrontMatterForm::Source(reason) => assert!(reason.contains(why), "{note:?}: {reason}"),
                other => panic!("a form for {note:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_note_without_front_matter_gets_one_when_a_property_is_added() {
        let FrontMatterForm::Absent(mut sheet) = PropertySheet::read("# Title\nBody") else { panic!("front matter found") };
        assert_eq!(sheet.applied("# Title\nBody").as_deref(), Some("# Title\nBody"));
        sheet.add("tags", PropertyValue::List(vec!["a".into()])).unwrap();
        assert_eq!(sheet.applied("# Title\nBody").as_deref(), Some("---\ntags:\n  - a\n---\n# Title\nBody"));
        let FrontMatterForm::Absent(mut crlf) = PropertySheet::read("Body\r\n") else { panic!("front matter found") };
        crlf.add("title", PropertyValue::Text("x".into())).unwrap();
        assert_eq!(crlf.applied("Body\r\n").as_deref(), Some("---\r\ntitle: x\r\n---\r\nBody\r\n"));
        // The note got front matter in the meantime: the edit is refused.
        assert_eq!(sheet.applied("---\nx: 1\n---\n# Title\nBody"), None);
    }

    #[test]
    fn an_edit_keeps_the_style_of_every_other_line() {
        let note = "---\ntitle: 'Single'\ntags: [one, \"two\"]\naliases:\n    - First\n    - \"Second\"\ncount: 3\n---\n# Body stays\ntags: [not, front, matter]";
        let mut sheet = form(note);
        sheet.set("title", PropertyValue::Text("It's new".into()));
        sheet.set("tags", PropertyValue::List(vec!["one".into(), "two".into(), "three, four".into()]));
        sheet.set("aliases", PropertyValue::List(vec!["Second".into(), "Third".into()]));
        assert_eq!(
            sheet.applied(note).unwrap(),
            "---\ntitle: 'It''s new'\ntags: [one, \"two\", \"three, four\"]\naliases:\n    - \"Second\"\n    - Third\ncount: 3\n---\n# Body stays\ntags: [not, front, matter]"
        );
    }

    #[test]
    fn rename_remove_and_add_keep_the_order() {
        let note = "---\na: 1\nb: [x, y]\n\nc: 'z'\n---\nbody";
        let mut sheet = form(note);
        sheet.rename("b", "due date").unwrap();
        assert_eq!(sheet.applied(note).unwrap(), "---\na: 1\ndue date: [x, y]\n\nc: 'z'\n---\nbody");
        assert_eq!(sheet.rename("c", "a"), Err(PropertyFailure::Exists("a".into())));
        assert_eq!(sheet.add("  ", PropertyValue::Empty), Err(PropertyFailure::BadName));
        sheet.rename("c", "key: with colon").unwrap();
        sheet.remove("a");
        sheet.add("done", PropertyValue::Text("true".into())).unwrap();
        sheet.add("empty", PropertyValue::Empty).unwrap();
        sheet.add("link", PropertyValue::Text("[[Note]]".into())).unwrap();
        sheet.add("nothing", PropertyValue::Text("null".into())).unwrap();
        assert_eq!(
            sheet.applied(note).unwrap(),
            "---\ndue date: [x, y]\n\n\"key: with colon\": 'z'\ndone: true\nempty:\nlink: \"[[Note]]\"\nnothing: \"null\"\n---\nbody"
        );
        let keys: Vec<String> = sheet.properties().iter().map(|p| p.key.clone()).collect();
        for key in keys {
            sheet.remove(&key);
        }
        assert_eq!(sheet.applied(note).unwrap(), "body");
    }

    #[test]
    fn an_edit_is_refused_when_the_front_matter_changed_on_disk() {
        let note = "---\ntitle: a\n---\nbody";
        let mut sheet = form(note);
        sheet.set("title", PropertyValue::Text("b".into()));
        assert_eq!(
            sheet.applied("---\ntitle: a\n---\nthe body changed, that is fine").unwrap(),
            "---\ntitle: b\n---\nthe body changed, that is fine"
        );
        assert_eq!(sheet.applied("---\ntitle: c\n---\nbody"), None);
        assert_eq!(sheet.applied("body"), None);
    }

    #[test]
    fn a_written_value_reads_back_the_same() {
        let values = [
            "plain",
            "with: colon",
            "# hash",
            "- dash",
            "-5",
            "it's",
            "say \"hi\"",
            "back\\slash",
            " padded ",
            "a #b",
            "[[Link]]",
            "null",
            "~",
            "{x}",
            "&amp",
            "*star",
            "%",
            "سلام",
            "tab\there",
            "a,b",
            "",
        ];
        for quote in [Quote::Plain, Quote::Single, Quote::Double] {
            for value in values.iter().filter(|v| !v.is_empty()) {
                for flow in [false, true] {
                    let text = quoted(value, quote, flow);
                    let Ok((read, _)) = scalar(&text, "k") else { panic!("{quote:?} {flow}: {text} does not read") };
                    assert_eq!(&read, value, "{quote:?} {flow}: {text}");
                }
            }
        }
        let mut sheet = form("---\nlist: [a]\nblock:\n  - a\n---\n");
        let list = PropertyValue::List(values.iter().map(ToString::to_string).collect());
        sheet.set("list", list.clone());
        sheet.set("block", list.clone());
        let again = form(&sheet.text());
        assert_eq!(again.value("list"), Some(&list));
        assert_eq!(again.value("block"), Some(&list));
    }

    #[test]
    fn types_are_inferred_as_obsidian_does() {
        let none = HashMap::new();
        let text = |t: &str| PropertyValue::Text(t.into());
        assert_eq!(PropertyType::infer("tags", &text("a"), &none), PropertyType::Tags);
        assert_eq!(PropertyType::infer("Aliases", &PropertyValue::List(vec![]), &none), PropertyType::Aliases);
        assert_eq!(PropertyType::infer("cssclasses", &text("wide"), &none), PropertyType::Multitext);
        assert_eq!(PropertyType::infer("list", &PropertyValue::List(vec!["a".into()]), &none), PropertyType::Multitext);
        assert_eq!(PropertyType::infer("done", &text("true"), &none), PropertyType::Checkbox);
        assert_eq!(PropertyType::infer("done", &text("False"), &none), PropertyType::Checkbox);
        assert_eq!(PropertyType::infer("count", &text("12"), &none), PropertyType::Number);
        assert_eq!(PropertyType::infer("count", &text("-1.5e3"), &none), PropertyType::Number);
        assert_eq!(PropertyType::infer("day", &text("2026-10-25"), &none), PropertyType::Date);
        assert_eq!(PropertyType::infer("when", &text("2026-10-25T09:30"), &none), PropertyType::Datetime);
        assert_eq!(PropertyType::infer("when", &text("2026-10-25 09:30:00"), &none), PropertyType::Datetime);
        assert_eq!(PropertyType::infer("title", &text("2026 plans"), &none), PropertyType::Text);
        assert_eq!(PropertyType::infer("title", &PropertyValue::Empty, &none), PropertyType::Text);
    }

    #[test]
    fn types_json_wins() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        std::fs::write(
            root.path().join(".obsidian/types.json"),
            r#"{"types":{"year":"text","rating":"number","related":"multitext","seen":"checkbox","kind":"unknown"}}"#,
        )
        .unwrap();
        let types = crate::bases::BaseEngine::types(root.path());
        let text = |t: &str| PropertyValue::Text(t.into());
        assert_eq!(PropertyType::infer("year", &text("2026"), &types), PropertyType::Text);
        assert_eq!(PropertyType::infer("Rating", &text("x"), &types), PropertyType::Number);
        assert_eq!(PropertyType::infer("related", &text("[[A]]"), &types), PropertyType::Multitext);
        assert_eq!(PropertyType::infer("seen", &PropertyValue::Empty, &types), PropertyType::Checkbox);
        assert_eq!(PropertyType::infer("kind", &text("12"), &types), PropertyType::Number);
    }

    #[test]
    fn dates_are_written_as_obsidian_writes_them() {
        let date = PropertyType::date("2026-10-25T09:30").unwrap();
        assert_eq!(PropertyType::format(date, true), "2026-10-25T09:30");
        assert_eq!(PropertyType::format(date, false), "2026-10-25");
    }

    /// The Mac counts UTF-16 units, as its text view does; here, characters.
    #[test]
    fn the_front_matter_length_is_in_characters() {
        assert_eq!(PropertySheet::length("no front matter"), 0);
        assert_eq!(PropertySheet::length("---\nt: 😀\n---\nbody"), "---\nt: 😀\n---\n".chars().count());
        assert_eq!(PropertySheet::length("---\nopen"), 0);
    }

    #[test]
    fn an_edit_goes_through_one_string_and_back() {
        let edits = [
            PropertyEdit::Set("due date".into(), PropertyValue::Text("2026-10-25".into())),
            PropertyEdit::Set("tags".into(), PropertyValue::List(vec!["a, b".into(), "c".into()])),
            PropertyEdit::Set("tags".into(), PropertyValue::List(Vec::new())),
            PropertyEdit::Add("done".into(), PropertyValue::Empty),
            PropertyEdit::Remove("x".into()),
            PropertyEdit::Rename("a".into(), "b: c".into()),
        ];
        for edit in edits {
            assert_eq!(PropertyEdit::decode(&edit.encode()), Some(edit));
        }
        assert_eq!(PropertyEdit::decode("set"), None);
        assert_eq!(PropertyEdit::decode("drop\u{1e}x"), None);
    }

    #[test]
    fn an_edit_is_made_to_a_note_or_says_why_not() {
        let note = "---\ntitle: a\n---\nbody";
        let set = PropertyEdit::Set("title".into(), PropertyValue::Text("b".into()));
        assert_eq!(PropertySheet::edit(note, &set).unwrap(), "---\ntitle: b\n---\nbody");
        let add = PropertyEdit::Add("title".into(), PropertyValue::Empty);
        assert_eq!(PropertySheet::edit(note, &add), Err("The note has a property “title” already.".into()));
        assert!(PropertySheet::edit("---\n# c\nx: 1\n---\n", &set).unwrap_err().contains("comment"));
        let tags = PropertyEdit::Add("tags".into(), PropertyValue::List(vec!["x".into()]));
        assert_eq!(PropertySheet::edit("body", &tags).unwrap(), "---\ntags:\n  - x\n---\nbody");
    }

    #[test]
    fn names_are_ranked_and_completed() {
        let names = PropertyNames::ranked(["tags", "status", "tags", "Status", "due", "tags", "status"]);
        assert_eq!(names, ["tags", "status", "due", "Status"]);
        assert_eq!(PropertyNames::complete("st", &names, 8), ["status", "Status"]);
        assert_eq!(PropertyNames::complete("u", &names, 8), ["status", "due", "Status"]);
        assert_eq!(PropertyNames::complete("", &names, 2), ["tags", "status"]);
    }
}
