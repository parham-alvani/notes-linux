//! A file that git left with conflict marks, in parts: plain text, and the
//! conflicts between `<<<<<<<` and `>>>>>>>`, with diff3's `|||||||` base
//! when it is there. Daftar only rebases, and in a rebase the side above
//! `=======` is the remote's ("ours", the upstream the commits are replayed
//! on) and the side below is the person's own commit ("theirs"); the names
//! here say that, so no view has to. The port of `ConflictText.swift`.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The upstream's lines, above `=======`, each with its line end.
    pub remote: String,
    /// The person's lines, below `=======`.
    pub mine: String,
    /// diff3's common ancestor, between `|||||||` and `=======`.
    pub base: Option<String>,
    /// The mark lines as they were, so an open conflict is written back unchanged.
    pub open: String,
    pub base_mark: Option<String>,
    pub separator: String,
    pub close: String,
}

impl Conflict {
    /// What follows `>>>>>>>`: in a rebase, the commit being replayed.
    pub fn mine_label(&self) -> String {
        label(&self.close)
    }

    /// What follows `<<<<<<<`: in a rebase, usually the upstream's commit.
    pub fn remote_label(&self) -> String {
        label(&self.open)
    }

    /// The conflict as git wrote it, marks and all.
    pub fn marked(&self) -> String {
        let base = self.base_mark.as_ref().map(|mark| format!("{mark}{}", self.base.as_deref().unwrap_or(""))).unwrap_or_default();
        format!("{}{}{base}{}{}{}", self.open, self.remote, self.separator, self.mine, self.close)
    }
}

fn label(mark: &str) -> String {
    mark.chars().skip(7).collect::<String>().trim().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Plain(String),
    Conflict(Conflict),
}

/// Which side of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Choice {
    Remote,
    Mine,
    /// The remote's lines, then the person's.
    Both,
}

impl Choice {
    pub const ALL: [Choice; 3] = [Choice::Remote, Choice::Mine, Choice::Both];
}

pub struct ConflictText;

impl ConflictText {
    /// [text] in parts. Text with no complete conflict is one plain part, and
    /// a mark that starts a conflict that never ends is plain text too.
    pub fn parse(text: &str) -> Vec<Part> {
        let mut parts = Vec::new();
        let mut plain = String::new();
        let lines = Self::lines(text);
        let mut index = 0;
        while index < lines.len() {
            match is_mark(lines[index], '<').then(|| conflict(&lines, index)).flatten() {
                Some((found, next)) => {
                    if !plain.is_empty() {
                        parts.push(Part::Plain(std::mem::take(&mut plain)));
                    }
                    parts.push(Part::Conflict(found));
                    index = next;
                }
                None => {
                    plain.push_str(lines[index]);
                    index += 1;
                }
            }
        }
        if !plain.is_empty() {
            parts.push(Part::Plain(plain));
        }
        if parts.is_empty() { vec![Part::Plain(text.to_string())] } else { parts }
    }

    /// The conflicts in [text], in order.
    pub fn conflicts(text: &str) -> Vec<Conflict> {
        Self::parse(text)
            .into_iter()
            .filter_map(|part| match part {
                Part::Conflict(conflict) => Some(conflict),
                Part::Plain(_) => None,
            })
            .collect()
    }

    /// How many conflicts [parts] has.
    pub fn count(parts: &[Part]) -> usize {
        parts.iter().filter(|part| matches!(part, Part::Conflict(_))).count()
    }

    /// The text with the [choices] made, the n-th for the n-th conflict. A
    /// conflict with no choice keeps its marks.
    pub fn resolve(parts: &[Part], choices: &HashMap<usize, Choice>) -> String {
        let mut out = String::new();
        let mut number = 0;
        for part in parts {
            match part {
                Part::Plain(text) => out.push_str(text),
                Part::Conflict(conflict) => {
                    match choices.get(&number) {
                        Some(Choice::Remote) => out.push_str(&conflict.remote),
                        Some(Choice::Mine) => out.push_str(&conflict.mine),
                        Some(Choice::Both) => {
                            out.push_str(&conflict.remote);
                            out.push_str(&conflict.mine);
                        }
                        None => out.push_str(&conflict.marked()),
                    }
                    number += 1;
                }
            }
        }
        out
    }

    /// The text with one [choice] for every conflict.
    pub fn resolve_all(parts: &[Part], choice: Choice) -> String {
        Self::resolve(parts, &(0..Self::count(parts)).map(|n| (n, choice)).collect())
    }

    /// The lines of [text], each with its line end; the last may have none.
    pub fn lines(text: &str) -> Vec<&str> {
        text.split_inclusive('\n').collect()
    }
}

/// The conflict that starts at [start], and the line after its end.
fn conflict(lines: &[&str], start: usize) -> Option<(Conflict, usize)> {
    let (mut remote, mut mine) = (String::new(), String::new());
    let (mut base, mut base_mark, mut separator): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if separator.is_none() {
            if is_mark(line, '=') {
                separator = Some(line.to_string());
            } else if is_mark(line, '|') && base_mark.is_none() {
                base_mark = Some(line.to_string());
                base = Some(String::new());
            } else if let Some(base) = base.as_mut() {
                base.push_str(line);
            } else if is_mark(line, '<') {
                // Another conflict starts before this one has a middle: this one is broken.
                return None;
            } else {
                remote.push_str(line);
            }
        } else if is_mark(line, '>') {
            let found =
                Conflict { remote, mine, base, open: lines[start].to_string(), base_mark, separator: separator?, close: line.to_string() };
            return Some((found, index + 1));
        } else if is_mark(line, '<') || is_mark(line, '=') {
            return None;
        } else {
            mine.push_str(line);
        }
    }
    None
}

/// Seven of [mark] alone, or followed by a space and a label. `=======`
/// takes no label.
fn is_mark(line: &str, mark: char) -> bool {
    let body = line.trim_end_matches(['\n', '\r']);
    let mut chars = body.chars();
    if !(0..7).all(|_| chars.next() == Some(mark)) {
        return false;
    }
    let rest = chars.as_str();
    if mark == '=' {
        return rest.is_empty();
    }
    rest.is_empty() || rest.starts_with(' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# Note\n<<<<<<< HEAD\nremote one\nremote two\n=======\nmine one\n>>>>>>> 6524752 (my change)\nmiddle\n<<<<<<< upstream\nr\n||||||| base\nb\n=======\nm\n>>>>>>> abc\n";

    fn choices(pairs: &[(usize, Choice)]) -> HashMap<usize, Choice> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn conflicts_are_read_with_their_base() {
        let parts = ConflictText::parse(TEXT);
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], Part::Plain("# Note\n".into()));
        assert_eq!(parts[2], Part::Plain("middle\n".into()));
        let conflicts = ConflictText::conflicts(TEXT);
        assert_eq!(conflicts.iter().map(|c| c.remote.as_str()).collect::<Vec<_>>(), ["remote one\nremote two\n", "r\n"]);
        assert_eq!(conflicts.iter().map(|c| c.mine.as_str()).collect::<Vec<_>>(), ["mine one\n", "m\n"]);
        assert_eq!(conflicts.iter().map(|c| c.base.clone()).collect::<Vec<_>>(), [None, Some("b\n".into())]);
        assert_eq!(conflicts.iter().map(Conflict::mine_label).collect::<Vec<_>>(), ["6524752 (my change)", "abc"]);
        assert_eq!(conflicts[0].remote_label(), "HEAD");
    }

    #[test]
    fn choices_make_the_text() {
        let parts = ConflictText::parse(TEXT);
        assert_eq!(ConflictText::resolve_all(&parts, Choice::Remote), "# Note\nremote one\nremote two\nmiddle\nr\n");
        assert_eq!(ConflictText::resolve_all(&parts, Choice::Mine), "# Note\nmine one\nmiddle\nm\n");
        assert_eq!(ConflictText::resolve_all(&parts, Choice::Both), "# Note\nremote one\nremote two\nmine one\nmiddle\nr\nm\n");
        // A conflict with no choice keeps its marks, to the byte.
        assert_eq!(ConflictText::resolve(&parts, &HashMap::new()), TEXT);
        let half = ConflictText::resolve(&parts, &choices(&[(1, Choice::Mine)]));
        assert!(half.starts_with("# Note\n<<<<<<< HEAD\n") && half.ends_with("middle\nm\n"));
        assert_eq!(ConflictText::conflicts(&half).len(), 1);
    }

    #[test]
    fn broken_marks_are_plain_text() {
        for broken in [
            "<<<<<<< HEAD\na\n=======\nb\n",
            "<<<<<<< HEAD\na\n>>>>>>> x\n",
            "<<<<<<<< not a mark\n=======\n>>>>>>> x\n",
            "=======\n>>>>>>> x\n",
            "<<<<<<< a\n<<<<<<< b\n",
        ] {
            assert_eq!(ConflictText::parse(broken), vec![Part::Plain(broken.into())], "{broken}");
        }
        assert_eq!(ConflictText::parse(""), vec![Part::Plain(String::new())]);
        // An inner start that never had a middle leaves the outer one broken and the inner one whole.
        let nested = "<<<<<<< a\n<<<<<<< b\nx\n=======\ny\n>>>>>>> c\n";
        assert_eq!(ConflictText::parse(nested).len(), 2);
        assert_eq!(ConflictText::conflicts(nested)[0].remote, "x\n");
    }

    #[test]
    fn windows_line_ends_and_a_last_line_without_end() {
        let crlf = "a\r\n<<<<<<< HEAD\r\nr\r\n=======\r\nm\r\n>>>>>>> x";
        let parts = ConflictText::parse(crlf);
        assert_eq!(ConflictText::conflicts(crlf)[0].mine, "m\r\n");
        assert_eq!(ConflictText::resolve_all(&parts, Choice::Remote), "a\r\nr\r\n");
        assert_eq!(ConflictText::resolve(&parts, &HashMap::new()), crlf);
    }

    /// The robustness line: made-up text with marks put in at random.
    #[test]
    fn conflict_marks_from_any_text() {
        let marks = ["<<<<<<< HEAD\n", "||||||| base\n", "=======\n", ">>>>>>> abc (mine)\n", "<<<<<<<\r\n", "=======\r\n", ">>>>>>>"];
        let words = ["a", "b c", "", "# h", "- [ ] t", "ü", "=", "<<", "\r"];
        let mut state: u64 = 14;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..1000 {
            let text: Vec<String> = (0..next(12)).map(|_| words[next(words.len())].to_string()).collect();
            let text = text.join("\n");
            assert_eq!(ConflictText::resolve(&ConflictText::parse(&text), &HashMap::new()), text);
            let mut marked = String::new();
            for line in text.split('\n') {
                marked.push_str(line);
                marked.push('\n');
                if next(3) == 0 {
                    marked.push_str(marks[next(marks.len())]);
                }
            }
            let parts = ConflictText::parse(&marked);
            assert_eq!(ConflictText::resolve(&parts, &HashMap::new()), marked);
            // Keeping both keeps every line of both sides.
            let both = ConflictText::resolve_all(&parts, Choice::Both);
            for conflict in ConflictText::conflicts(&marked) {
                for line in ConflictText::lines(&conflict.remote).into_iter().chain(ConflictText::lines(&conflict.mine)) {
                    assert!(both.contains(line));
                }
            }
            for choice in Choice::ALL {
                ConflictText::resolve_all(&parts, choice);
            }
        }
    }
}
