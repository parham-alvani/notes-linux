//! Format on save (`docs/specs/problems-and-format.md`): when a note the
//! editor held is formatted with `daftar-cli fmt --stdin`, and what of it
//! changes. The port of `NoteFormat.swift`.

use std::ops::Range;

use crate::vault::Vault;

/// When the editor might format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moment {
    /// The autosave a second after the typing stops.
    Autosave,
    /// The editor lost the keys (a click in the sidebar), and still shows the note.
    FocusLost,
    /// Another note in the same editor.
    OtherNote,
    /// The editor went away: Ctrl+E back to the reader, or the pane, the tab
    /// or the window closed.
    Closed,
    /// The app quits.
    Quit,
    /// Edit › Format Note, asked for.
    Command,
}

impl Moment {
    pub const ALL: [Moment; 6] = [Moment::Autosave, Moment::FocusLost, Moment::OtherNote, Moment::Closed, Moment::Quit, Moment::Command];
}

/// The run of whole lines that differs: lines `lines` of the old text (from
/// 0) become `new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedLines {
    pub lines: Range<usize>,
    pub new: Vec<String>,
}

pub struct NoteFormat;

impl NoteFormat {
    /// Whether the note at [path] is formatted at [moment]. Never while the
    /// note is still being typed in (the autosave, a lost focus): text must
    /// not move under the cursor. On leaving, only with
    /// `editor.format_on_save` and only a note that was [edited] there:
    /// opening an unformatted note to read it in the editor does not change
    /// it. The command formats any note. Only Markdown notes are formatted,
    /// never a base or a canvas.
    pub fn formats(moment: Moment, setting: bool, path: &str, edited: bool) -> bool {
        if !Vault::is_note(path) {
            return false;
        }
        match moment {
            Moment::Autosave | Moment::FocusLost => false,
            Moment::Command => true,
            Moment::OtherNote | Moment::Closed | Moment::Quit => setting && edited,
        }
    }

    /// The smallest run of whole lines that differs between [old] and
    /// [new], or `None` when the two are the same. For Neovim, whose buffer
    /// takes lines: replacing only these keeps its cursor, marks and folds
    /// elsewhere, and is one undo step.
    pub fn changed_lines(old: &str, new: &str) -> Option<ChangedLines> {
        if old == new {
            return None;
        }
        let (a, b) = (Self::lines(old), Self::lines(new));
        let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
        Some(ChangedLines { lines: prefix..a.len() - suffix, new: b[prefix..b.len() - suffix].to_vec() })
    }

    /// The lines of [text] as Neovim's buffer holds them: split at `\n`,
    /// without the newline that ends the last one.
    pub fn lines(text: &str) -> Vec<String> {
        let mut parts: Vec<String> = text.split('\n').map(String::from).collect();
        if text.ends_with('\n') {
            parts.pop();
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Format on save: never while typing; on leaving, with the setting, a
    /// note typed in; the command, any note; never a base or a canvas.
    #[test]
    fn format_on_leave_is_decided_by_the_moment() {
        for moment in Moment::ALL {
            let leaving = matches!(moment, Moment::OtherNote | Moment::Closed | Moment::Quit);
            assert_eq!(NoteFormat::formats(moment, true, "Note.md", true), leaving || moment == Moment::Command);
            assert_eq!(NoteFormat::formats(moment, false, "Note.md", true), moment == Moment::Command);
            assert_eq!(NoteFormat::formats(moment, true, "Note.md", false), moment == Moment::Command);
            assert!(!NoteFormat::formats(moment, true, "Board.canvas", true));
            assert!(!NoteFormat::formats(moment, true, "Tasks.base", true));
        }
        assert!(!NoteFormat::formats(Moment::Autosave, true, "Note.md", true));
        assert!(!NoteFormat::formats(Moment::FocusLost, true, "Note.md", true));
        assert!(NoteFormat::formats(Moment::Closed, true, "Folder/NOTE.MD", true));
    }

    fn applied(old: &str, new: &str) -> Vec<String> {
        let mut lines = NoteFormat::lines(old);
        if let Some(change) = NoteFormat::changed_lines(old, new) {
            lines.splice(change.lines, change.new);
        }
        lines
    }

    /// Neovim takes the formatted note as the lines that changed.
    #[test]
    fn only_the_lines_that_changed_are_replaced() {
        assert_eq!(NoteFormat::changed_lines("a\nb\n", "a\nb\n"), None);
        let one = NoteFormat::changed_lines("# T\n*  item\ntext\n", "# T\n- item\ntext\n").unwrap();
        assert_eq!(one, ChangedLines { lines: 1..2, new: vec!["- item".into()] });
        // A line taken out, and one put in.
        let out = NoteFormat::changed_lines("a\n\n\nb\n", "a\n\nb\n").unwrap();
        assert!(out.lines.len() == 1 && out.new.is_empty());
        let added = NoteFormat::changed_lines("# T\ntext\n", "# T\n\ntext\n").unwrap();
        assert!(added.lines.is_empty() && added.new == [""]);
        assert_eq!(NoteFormat::lines("a\nb\n"), ["a", "b"]);
        assert_eq!(NoteFormat::lines("a\nb"), ["a", "b"]);
        // Applied to the old lines, the change gives the new ones.
        for (old, new) in [("x\ny\nz\n", "x\nY\nz\n"), ("", "a\n"), ("a\n", ""), ("1\n2\n3\n4\n", "1\n4\n")] {
            assert_eq!(applied(old, new), NoteFormat::lines(new));
        }
    }

    /// The robustness line: for made-up notes, the change applied to the old
    /// lines gives the new.
    #[test]
    fn format_changes_from_any_text() {
        let words = ["# T", "- item", "*  item", "", "text", "> quote", "|a|b|"];
        let mut state: u64 = 18;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        let mut note = || {
            let mut text = (0..next(8)).map(|_| words[next(words.len())]).collect::<Vec<_>>().join("\n");
            if next(2) == 0 {
                text.push('\n');
            }
            text
        };
        for _ in 0..500 {
            let (old, new) = (note(), note());
            assert_eq!(applied(&old, &new), NoteFormat::lines(&new));
        }
    }
}
