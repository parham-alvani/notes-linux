//! The Obsidian Tasks plugin's query language, the part of it that can be
//! answered from the index: filters on status, dates, path, heading, text and
//! tags, joined with `AND`/`OR`/`NOT`, then `group by`, `sort by` and
//! `limit`. The rules are Daftar's on the phone (`TasksQuery.kt`).
//!
//! What it cannot answer is kept in `not_understood` and shown. Dropping a
//! filter silently would list tasks the query was written to leave out.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::Datelike;
use regex::Regex;

use crate::day::Day;
use crate::task_expression::{TaskExpression, Value};
use crate::vault::VaultTask;

/// The four states the Tasks plugin knows a task by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Todo,
    InProgress,
    Done,
    Cancelled,
}

impl TaskState {
    pub fn new(status: char) -> TaskState {
        match status {
            'x' | 'X' => TaskState::Done,
            '-' => TaskState::Cancelled,
            '/' => TaskState::InProgress,
            // The plugin treats a status it does not know as still to do.
            _ => TaskState::Todo,
        }
    }

    /// Ticked or cancelled: no longer to be done.
    pub fn is_finished(self) -> bool {
        matches!(self, TaskState::Done | TaskState::Cancelled)
    }

    pub fn type_name(self) -> &'static str {
        match self {
            TaskState::Todo => "TODO",
            TaskState::InProgress => "IN_PROGRESS",
            TaskState::Done => "DONE",
            TaskState::Cancelled => "CANCELLED",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TaskState::Todo => "Todo",
            TaskState::InProgress => "In Progress",
            TaskState::Done => "Done",
            TaskState::Cancelled => "Cancelled",
        }
    }
}

/// One task as a ` ```tasks ` query sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTask {
    pub text: String,
    pub state: TaskState,
    /// The note's path inside the vault, `Folder/Note.md`.
    pub path: String,
    /// The heading above the task.
    pub section: String,
    pub due: Option<String>,
    pub scheduled: Option<String>,
    pub done: Option<String>,
    pub recurring: Option<String>,
}

impl QueryTask {
    pub fn new(text: &str, state: TaskState, path: &str) -> QueryTask {
        QueryTask {
            text: text.into(),
            state,
            path: path.into(),
            section: String::new(),
            due: None,
            scheduled: None,
            done: None,
            recurring: None,
        }
    }

    pub fn folder(&self) -> &str {
        Self::folder_of(&self.path)
    }

    pub fn filename(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// A note's folder as the plugin writes it: with its slash, or `/` at the root.
    pub fn folder_of(path: &str) -> &str {
        match path.rfind('/') {
            Some(slash) => &path[..=slash],
            None => "/",
        }
    }
}

impl From<&VaultTask> for QueryTask {
    fn from(task: &VaultTask) -> QueryTask {
        QueryTask {
            text: task.task.text.clone(),
            state: TaskState::new(task.task.status),
            path: task.path.clone(),
            section: task.task.section.clone(),
            due: task.task.due.clone(),
            scheduled: task.task.scheduled.clone(),
            done: task.task.done.clone(),
            recurring: task.task.recurring.clone(),
        }
    }
}

type Filter = Box<dyn Fn(&QueryTask, Day) -> bool + Send + Sync>;
type Comparison = Box<dyn Fn(&QueryTask, &QueryTask) -> Ordering + Send + Sync>;
type Grouper = Box<dyn Fn(&QueryTask) -> String + Send + Sync>;
type DateField = fn(&QueryTask) -> Option<String>;

#[derive(Default)]
pub struct TasksQuery {
    filters: Vec<Filter>,
    sorts: Vec<Comparison>,
    groupers: Vec<Grouper>,
    pub limit: Option<usize>,
    /// The lines this could not read, as written.
    pub not_understood: Vec<String>,
}

static LIMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^limit (?:to )?(\d+)(?: tasks?)?$").unwrap());
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{([^}]*)\}\}").unwrap());
static NEGATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)NOT\s*\(").unwrap());
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i)status\.type is( not)? (todo|in[_ ]progress|done|cancelled)$").unwrap());
static HAS_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)(has|no) (due|scheduled|done|happens) date$").unwrap());
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i)(due|scheduled|done|happens)( on or before| on or after| before| after| on| in)?\s+(.+)$").unwrap());
static TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i)(description|path|heading|filename|folder) (includes|does not include)\s+(.+)$").unwrap());
static TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?i)(tag|tags) (include|includes|do not include|does not include)\s+(.+)$").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^\p{L}\p{N}_])#([\p{L}\p{N}_/-]*[\p{L}_/-][\p{L}\p{N}_/-]*)").unwrap());

/// A layout line says how to draw the list, not which tasks are in it.
const LAYOUT: [&str; 6] = ["hide ", "show ", "short mode", "full mode", "explain", "ignore global query"];

const NAMED_GROUPS: [(&str, &str); 7] = [
    ("filename", "task.file.filenameWithoutExtension"),
    ("path", "task.file.path"),
    ("folder", "task.file.folder"),
    ("heading", "task.heading"),
    ("status", "task.status.name"),
    ("status.name", "task.status.name"),
    ("status.type", "task.status.type"),
];

impl TasksQuery {
    /// The tasks the query selects, in the order it asks for. With no
    /// `sort by`, open tasks come before finished ones and sooner before later;
    /// ties keep the order they were given in.
    pub fn run<T>(&self, tasks: impl IntoIterator<Item = T>, today: Day, view: impl Fn(&T) -> QueryTask) -> Vec<T> {
        let mut kept: Vec<(T, QueryTask)> = tasks
            .into_iter()
            .map(|t| {
                let task = view(&t);
                (t, task)
            })
            .filter(|(_, task)| self.filters.iter().all(|f| f(task, today)))
            .collect();
        kept.sort_by(|(_, a), (_, b)| {
            self.groupers
                .iter()
                .map(|group| group(a).cmp(&group(b)))
                .chain(self.sorts.iter().map(|sort| sort(a, b)))
                .chain(std::iter::once(default_order(a, b)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        });
        let sorted = kept.into_iter().map(|(t, _)| t);
        match self.limit {
            Some(limit) => sorted.take(limit).collect(),
            None => sorted.collect(),
        }
    }

    /// The groups a task is listed under, outermost first. A group may be
    /// named "" -- no heading, grouped by heading -- and has no title.
    pub fn groups(&self, task: &QueryTask) -> Vec<String> {
        self.groupers.iter().map(|group| group(task)).collect()
    }

    /// Reads the body of the fence. [file] is the path of the note the query
    /// is written in, which `{{query.file.*}}` stands for.
    pub fn parse(source: &str, file: Option<&str>) -> TasksQuery {
        let mut query = TasksQuery::default();
        for raw in source.split('\n') {
            let line = raw.trim();
            let lower = line.to_lowercase();
            // A blank line and a comment say nothing.
            if line.is_empty() || line.starts_with('#') || LAYOUT.iter().any(|l| lower.starts_with(l)) {
                continue;
            }
            let Some(expanded) = placeholders(line, file) else {
                query.not_understood.push(line.into());
                continue;
            };
            let low = expanded.to_lowercase();
            let understood = if low.starts_with("group by ") {
                group(&expanded["group by ".len()..]).map(|g| query.groupers.push(g)).is_some()
            } else if low.starts_with("filter by function ") {
                TaskExpression::parse(&expanded["filter by function ".len()..])
                    .map(|e| query.filters.push(Box::new(move |task, _| e.test(task))))
                    .is_some()
            } else if let Some(caps) = LIMIT.captures(&low) {
                query.limit = caps[1].parse().ok();
                true
            } else if let Some(spec) = low.strip_prefix("sort by ") {
                sort(spec).map(|s| query.sorts.push(s)).is_some()
            } else {
                expression(&expanded).map(|f| query.filters.push(f)).is_some()
            };
            if !understood {
                query.not_understood.push(line.into());
            }
        }
        query
    }
}

fn group(spec: &str) -> Option<Grouper> {
    let source = if spec.to_lowercase().starts_with("function ") {
        &spec["function ".len()..]
    } else {
        let wanted = spec.trim().to_lowercase();
        NAMED_GROUPS.iter().find(|(name, _)| *name == wanted)?.1
    };
    let expression = TaskExpression::parse(source)?;
    Some(Box::new(move |task| match expression.evaluate(task) {
        Value::Null => String::new(),
        Value::List(items) => items.iter().map(Value::display).collect::<Vec<_>>().join(", "),
        value => value.display(),
    }))
}

fn placeholders(line: &str, file: Option<&str>) -> Option<String> {
    if !line.contains("{{") {
        return Some(line.into());
    }
    let file = file?;
    let name = file.rsplit('/').next().unwrap_or(file);
    let values = HashMap::from([
        ("query.file.path", file),
        ("query.file.folder", QueryTask::folder_of(file)),
        ("query.file.filename", name),
        ("query.file.filenameWithoutExtension", name.strip_suffix(".md").unwrap_or(name)),
    ]);
    let mut unknown = false;
    let out = PLACEHOLDER.replace_all(line, |caps: &regex::Captures| match values.get(caps[1].trim()) {
        Some(value) => value.to_string(),
        None => {
            unknown = true;
            caps[0].to_string()
        }
    });
    (!unknown).then(|| out.into_owned())
}

// Filters

/// A filter line, which may be a boolean combination:
/// `(due today) OR (scheduled today)`, `NOT (done)`.
fn expression(line: &str) -> Option<Filter> {
    let text = line.trim();
    if !text.starts_with('(') && !NEGATION.is_match(text) {
        return filter(text);
    }
    Combination { text: text.chars().collect(), at: 0 }.parse()
}

fn filter(line: &str) -> Option<Filter> {
    let text = line.trim();
    match text.to_lowercase().as_str() {
        "done" => return Some(Box::new(|task, _| task.state.is_finished())),
        "not done" => return Some(Box::new(|task, _| !task.state.is_finished())),
        "is recurring" => return Some(Box::new(|task, _| task.recurring.is_some())),
        "is not recurring" => return Some(Box::new(|task, _| task.recurring.is_none())),
        _ => {}
    }
    if let Some(caps) = STATUS.captures(text) {
        let wanted = match caps[2].to_lowercase().replace(' ', "_").as_str() {
            "todo" => TaskState::Todo,
            "in_progress" => TaskState::InProgress,
            "done" => TaskState::Done,
            _ => TaskState::Cancelled,
        };
        let negate = caps.get(1).is_some();
        return Some(Box::new(move |task, _| (task.state == wanted) != negate));
    }
    if let Some(caps) = HAS_DATE.captures(text) {
        let field = date_field(&caps[2])?;
        let wants_one = caps[1].eq_ignore_ascii_case("has");
        return Some(Box::new(move |task, _| field(task).is_some() == wants_one));
    }
    if let Some(caps) = DATE.captures(text)
        && let Some(field) = date_field(&caps[1])
    {
        let relation = caps.get(2).map_or("on".to_string(), |m| m.as_str().trim().to_lowercase());
        let phrase = caps[3].trim().to_string();
        DateRange::new(&phrase, Day::new(2000, 1, 1).unwrap())?;
        return Some(Box::new(move |task, today| {
            let (Some(range), Some(day)) = (DateRange::new(&phrase, today), field(task).as_deref().and_then(Day::parse)) else {
                return false;
            };
            range.relation(&relation, day).unwrap_or(false)
        }));
    }
    if let Some(caps) = TEXT.captures(text) {
        let field: fn(&QueryTask) -> &str = match caps[1].to_lowercase().as_str() {
            "description" => |t| &t.text,
            "path" => |t| &t.path,
            "heading" => |t| &t.section,
            "filename" => QueryTask::filename,
            _ => QueryTask::folder,
        };
        let negate = caps[2].to_lowercase().starts_with("does not");
        let value = caps[3].trim().to_lowercase();
        return Some(Box::new(move |task, _| field(task).to_lowercase().contains(&value) != negate));
    }
    if let Some(caps) = TAGS.captures(text) {
        let negate = caps[2].to_lowercase().contains("not");
        let wanted = caps[3].trim();
        let tag = wanted.strip_prefix('#').unwrap_or(wanted).to_lowercase();
        return Some(Box::new(move |task, _| {
            let has = TAG.captures_iter(&task.text).any(|c| c[1].to_lowercase().contains(&tag));
            has != negate
        }));
    }
    None
}

fn date_field(name: &str) -> Option<DateField> {
    Some(match name.to_lowercase().as_str() {
        "due" => |t| t.due.clone(),
        "scheduled" => |t| t.scheduled.clone(),
        "done" => |t| t.done.clone(),
        // The plugin counts the start date too; the index does not keep it.
        "happens" => happens,
        _ => return None,
    })
}

fn happens(task: &QueryTask) -> Option<String> {
    [&task.scheduled, &task.due].into_iter().flatten().min().cloned()
}

// Sorting

fn sort(spec: &str) -> Option<Comparison> {
    let parts: Vec<&str> = spec.split_whitespace().collect();
    let reverse = parts.len() == 2 && parts[1] == "reverse";
    if parts.len() != 1 && !reverse {
        return None;
    }
    let key: Comparison = match parts[0] {
        "due" => Box::new(|a, b| dates(a.due.as_ref(), b.due.as_ref())),
        "scheduled" => Box::new(|a, b| dates(a.scheduled.as_ref(), b.scheduled.as_ref())),
        "done" => Box::new(|a, b| dates(a.done.as_ref(), b.done.as_ref())),
        "happens" => Box::new(|a, b| dates(happens(a).as_ref(), happens(b).as_ref())),
        "status" => Box::new(|a, b| a.state.is_finished().cmp(&b.state.is_finished())),
        "path" => Box::new(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase())),
        "filename" => Box::new(|a, b| a.filename().to_lowercase().cmp(&b.filename().to_lowercase())),
        "heading" => Box::new(|a, b| a.section.to_lowercase().cmp(&b.section.to_lowercase())),
        "description" => Box::new(|a, b| a.text.to_lowercase().cmp(&b.text.to_lowercase())),
        _ => return None,
    };
    Some(if reverse { Box::new(move |a, b| key(b, a)) } else { key })
}

/// Dated first, soonest first, as the plugin sorts a date.
fn dates(a: Option<&String>, b: Option<&String>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

fn default_order(a: &QueryTask, b: &QueryTask) -> Ordering {
    a.state
        .is_finished()
        .cmp(&b.state.is_finished())
        .then_with(|| dates(a.due.as_ref().or(a.scheduled.as_ref()), b.due.as_ref().or(b.scheduled.as_ref())))
}

/// `(a) AND (b)`, `(a) OR NOT (b)`, nested to any depth, left to right.
struct Combination {
    text: Vec<char>,
    at: usize,
}

impl Combination {
    fn parse(&mut self) -> Option<Filter> {
        let result = self.sequence()?;
        self.skip_space();
        (self.at == self.text.len()).then_some(result)
    }

    fn sequence(&mut self) -> Option<Filter> {
        let mut left = self.operand()?;
        loop {
            self.skip_space();
            let Some(op) = ["AND NOT ", "AND ", "OR ", "XOR "].into_iter().find(|op| self.starts(op)) else {
                return Some(left);
            };
            self.at += op.len();
            let right = self.operand()?;
            let l = left;
            left = match op.trim() {
                "AND" => Box::new(move |t, d| l(t, d) && right(t, d)),
                "OR" => Box::new(move |t, d| l(t, d) || right(t, d)),
                "XOR" => Box::new(move |t, d| l(t, d) != right(t, d)),
                _ => Box::new(move |t, d| l(t, d) && !right(t, d)),
            };
        }
    }

    fn starts(&self, word: &str) -> bool {
        let end = self.at + word.len();
        end <= self.text.len() && self.text[self.at..end].iter().collect::<String>().to_uppercase() == word
    }

    fn operand(&mut self) -> Option<Filter> {
        self.skip_space();
        let rest: String = self.text[self.at..].iter().collect();
        if let Some(found) = NEGATION.find(&rest) {
            // Up to the bracket, which the operand then reads.
            self.at += found.as_str().chars().count() - 1;
            let inner = self.operand()?;
            return Some(Box::new(move |t, d| !inner(t, d)));
        }
        if self.text.get(self.at) != Some(&'(') {
            return None;
        }
        let close = self.closing(self.at)?;
        let inside: String = self.text[self.at + 1..close].iter().collect();
        let inside = inside.trim();
        self.at = close + 1;
        if inside.starts_with('(') || NEGATION.is_match(inside) {
            return Combination { text: inside.chars().collect(), at: 0 }.parse();
        }
        filter(inside)
    }

    fn closing(&self, open: usize) -> Option<usize> {
        let mut depth = 0;
        for (index, &c) in self.text.iter().enumerate().skip(open) {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        None
    }

    fn skip_space(&mut self) {
        while self.text.get(self.at).is_some_and(|c| c.is_whitespace()) {
            self.at += 1;
        }
    }
}

/// The days a date phrase in a query means: `today`, `2026-10-09`, `this week`,
/// `next month`, `in 3 days`, or two dates for a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub start: Day,
    pub end: Day,
}

static IN_DAYS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^in (\d+) days?$").unwrap());
static DAYS_AGO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+) days? ago$").unwrap());
static BETWEEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})\s+(\d{4}-\d{2}-\d{2})$").unwrap());

impl DateRange {
    pub fn new(phrase: &str, today: Day) -> Option<DateRange> {
        let text = phrase.trim().to_lowercase();
        let monday = today.adding_days(-(today.date().weekday().num_days_from_monday() as i64));
        let first = Day::new(today.year(), today.month(), 1)?;
        let day = |d: Day| DateRange { start: d, end: d };
        let span = |start: Day, days: i64| DateRange { start, end: start.adding_days(days) };
        let month = |start: Day| DateRange { start, end: start.adding_months(1).adding_days(-1) };
        Some(match text.as_str() {
            "today" => day(today),
            "tomorrow" => day(today.adding_days(1)),
            "yesterday" => day(today.adding_days(-1)),
            "this week" => span(monday, 6),
            "next week" => span(monday.adding_days(7), 6),
            "last week" => span(monday.adding_days(-7), 6),
            "this month" => month(first),
            "next month" => month(first.adding_months(1)),
            "last month" => month(first.adding_months(-1)),
            _ => {
                if let Some(caps) = IN_DAYS.captures(&text) {
                    day(today.adding_days(caps[1].parse().ok()?))
                } else if let Some(caps) = DAYS_AGO.captures(&text) {
                    day(today.adding_days(-caps[1].parse::<i64>().ok()?))
                } else if let Some(caps) = BETWEEN.captures(&text) {
                    let (a, b) = (Day::parse(&caps[1])?, Day::parse(&caps[2])?);
                    if a > b {
                        return None;
                    }
                    DateRange { start: a, end: b }
                } else {
                    day(Day::parse(&text)?)
                }
            }
        })
    }

    pub fn relation(&self, relation: &str, date: Day) -> Option<bool> {
        Some(match relation {
            "before" => date < self.start,
            "after" => date > self.end,
            "on" | "in" => date >= self.start && date <= self.end,
            "on or before" => date <= self.end,
            "on or after" => date >= self.start,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // A Wednesday.
    fn today() -> Day {
        Day::parse("2026-10-21").unwrap()
    }

    fn tasks() -> Vec<QueryTask> {
        let task =
            |text: &str, state, path: &str, section: &str| QueryTask { section: section.into(), ..QueryTask::new(text, state, path) };
        vec![
            QueryTask { scheduled: Some("2026-10-20".into()), ..task("Ship the plan #work", TaskState::Todo, "Tasks/Work.md", "Alpha") },
            task("Half way", TaskState::InProgress, "Tasks/Work.md", "Beta"),
            QueryTask { due: Some("2026-10-28".into()), ..task("Book flights", TaskState::Todo, "Tasks/Home.md", "") },
            QueryTask { done: Some("2026-10-01".into()), ..task("Old one", TaskState::Done, "Tasks/Home.md", "Done") },
            QueryTask { scheduled: Some("2026-10-22".into()), ..task("Reading", TaskState::Todo, "Books/Reading.md", "") },
            task("Dropped", TaskState::Cancelled, "Tasks/Work.md", "Alpha"),
        ]
    }

    fn run(source: &str) -> Vec<String> {
        TasksQuery::parse(source, Some("Tasks/Tasks.md")).run(tasks(), today(), |t| t.clone()).into_iter().map(|t| t.text).collect()
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_vaults_in_progress_query() {
        let source = "folder includes {{query.file.folder}}\n\
            filter by function task.status.type === \"IN_PROGRESS\" && task.file.filenameWithoutExtension != \"Books\"\n\
            group by function task.file.filenameWithoutExtension\n\
            group by function task.hasHeading ? task.heading : ''\n\
            show tree\n\
            short mode";
        let query = TasksQuery::parse(source, Some("Tasks/Tasks.md"));
        assert!(query.not_understood.is_empty(), "{:?}", query.not_understood);
        let found = query.run(tasks(), today(), |t| t.clone());
        assert_eq!(found.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), ["Half way"]);
        assert_eq!(query.groups(&found[0]), ["Work", "Beta"]);
    }

    #[test]
    fn dates_and_ranges() {
        assert_eq!(run("scheduled on or before this week\nnot done"), ["Ship the plan #work", "Reading"]);
        assert_eq!(run("due next week"), ["Book flights"]);
        assert_eq!(run("scheduled before today"), ["Ship the plan #work"]);
        assert_eq!(run("no due date\nnot done\nfolder includes Tasks/"), ["Ship the plan #work", "Half way"]);
        assert_eq!(run("done on 2026-10-01"), ["Old one"]);
    }

    #[test]
    fn booleans_and_text() {
        assert_eq!(run("(due next week) OR (scheduled tomorrow)").into_iter().collect::<HashSet<_>>(), set(&["Book flights", "Reading"]));
        assert_eq!(run("NOT (done)\nheading includes alpha"), ["Ship the plan #work"]);
        assert_eq!(run("tags include work"), ["Ship the plan #work"]);
        assert_eq!(run("description does not include o\nnot done"), ["Reading", "Half way"]);
    }

    #[test]
    fn done_means_finished() {
        assert_eq!(run("done").into_iter().collect::<HashSet<_>>(), set(&["Old one", "Dropped"]));
    }

    #[test]
    fn sort_and_limit() {
        assert_eq!(run("not done\nsort by description\nlimit 2"), ["Book flights", "Half way"]);
        assert_eq!(run("not done\nsort by path reverse\nlimit 1"), ["Ship the plan #work"]);
    }

    #[test]
    fn unknown_lines_are_reported() {
        let query = TasksQuery::parse(
            "priority is high\nfilter by function task.urgency > 3\nfolder includes {{query.file.unknown}}",
            Some("A.md"),
        );
        assert_eq!(query.not_understood.len(), 3);
        assert_eq!(TasksQuery::parse("folder includes {{query.file.folder}}", None).not_understood.len(), 1);
    }

    #[test]
    fn expressions() {
        let task = &tasks()[0];
        let parse = |s| TaskExpression::parse(s).expect(s);
        assert!(parse("task.description.toLowerCase().includes('ship')").test(task));
        assert!(parse("['Work', 'Home'].includes(task.file.filenameWithoutExtension)").test(task));
        assert!(parse("!task.isDone && task.file.folder === 'Tasks/'").test(task));
        assert!(TaskExpression::parse("task.urgency").is_none());
        assert!(TaskExpression::parse("1 + 2").is_none());
    }
}
