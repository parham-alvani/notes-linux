//! A finding as GitHub Actions shows it: a workflow command, which the run's
//! page puts on the file's line in the pull request and the summary.

/// Whether this runs in GitHub Actions.
pub fn github() -> bool {
    std::env::var("GITHUB_ACTIONS").is_ok_and(|v| v == "true")
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
}
