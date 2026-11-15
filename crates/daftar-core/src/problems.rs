//! The Problems panel's findings (`docs/specs/problems-and-format.md`): what
//! `daftar-cli check`, `lint` and `fmt --check` say with `--json`, read,
//! counted and grouped by note. The port of `Problems.swift`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

use crate::text::standard_compare;

/// The command that found a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tool {
    Check,
    Lint,
    Fmt,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Check, Tool::Lint, Tool::Fmt];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Check => "check",
            Tool::Lint => "lint",
            Tool::Fmt => "fmt",
        }
    }

    fn parse(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.name() == name)
    }
}

/// An error comes before a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
}

/// One finding: what `just audit` and `just check` report about a note, at a line of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Problem {
    pub tool: Tool,
    pub level: Level,
    /// The note's vault path.
    pub path: String,
    /// From 1; 0 is the whole file.
    pub line: usize,
    /// From 1, when the tool knows it.
    pub column: Option<usize>,
    pub code: String,
    pub message: String,
    /// The tool's own fix mends it: `lint --fix`, `check --fix` or `fmt`.
    pub fixable: bool,
}

impl Problem {
    /// The source line from 0 the note opens at, or `None` for the whole file.
    pub fn source_line(&self) -> Option<usize> {
        self.line.checked_sub(1)
    }

    fn order(a: &Problem, b: &Problem) -> Ordering {
        a.level
            .cmp(&b.level)
            .then(a.line.cmp(&b.line))
            .then(a.column.unwrap_or(0).cmp(&b.column.unwrap_or(0)))
            .then_with(|| a.code.cmp(&b.code))
    }
}

/// What the panel lists: errors, warnings or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Filter {
    Errors,
    Warnings,
    #[default]
    All,
}

impl Filter {
    pub fn admits(self, problem: &Problem) -> bool {
        match self {
            Filter::Errors => problem.level == Level::Error,
            Filter::Warnings => problem.level == Level::Warning,
            Filter::All => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
}

impl Counts {
    pub fn total(self) -> usize {
        self.errors + self.warnings
    }

    /// "2 errors, 1 warning", "No problems".
    pub fn spoken(self) -> String {
        if self.total() == 0 {
            return "No problems".into();
        }
        let count = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
        let mut parts = Vec::new();
        if self.errors > 0 {
            parts.push(count(self.errors, "error"));
        }
        if self.warnings > 0 {
            parts.push(count(self.warnings, "warning"));
        }
        parts.join(", ")
    }
}

/// The findings of one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub path: String,
    /// Errors first, then by line and column.
    pub problems: Vec<Problem>,
    pub counts: Counts,
}

impl Note {
    /// Something in it a tool can mend.
    pub fn fixable(&self) -> bool {
        self.problems.iter().any(|problem| problem.fixable)
    }
}

/// Reading, counting and grouping the findings.
pub struct Problems;

impl Problems {
    /// The findings of JSON Lines output, one object a line. A line that is
    /// not a finding (blank, a progress line, an object without a path, a
    /// level or a tool this does not know) is skipped, never an error: a
    /// newer `daftar-cli` may say more than this reads.
    pub fn parse(output: &str) -> Vec<Problem> {
        output.lines().map(str::trim).filter(|line| line.starts_with('{')).filter_map(Self::finding).collect()
    }

    fn finding(line: &str) -> Option<Problem> {
        let Value::Object(object) = serde_json::from_str::<Value>(line).ok()? else { return None };
        let tool = Tool::parse(object.get("tool")?.as_str()?)?;
        let level = match object.get("level")?.as_str()? {
            "error" => Level::Error,
            "warning" => Level::Warning,
            _ => return None,
        };
        let path = object.get("path")?.as_str().filter(|path| !path.is_empty())?.to_string();
        let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Problem {
            tool,
            level,
            path,
            line: integer(object.get("line")).map_or(0, |line| usize::try_from(line).unwrap_or(0)),
            column: integer(object.get("column")).and_then(|column| usize::try_from(column).ok()).filter(|column| *column > 0),
            code: text("code"),
            message: text("message"),
            fixable: object.get("fixable").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub fn counts(problems: &[Problem]) -> Counts {
        let errors = problems.iter().filter(|problem| problem.level == Level::Error).count();
        Counts { errors, warnings: problems.len() - errors }
    }

    /// The findings the filter admits, by note: notes with errors first,
    /// then by path; in each note errors first, then by line. The same
    /// finding twice (two tools that say the same) is listed once.
    pub fn grouped(problems: &[Problem], filter: Filter) -> Vec<Note> {
        let mut seen = HashSet::new();
        let mut by_path: HashMap<&str, Vec<Problem>> = HashMap::new();
        for problem in problems {
            if filter.admits(problem) && seen.insert(problem) {
                by_path.entry(&problem.path).or_default().push(problem.clone());
            }
        }
        let mut notes: Vec<Note> = by_path
            .into_iter()
            .map(|(path, mut list)| {
                list.sort_by(Problem::order);
                Note { path: path.to_string(), counts: Self::counts(&list), problems: list }
            })
            .collect();
        notes.sort_by(|a, b| (b.counts.errors > 0).cmp(&(a.counts.errors > 0)).then_with(|| standard_compare(&a.path, &b.path)));
        notes
    }

    /// The findings of [path] by source line from 0, for the margin of the
    /// note on screen: errors first on each line. A finding of the whole
    /// file is on no line.
    pub fn lines(problems: &[Problem], path: &str) -> BTreeMap<usize, Vec<Problem>> {
        let mut out: BTreeMap<usize, Vec<Problem>> = BTreeMap::new();
        for problem in problems.iter().filter(|problem| problem.path == path) {
            if let Some(line) = problem.source_line() {
                out.entry(line).or_default().push(problem.clone());
            }
        }
        for list in out.values_mut() {
            list.sort_by(Problem::order);
        }
        out
    }
}

/// A JSON number that is a whole number; `true` is not one.
fn integer(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_number()?;
    if let Some(whole) = number.as_i64() {
        return Some(whole);
    }
    let float = number.as_f64()?;
    #[expect(clippy::cast_possible_truncation, reason = "a whole number below 1e15")]
    (float.fract() == 0.0 && float.abs() < 1e15).then_some(float as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = r#"{"tool":"check","level":"warning","path":"Notes/Plan.md","line":12,"column":null,"code":"link-heading","message":"no heading Goals","fixable":false}
{"tool":"lint","level":"error","path":"Notes/Plan.md","line":3,"column":5,"code":"MD034","message":"bare URL","fixable":true}

daftar-cli: 2 errors, 1 warnings in 2 notes
{"tool":"check","level":"error","path":"Home.md","line":9,"column":1,"code":"callout-body","message":"the body is joined to the title","fixable":false}
{"tool":"fmt","level":"error","path":"Notes/Plan.md","line":0,"column":null,"code":"fmt","message":"not formatted","fixable":true}
{"tool":"spell","level":"error","path":"Home.md","line":1,"code":"x","message":"a tool this does not know"}
{"tool":"check","level":"info","path":"Home.md","line":1,"code":"x","message":"a level this does not know"}
{"tool":"check","level":"error","line":1,"code":"x","message":"no path"}
{"tool":"lint","level":"warning","path":"Home.md","line":true,"column":-2,"code":"MD009","message":"spaces at the end"}
{not json
"#;

    fn problem(tool: Tool, level: Level, path: &str, line: usize, code: &str, message: &str) -> Problem {
        Problem { tool, level, path: path.into(), line, column: None, code: code.into(), message: message.into(), fixable: false }
    }

    #[test]
    fn findings_are_read_and_what_is_not_one_is_skipped() {
        let problems = Problems::parse(OUTPUT);
        assert_eq!(problems.len(), 5);
        assert_eq!(problems[0], problem(Tool::Check, Level::Warning, "Notes/Plan.md", 12, "link-heading", "no heading Goals"));
        assert_eq!(
            problems[1],
            Problem { column: Some(5), fixable: true, ..problem(Tool::Lint, Level::Error, "Notes/Plan.md", 3, "MD034", "bare URL") }
        );
        // Line 0 is the whole note: it opens at no line.
        assert!(problems[3].line == 0 && problems[3].source_line().is_none());
        assert_eq!(problems[1].source_line(), Some(2));
        // A line that is not a number, and a column below 1, are not taken as one.
        assert!(problems[4].line == 0 && problems[4].column.is_none());
        assert!(Problems::parse("").is_empty());
        assert!(Problems::parse("error: unexpected argument '--json' found\n").is_empty());
    }

    #[test]
    fn notes_with_errors_come_first_and_errors_first_in_each() {
        let problems = Problems::parse(OUTPUT);
        let notes = Problems::grouped(&problems, Filter::All);
        assert_eq!(notes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Home.md", "Notes/Plan.md"]);
        assert_eq!(notes[1].problems.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), ["fmt", "MD034", "link-heading"]);
        assert_eq!(notes[1].counts, Counts { errors: 2, warnings: 1 });
        assert!(notes[1].fixable());
        // A note with only warnings goes after every note with an error.
        let warned = problem(Tool::Check, Level::Warning, "A.md", 1, "w", "w");
        let more: Vec<Problem> = problems.iter().cloned().chain([warned]).collect();
        assert_eq!(
            Problems::grouped(&more, Filter::All).iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
            ["Home.md", "Notes/Plan.md", "A.md"]
        );
        assert!(Problems::grouped(&problems, Filter::Errors).iter().flat_map(|n| &n.problems).all(|p| p.level == Level::Error));
        assert_eq!(
            Problems::grouped(&problems, Filter::Warnings).iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
            ["Home.md", "Notes/Plan.md"]
        );
        // The same finding twice is listed once.
        let twice: Vec<Problem> = problems.iter().chain(&problems).cloned().collect();
        assert_eq!(Problems::grouped(&twice, Filter::All).iter().map(|n| n.problems.len()).sum::<usize>(), problems.len());
    }

    #[test]
    fn counts_are_said_in_words() {
        let problems = Problems::parse(OUTPUT);
        assert_eq!(Problems::counts(&problems), Counts { errors: 3, warnings: 2 });
        assert_eq!(Problems::counts(&problems).spoken(), "3 errors, 2 warnings");
        assert_eq!(Counts { errors: 1, warnings: 0 }.spoken(), "1 error");
        assert_eq!(Counts { errors: 0, warnings: 1 }.spoken(), "1 warning");
        assert_eq!(Counts::default().spoken(), "No problems");
    }

    #[test]
    fn the_margin_gets_the_lines_of_one_note() {
        let lines = Problems::lines(&Problems::parse(OUTPUT), "Notes/Plan.md");
        assert_eq!(lines.keys().copied().collect::<Vec<_>>(), [2, 11]);
        assert_eq!(lines[&2].iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), ["MD034"]);
    }

    /// The robustness line: from any JSON-ish text, every finding has a path,
    /// a line of at least 0 and a column of at least 1 or none, and the
    /// grouping keeps each distinct finding once.
    #[test]
    fn problems_from_any_text() {
        let words = [
            "{",
            "}",
            ",",
            ":",
            "\"tool\"",
            "\"level\"",
            "\"path\"",
            "\"line\"",
            "\"column\"",
            "\"code\"",
            "\"message\"",
            "\"fixable\"",
            "\"check\"",
            "\"lint\"",
            "\"fmt\"",
            "\"error\"",
            "\"warning\"",
            "\"a.md\"",
            "\"\"",
            "0",
            "-3",
            "12",
            "1e999",
            "2.0",
            "null",
            "true",
            "\n",
            " ",
        ];
        let mut state: u64 = 17;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..2000 {
            let text: String = (0..next(41)).map(|_| words[next(words.len())]).collect();
            let problems = Problems::parse(&text);
            assert!(problems.iter().all(|p| !p.path.is_empty() && p.column.is_none_or(|c| c >= 1)));
            let distinct: HashSet<&Problem> = problems.iter().collect();
            assert_eq!(Problems::grouped(&problems, Filter::All).iter().map(|n| n.problems.len()).sum::<usize>(), distinct.len());
        }
    }
}
