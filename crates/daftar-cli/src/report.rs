//! How `lint`, `check` and `fmt --check` print a finding: as a plain line
//! for a person, `grep` or `fzf`; as a GitHub Actions workflow command,
//! which the run's page puts on the file's line in the pull request and the
//! summary; or with `--json` as a JSON object on one line, for a program
//! (Daftar for macOS runs the command and reads them).

/// Whether this runs in GitHub Actions.
pub fn github() -> bool {
    std::env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true")
}

/// How the findings are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The command's own plain lines.
    Plain,
    /// GitHub Actions annotations.
    GitHub,
    /// JSON Lines, one finding a line, and nothing else on stdout.
    Json,
}

impl Mode {
    /// JSON when asked, else annotations in GitHub Actions, else plain.
    pub fn of(json: bool) -> Mode {
        if json {
            Mode::Json
        } else if github() {
            Mode::GitHub
        } else {
            Mode::Plain
        }
    }
}

/// One thing a tool found in a file.
pub struct Finding<'a> {
    /// `check`, `lint` or `fmt`.
    pub tool: &'a str,
    /// `error` or `warning`.
    pub level: &'a str,
    /// The file, from the vault's folder.
    pub path: &'a str,
    /// From 1; 0 is the whole file.
    pub line: usize,
    pub column: Option<usize>,
    /// What kind of finding: the annotation's title, as `MD009/no-trailing-spaces`,
    /// `callout-body` or `fmt`.
    pub code: &'a str,
    pub message: &'a str,
    /// Whether `lint --fix`, `check --fix` or `fmt` would mend it.
    pub fixable: bool,
}

impl Finding<'_> {
    /// The finding as [mode] prints it; none in [Mode::Plain], which each
    /// command writes in its own way.
    pub fn printed(&self, mode: Mode) -> Option<String> {
        match mode {
            Mode::Plain => None,
            Mode::GitHub => Some(annotation(self.level, self.path, self.line, self.column, self.code, self.message)),
            Mode::Json => Some(self.json()),
        }
    }

    /// `{"tool":…,"level":…,"path":…,"line":…,"column":…,"code":…,"message":…,"fixable":…}`,
    /// the keys in that order, on one line.
    pub fn json(&self) -> String {
        let mut object = serde_json::Map::new();
        object.insert("tool".into(), self.tool.into());
        object.insert("level".into(), self.level.into());
        object.insert("path".into(), self.path.into());
        object.insert("line".into(), self.line.into());
        object.insert("column".into(), self.column.into());
        object.insert("code".into(), self.code.into());
        object.insert("message".into(), self.message.into());
        object.insert("fixable".into(), self.fixable.into());
        serde_json::Value::Object(object).to_string()
    }
}

/// `::error file=…,line=…,col=…,title=…::message`; [level] is `error`,
/// `warning` or `notice`, and a [line] of 0 is the whole file.
pub fn annotation(level: &str, path: &str, line: usize, column: Option<usize>, title: &str, message: &str) -> String {
    let mut properties = format!("file={}", property(path));
    if line > 0 {
        properties.push_str(&format!(",line={line}"));
    }
    if let Some(column) = column {
        properties.push_str(&format!(",col={column}"));
    }
    properties.push_str(&format!(",title={}", property(title)));
    format!("::{level} {properties}::{}", data(message))
}

fn data(text: &str) -> String {
    text.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A")
}

fn property(text: &str) -> String {
    data(text).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_what_the_command_reads() {
        assert_eq!(
            annotation("error", "A, B.md", 3, Some(2), "MD009/no-trailing-spaces", "50% done\nnext"),
            "::error file=A%2C B.md,line=3,col=2,title=MD009/no-trailing-spaces::50%25 done%0Anext"
        );
        assert_eq!(
            annotation("warning", "x.png", 0, None, "unused-picture", "no note shows it"),
            "::warning file=x.png,title=unused-picture::no note shows it"
        );
    }

    #[test]
    fn a_finding_is_one_json_line_in_a_fixed_order() {
        let finding = Finding {
            tool: "lint",
            level: "error",
            path: "A/B \"c\".md",
            line: 12,
            column: None,
            code: "MD009/no-trailing-spaces",
            message: "Trailing spaces\nnext",
            fixable: true,
        };
        assert_eq!(
            finding.json(),
            r#"{"tool":"lint","level":"error","path":"A/B \"c\".md","line":12,"column":null,"code":"MD009/no-trailing-spaces","message":"Trailing spaces\nnext","fixable":true}"#
        );
        assert_eq!(finding.printed(Mode::Plain), None);
        assert_eq!(finding.printed(Mode::Json), Some(finding.json()));
        assert!(finding.printed(Mode::GitHub).unwrap().starts_with("::error file=A/B \"c\".md,line=12,title="));
    }
}
