//! Quick capture: a line typed in the small capture window or given to
//! `daftar-cli capture`, added to the end of a note or of one of its
//! sections. Every line the note had stays, in order. The port of
//! `QuickCapture.swift`; the Mac's global hot key is the compositor's bind
//! here, so `HotKeySpec` has no port.

use std::path::Path;
use std::sync::LazyLock;

use chrono::{DateTime, Local};
use regex::Regex;

use crate::bases::format_date;
use crate::note_parser::NoteParser;
use crate::vault::Vault;
use crate::vault_edits::VaultEdits;

static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[ \t]*(?:[-*+]|\d+[.)])[ \t]").unwrap());
static VARIABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{\s*(title|date|time)\s*(?::([^}]*))?\}\}").unwrap());

pub struct QuickCapture;

impl QuickCapture {
    /// The line to add for what was typed, or none for nothing. A capture is
    /// one line: line breaks become spaces. As a task, it is `- [ ] text`.
    pub fn line(typed: &str, task: bool) -> Option<String> {
        let text = typed.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return None;
        }
        if !task {
            return Some(text);
        }
        // "- [ ] x" or "[ ] x" typed by hand is a task already.
        if text.starts_with("- [") {
            return Some(text);
        }
        Some(format!("- [ ] {}", text.strip_prefix("[ ] ").unwrap_or(&text)))
    }

    /// [text] with [line] added: at the end of the section under [heading]
    /// (with its subsections), or at the end of the note when [heading] is
    /// none or empty. A heading not in the note is made at its end, as
    /// `## Name`, or at the level its own `#` marks give (`### Inbox`). The
    /// match is the heading's text, without case; the first one wins.
    pub fn append(line: &str, text: &str, heading: Option<&str>) -> String {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        // A note written with CRLF gets CRLF lines.
        let eol = if lines.len() > 1 && lines[0].ends_with('\r') { "\r" } else { "" };
        let (name, level) = heading_name(heading.unwrap_or(""));
        if name.is_empty() {
            let at = last_filled(&lines, 0, lines.len()).map_or(0, |l| l + 1);
            insert(&[line.to_string()], at, &mut lines, eol);
            return lines.join("\n");
        }
        let headings = NoteParser::parse(text).headings;
        match headings.iter().position(|h| fold(&h.text) == fold(&name)) {
            Some(found) => {
                let start = headings[found].line;
                let end = headings[found + 1..].iter().find(|h| h.level <= headings[found].level).map_or(lines.len(), |h| h.line);
                let at = last_filled(&lines, start + 1, end).unwrap_or(start) + 1;
                insert(&[line.to_string()], at, &mut lines, eol);
            }
            None => {
                let at = last_filled(&lines, 0, lines.len()).map_or(0, |l| l + 1);
                insert(&[format!("{} {name}", "#".repeat(level)), line.to_string()], at, &mut lines, eol);
            }
        }
        lines.join("\n")
    }

    /// Adds [line] to the note [target] names (`capture.target`), under
    /// [heading] (`capture.heading`), made first when it is not there: the
    /// daily note from its template, a named note empty. The file is read
    /// again at the write, so a change made meanwhile stays; never a file
    /// outside the vault. The note's vault path.
    pub fn capture(root: &Path, target: &str, heading: &str, line: &str, now: DateTime<Local>) -> Result<String, String> {
        let mut path = Self::target_path(root, target, now);
        if path.starts_with('/') || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            return Err(format!("{path} is not a note in the vault."));
        }
        // "inbox" is Inbox.md, as the quick switcher finds it.
        if !root.join(&path).exists()
            && let Some(found) = Vault::index(root, None).files.iter().find(|file| file.to_lowercase() == path.to_lowercase())
        {
            path = found.clone();
        }
        let file = root.join(&path);
        if !file.exists() {
            let daily = target.trim().is_empty() || target.trim().eq_ignore_ascii_case("daily");
            let first = if daily { DailyNotes::read(root).first_text(root, &Vault::title(&path), now) } else { String::new() };
            if let Some(folder) = file.parent() {
                std::fs::create_dir_all(folder).map_err(|e| format!("{path}: {e}"))?;
            }
            std::fs::write(&file, first).map_err(|e| format!("{path}: {e}"))?;
        }
        let heading = Some(heading.trim()).filter(|h| !h.is_empty());
        VaultEdits::apply(&path, root, |text| Ok(Self::append(line, text, heading))).map_err(|e| format!("{path}: {e}"))?;
        Ok(path)
    }

    /// The note a capture goes to: [target] as the settings write it,
    /// "daily" (or nothing) for today's daily note, else a note's path, with
    /// `.md` added when it has none.
    pub fn target_path(root: &Path, target: &str, now: DateTime<Local>) -> String {
        let target = target.trim();
        if target.is_empty() || target.eq_ignore_ascii_case("daily") {
            return DailyNotes::read(root).path(now);
        }
        if target.to_lowercase().ends_with(".md") { target.to_string() } else { format!("{target}.md") }
    }
}

/// "## Inbox" is Inbox at level 2; a bare name is level 2 too.
fn heading_name(spec: &str) -> (String, usize) {
    let trimmed = spec.trim();
    let marks = trimmed.chars().take_while(|&c| c == '#').count();
    let name = trimmed[marks..].trim().to_string();
    (name, if (1..=6).contains(&marks) { marks } else { 2 })
}

fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

/// The last line in `from..to` with something on it.
fn last_filled(lines: &[String], from: usize, to: usize) -> Option<usize> {
    (from..to.min(lines.len())).rev().find(|&i| !lines[i].trim().is_empty())
}

/// [new] at [at], after a blank line when the line above would take it in:
/// a paragraph goes on into a plain line, and `---` above text makes a
/// heading of it. A list item after a list item, or anything right under a
/// heading, needs none.
fn insert(new: &[String], at: usize, lines: &mut Vec<String>, eol: &str) {
    let mut block: Vec<String> = new.to_vec();
    if at > 0 {
        let above = lines[at - 1].trim();
        let after_marks = above.trim_start_matches('#');
        let heading = above.starts_with('#') && after_marks.chars().next().is_none_or(|c| c == ' ' || c == '\t');
        let list = LIST_ITEM.is_match(above) && LIST_ITEM.is_match(&new[0]);
        if !above.is_empty() && !heading && !list {
            block.insert(0, String::new());
        }
    }
    let at = at.min(lines.len());
    lines.splice(at..at, block.into_iter().map(|l| l + eol));
}

/// Where Obsidian's Daily notes plugin keeps a day's note: the settings of
/// `.obsidian/daily-notes.json`. Blank or missing means the default. The
/// part of `DailyNotes` (`PeriodicNotes.swift`) a capture needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyNotes {
    /// Vault-relative, empty for the root.
    pub folder: String,
    /// A moment format, which may itself hold `/` and so subfolders.
    pub format: String,
    /// The template's path without `.md`, empty for none.
    pub template: String,
}

impl DailyNotes {
    pub fn new(folder: &str, format: &str, template: &str) -> DailyNotes {
        let format = format.trim();
        DailyNotes {
            folder: folder.trim().trim_matches('/').to_string(),
            format: if format.is_empty() { "YYYY-MM-DD".into() } else { format.into() },
            template: template.trim().to_string(),
        }
    }

    pub fn read(root: &Path) -> DailyNotes {
        let json: serde_json::Value = std::fs::read_to_string(root.join(".obsidian/daily-notes.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let field = |key: &str| json.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
        DailyNotes::new(&field("folder"), &field("format"), &field("template"))
    }

    /// The note of the day [now] falls on.
    pub fn path(&self, now: DateTime<Local>) -> String {
        let name = format!("{}.md", format_date(now, &self.format));
        if self.folder.is_empty() { name } else { format!("{}/{name}", self.folder) }
    }

    /// The text a new daily note starts with: its template filled in, as
    /// Obsidian's Templates plugin fills it, or nothing.
    pub fn first_text(&self, root: &Path, title: &str, now: DateTime<Local>) -> String {
        if self.template.is_empty() {
            return String::new();
        }
        let file = if self.template.to_lowercase().ends_with(".md") { self.template.clone() } else { format!("{}.md", self.template) };
        let Ok(template) = std::fs::read_to_string(root.join(file)) else { return String::new() };
        fill(&template, title, now)
    }
}

/// [template] with `{{title}}`, `{{date}}`, `{{time}}` and their `:FORMAT`
/// forms filled in; a variable it does not know stays as it is.
pub fn fill(template: &str, title: &str, now: DateTime<Local>) -> String {
    VARIABLE
        .replace_all(template, |caps: &regex::Captures| {
            let format = caps.get(2).map(|m| m.as_str().trim().to_string());
            match &caps[1] {
                "title" => title.to_string(),
                "date" => format_date(now, format.as_deref().unwrap_or("YYYY-MM-DD")),
                _ => format_date(now, format.as_deref().unwrap_or("HH:mm")),
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn append(line: &str, text: &str, heading: Option<&str>) -> String {
        QuickCapture::append(line, text, heading)
    }

    #[test]
    fn the_typed_text_is_one_line() {
        assert_eq!(QuickCapture::line("  Call Sara \n about the trip ", false).as_deref(), Some("Call Sara about the trip"));
        assert_eq!(QuickCapture::line("Call Sara", true).as_deref(), Some("- [ ] Call Sara"));
        assert_eq!(QuickCapture::line("[ ] Call Sara", true).as_deref(), Some("- [ ] Call Sara"));
        assert_eq!(QuickCapture::line("- [ ] Call Sara 📅 2026-10-27", true).as_deref(), Some("- [ ] Call Sara 📅 2026-10-27"));
        assert_eq!(QuickCapture::line(" \n ", true), None);
        assert_eq!(QuickCapture::line("", false), None);
    }

    #[test]
    fn goes_at_the_end_of_the_note() {
        assert_eq!(append("- [ ] New", "", None), "- [ ] New\n");
        assert_eq!(append("- [ ] New", "# Day\n- [ ] Old\n", None), "# Day\n- [ ] Old\n- [ ] New\n");
        // Blank lines at the end stay at the end.
        assert_eq!(append("- [ ] New", "# Day\n- [ ] Old\n\n\n", None), "# Day\n- [ ] Old\n- [ ] New\n\n\n");
        assert_eq!(append("- [ ] New", "- [ ] Old", None), "- [ ] Old\n- [ ] New");
        // Under a heading, at once.
        assert_eq!(append("Idea", "# Day\n", None), "# Day\nIdea\n");
    }

    #[test]
    fn a_blank_line_keeps_it_out_of_the_paragraph_above() {
        assert_eq!(append("Idea", "Some text\n", None), "Some text\n\nIdea\n");
        assert_eq!(append("- [ ] New", "Some text\n", None), "Some text\n\n- [ ] New\n");
        assert_eq!(append("Idea", "- item\n", None), "- item\n\nIdea\n");
        // Text right under `---` would make a heading of it.
        assert_eq!(append("Idea", "---\nkind: day\n---\n", None), "---\nkind: day\n---\n\nIdea\n");
        assert_eq!(append("- b", "1. a\n    - nested\n", None), "1. a\n    - nested\n- b\n");
    }

    #[test]
    fn goes_at_the_end_of_its_section() {
        let note = "# 2026-10-26\n## Inbox\n- [ ] Old\n\n### Later\n- [ ] Sub\n\n## Notes\nText\n";
        assert_eq!(
            append("- [ ] New", note, Some("inbox")),
            "# 2026-10-26\n## Inbox\n- [ ] Old\n\n### Later\n- [ ] Sub\n- [ ] New\n\n## Notes\nText\n"
        );
        assert_eq!(
            append("- [ ] New", note, Some("## Notes")),
            "# 2026-10-26\n## Inbox\n- [ ] Old\n\n### Later\n- [ ] Sub\n\n## Notes\nText\n\n- [ ] New\n"
        );
        // An empty section: right under its heading.
        assert_eq!(append("x", "## Inbox\n\n## Notes\n", Some("Inbox")), "## Inbox\nx\n\n## Notes\n");
        // An empty heading setting is the end of the note.
        assert_eq!(append("x", "## Inbox\n- a\n## Notes\n", Some(" ")), "## Inbox\n- a\n## Notes\nx\n");
    }

    #[test]
    fn a_missing_heading_is_made_at_the_end() {
        assert_eq!(append("- [ ] New", "# Day\nText\n", Some("Inbox")), "# Day\nText\n\n## Inbox\n- [ ] New\n");
        assert_eq!(append("- [ ] New", "", Some("### Inbox")), "### Inbox\n- [ ] New\n");
        // A heading in code is not a heading.
        assert_eq!(append("x", "```\n## Inbox\n```\n", Some("Inbox")), "```\n## Inbox\n```\n\n## Inbox\nx\n");
    }

    #[test]
    fn keeps_crlf() {
        assert_eq!(append("x", "## Inbox\r\n- a\r\n", Some("Inbox")), "## Inbox\r\n- a\r\n\r\nx\r\n");
        assert_eq!(append("- b", "- a\r\n", None), "- a\r\n- b\r\n");
    }

    #[test]
    fn the_target_is_todays_note_or_a_named_one() {
        let root = tempfile::tempdir().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 26, 9, 5, 0).unwrap();
        assert_eq!(QuickCapture::target_path(root.path(), "daily", now), "2026-10-26.md");
        assert_eq!(QuickCapture::target_path(root.path(), " ", now), "2026-10-26.md");
        assert_eq!(QuickCapture::target_path(root.path(), "Inbox", now), "Inbox.md");
        assert_eq!(QuickCapture::target_path(root.path(), "Notes/Later.md", now), "Notes/Later.md");
        std::fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        std::fs::create_dir_all(root.path().join("Templates")).unwrap();
        std::fs::write(
            root.path().join(".obsidian/daily-notes.json"),
            r#"{"format":"YYYY/MM/YYYY-MM-DD","folder":"/Journal/","template":"Templates/Day"}"#,
        )
        .unwrap();
        std::fs::write(root.path().join("Templates/Day.md"), "# {{title}}\n{{date:dddd}} {{time}} {{other}}\n## Inbox\n").unwrap();
        assert_eq!(QuickCapture::target_path(root.path(), "DAILY", now), "Journal/2026/10/2026-10-26.md");
        let daily = DailyNotes::read(root.path());
        assert_eq!(daily.first_text(root.path(), "2026-10-26", now), "# 2026-10-26\nMonday 09:05 {{other}}\n## Inbox\n");
    }

    /// The robustness line: every line the note had stays, in order, and the
    /// capture is there once.
    #[test]
    fn a_capture_keeps_every_line() {
        let words = ["# Inbox", "## Note", "text", "", "- a", "```", "---", "word", "1. x", "\r"];
        let mut state: u64 = 3;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..500 {
            let text: Vec<&str> = (0..next(12)).map(|_| words[next(words.len())]).collect();
            let text = text.join("\n");
            for heading in [None, Some("Inbox"), Some("## Note"), Some("word")] {
                let added = append("captured ✱", &text, heading);
                let mut rest = added.split('\n');
                for line in text.split('\n') {
                    assert!(rest.any(|l| l == line), "{text:?} {heading:?} {added:?}");
                }
                assert_eq!(added.split('\n').filter(|l| l.starts_with("captured ✱")).count(), 1);
            }
        }
    }
}
