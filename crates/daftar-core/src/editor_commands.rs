//! The editing rules that are more than inserting a character: what Return
//! does in a list, what Tab does to an item, what Ctrl+Return does to a line.
//! Pure functions of one line, so the editor stays thin and these are tested.
//!
//! Offsets into a text are in **characters** (Unicode scalar values), as
//! GtkTextBuffer counts them; the Mac counts UTF-16 units instead.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use chrono::NaiveDateTime;
use regex::Regex;

use crate::day::Day;
use crate::task_line::TaskLine;
use crate::vault::Vault;

static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*(?:>\s?)*\s*)([-*+]|(\d+)([.)]))(\s+)(\[[^\]]\]\s+)?(.*)$").unwrap());
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9+.-]*):(.*)$").unwrap());
static EXTRACT_LEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(?:#{1,6}\s+|[-*+]\s+(?:\[.\]\s+)?|\d+[.)]\s+)?").unwrap());
static EXTRACT_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").unwrap());
static HEADING_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*#{1,6}\s+.*$").unwrap());

const MEDIA: [&str; 18] =
    ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif", "mp3", "m4a", "wav", "ogg", "flac", "mp4", "mov", "webm", "mkv", "pdf"];

/// What Return inserts after a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Return {
    /// The next item's marker, with the newline before it.
    Insert(String),
    /// An empty item ends the list instead: the editor clears the marker.
    EndList,
}

/// An edit to a text: the characters in [range] become [text], and the
/// selection after it. Offsets in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

pub struct EditorCommands;

impl EditorCommands {
    /// The vault indents a nested item by four spaces.
    pub const INDENT_UNIT: &'static str = "    ";

    /// What Return inserts after [line], or `None` for a plain newline.
    pub fn on_return(line: &str) -> Option<Return> {
        let caps = ITEM.captures(line)?;
        let (indent, space, text) = (&caps[1], &caps[5], &caps[7]);
        if text.trim().is_empty() {
            return Some(Return::EndList);
        }
        let mut marker = caps[2].to_string();
        if let (Some(number), Some(close)) = (caps.get(3).and_then(|n| n.as_str().parse::<u64>().ok()), caps.get(4)) {
            marker = format!("{}{}", number + 1, close.as_str());
        }
        let task = if caps.get(6).is_some() { "[ ] " } else { "" };
        Some(Return::Insert(format!("\n{indent}{marker}{space}{task}")))
    }

    pub fn indent(line: &str) -> String {
        format!("{}{line}", Self::INDENT_UNIT)
    }

    pub fn outdent(line: &str) -> String {
        if let Some(rest) = line.strip_prefix(Self::INDENT_UNIT) {
            return rest.into();
        }
        if let Some(rest) = line.strip_prefix('\t') {
            return rest.into();
        }
        line.trim_start_matches(' ').into()
    }

    pub fn is_list_item(line: &str) -> bool {
        ITEM.is_match(line)
    }

    /// The link a dropped file gets: `[[Note]]`, or the path when another
    /// note has the same name; pictures, sound, video and PDFs embedded.
    pub fn link(path: &str, files: &BTreeSet<String>) -> String {
        let name = path.rsplit('/').next().unwrap_or(path);
        let note = Vault::is_note(path);
        let short = if note { &name[..name.len() - 3] } else { name };
        let same = files.iter().filter(|f| f.rsplit('/').next().unwrap_or(f).to_lowercase() == name.to_lowercase()).count();
        let target = if same > 1 { if note { &path[..path.len() - 3] } else { path } } else { short };
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let bang = if MEDIA.contains(&extension.as_str()) { "!" } else { "" };
        format!("{bang}[[{target}]]")
    }

    /// One web address and nothing else, as a paste over a selection takes it.
    pub fn is_web_address(text: &str) -> bool {
        if text.is_empty() || text.chars().any(char::is_whitespace) {
            return false;
        }
        let Some(caps) = SCHEME.captures(text) else { return false };
        match caps[1].to_lowercase().as_str() {
            "mailto" => true,
            "http" | "https" => caps[2].strip_prefix("//").is_some_and(|rest| !rest.split(['/', '?', '#']).next().unwrap_or("").is_empty()),
            _ => false,
        }
    }

    /// Ctrl+Return: a task is ticked or opened again, as a click on its box
    /// does; a list item becomes a task; any other line becomes a task item.
    pub fn toggle_task(line: &str, today: Day) -> Vec<String> {
        if let Some(toggled) = TaskLine::toggled(line, &today.to_string()) {
            return toggled;
        }
        if let Some(caps) = ITEM.captures(line) {
            return vec![format!("{}{}{}[ ] {}", &caps[1], &caps[2], &caps[5], &caps[7])];
        }
        let text = line.trim_start_matches([' ', '\t']);
        let indent = &line[..line.len() - text.len()];
        vec![format!("{indent}- [ ] {text}")]
    }

    /// The name Obsidian gives a pasted image, in the attachment folder.
    pub fn pasted_image_name(at: NaiveDateTime, extension: &str) -> String {
        format!("Pasted image {}.{extension}", at.format("%Y%m%d%H%M%S"))
    }

    /// The text after an unclosed `[[` that ends at character [location],
    /// which `[[` completion replaces, or `None` when the cursor is not in one.
    pub fn link_query(text: &str, location: usize) -> Option<Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        let location = location.min(chars.len());
        let start = line_start(&chars, location);
        let before = &chars[start..location];
        let open = (0..before.len().saturating_sub(1)).rev().find(|&i| before[i] == '[' && before[i + 1] == '[')?;
        let typed: String = before[open + 2..].iter().collect();
        if typed.contains("]]") || typed.contains('|') || typed.contains('#') {
            return None;
        }
        Some(start + open + 2..location)
    }

    /// Alt+Up and Alt+Down: the lines that hold [selection] change places
    /// with the line above or below. `None` at the top or the bottom.
    pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<TextEdit> {
        let chars: Vec<char> = text.chars().collect();
        let lines = line_range(&chars, selection.clone());
        let body = |range: &Range<usize>| -> String {
            let line: String = chars[range.clone()].iter().collect();
            line.strip_suffix('\n').map(String::from).unwrap_or(line)
        };
        let (whole, moved, shift): (Range<usize>, String, isize) = if up {
            if lines.start == 0 {
                return None;
            }
            let above = line_range(&chars, lines.start - 1..lines.start - 1);
            let whole = above.start..lines.end;
            let ending = if chars[whole.end - 1] == '\n' { "\n" } else { "" };
            (whole, format!("{}\n{}{ending}", body(&lines), body(&above)), -(above.len() as isize))
        } else {
            if lines.end >= chars.len() {
                return None;
            }
            let below = line_range(&chars, lines.end..lines.end);
            let whole = lines.start..below.end;
            let ending = if chars[whole.end - 1] == '\n' { "\n" } else { "" };
            let below_body = body(&below);
            let shift = below_body.chars().count() as isize + 1;
            (whole, format!("{below_body}\n{}{ending}", body(&lines)), shift)
        };
        let start = (selection.start as isize + shift) as usize;
        Some(TextEdit { range: whole, text: moved, selection: start..start + selection.len() })
    }

    /// A name for a note made from [text]: its first line, without the
    /// heading marks, bullets and characters a file name cannot have.
    pub fn extract_name(text: &str) -> String {
        let first = text.split(['\n', '\r']).find(|l| !l.is_empty()).unwrap_or("");
        let name = EXTRACT_LEAD.replace(first, "");
        let name = EXTRACT_LINK
            .replace_all(&name, |caps: &regex::Captures| caps.get(2).unwrap_or_else(|| caps.get(1).unwrap()).as_str().to_string());
        let name: String = name.chars().map(|c| if "/\\:#^[]|*\"<>?".contains(c) { ' ' } else { c }).collect();
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        // A leading dot would hide the file.
        let mut name = name.trim_start_matches(['.', ' ']).to_string();
        if name.chars().count() > 60 {
            name = name.chars().take(60).collect::<String>().trim().to_string();
        }
        if name.is_empty() { "Untitled".into() } else { name }
    }

    /// The text of a note made from a selection: a heading on its first
    /// line is moved up to the note's title, so it is not said twice.
    pub fn extract_body(text: &str) -> String {
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.first().is_some_and(|first| HEADING_LINE.is_match(first)) {
            lines.remove(0);
        }
        while lines.first().is_some_and(|l| l.trim().is_empty()) {
            lines.remove(0);
        }
        lines.join("\n")
    }

    pub fn slash_commands(today: Day, time: &str) -> Vec<Snippet> {
        let today = today.to_string();
        let s = Snippet::new;
        vec![
            s("h1", "Heading 1", "Text", "textformat.size.larger", "orange", "# ", None, true),
            s("h2", "Heading 2", "Text", "textformat.size", "vividYellow", "## ", None, true),
            s("h3", "Heading 3", "Text", "textformat.size.smaller", "blue", "### ", None, true),
            s("task", "Task", "Lists", "checkmark.square", "orange", "- [ ] ", None, true),
            s(
                "task-today",
                "Task due today",
                "Lists",
                "calendar.badge.exclamationmark",
                "red",
                &format!("- [ ]  📅 {today}"),
                Some(6),
                true,
            ),
            s("bullet", "Bulleted list", "Lists", "list.bullet", "aqua", "- ", None, true),
            s("number", "Numbered list", "Lists", "list.number", "aqua", "1. ", None, true),
            s("quote", "Quote", "Blocks", "text.quote", "warmGrey", "> ", None, true),
            s("note", "Note callout", "Blocks", "pencil", "blue", "> [!note] \n> ", None, true),
            s("tip", "Tip callout", "Blocks", "flame.fill", "chartreuse", "> [!tip] \n> ", None, true),
            s("warning", "Warning callout", "Blocks", "exclamationmark.triangle.fill", "orange", "> [!warning] \n> ", None, true),
            s("danger", "Danger callout", "Blocks", "bolt.fill", "red", "> [!danger] \n> ", None, true),
            s("code", "Code block", "Blocks", "chevron.left.forwardslash.chevron.right", "pink", "```\n\n```", Some(4), true),
            s(
                "mermaid",
                "Mermaid diagram",
                "Blocks",
                "flowchart.fill",
                "purple",
                "```mermaid\nflowchart LR\n    A --> B\n```",
                Some(35),
                true,
            ),
            s("math", "Math block", "Blocks", "function", "purple", "$$\n\n$$", Some(3), true),
            s(
                "table",
                "Table",
                "Blocks",
                "tablecells",
                "green",
                "| Column | Column |\n| ------ | ------ |\n|        |        |",
                Some(42),
                true,
            ),
            s("tasks", "Tasks query", "Blocks", "checklist", "orange", "```tasks\nnot done\ndue before tomorrow\n```", None, true),
            s("divider", "Divider", "Blocks", "minus", "grey", "---\n", None, true),
            s("link", "Link to a note", "Links", "link", "aqua", "[[]]", Some(2), false),
            s("embed", "Embed a note or a file", "Links", "photo.on.rectangle", "pink", "![[]]", Some(3), false),
            s("date", "Today's date", "Insert", "calendar", "orange", &today, None, false),
            s("time", "The time now", "Insert", "clock", "blue", time, None, false),
            s("red", "Red highlight", "Highlight", "highlighter", "red", "==🔴 ==", Some(5), false),
            s("orange", "Orange highlight", "Highlight", "highlighter", "orange", "==🟠 ==", Some(5), false),
            s("yellow", "Yellow highlight", "Highlight", "highlighter", "vividYellow", "==🟡 ==", Some(5), false),
            s("green", "Green highlight", "Highlight", "highlighter", "green", "==🟢 ==", Some(5), false),
            s("blue", "Blue highlight", "Highlight", "highlighter", "blue", "==🔵 ==", Some(5), false),
            s("purple", "Purple highlight", "Highlight", "highlighter", "purple", "==🟣 ==", Some(5), false),
        ]
    }

    /// Whether the `/` just before character [location] opens the menu: at
    /// the start of a line, or after a space, so a path or a URL never does.
    pub fn opens_slash_menu(text: &str, location: usize) -> bool {
        let chars: Vec<char> = text.chars().collect();
        if location < 1 || location > chars.len() || chars[location - 1] != '/' {
            return false;
        }
        location == 1 || matches!(chars[location - 2], '\n' | ' ' | '\t')
    }

    /// The edit that puts [snippet] where the `/` at character [slash] is.
    /// The selection is the cursor, empty.
    pub fn apply(snippet: &Snippet, slash: usize, text: &str) -> TextEdit {
        let chars: Vec<char> = text.chars().collect();
        let start = line_start(&chars, slash);
        let before: String = chars[start..slash].iter().collect();
        // A block after other text on the line starts a line of its own.
        let lead = if snippet.block && !before.trim().is_empty() { "\n" } else { "" };
        let cursor = slash + lead.len() + snippet.cursor;
        TextEdit { range: slash..slash + 1, text: format!("{lead}{}", snippet.text), selection: cursor..cursor }
    }
}

/// What a slash command inserts: `/` at the start of a line or after a
/// space opens the menu, and the chosen snippet takes the slash's place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub group: String,
    /// An SF Symbol on the Mac, and the palette colour it is drawn in.
    pub symbol: String,
    pub color: String,
    pub text: String,
    /// Where the cursor goes, in characters into [text].
    pub cursor: usize,
    /// A block (a heading, a fence, a table) starts a line of its own.
    pub block: bool,
}

impl Snippet {
    /// [cursor] is given in UTF-16 units, as the Mac writes it, and kept in characters.
    #[allow(clippy::too_many_arguments)]
    fn new(id: &str, title: &str, group: &str, symbol: &str, color: &str, text: &str, cursor: Option<usize>, block: bool) -> Snippet {
        let cursor = match cursor {
            None => text.chars().count(),
            Some(units) => {
                let mut seen = 0;
                text.chars()
                    .take_while(|c| {
                        seen += c.len_utf16();
                        seen <= units
                    })
                    .count()
            }
        };
        Snippet {
            id: id.into(),
            title: title.into(),
            group: group.into(),
            symbol: symbol.into(),
            color: color.into(),
            text: text.into(),
            cursor,
            block,
        }
    }
}

/// The character offset where the line holding [location] starts.
pub(crate) fn line_start(chars: &[char], location: usize) -> usize {
    chars[..location.min(chars.len())].iter().rposition(|&c| c == '\n').map_or(0, |at| at + 1)
}

/// The lines that hold [range], each with its newline, as NSString's
/// `lineRange(for:)`: an empty range takes its line; a range that ends
/// just after a newline does not take the next line.
pub(crate) fn line_range(chars: &[char], range: Range<usize>) -> Range<usize> {
    let start = line_start(chars, range.start);
    let last = if range.end > range.start { range.end - 1 } else { range.start };
    let end = chars.iter().skip(last).position(|&c| c == '\n').map_or(chars.len(), |at| last + at + 1);
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn substring(text: &str, range: Range<usize>) -> String {
        text.chars().skip(range.start).take(range.len()).collect()
    }

    fn replaced(text: &str, edit: &TextEdit) -> String {
        let chars: Vec<char> = text.chars().collect();
        format!(
            "{}{}{}",
            chars[..edit.range.start].iter().collect::<String>(),
            edit.text,
            chars[edit.range.end..].iter().collect::<String>()
        )
    }

    // EditorCommandTests

    #[test]
    fn return_continues_lists() {
        let insert = |s: &str| Some(Return::Insert(s.into()));
        assert_eq!(EditorCommands::on_return("- item"), insert("\n- "));
        assert_eq!(EditorCommands::on_return("    - [x] done"), insert("\n    - [ ] "));
        assert_eq!(EditorCommands::on_return("9. ninth"), insert("\n10. "));
        assert_eq!(EditorCommands::on_return("> - quoted"), insert("\n> - "));
        assert_eq!(EditorCommands::on_return("- "), Some(Return::EndList));
        assert_eq!(EditorCommands::on_return("- [ ] "), Some(Return::EndList));
        assert_eq!(EditorCommands::on_return("plain"), None);
    }

    #[test]
    fn toggle_makes_tasks() {
        let day = Day::parse("2026-10-05").unwrap();
        assert_eq!(EditorCommands::toggle_task("    - item", day), ["    - [ ] item"]);
        assert_eq!(EditorCommands::toggle_task("  text", day), ["  - [ ] text"]);
        assert_eq!(EditorCommands::toggle_task("- [ ] a", day), ["- [x] a ✅ 2026-10-05"]);
    }

    #[test]
    fn indent_by_four() {
        assert_eq!(EditorCommands::indent("- a"), "    - a");
        assert_eq!(EditorCommands::outdent("        - a"), "    - a");
        assert_eq!(EditorCommands::outdent("  - a"), "- a");
    }

    #[test]
    fn link_query() {
        let text = "See [[Kube and [[done]] here";
        assert_eq!(EditorCommands::link_query(text, 10).map(|r| substring(text, r)).as_deref(), Some("Kube"));
        assert_eq!(EditorCommands::link_query(text, text.chars().count()), None);
    }

    #[test]
    fn pasted_name() {
        let at = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap().and_hms_opt(9, 5, 7).unwrap();
        assert_eq!(EditorCommands::pasted_image_name(at, "png"), "Pasted image 20261025090507.png");
    }

    // SlashTests

    fn commands() -> Vec<Snippet> {
        EditorCommands::slash_commands(Day::new(2026, 10, 25).unwrap(), "15:30")
    }

    fn snippet(id: &str) -> Snippet {
        commands().into_iter().find(|s| s.id == id).unwrap()
    }

    #[test]
    fn opens_only_at_a_word_start() {
        assert!(EditorCommands::opens_slash_menu("/", 1));
        assert!(EditorCommands::opens_slash_menu("Text /", 6));
        assert!(EditorCommands::opens_slash_menu("Line\n/", 6));
        assert!(!EditorCommands::opens_slash_menu("a/b", 2));
        assert!(!EditorCommands::opens_slash_menu("https://", 7));
        assert!(!EditorCommands::opens_slash_menu("text", 4));
    }

    #[test]
    fn an_inline_snippet_takes_the_slashs_place() {
        let text = "See /";
        let edit = EditorCommands::apply(&snippet("link"), 4, text);
        assert_eq!(edit.range, 4..5);
        assert_eq!(edit.text, "[[]]");
        assert_eq!(edit.selection.start, 6);
        assert_eq!(EditorCommands::apply(&snippet("date"), 4, text).text, "2026-10-25");
        // After `==🔴 `: five UTF-16 units, four characters.
        assert_eq!(EditorCommands::apply(&snippet("red"), 4, text).selection.start, 4 + 4);
    }

    #[test]
    fn a_block_starts_its_own_line() {
        let start = EditorCommands::apply(&snippet("code"), 0, "/");
        assert_eq!(start.text, "```\n\n```");
        assert_eq!(start.selection.start, 4);
        assert_eq!(EditorCommands::apply(&snippet("math"), 0, "/").selection.start, 3);
        let after = EditorCommands::apply(&snippet("h2"), 5, "Some /");
        assert_eq!(after.text, "\n## ");
        assert_eq!(after.selection.start, 5 + 4);
    }

    #[test]
    fn every_cursor_is_inside_its_snippet() {
        for snippet in commands() {
            assert!(snippet.cursor <= snippet.text.chars().count(), "{}", snippet.id);
        }
        // The table's cursor is in its first empty cell; the task's before its date.
        let table = snippet("table");
        assert_eq!(substring(&table.text, table.cursor - 2..table.cursor), "| ");
        let mermaid = snippet("mermaid");
        assert!(substring(&mermaid.text, 0..mermaid.cursor).ends_with("A --> B"));
        let task = snippet("task-today");
        assert_eq!(substring(&task.text, task.cursor..task.text.chars().count()), " 📅 2026-10-25");
    }

    // ExtractTests

    #[test]
    fn names_from_the_first_line() {
        assert_eq!(EditorCommands::extract_name("## Kafka: partitions & [[Consumer|consumers]]\nbody"), "Kafka partitions & consumers");
        assert_eq!(EditorCommands::extract_name("- [ ] Call the bank"), "Call the bank");
        assert_eq!(EditorCommands::extract_name("   \n"), "Untitled");
        assert_eq!(EditorCommands::extract_name("...hidden thought"), "hidden thought");
        assert_eq!(EditorCommands::extract_name("# .env notes"), "env notes");
        assert_eq!(EditorCommands::extract_name(". . ."), "Untitled");
        assert!(EditorCommands::extract_name(&"word ".repeat(30)).chars().count() <= 60);
    }

    #[test]
    fn a_heading_moves_to_the_title() {
        assert_eq!(EditorCommands::extract_body("## Partitions\n\nThey order messages."), "They order messages.");
        assert_eq!(EditorCommands::extract_body("Plain text\nmore"), "Plain text\nmore");
    }

    // MoveLinesTests

    #[test]
    fn a_line_goes_up_and_down() {
        let text = "one\ntwo\nthree\n";
        let up = EditorCommands::move_lines(text, 5..5, true).unwrap();
        assert_eq!(replaced(text, &up), "two\none\nthree\n");
        assert_eq!(up.selection.start, 1);
        let down = EditorCommands::move_lines(text, 5..5, false).unwrap();
        assert_eq!(replaced(text, &down), "one\nthree\ntwo\n");
        assert_eq!(down.selection.start, 11);
        assert_eq!(EditorCommands::move_lines(text, 0..0, true), None);
    }

    #[test]
    fn the_last_line_with_no_newline_and_a_selection_of_lines() {
        let text = "a\nb\nc";
        let down = EditorCommands::move_lines(text, 2..2, false).unwrap();
        assert_eq!(replaced(text, &down), "a\nc\nb");
        let up = EditorCommands::move_lines(text, 4..4, true).unwrap();
        assert_eq!(replaced(text, &up), "a\nc\nb");
        // Two lines, selected to the start of the next: those two move.
        let two = EditorCommands::move_lines(text, 0..4, false).unwrap();
        assert_eq!(replaced(text, &two), "c\na\nb");
    }

    // DroppedLinkTests

    #[test]
    fn a_dropped_file_gets_the_shortest_link() {
        let files: BTreeSet<String> =
            ["Notes/Plan.md", "Old/Plan.md", "Notes/Kit.md", "uploads/photo.png", "docs/spec.pdf", "data.csv"].map(String::from).into();
        assert_eq!(EditorCommands::link("Notes/Kit.md", &files), "[[Kit]]");
        assert_eq!(EditorCommands::link("Notes/Plan.md", &files), "[[Notes/Plan]]");
        assert_eq!(EditorCommands::link("uploads/photo.png", &files), "![[photo.png]]");
        assert_eq!(EditorCommands::link("docs/spec.pdf", &files), "![[spec.pdf]]");
        assert_eq!(EditorCommands::link("data.csv", &files), "[[data.csv]]");
        assert!(EditorCommands::is_web_address("https://example.com/a?b=1"));
        assert!(!EditorCommands::is_web_address("two words") && !EditorCommands::is_web_address("example.com"));
    }
}
