//! JSON that keeps the order of its keys and writes itself back as
//! `JSON.stringify(value, null, 2)` does, so a plugin's `data.json` that the
//! app edits changes by the lines that changed and no others. Numbers keep
//! the text they were written with.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderedJson {
    Object(Vec<(String, OrderedJson)>),
    Array(Vec<OrderedJson>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonFailure;

impl fmt::Display for JsonFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not JSON")
    }
}

impl std::error::Error for JsonFailure {}

impl OrderedJson {
    pub fn parse(text: &str) -> Result<OrderedJson, JsonFailure> {
        let mut parser = Parser { chars: text.chars().collect(), at: 0 };
        let value = parser.value()?;
        parser.space();
        if parser.at == parser.chars.len() { Ok(value) } else { Err(JsonFailure) }
    }

    /// As `JSON.stringify(value, null, 2)`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let (pad, close) = ("  ".repeat(depth + 1), "  ".repeat(depth));
        match self {
            OrderedJson::Object(members) if members.is_empty() => out.push_str("{}"),
            OrderedJson::Object(members) => {
                out.push_str("{\n");
                for (index, (key, value)) in members.iter().enumerate() {
                    out.push_str(&format!("{pad}{}: ", quoted(key)));
                    value.write(out, depth + 1);
                    out.push_str(if index < members.len() - 1 { ",\n" } else { "\n" });
                }
                out.push_str(&close);
                out.push('}');
            }
            OrderedJson::Array(items) if items.is_empty() => out.push_str("[]"),
            OrderedJson::Array(items) => {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    out.push_str(&pad);
                    item.write(out, depth + 1);
                    out.push_str(if index < items.len() - 1 { ",\n" } else { "\n" });
                }
                out.push_str(&close);
                out.push(']');
            }
            OrderedJson::String(text) => out.push_str(&quoted(text)),
            OrderedJson::Number(text) => out.push_str(text),
            OrderedJson::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            OrderedJson::Null => out.push_str("null"),
        }
    }

    pub fn get(&self, key: &str) -> Option<&OrderedJson> {
        match self {
            OrderedJson::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets [key] to [value], in its place or last; `None` takes it away.
    /// Nothing happens to a value that is not an object.
    pub fn set(&mut self, key: &str, value: Option<OrderedJson>) {
        let OrderedJson::Object(members) = self else { return };
        match (members.iter().position(|(k, _)| k == key), value) {
            (Some(index), Some(value)) => members[index].1 = value,
            (Some(index), None) => {
                members.remove(index);
            }
            (None, Some(value)) => members.push((key.into(), value)),
            (None, None) => {}
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        match self {
            OrderedJson::Object(members) => members.iter().map(|(k, _)| k.as_str()).collect(),
            _ => vec![],
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            OrderedJson::String(text) => Some(text),
            _ => None,
        }
    }
}

/// A string as JavaScript quotes it: only the quote, the backslash and the
/// control characters are escaped.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, text: &str) -> Result<(), JsonFailure> {
        for c in text.chars() {
            if self.peek() != Some(c) {
                return Err(JsonFailure);
            }
            self.at += 1;
        }
        Ok(())
    }

    fn value(&mut self) -> Result<OrderedJson, JsonFailure> {
        self.space();
        match self.peek().ok_or(JsonFailure)? {
            '{' => {
                self.at += 1;
                let mut members = Vec::new();
                self.space();
                if self.peek() == Some('}') {
                    self.at += 1;
                    return Ok(OrderedJson::Object(members));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    self.expect(":")?;
                    members.push((key, self.value()?));
                    self.space();
                    if self.peek() == Some(',') {
                        self.at += 1;
                    } else {
                        self.expect("}")?;
                        return Ok(OrderedJson::Object(members));
                    }
                }
            }
            '[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.peek() == Some(']') {
                    self.at += 1;
                    return Ok(OrderedJson::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.space();
                    if self.peek() == Some(',') {
                        self.at += 1;
                    } else {
                        self.expect("]")?;
                        return Ok(OrderedJson::Array(items));
                    }
                }
            }
            '"' => Ok(OrderedJson::String(self.string()?)),
            't' => self.expect("true").map(|()| OrderedJson::Bool(true)),
            'f' => self.expect("false").map(|()| OrderedJson::Bool(false)),
            'n' => self.expect("null").map(|()| OrderedJson::Null),
            _ => {
                let start = self.at;
                while self.peek().is_some_and(|c| "+-0123456789.eE".contains(c)) {
                    self.at += 1;
                }
                if self.at == start {
                    return Err(JsonFailure);
                }
                Ok(OrderedJson::Number(self.chars[start..self.at].iter().collect()))
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonFailure> {
        self.expect("\"")?;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.at += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escape = self.peek().ok_or(JsonFailure)?;
                    self.at += 1;
                    match escape {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{C}'),
                        'u' => {
                            let mut unit = self.hex()?;
                            // A pair of surrogates is one character.
                            if (0xD800..=0xDBFF).contains(&unit) && self.peek() == Some('\\') && self.chars.get(self.at + 1) == Some(&'u') {
                                self.at += 2;
                                let low = self.hex()?;
                                unit = 0x10000 + ((unit - 0xD800) << 10) + low.wrapping_sub(0xDC00);
                            }
                            out.push(char::from_u32(unit).ok_or(JsonFailure)?);
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Err(JsonFailure)
    }

    fn hex(&mut self) -> Result<u32, JsonFailure> {
        let digits: String = self.chars.get(self.at..self.at + 4).ok_or(JsonFailure)?.iter().collect();
        let value = u32::from_str_radix(&digits, 16).map_err(|_| JsonFailure)?;
        self.at += 4;
        Ok(value)
    }
}
