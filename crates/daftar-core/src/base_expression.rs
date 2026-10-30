//! The expression language of a `.base` file, as Obsidian documents it: the
//! operators `|| && == != < > <= >= + - * / % !`, method calls, `[index]`,
//! strings, numbers, lists and regular expressions.

use std::fmt;
use std::sync::LazyLock;

use chrono::{DateTime, Local};
use regex::Regex;

/// A value in an Obsidian Bases expression.
#[derive(Debug, Clone)]
pub enum BaseValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// [time] is false for a date without a time of day, such as `2026-10-25`.
    Date(DateTime<Local>, bool),
    Duration(BaseSpan),
    List(Vec<BaseValue>),
    /// A link as written, the vault path it goes to when it goes anywhere,
    /// and what it shows.
    Link {
        target: String,
        path: Option<String>,
        display: Option<String>,
    },
    File(String),
    Object(Vec<(String, BaseValue)>),
    Regex(Regex, bool),
}

/// A length of time: whole months, which have no fixed length, and milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BaseSpan {
    pub months: i64,
    pub milliseconds: f64,
}

static SPAN_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([+-]?\d+(?:\.\d+)?)\s*([A-Za-z]+)\s*,?").unwrap());

impl BaseSpan {
    pub fn new(months: i64, milliseconds: f64) -> BaseSpan {
        BaseSpan { months, milliseconds }
    }

    #[must_use]
    pub fn plus(self, other: BaseSpan) -> BaseSpan {
        BaseSpan { months: self.months + other.months, milliseconds: self.milliseconds + other.milliseconds }
    }

    #[must_use]
    pub fn negated(self) -> BaseSpan {
        BaseSpan { months: -self.months, milliseconds: -self.milliseconds }
    }

    /// For arithmetic and comparison, a month counts as 30 days.
    pub fn approximate_milliseconds(self) -> f64 {
        self.milliseconds + self.months as f64 * 30.0 * 86_400_000.0
    }

    /// `"30 days"`, `"1d"`, `"1M 4h"`, `"-2 weeks"`: units y, M, w, d, h, m, s,
    /// ms, or their names.
    pub fn parse(text: &str) -> Option<BaseSpan> {
        let unit = |name: &str| -> Option<(i64, f64)> {
            Some(match name {
                "y" | "year" | "years" => (12, 0.0),
                "M" | "month" | "months" => (1, 0.0),
                "w" | "week" | "weeks" => (0, 604_800_000.0),
                "d" | "day" | "days" => (0, 86_400_000.0),
                "h" | "hour" | "hours" => (0, 3_600_000.0),
                "m" | "minute" | "minutes" => (0, 60_000.0),
                "s" | "second" | "seconds" => (0, 1000.0),
                "ms" | "millisecond" | "milliseconds" => (0, 1.0),
                _ => return None,
            })
        };
        let mut span = BaseSpan::default();
        let mut found = false;
        let mut rest = text.trim_matches([' ', '\t']);
        while !rest.is_empty() {
            let caps = SPAN_PART.captures(rest)?;
            let amount: f64 = caps[1].parse().ok()?;
            let name = &caps[2];
            let (months, ms) = unit(name).or_else(|| if name.len() > 1 { unit(&name.to_lowercase()) } else { None })?;
            if months != 0 {
                if amount != amount.round() {
                    return None;
                }
                let whole = whole(amount)?;
                span.months = span.months.checked_add(whole.checked_mul(months)?)?;
            } else {
                span.milliseconds += amount * ms;
            }
            found = true;
            rest = &rest[caps[0].len()..];
        }
        found.then_some(span)
    }
}

/// The whole number toward zero, or `None` when there is none: not a
/// number, infinite, or past what an `i64` holds.
pub fn whole(value: f64) -> Option<i64> {
    let t = value.trunc();
    (t.is_finite() && (-9.223_372_036_854_775e18..9.223_372_036_854_775e18).contains(&t)).then_some(t as i64)
}

/// An expression as it is parsed, before it meets a file.
#[derive(Debug, Clone)]
pub enum BaseExpr {
    Value(BaseValue),
    List(Vec<BaseExpr>),
    Name(String),
    Member(Box<BaseExpr>, String),
    Index(Box<BaseExpr>, Box<BaseExpr>),
    /// A global function: `if(…)`, `now()`.
    Call(String, Vec<BaseExpr>),
    Method(Box<BaseExpr>, String, Vec<BaseExpr>),
    Unary(String, Box<BaseExpr>),
    Binary(String, Box<BaseExpr>, Box<BaseExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseParseError {
    pub message: String,
}

impl fmt::Display for BaseParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BaseParseError {}

fn error<T>(message: impl Into<String>) -> Result<T, BaseParseError> {
    Err(BaseParseError { message: message.into() })
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    String(String),
    Regex(String, String),
    Name(String),
    Symbol(String),
}

pub struct BaseParser;

impl BaseParser {
    pub fn parse(source: &str) -> Result<BaseExpr, BaseParseError> {
        let tokens = tokenize(source)?;
        let mut parser = Parser { tokens: &tokens, at: 0 };
        let expression = parser.or()?;
        if parser.at != tokens.len() {
            return error(format!("Unexpected {}", describe(&tokens[parser.at])));
        }
        Ok(expression)
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, BaseParseError> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut at = 0;
    let ends_value = |tokens: &[Token]| match tokens.last() {
        Some(Token::Number(_) | Token::String(_) | Token::Regex(..) | Token::Name(_)) => true,
        Some(Token::Symbol(symbol)) => symbol == ")" || symbol == "]",
        None => false,
    };
    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
        } else if c.is_ascii_digit() {
            let mut end = at;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
            if end + 1 < chars.len() && chars[end] == '.' && chars[end + 1].is_ascii_digit() {
                end += 1;
                while end < chars.len() && chars[end].is_ascii_digit() {
                    end += 1;
                }
            }
            tokens.push(Token::Number(chars[at..end].iter().collect::<String>().parse().unwrap_or(0.0)));
            at = end;
        } else if c == '"' || c == '\'' {
            let mut text = String::new();
            let mut end = at + 1;
            while end < chars.len() && chars[end] != c {
                if chars[end] == '\\' && end + 1 < chars.len() {
                    end += 1;
                    text.push(match chars[end] {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                } else {
                    text.push(chars[end]);
                }
                end += 1;
            }
            if end >= chars.len() {
                return error(format!("A string has no closing {c}"));
            }
            tokens.push(Token::String(text));
            at = end + 1;
        } else if c == '/' && !ends_value(&tokens) {
            let mut pattern = String::new();
            let mut end = at + 1;
            while end < chars.len() && chars[end] != '/' {
                if chars[end] == '\\' && end + 1 < chars.len() {
                    pattern.push(chars[end]);
                    end += 1;
                }
                pattern.push(chars[end]);
                end += 1;
            }
            if end >= chars.len() {
                return error("A regular expression has no closing /");
            }
            end += 1;
            let mut flags = String::new();
            while end < chars.len() && chars[end].is_alphabetic() {
                flags.push(chars[end]);
                end += 1;
            }
            tokens.push(Token::Regex(pattern, flags));
            at = end;
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let mut end = at;
            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_' || chars[end] == '$') {
                end += 1;
            }
            tokens.push(Token::Name(chars[at..end].iter().collect()));
            at = end;
        } else {
            let two: String = chars[at..(at + 2).min(chars.len())].iter().collect();
            if ["==", "!=", "<=", ">=", "&&", "||"].contains(&two.as_str()) {
                tokens.push(Token::Symbol(two));
                at += 2;
            } else if "()[],.!+-*/%<>".contains(c) {
                tokens.push(Token::Symbol(c.to_string()));
                at += 1;
            } else {
                return error(format!("Unexpected “{c}”"));
            }
        }
    }
    Ok(tokens)
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(n) => format!("{n}"),
        Token::String(text) => format!("\"{text}\""),
        Token::Regex(pattern, _) => format!("/{pattern}/"),
        Token::Name(name) => name.clone(),
        Token::Symbol(symbol) => format!("“{symbol}”"),
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

type Parsed = Result<BaseExpr, BaseParseError>;

impl Parser<'_> {
    fn take(&mut self, symbol: &str) -> bool {
        if matches!(self.tokens.get(self.at), Some(Token::Symbol(s)) if s == symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, symbol: &str) -> Result<(), BaseParseError> {
        if self.take(symbol) {
            return Ok(());
        }
        match self.tokens.get(self.at) {
            Some(token) => error(format!("Expected “{symbol}”, found {}", describe(token))),
            None => error(format!("Expected “{symbol}” at the end")),
        }
    }

    fn binary(&mut self, symbols: &[&str], next: fn(&mut Self) -> Parsed) -> Parsed {
        let mut left = next(self)?;
        while let Some(symbol) = symbols.iter().find(|s| self.take(s)) {
            left = BaseExpr::Binary(symbol.to_string(), Box::new(left), Box::new(next(self)?));
        }
        Ok(left)
    }

    fn or(&mut self) -> Parsed {
        self.binary(&["||"], Self::and)
    }
    fn and(&mut self) -> Parsed {
        self.binary(&["&&"], Self::equality)
    }
    fn equality(&mut self) -> Parsed {
        self.binary(&["==", "!="], Self::comparison)
    }
    fn comparison(&mut self) -> Parsed {
        self.binary(&["<=", ">=", "<", ">"], Self::additive)
    }
    fn additive(&mut self) -> Parsed {
        self.binary(&["+", "-"], Self::multiplicative)
    }
    fn multiplicative(&mut self) -> Parsed {
        self.binary(&["*", "/", "%"], Self::unary)
    }

    fn unary(&mut self) -> Parsed {
        if self.take("!") {
            return Ok(BaseExpr::Unary("!".into(), Box::new(self.unary()?)));
        }
        if self.take("-") {
            return Ok(BaseExpr::Unary("-".into(), Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn arguments(&mut self) -> Result<Vec<BaseExpr>, BaseParseError> {
        let mut items = Vec::new();
        if self.take(")") {
            return Ok(items);
        }
        loop {
            items.push(self.or()?);
            if !self.take(",") {
                break;
            }
        }
        self.expect(")")?;
        Ok(items)
    }

    fn postfix(&mut self) -> Parsed {
        let mut expression = self.primary()?;
        loop {
            if self.take(".") {
                let Some(Token::Name(name)) = self.tokens.get(self.at) else { return error("Expected a name after “.”") };
                let name = name.clone();
                self.at += 1;
                expression = if self.take("(") {
                    BaseExpr::Method(Box::new(expression), name, self.arguments()?)
                } else {
                    BaseExpr::Member(Box::new(expression), name)
                };
            } else if self.take("[") {
                let key = self.or()?;
                self.expect("]")?;
                expression = BaseExpr::Index(Box::new(expression), Box::new(key));
            } else if self.take("(") {
                let BaseExpr::Name(name) = expression else { return error("Only a function can be called") };
                expression = BaseExpr::Call(name, self.arguments()?);
            } else {
                return Ok(expression);
            }
        }
    }

    fn primary(&mut self) -> Parsed {
        let Some(token) = self.tokens.get(self.at) else { return error("The expression stops too early") };
        self.at += 1;
        Ok(match token {
            Token::Number(n) => BaseExpr::Value(BaseValue::Number(*n)),
            Token::String(text) => BaseExpr::Value(BaseValue::String(text.clone())),
            Token::Regex(pattern, flags) => {
                let mut inline = String::new();
                for (flag, letter) in [('i', 'i'), ('m', 'm'), ('s', 's')] {
                    if flags.contains(flag) {
                        inline.push(letter);
                    }
                }
                let source = if inline.is_empty() { pattern.clone() } else { format!("(?{inline}){pattern}") };
                let Ok(regex) = Regex::new(&source) else { return error(format!("Not a regular expression: /{pattern}/")) };
                BaseExpr::Value(BaseValue::Regex(regex, flags.contains('g')))
            }
            Token::Name(name) => match name.as_str() {
                "true" => BaseExpr::Value(BaseValue::Bool(true)),
                "false" => BaseExpr::Value(BaseValue::Bool(false)),
                "null" => BaseExpr::Value(BaseValue::Null),
                _ => BaseExpr::Name(name.clone()),
            },
            Token::Symbol(symbol) if symbol == "(" => {
                let inner = self.or()?;
                self.expect(")")?;
                inner
            }
            Token::Symbol(symbol) if symbol == "[" => {
                let mut items = Vec::new();
                if !self.take("]") {
                    loop {
                        items.push(self.or()?);
                        if !self.take(",") {
                            break;
                        }
                    }
                    self.expect("]")?;
                }
                BaseExpr::List(items)
            }
            Token::Symbol(_) => return error(format!("Unexpected {}", describe(token))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_expressions_are_errors() {
        assert!(BaseParser::parse("a ==").is_err());
        assert!(BaseParser::parse("\"open").is_err());
        assert!(BaseParser::parse("a b").is_err());
    }

    #[test]
    fn spans() {
        assert_eq!(BaseSpan::parse("30 days"), Some(BaseSpan::new(0, 30.0 * 86_400_000.0)));
        assert_eq!(BaseSpan::parse("1M 4h"), Some(BaseSpan::new(1, 4.0 * 3_600_000.0)));
        assert_eq!(BaseSpan::parse("2y"), Some(BaseSpan::new(24, 0.0)));
        assert_eq!(BaseSpan::parse("soon"), None);
        assert_eq!(BaseSpan::parse("99999999999999999999 years"), None);
    }
}
