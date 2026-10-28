//! One task as it is written in the file, and the edits the app makes to it.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::day::Day;
use crate::task_status::{TaskStatuses, TaskType};
use crate::text::{is_blank, trim, trim_end};

/// A task as the index keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParsedTask {
    /// Zero-based line in the file.
    pub line: usize,
    pub status: char,
    /// The sentence, without the Tasks plugin's trailing emoji.
    pub text: String,
    /// The nearest heading above it, which in this vault is the project.
    pub section: String,
    pub created: Option<String>,
    pub scheduled: Option<String>,
    pub due: Option<String>,
    pub start: Option<String>,
    pub done: Option<String>,
    pub cancelled: Option<String>,
    pub recurring: Option<String>,
    pub priority: Option<String>,
    /// Why it is cancelled, or waiting: its `[reason:: …]` field.
    pub reason: Option<String>,
    /// Indented under another item.
    pub nested: bool,
}

impl ParsedTask {
    /// Still to do: open or in progress.
    pub fn is_open(&self) -> bool {
        !TaskLine::is_closed(self.status)
    }

    /// The day the task belongs to: due, else scheduled.
    pub fn day(&self) -> Option<&str> {
        self.due.as_deref().or(self.scheduled.as_deref())
    }
}

/// One of the Tasks plugin's signifiers and the value after it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Meta {
    pub emoji: String,
    pub value: String,
}

impl Meta {
    pub fn new(emoji: &str, value: &str) -> Meta {
        Meta { emoji: emoji.to_string(), value: value.to_string() }
    }
}

/// The Tasks plugin's trailing emoji, pulled off a task line.
pub struct TaskMetadata;

static SYMBOLS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        ("✅", "done"),
        ("➕", "created"),
        ("⏳", "scheduled"),
        ("❌", "cancelled"),
        ("📅", "due"),
        ("🔁", "recurring"),
        ("⛔", "blocked"),
        ("🛫", "start"),
        ("🆔", "id"),
        ("🔺", "priority"),
        ("⏫", "priority"),
        ("🔼", "priority"),
        ("🔽", "priority"),
        ("⏬", "priority"),
        ("🏁", "onCompletion"),
    ])
});
static VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9A-Za-z/,:. -]{0,32}$").unwrap());
static DATE_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());
static TRAILING_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?)((?:\s*#[\p{L}\p{N}_/-]+)+)$").unwrap());
const DATED: [&str; 6] = ["done", "created", "scheduled", "cancelled", "due", "start"];

impl TaskMetadata {
    /// The kind of field a signifier starts: `due`, `priority`, …
    pub fn kind(emoji: &str) -> Option<&'static str> {
        SYMBOLS.get(emoji).copied()
    }

    /// Whether [value] can follow [symbol]: nothing after a priority, only
    /// a date after a date, a rule or an id after the rest.
    fn takes(symbol: &str, value: &str) -> bool {
        if value.is_empty() {
            return true;
        }
        let kind = SYMBOLS.get(symbol).copied().unwrap_or("");
        if kind == "priority" {
            return false;
        }
        if DATED.contains(&kind) { DATE_VALUE.is_match(value) } else { VALUE.is_match(value) }
    }

    /// The sentence and its metadata, in the order written. From the right:
    /// the metadata is always a suffix, and an emoji with prose after it is
    /// part of the sentence, as is a second field of a kind already found.
    pub fn split(line: &str) -> (String, Vec<Meta>) {
        let mut remaining = line.to_string();
        let mut found: Vec<Meta> = Vec::new();
        // Tags after the signifiers (`📅 2026-10-20 ⏫ #kit`) stay part of
        // the text, as the Tasks plugin keeps them.
        let mut tags: Vec<String> = Vec::new();
        let mut kinds: HashSet<&str> = HashSet::new();
        // Only the symbols in the line are looked for, not all of them at each step.
        let present: Vec<&str> = SYMBOLS.keys().copied().filter(|symbol| line.contains(symbol)).collect();
        loop {
            let last = present.iter().filter_map(|symbol| remaining.rfind(symbol).map(|at| (*symbol, at))).max_by_key(|(_, at)| *at);
            let Some((symbol, at)) = last else { break };
            let kind = SYMBOLS[symbol];
            if kinds.contains(kind) {
                break;
            }
            let mut value = trim(&remaining[at + symbol.len()..]).to_string();
            if !Self::takes(symbol, &value) {
                let Some(caps) = TRAILING_TAGS.captures(&value) else { break };
                if !Self::takes(symbol, &caps[1]) {
                    break;
                }
                tags.insert(0, trim(&caps[2]).to_string());
                value = caps[1].to_string();
            }
            found.push(Meta { emoji: symbol.to_string(), value });
            kinds.insert(kind);
            remaining.truncate(at);
            remaining.truncate(trim_end(&remaining).len());
        }
        if !tags.is_empty() {
            remaining.push(' ');
            remaining.push_str(&tags.join(" "));
        }
        found.reverse();
        (remaining, found)
    }
}

/// `- [ ] `, `* [x] `, `1. [/] `, and the same inside a quote or callout.
static PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*(?:>\s?)*\s*(?:[-*+]|\d+[.)])\s+)\[([^\]\n])\]\s*(.*)$").unwrap());
static FIELDS_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*(?:>\s?)*\s*(?:[-*+]|\d+[.)])\s+)\[([^\]\n])\]\s*(.*?)(\s+\^[A-Za-z0-9-]+)?\s*$").unwrap());
static BLOCK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+(\^[A-Za-z0-9-]+)\s*$").unwrap());
static CLOSED_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]*(?:✅|❌)[ \t]*\d{4}-\d{2}-\d{2}").unwrap());

const DONE: &str = "✅";
const CANCELLED: &str = "❌";
const RECUR: &str = "🔁";
const MOVED: [&str; 3] = ["📅", "⏳", "🛫"];

/// The rules are Daftar's on Android (`TaskLine.kt`) and macOS
/// (`TaskLine.swift`): the done date goes on the end, a repeating task
/// leaves its next occurrence above the one just done, and every edit is a
/// function of the line as it is now.
pub struct TaskLine;

impl TaskLine {
    /// Done or cancelled, as the vault's Tasks plugin settings type the
    /// status. In progress and a status the settings do not know are open.
    pub fn is_closed(status: char) -> bool {
        TaskStatuses::current().kind(status).is_closed()
    }

    pub fn is_task(raw: &str) -> bool {
        PATTERN.is_match(raw)
    }

    pub fn parse(raw: &str, line: usize, section: &str) -> Option<ParsedTask> {
        if !raw.contains('[') {
            return None;
        }
        let caps = PATTERN.captures(raw)?;
        let status = caps[2].chars().next()?;
        let body = if caps[3].contains('^') { BLOCK_ID.replace(&caps[3], "") } else { Cow::Borrowed(&caps[3]) };
        let (text, meta) = TaskMetadata::split(&body);
        let (text, reason) = TaskReason::take(&text);
        let date = |emoji: &str| meta.iter().find(|m| m.emoji == emoji).map(|m| m.value.clone()).filter(|v| !v.is_empty());
        let prefix = &caps[1];
        let indent: String = prefix.chars().take_while(|c| matches!(c, ' ' | '\t' | '>')).collect();
        Some(ParsedTask {
            line,
            status,
            text,
            section: section.to_string(),
            created: date("➕"),
            scheduled: date("⏳"),
            due: date("📅"),
            start: date("🛫"),
            done: date(DONE),
            cancelled: date(CANCELLED),
            recurring: date(RECUR),
            priority: meta.iter().find(|m| TaskMetadata::kind(&m.emoji) == Some("priority")).map(|m| m.emoji.clone()),
            reason,
            nested: indent.contains([' ', '\t']),
        })
    }

    /// The line after a click on its checkbox: an open task is completed, and
    /// a closed one is opened again without its done or cancelled date. More
    /// than one line when a repeating task leaves its next occurrence.
    pub fn toggled(raw: &str, today: &str) -> Option<Vec<String>> {
        let caps = PATTERN.captures(raw)?;
        let status = caps[2].chars().next()?;
        let prefix = &caps[1];
        // The block id `^abc` stays the line's last word, on the line it named.
        let mut body = caps[3].to_string();
        let mut id = String::new();
        if let Some(found) = BLOCK_ID.captures(&body) {
            id = format!(" {}", &found[1]);
            let start = found.get(0).unwrap().start();
            body.truncate(start);
        }
        // The reason was the old status's: it goes with it.
        let body = TaskReason::remove(&body);
        if Self::is_closed(status) {
            let text = CLOSED_DATE.replace_all(&body, "");
            return Some(vec![format!("{prefix}[ ] {text}{id}")]);
        }
        if let Some(mut lines) = Self::complete_recurring(prefix, &body, today) {
            lines.last_mut().unwrap().push_str(&id);
            return Some(lines);
        }
        Some(vec![format!("{prefix}[x] {} {DONE} {today}{id}", trim_end(&body))])
    }

    /// The line after its status is set to [status] from a status menu, a
    /// board column or the command line. Done from open is the tick, a
    /// repeat and all; else the done and cancelled dates follow the status's
    /// type, kept when it was of that type already. The old reason goes with
    /// the old status unless [reason] gives one, and a blank one takes it
    /// away; to the same status only the reason changes. `None` when nothing
    /// does.
    pub fn with_status(raw: &str, status: char, reason: Option<&str>, today: &str) -> Option<Vec<String>> {
        let statuses = TaskStatuses::current();
        let mut fields = Self::fields(raw)?;
        let given = reason.map(|r| Some(r.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|r| !r.is_empty()));
        if status == fields.status {
            let reason = given?;
            if fields.reason == reason {
                return None;
            }
            fields.reason = reason;
            return Self::edited(raw, &fields).map(|line| vec![line]);
        }
        let reason = given.flatten();
        let (from, to) = (statuses.kind(fields.status), statuses.kind(status));
        if to == TaskType::Done && !from.is_closed() {
            let mut lines = Self::toggled(raw, today)?;
            if status != 'x' || reason.is_some() {
                let last = lines.last_mut()?;
                let mut done = Self::fields(last)?;
                done.status = status;
                done.reason = reason;
                *last = Self::edited(last, &done)?;
            }
            return Some(lines);
        }
        let date = |kind: TaskType, now: &Option<String>| match (to == kind, from == kind) {
            (true, true) => now.clone(),
            (true, false) => Some(today.to_string()),
            _ => None,
        };
        fields.done = date(TaskType::Done, &fields.done);
        fields.cancelled = date(TaskType::Cancelled, &fields.cancelled);
        fields.status = status;
        fields.reason = reason;
        Self::edited(raw, &fields).map(|line| vec![line])
    }

    /// The next occurrence above the one just done, every date moved by the
    /// same number of days. `None` when the rule is not a plain interval or
    /// the task has no date to move.
    fn complete_recurring(prefix: &str, body: &str, today: &str) -> Option<Vec<String>> {
        let (text, meta) = TaskMetadata::split(body);
        let rule = &meta.iter().find(|m| m.emoji == RECUR)?.value;
        let recurrence = Recurrence::parse(rule)?;
        let now = Day::parse(today)?;
        let mut dates: HashMap<&str, Day> = HashMap::new();
        for m in &meta {
            if let Some(day) = Day::parse(&m.value) {
                dates.entry(m.emoji.as_str()).or_insert(day);
            }
        }
        let leading = MOVED.iter().find_map(|e| dates.get(e).copied())?;
        let from = if recurrence.when_done { now } else { leading };
        let shift = leading.days_to(recurrence.next(from, now));
        let mut parts = vec![trim_end(&text).to_string()];
        for m in &meta {
            let value = match dates.get(m.emoji.as_str()) {
                Some(date) if MOVED.contains(&m.emoji.as_str()) => date.adding_days(shift).to_string(),
                _ => m.value.clone(),
            };
            parts.push(trim_end(&format!("{} {}", m.emoji, value)).to_string());
        }
        parts.retain(|p| !p.is_empty());
        Some(vec![format!("{prefix}[ ] {}", parts.join(" ")), format!("{prefix}[x] {} {DONE} {today}", trim_end(body))])
    }

    /// The line a task is on now: the recorded line when it still holds the
    /// same task, else the first line that does.
    pub fn locate(lines: &[&str], line: usize, text: &str) -> Option<usize> {
        let holds = |at: usize| Self::parse(lines[at], at, "").is_some_and(|task| task.text == text);
        if line < lines.len() && holds(line) {
            return Some(line);
        }
        (0..lines.len()).find(|&at| holds(at))
    }

    /// The task on [raw], in parts; `None` when the line is not a task.
    pub fn fields(raw: &str) -> Option<TaskFields> {
        let caps = FIELDS_LINE.captures(raw)?;
        let (text, meta) = TaskMetadata::split(&caps[3]);
        let (text, reason) = TaskReason::take(&text);
        let mut fields = TaskFields { status: caps[2].chars().next()?, text, reason, ..TaskFields::default() };
        for m in meta {
            let value = (!m.value.is_empty()).then(|| m.value.clone());
            match m.emoji.as_str() {
                "➕" => fields.created = value,
                "🛫" => fields.start = value,
                "⏳" => fields.scheduled = value,
                "📅" => fields.due = value,
                "❌" => fields.cancelled = value,
                "✅" => fields.done = value,
                "🔁" => fields.recurring = value,
                "🔺" | "⏫" | "🔼" | "🔽" | "⏬" => fields.priority = Some(m.emoji),
                _ => fields.other.push(m),
            }
        }
        Some(fields)
    }

    /// [raw] with its task written from [fields]: the same indent, bullet
    /// and block id, the signifiers in the Tasks plugin's order.
    pub fn edited(raw: &str, fields: &TaskFields) -> Option<String> {
        let caps = FIELDS_LINE.captures(raw)?;
        let mut parts = vec![trim(&fields.text).to_string()];
        if let Some(reason) = fields.reason.as_deref().and_then(TaskReason::field) {
            parts.push(reason);
        }
        if let Some(priority) = &fields.priority {
            parts.push(priority.clone());
        }
        if let Some(rule) = fields.recurring.as_deref().map(trim).filter(|r| !r.is_empty()) {
            parts.push(format!("🔁 {rule}"));
        }
        const AFTER_RULE: [&str; 3] = ["🏁", "⛔", "🆔"];
        for emoji in AFTER_RULE {
            for m in fields.other.iter().filter(|m| m.emoji == emoji) {
                parts.push(format!("{} {}", m.emoji, m.value));
            }
        }
        let dated = [
            ("➕", &fields.created),
            ("🛫", &fields.start),
            ("⏳", &fields.scheduled),
            ("📅", &fields.due),
            ("❌", &fields.cancelled),
            ("✅", &fields.done),
        ];
        for (emoji, value) in dated {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                parts.push(format!("{emoji} {value}"));
            }
        }
        for m in fields.other.iter().filter(|m| !AFTER_RULE.contains(&m.emoji.as_str())) {
            parts.push(format!("{} {}", m.emoji, m.value));
        }
        parts.retain(|p| !p.is_empty());
        let id = caps.get(4).map_or("", |m| m.as_str());
        Some(format!("{}[{}] {}{}", &caps[1], fields.status, parts.join(" "), id))
    }
}

/// Why a task is in its status, cancelled or waiting: a Dataview field in
/// its description, `[reason:: no budget left]`, after the text and before
/// the Tasks plugin's signifiers, which the plugin keeps as part of the
/// description. The rules are Daftar's on macOS and Android.
pub struct TaskReason;

impl TaskReason {
    /// Where the first field is in [text], in bytes, and its value. The key
    /// comes straight after the bracket, in any case, and the value runs to
    /// the bracket that closes the field, so a `[[link]]` can be in it.
    /// A field that never closes is text.
    fn find(text: &str) -> Option<(std::ops::Range<usize>, String)> {
        let mut from = 0;
        while let Some(at) = text[from..].find('[').map(|i| from + i) {
            from = at + 1;
            let rest = &text[at + 1..];
            let Some(key) = rest.get(..6).filter(|k| k.eq_ignore_ascii_case("reason")) else { continue };
            let after = rest[key.len()..].trim_start_matches(is_blank);
            let Some(value) = after.strip_prefix("::") else { continue };
            let start = text.len() - value.len();
            let mut depth = 1;
            let close = text[start..].char_indices().find_map(|(i, c)| {
                depth += match c {
                    '[' => 1,
                    ']' => -1,
                    _ => 0,
                };
                (depth == 0).then_some(start + i)
            });
            let Some(close) = close else { continue };
            return Some((at..close + 1, trim(&text[start..close]).to_string()));
        }
        None
    }

    /// [text] without its field, the spaces around it folded into one, and
    /// the reason; `None` for none or an empty one.
    pub fn take(text: &str) -> (String, Option<String>) {
        if !text.contains("::") {
            return (text.to_string(), None);
        }
        let Some((range, value)) = Self::find(text) else { return (text.to_string(), None) };
        let (before, after) = (trim_end(&text[..range.start]), text[range.end..].trim_start_matches(is_blank));
        let joined = if before.is_empty() || after.is_empty() { format!("{before}{after}") } else { format!("{before} {after}") };
        (trim(&joined).to_string(), Some(value).filter(|v| !v.is_empty()))
    }

    /// Where the field is in [text], in bytes.
    pub fn range(text: &str) -> Option<std::ops::Range<usize>> {
        if text.contains("::") { Self::find(text).map(|(range, _)| range) } else { None }
    }

    /// [text] without its field.
    pub fn remove(text: &str) -> String {
        Self::take(text).0
    }

    /// The field for [value]: on one line, and with brackets that do not
    /// balance made round, so the field still closes where it should.
    /// `None` for a blank one.
    pub fn field(value: &str) -> Option<String> {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if value.is_empty() {
            return None;
        }
        let mut depth = 0i32;
        let balanced = value.chars().all(|c| {
            depth += match c {
                '[' => 1,
                ']' => -1,
                _ => 0,
            };
            depth >= 0
        }) && depth == 0;
        let value = if balanced { value } else { value.replace('[', "(").replace(']', ")") };
        Some(format!("[reason:: {value}]"))
    }
}

/// How often a task comes back: the plain intervals only. A rule with
/// weekdays or ordinals is refused rather than guessed at, because a wrong
/// guess writes a wrong date into the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recurrence {
    pub amount: u32,
    pub unit: Unit,
    pub when_done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^every(?:\s+(\d+))?\s+(day|week|month|year)s?$").unwrap());

impl Recurrence {
    pub fn parse(rule: &str) -> Option<Recurrence> {
        let mut text = trim(rule).to_lowercase();
        let when_done = text.ends_with("when done");
        if when_done {
            text = trim(&text[..text.len() - "when done".len()]).to_string();
        }
        let caps = RULE.captures(&text)?;
        let amount = match caps.get(1) {
            Some(m) => m.as_str().parse().ok()?,
            None => 1,
        };
        if amount == 0 {
            return None;
        }
        let unit = match &caps[2] {
            "day" => Unit::Day,
            "week" => Unit::Week,
            "month" => Unit::Month,
            _ => Unit::Year,
        };
        Some(Recurrence { amount, unit, when_done })
    }

    /// Stepped until it lands after [not_before], so a daily task ticked ten
    /// days late does not come back already overdue.
    pub fn next(&self, from: Day, not_before: Day) -> Day {
        let mut date = self.step(from);
        let mut guard = 0;
        while date <= not_before && guard < 4000 {
            date = self.step(date);
            guard += 1;
        }
        date
    }

    fn step(&self, day: Day) -> Day {
        let n = self.amount as i64;
        match self.unit {
            Unit::Day => day.adding_days(n),
            Unit::Week => day.adding_days(7 * n),
            Unit::Month => day.adding_months(n as i32),
            Unit::Year => day.adding_months(12 * n as i32),
        }
    }
}

/// A task's parts, as the edit form shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFields {
    pub status: char,
    pub text: String,
    /// The priority's emoji, 🔺 ⏫ 🔼 🔽 or ⏬; `None` for none.
    pub priority: Option<String>,
    pub recurring: Option<String>,
    pub created: Option<String>,
    pub start: Option<String>,
    pub scheduled: Option<String>,
    pub due: Option<String>,
    pub cancelled: Option<String>,
    pub done: Option<String>,
    /// The `[reason:: …]` field's value.
    pub reason: Option<String>,
    /// Signifiers the form does not show (🏁, ⛔, 🆔), kept as they are.
    pub other: Vec<Meta>,
}

impl Default for TaskFields {
    fn default() -> Self {
        TaskFields {
            status: ' ',
            text: String::new(),
            priority: None,
            recurring: None,
            created: None,
            start: None,
            scheduled: None,
            due: None,
            cancelled: None,
            done: None,
            reason: None,
            other: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggled(raw: &str, today: &str) -> Vec<String> {
        TaskLine::toggled(raw, today).unwrap()
    }

    #[test]
    fn tags_after_the_dates_stay_in_the_text() {
        let task = TaskLine::parse("- [ ] Fix the oven door 📅 2026-10-20 ⏫ #kit #home", 0, "").unwrap();
        assert_eq!(task.due.as_deref(), Some("2026-10-20"));
        assert_eq!(task.priority.as_deref(), Some("⏫"));
        assert_eq!(task.text, "Fix the oven door #kit #home");
    }

    #[test]
    fn complete_adds_the_done_date() {
        assert_eq!(toggled("- [ ] Ship it ⏳ 2026-10-04", "2026-10-05"), ["- [x] Ship it ⏳ 2026-10-04 ✅ 2026-10-05"]);
        assert_eq!(toggled("    - [/] Half done", "2026-10-05"), ["    - [x] Half done ✅ 2026-10-05"]);
    }

    #[test]
    fn reopen_takes_the_date_off() {
        assert_eq!(toggled("- [x] Ship it ⏳ 2026-10-04 ✅ 2026-10-05", "2026-10-06"), ["- [ ] Ship it ⏳ 2026-10-04"]);
        assert_eq!(toggled("- [-] Dropped ❌ 2026-10-05", "2026-10-06"), ["- [ ] Dropped"]);
    }

    #[test]
    fn repeat_leaves_the_next_one_above() {
        assert_eq!(
            toggled("- [ ] Water plants 🔁 every week ⏳ 2026-10-01 📅 2026-10-03", "2026-10-02"),
            [
                "- [ ] Water plants 🔁 every week ⏳ 2026-10-08 📅 2026-10-10",
                "- [x] Water plants 🔁 every week ⏳ 2026-10-01 📅 2026-10-03 ✅ 2026-10-02",
            ]
        );
    }

    #[test]
    fn late_repeat_lands_in_the_future() {
        assert_eq!(toggled("- [ ] Daily 🔁 every day ⏳ 2026-10-01", "2026-10-10")[0], "- [ ] Daily 🔁 every day ⏳ 2026-10-11");
    }

    #[test]
    fn repeat_when_done_counts_from_today() {
        assert_eq!(
            toggled("- [ ] Haircut 🔁 every month when done ⏳ 2026-01-31", "2026-03-15")[0],
            "- [ ] Haircut 🔁 every month when done ⏳ 2026-04-15"
        );
    }

    #[test]
    fn unknown_rule_completes_without_a_repeat() {
        assert_eq!(
            toggled("- [ ] Gym 🔁 every Monday ⏳ 2026-10-05", "2026-10-05"),
            ["- [x] Gym 🔁 every Monday ⏳ 2026-10-05 ✅ 2026-10-05"]
        );
    }

    #[test]
    fn the_block_id_stays_last() {
        assert_eq!(
            toggled("- [ ] t 🔁 every day 📅 2026-10-25 ^abc", "2026-10-25"),
            ["- [ ] t 🔁 every day 📅 2026-10-26", "- [x] t 🔁 every day 📅 2026-10-25 ✅ 2026-10-25 ^abc"]
        );
        assert_eq!(toggled("- [ ] t ^abc", "2026-10-25"), ["- [x] t ✅ 2026-10-25 ^abc"]);
        assert_eq!(toggled("- [x] t ✅ 2026-10-25 ^abc", "2026-10-26"), ["- [ ] t ^abc"]);
    }

    #[test]
    fn emoji_in_prose_is_prose() {
        let (text, meta) = TaskMetadata::split("Book ✅ the table — for dinner ⏳ 2026-10-04");
        assert_eq!(text, "Book ✅ the table — for dinner");
        assert_eq!(meta, [Meta::new("⏳", "2026-10-04")]);
    }

    const FULL: &str = "    - [/] Call [[Alpha]] #work 🔼 🔁 every week 🆔 abc ⏳ 2026-10-24 📅 2026-10-30 ^block-1";

    #[test]
    fn fields_reads_every_part() {
        let fields = TaskLine::fields(FULL).unwrap();
        assert_eq!(fields.status, '/');
        assert_eq!(fields.text, "Call [[Alpha]] #work");
        assert_eq!(fields.priority.as_deref(), Some("🔼"));
        assert_eq!(fields.recurring.as_deref(), Some("every week"));
        assert_eq!(fields.scheduled.as_deref(), Some("2026-10-24"));
        assert_eq!(fields.due.as_deref(), Some("2026-10-30"));
        assert_eq!(fields.other, [Meta::new("🆔", "abc")]);
        assert_eq!(TaskLine::fields("Just a line"), None);
    }

    #[test]
    fn edited_writes_in_the_plugins_order_and_keeps_the_rest() {
        let mut fields = TaskLine::fields(FULL).unwrap();
        assert_eq!(TaskLine::edited(FULL, &fields).as_deref(), Some(FULL));
        fields.due = Some("2026-11-02".into());
        fields.priority = Some("⏫".into());
        fields.start = Some("2026-10-26".into());
        fields.recurring = None;
        assert_eq!(
            TaskLine::edited(FULL, &fields).as_deref(),
            Some("    - [/] Call [[Alpha]] #work ⏫ 🆔 abc 🛫 2026-10-26 ⏳ 2026-10-24 📅 2026-11-02 ^block-1")
        );
    }

    #[test]
    fn a_field_cleared_leaves_no_emoji() {
        let mut fields = TaskLine::fields("> - [ ] Pay 📅 2026-10-30").unwrap();
        fields.due = None;
        fields.text = "Pay the rent".into();
        assert_eq!(TaskLine::edited("> - [ ] Pay 📅 2026-10-30", &fields).as_deref(), Some("> - [ ] Pay the rent"));
    }

    #[test]
    fn a_priority_takes_no_value_and_a_date_only_a_date() {
        let urgent = "- [ ] 🔺 urgent thing";
        let fields = TaskLine::fields(urgent).unwrap();
        assert_eq!(fields.priority, None);
        assert_eq!(fields.text, "🔺 urgent thing");
        assert_eq!(TaskLine::edited(urgent, &fields).as_deref(), Some(urgent));
        let (text, meta) = TaskMetadata::split("Call 📅 tomorrow morning ⏫");
        assert_eq!(text, "Call 📅 tomorrow morning");
        assert_eq!(meta, [Meta::new("⏫", "")]);
        assert_eq!(TaskMetadata::split("Gym 🔁 every week").1, [Meta::new("🔁", "every week")]);
    }

    #[test]
    fn two_fields_of_one_kind_drop_nothing() {
        for raw in ["- [ ] Move 📅 2026-10-25 then 📅 2026-10-26", "- [ ] Run 🔁 every day or 🔁 every week", "- [ ] Pick 🔼 ⏫"]
        {
            let fields = TaskLine::fields(raw).unwrap();
            assert_eq!(TaskLine::edited(raw, &fields).as_deref(), Some(raw));
        }
    }

    #[test]
    fn on_completion_is_a_field() {
        let raw = "- [ ] Bins 🔁 every day 🏁 delete 📅 2026-10-25";
        let fields = TaskLine::fields(raw).unwrap();
        assert_eq!(fields.text, "Bins");
        assert_eq!(fields.recurring.as_deref(), Some("every day"));
        assert_eq!(fields.due.as_deref(), Some("2026-10-25"));
        assert_eq!(fields.other, [Meta::new("🏁", "delete")]);
        assert_eq!(TaskLine::edited(raw, &fields).as_deref(), Some(raw));
    }

    #[test]
    fn the_reason_is_a_field_after_the_text() {
        let task = TaskLine::parse("- [-] Deploy [reason:: no budget] ➕ 2025-05-17 ❌ 2025-11-16", 0, "").unwrap();
        assert_eq!((task.text.as_str(), task.reason.as_deref()), ("Deploy", Some("no budget")));
        assert_eq!((task.created.as_deref(), task.cancelled.as_deref()), (Some("2025-05-17"), Some("2025-11-16")));
        let linked = TaskLine::parse("- [r] Ship [reason:: waiting on [[Amir]]] 📅 2026-11-02 ^imp", 0, "").unwrap();
        assert_eq!(
            (linked.text.as_str(), linked.reason.as_deref(), linked.due.as_deref()),
            ("Ship", Some("waiting on [[Amir]]"), Some("2026-11-02"))
        );
        let middle = TaskLine::parse("- [ ] Call [Reason ::busy] the bank", 0, "").unwrap();
        assert_eq!((middle.text.as_str(), middle.reason.as_deref()), ("Call the bank", Some("busy")));
        // Only the first is the reason; one that never closes is text.
        assert_eq!(TaskReason::take("a [reason:: one] b [reason:: two]"), ("a b [reason:: two]".into(), Some("one".into())));
        assert_eq!(TaskReason::take("a [reason:: open"), ("a [reason:: open".into(), None));
        assert_eq!(TaskReason::take("a [reason::] b"), ("a b".into(), None));
    }

    #[test]
    fn the_reason_is_written_before_the_signifiers() {
        let raw = "- [ ] T 📅 2026-11-02 ^b";
        let mut fields = TaskLine::fields(raw).unwrap();
        fields.reason = Some("a ] b".into());
        let with = TaskLine::edited(raw, &fields).unwrap();
        assert_eq!(with, "- [ ] T [reason:: a ) b] 📅 2026-11-02 ^b");
        let mut back = TaskLine::fields(&with).unwrap();
        assert_eq!(back.reason.as_deref(), Some("a ) b"));
        back.reason = Some("  ".into());
        assert_eq!(TaskLine::edited(&with, &back).as_deref(), Some(raw));
    }

    #[test]
    fn cancelling_with_a_reason_and_opening_again() {
        let cancelled = TaskLine::with_status("- [ ] Write ⏳ 2026-10-04", '-', Some("no\n time"), "2026-10-28").unwrap();
        assert_eq!(cancelled, ["- [-] Write [reason:: no time] ⏳ 2026-10-04 ❌ 2026-10-28"]);
        assert_eq!(toggled(&cancelled[0], "2026-10-29"), ["- [ ] Write ⏳ 2026-10-04"]);
        // A new status without a reason leaves the old one behind.
        assert_eq!(TaskLine::with_status(&cancelled[0], '/', None, "2026-10-29").unwrap(), ["- [/] Write ⏳ 2026-10-04"]);
        // The same status: only the reason changes, or nothing does.
        assert_eq!(
            TaskLine::with_status(&cancelled[0], '-', Some("later"), "2026-10-29").unwrap(),
            ["- [-] Write [reason:: later] ⏳ 2026-10-04 ❌ 2026-10-28"]
        );
        assert_eq!(TaskLine::with_status(&cancelled[0], '-', None, "2026-10-29"), None);
        assert_eq!(TaskLine::with_status("- [ ] W", 'x', None, "2026-10-29").unwrap(), ["- [x] W ✅ 2026-10-29"]);
        assert_eq!(TaskLine::with_status("- [x] W ✅ 2026-10-20", 'X', None, "2026-10-29").unwrap(), ["- [X] W ✅ 2026-10-20"]);
    }

    #[test]
    fn a_repeat_ticked_leaves_its_reason_behind() {
        assert_eq!(
            toggled("- [/] Water [reason:: away] 🔁 every week 📅 2026-10-25", "2026-10-25"),
            ["- [ ] Water 🔁 every week 📅 2026-11-01", "- [x] Water 🔁 every week 📅 2026-10-25 ✅ 2026-10-25"]
        );
    }

    #[test]
    fn locate_follows_a_moved_task() {
        let lines = ["# T", "- [ ] One", "- [ ] Two"];
        assert_eq!(TaskLine::locate(&lines, 2, "Two"), Some(2));
        assert_eq!(TaskLine::locate(&lines, 0, "Two"), Some(2));
        assert_eq!(TaskLine::locate(&lines, 1, "Three"), None);
    }
}
