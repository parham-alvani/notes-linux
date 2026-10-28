//! The JavaScript of a query's `filter by function` and `group by function`
//! lines: the small part of it such lines use, read without a JavaScript
//! engine. Strings, arrays, `true`/`false`/`null`, the `task.*` properties the
//! index can answer, `===` `!==` `==` `!=`, `&&` `||` `!`, `? :`, brackets,
//! and `includes`, `startsWith`, `endsWith`, `toLowerCase`, `toUpperCase`.
//! Anything else fails to parse, so the line is reported rather than guessed
//! at. The rules are Daftar's on the phone (`TaskExpression.kt`).

use crate::tasks_query::{QueryTask, TaskState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    String(String),
    List(Vec<Value>),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(flag) => *flag,
            Value::String(text) => !text.is_empty(),
            Value::List(_) => true,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(flag) => flag.to_string(),
            Value::String(text) => text.clone(),
            Value::List(items) => items.iter().map(Value::display).collect::<Vec<_>>().join(","),
        }
    }
}

type Node = Box<dyn Fn(&QueryTask) -> Value + Send + Sync>;

pub struct TaskExpression {
    root: Node,
}

impl TaskExpression {
    pub fn evaluate(&self, task: &QueryTask) -> Value {
        (self.root)(task)
    }

    pub fn test(&self, task: &QueryTask) -> bool {
        self.evaluate(task).truthy()
    }

    pub fn parse(source: &str) -> Option<TaskExpression> {
        let tokens = tokens(source)?;
        let mut parser = Parser { tokens: &tokens, at: 0 };
        let root = parser.expression()?;
        (parser.at == tokens.len()).then_some(TaskExpression { root })
    }

    /// What `task.<path>` reads, or `None` when the index does not keep it.
    fn property(path: &str) -> Option<Node> {
        let node: Node = match path {
            "status.type" => Box::new(|t| Value::String(t.state.type_name().into())),
            "status.name" => Box::new(|t| Value::String(t.status_name.clone())),
            "status.symbol" => Box::new(|t| Value::String(t.symbol.to_string())),
            "reason" => Box::new(|t| t.reason.clone().map_or(Value::Null, Value::String)),
            "description" => Box::new(|t| Value::String(t.text.clone())),
            "heading" => Box::new(|t| if t.section.trim().is_empty() { Value::Null } else { Value::String(t.section.clone()) }),
            "hasHeading" => Box::new(|t| Value::Bool(!t.section.trim().is_empty())),
            "isDone" => Box::new(|t| Value::Bool(t.state == TaskState::Done)),
            "isRecurring" => Box::new(|t| Value::Bool(t.recurring.is_some())),
            "file.path" => Box::new(|t| Value::String(t.path.clone())),
            "file.filename" => Box::new(|t| Value::String(t.filename().into())),
            "file.filenameWithoutExtension" => Box::new(|t| Value::String(t.filename().strip_suffix(".md").unwrap_or(t.filename()).into())),
            "file.folder" => Box::new(|t| Value::String(t.folder().into())),
            _ => return None,
        };
        Some(node)
    }
}

const OPERATORS: [&str; 15] = ["===", "!==", "==", "!=", "&&", "||", "!", "?", ":", "(", ")", "[", "]", ",", "."];

/// A string token keeps its opening quote, so it is never read as a name.
fn tokens(source: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '"' || c == '\'' {
            let end = i + 1 + chars[i + 1..].iter().position(|&x| x == c)?;
            out.push(format!("\"{}", chars[i + 1..end].iter().collect::<String>()));
            i = end + 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            // Operators are ASCII: a byte is a character.
            let op = OPERATORS.iter().find(|op| i + op.len() <= chars.len() && chars[i..i + op.len()].iter().copied().eq(op.chars()))?;
            out.push(op.to_string());
            i += op.len();
        }
    }
    Some(out)
}

/// Recursive descent, lowest precedence first: `? :`, `||`, `&&`, equality, `!`.
struct Parser<'a> {
    tokens: &'a [String],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.at).map(String::as_str)
    }

    fn take(&mut self, expected: &str) -> bool {
        if self.peek() != Some(expected) {
            return false;
        }
        self.at += 1;
        true
    }

    fn expression(&mut self) -> Option<Node> {
        let condition = self.or()?;
        if !self.take("?") {
            return Some(condition);
        }
        let yes = self.expression()?;
        if !self.take(":") {
            return None;
        }
        let no = self.expression()?;
        Some(Box::new(move |t| if condition(t).truthy() { yes(t) } else { no(t) }))
    }

    fn or(&mut self) -> Option<Node> {
        let mut left = self.and()?;
        while self.take("||") {
            let right = self.and()?;
            let l = left;
            left = Box::new(move |t| {
                let v = l(t);
                if v.truthy() { v } else { right(t) }
            });
        }
        Some(left)
    }

    fn and(&mut self) -> Option<Node> {
        let mut left = self.equality()?;
        while self.take("&&") {
            let right = self.equality()?;
            let l = left;
            left = Box::new(move |t| {
                let v = l(t);
                if v.truthy() { right(t) } else { v }
            });
        }
        Some(left)
    }

    fn equality(&mut self) -> Option<Node> {
        let mut left = self.unary()?;
        while let Some(op) = self.peek().filter(|op| ["===", "!==", "==", "!="].contains(op)) {
            let negate = op.starts_with('!');
            self.at += 1;
            let right = self.unary()?;
            let l = left;
            left = Box::new(move |t| Value::Bool((l(t) == right(t)) != negate));
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<Node> {
        if self.take("!") {
            let inner = self.unary()?;
            return Some(Box::new(move |t| Value::Bool(!inner(t).truthy())));
        }
        let base = self.primary()?;
        self.postfix(base)
    }

    fn primary(&mut self) -> Option<Node> {
        let token = self.peek()?.to_string();
        self.at += 1;
        if let Some(text) = token.strip_prefix('"') {
            let text = text.to_string();
            return Some(Box::new(move |_| Value::String(text.clone())));
        }
        match token.as_str() {
            "true" => Some(Box::new(|_| Value::Bool(true))),
            "false" => Some(Box::new(|_| Value::Bool(false))),
            "null" | "undefined" => Some(Box::new(|_| Value::Null)),
            "(" => {
                let inner = self.expression()?;
                self.take(")").then_some(inner)
            }
            "[" => {
                let items = self.arguments("]")?;
                Some(Box::new(move |t| Value::List(items.iter().map(|item| item(t)).collect())))
            }
            "task" => self.task_property(),
            _ => None,
        }
    }

    /// Expressions separated by commas, up to and with [close].
    fn arguments(&mut self, close: &str) -> Option<Vec<Node>> {
        let mut items = Vec::new();
        if self.peek() != Some(close) {
            items.push(self.expression()?);
            while self.take(",") {
                items.push(self.expression()?);
            }
        }
        self.take(close).then_some(items)
    }

    /// `task.a.b`, as long as the dotted names run -- but not into a method.
    fn task_property(&mut self) -> Option<Node> {
        let mut names: Vec<&str> = Vec::new();
        while self.peek() == Some(".") && self.at + 1 < self.tokens.len() {
            let mut candidate = names.clone();
            candidate.push(&self.tokens[self.at + 1]);
            let candidate = candidate.join(".");
            if TaskExpression::property(&candidate).is_none() && !["status", "file"].contains(&candidate.as_str()) {
                break;
            }
            self.at += 2;
            names.push(&self.tokens[self.at - 1]);
        }
        TaskExpression::property(&names.join("."))
    }

    fn postfix(&mut self, start: Node) -> Option<Node> {
        let mut node = start;
        while self.take(".") {
            let method = self.peek()?.to_string();
            self.at += 1;
            if !self.take("(") {
                return None;
            }
            let mut args = self.arguments(")")?;
            let target = node;
            node = match (method.as_str(), args.len()) {
                ("includes", 1) => {
                    let arg = args.remove(0);
                    Box::new(move |t| match (target(t), arg(t)) {
                        (Value::List(items), value) => Value::Bool(items.contains(&value)),
                        (Value::String(text), Value::String(part)) => Value::Bool(text.contains(&part)),
                        _ => Value::Bool(false),
                    })
                }
                ("startsWith" | "endsWith", 1) => {
                    let arg = args.remove(0);
                    let starts = method == "startsWith";
                    Box::new(move |t| match (target(t), arg(t)) {
                        (Value::String(text), Value::String(part)) => {
                            Value::Bool(if starts { text.starts_with(&part) } else { text.ends_with(&part) })
                        }
                        _ => Value::Bool(false),
                    })
                }
                ("toLowerCase" | "toUpperCase", _) => {
                    let lower = method == "toLowerCase";
                    Box::new(move |t| match target(t) {
                        Value::String(text) => Value::String(if lower { text.to_lowercase() } else { text.to_uppercase() }),
                        _ => Value::Null,
                    })
                }
                _ => return None,
            };
        }
        Some(node)
    }
}
