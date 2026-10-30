//! A note's sections: a heading, its body and the headings under it, to
//! the next heading of its level or above. The headings are the outline's,
//! found as the index finds them (`NoteParser`): `#` headings outside the
//! front matter, code fences and `%%` comments.
//!
//! The port of the outline's part of `Sections.swift`: closing a heading in
//! the outline, and sorting the headings under one.

use crate::note_parser::{Heading, NoteParser};
use crate::text::standard_compare;

pub struct Sections;

/// A section moved: the new text, and where the moved section's first line
/// was and is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub text: String,
    pub from: usize,
    pub to: usize,
}

impl Sections {
    /// The note's lines, split on `\n`, without the empty one after a last
    /// line break; and whether there was that line break.
    pub fn lines(text: &str) -> (Vec<&str>, bool) {
        let mut lines: Vec<&str> = text.split('\n').collect();
        let newline = lines.len() > 1 && lines.last() == Some(&"");
        if newline {
            lines.pop();
        }
        (lines, newline)
    }

    pub fn headings(text: &str) -> Vec<Heading> {
        NoteParser::parse(text).headings
    }

    /// The line after the last of heading [index]'s section: the next
    /// heading of its level or above, or [line_count].
    pub fn end(index: usize, headings: &[Heading], line_count: usize) -> usize {
        let level = headings[index].level;
        headings[index + 1..].iter().find(|h| h.level <= level).map_or(line_count, |h| h.line)
    }

    /// The heading whose own lines hold [line]: the last heading at or above
    /// it. `None` above the first heading.
    pub fn heading_at(line: usize, headings: &[Heading]) -> Option<usize> {
        headings.iter().rposition(|h| h.line <= line)
    }

    /// How the outline names [heading] to keep it closed: `level:text`, as
    /// the reader's folds, which an edit above it does not move.
    pub fn key(heading: &Heading) -> String {
        format!("{}:{}", heading.level, heading.text)
    }

    /// True when headings come under heading [index]: the next one is deeper.
    pub fn has_children(index: usize, headings: &[Heading]) -> bool {
        headings.get(index + 1).is_some_and(|next| next.level > headings[index].level)
    }

    /// The headings the outline shows, in order, when those in [closed] (by
    /// `key`) are closed: the headings under a closed one are left out.
    pub fn visible(headings: &[Heading], closed: &[String]) -> Vec<usize> {
        let mut shown = Vec::new();
        let mut under: Option<usize> = None;
        for (index, heading) in headings.iter().enumerate() {
            if let Some(level) = under {
                if heading.level > level {
                    continue;
                }
                under = None;
            }
            shown.push(index);
            if Self::has_children(index, headings) && closed.contains(&Self::key(heading)) {
                under = Some(heading.level);
            }
        }
        shown
    }

    /// The heading the outline marks for heading [index]: itself, or the
    /// closed heading it is under. [visible] is what `visible` gave.
    pub fn shown(index: usize, visible: &[usize]) -> Option<usize> {
        visible.iter().rev().find(|v| **v <= index).copied()
    }

    /// The headings right under heading [index]: those of the highest level
    /// in its section. Empty when it has none.
    pub fn children(index: usize, headings: &[Heading]) -> Vec<usize> {
        if !Self::has_children(index, headings) {
            return Vec::new();
        }
        let level = headings[index].level;
        let below = headings[index + 1..].iter().take_while(|h| h.level > level).count();
        let range = index + 1..index + 1 + below;
        let top = headings[range.clone()].iter().map(|h| h.level).min().unwrap_or(level);
        range.filter(|i| headings[*i].level == top).collect()
    }

    /// The headings of heading [index]'s level under the same parent, in
    /// order, [index] among them.
    pub fn siblings(index: usize, headings: &[Heading]) -> Vec<usize> {
        let level = headings[index].level;
        let mut first = index;
        while first > 0 && headings[first - 1].level >= level {
            first -= 1;
        }
        let mut last = index;
        while last + 1 < headings.len() && headings[last + 1].level >= level {
            last += 1;
        }
        (first..=last).filter(|i| headings[*i].level == level).collect()
    }

    /// The text with the headings right under heading [index] in the order
    /// of their names, as a file manager orders them, each with its section.
    /// The text before the first of them stays first. `None` when they are
    /// in order already or there are fewer than two.
    pub fn sort_children(text: &str, index: usize) -> Option<String> {
        let start = Self::headings(text);
        if index >= start.len() {
            return None;
        }
        let names: Vec<&str> = Self::children(index, &start).iter().map(|i| start[*i].text.as_str()).collect();
        let mut sorted: Vec<usize> = (0..names.len()).collect();
        // A stable sort keeps equal names in their order.
        sorted.sort_by(|a, b| standard_compare(names[*a], names[*b]));
        if names.len() < 2 || sorted.iter().enumerate().all(|(at, i)| at == *i) {
            return None;
        }
        let mut text = text.to_string();
        // Where each section is now, by its first place.
        let mut places: Vec<usize> = (0..names.len()).collect();
        for (position, wanted) in sorted.into_iter().enumerate() {
            let at = places.iter().position(|p| *p == wanted)?;
            if at == position {
                continue;
            }
            let now = Self::headings(&text);
            let child = Self::children(index, &now)[at];
            text = Self::move_section(&text, child, position)?.text;
            let place = places.remove(at);
            places.insert(position, place);
        }
        Some(text)
    }

    /// "Sort Headings Under This" on heading [index], or on the heading of
    /// the section [line] is in: the sorted text, or what to say instead.
    pub fn sort_headings(text: &str, index: Option<usize>, line: usize) -> Result<String, String> {
        let headings = Self::headings(text);
        let index = index.filter(|i| *i < headings.len()).or_else(|| Self::heading_at(line, &headings));
        let Some(index) = index else { return Err("Put the cursor in a section to sort the headings under it.".into()) };
        let name = &headings[index].text;
        if Self::children(index, &headings).len() < 2 {
            return Err(format!("“{name}” has fewer than two headings right under it."));
        }
        Self::sort_children(text, index).ok_or_else(|| format!("The headings under “{name}” are in order already."))
    }

    /// The section of heading [index] at place [position] among its
    /// siblings, as a list's move does: the siblings after it close up. The
    /// blank lines after each section stay where they are, so a section
    /// moved to the end does not take the blank line before the next heading
    /// with it. `None` when it is there already.
    pub fn move_section(text: &str, index: usize, position: usize) -> Option<Moved> {
        let (lines, newline) = Self::lines(text);
        let headings = Self::headings(text);
        if index >= headings.len() {
            return None;
        }
        let siblings = Self::siblings(index, &headings);
        let from = siblings.iter().position(|s| *s == index)?;
        if position >= siblings.len() || position == from {
            return None;
        }
        let starts: Vec<usize> = siblings.iter().map(|s| headings[*s].line).collect();
        let stop = Self::end(*siblings.last()?, &headings, lines.len());
        // Each section is its content and the blank lines after it.
        let (mut contents, mut gaps) = (Vec::new(), Vec::new());
        for (at, start) in starts.iter().enumerate() {
            let finish = starts.get(at + 1).copied().unwrap_or(stop);
            let mut content = finish;
            while content > start + 1 && lines[content - 1].trim().is_empty() {
                content -= 1;
            }
            contents.push(&lines[*start..content]);
            gaps.push(&lines[content..finish]);
        }
        let mut order: Vec<usize> = (0..contents.len()).collect();
        let moved = order.remove(from);
        order.insert(position, moved);
        let mut out: Vec<&str> = lines[..starts[0]].to_vec();
        let mut to = 0;
        for (slot, section) in order.into_iter().enumerate() {
            if section == from {
                to = out.len();
            }
            out.extend_from_slice(contents[section]);
            out.extend_from_slice(gaps[slot]);
        }
        out.extend_from_slice(&lines[stop..]);
        Some(Moved { text: out.join("\n") + if newline { "\n" } else { "" }, from: starts[from], to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(keys: &[&str]) -> Vec<String> {
        keys.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_closed_heading_hides_the_headings_under_it() {
        let headings = Sections::headings("# A\n## A1\n### A1a\n## A2\n# B\n## B1\n# C");
        assert!(Sections::has_children(0, &headings) && Sections::has_children(1, &headings));
        assert!(!Sections::has_children(2, &headings) && !Sections::has_children(6, &headings));
        assert_eq!(Sections::visible(&headings, &[]), (0..7).collect::<Vec<_>>());
        assert_eq!(Sections::visible(&headings, &closed(&["1:A"])), vec![0, 4, 5, 6]);
        assert_eq!(Sections::visible(&headings, &closed(&["2:A1"])), vec![0, 1, 3, 4, 5, 6]);
        // A heading closed under a closed one stays closed when that opens.
        assert_eq!(Sections::visible(&headings, &closed(&["1:A", "2:A1", "1:B"])), vec![0, 4, 6]);
        // A heading with nothing under it has nothing to hide.
        assert_eq!(Sections::visible(&headings, &closed(&["1:C", "3:A1a"])), (0..7).collect::<Vec<_>>());
        // The section inside a closed heading marks that heading.
        let visible = Sections::visible(&headings, &closed(&["1:A"]));
        assert_eq!(Sections::shown(2, &visible), Some(0));
        assert_eq!(Sections::shown(5, &visible), Some(5));
        assert_eq!(Sections::key(&headings[1]), "2:A1");
    }

    #[test]
    fn the_headings_under_one_are_sorted_with_their_sections() {
        let text = "# Plants\n\nIntro stays first.\n\n## Tomato\n\nRed.\n\n### Cherry tomato\n\n## apple\n\nGreen.\n\n## Bean 10\n\n## Bean 9\nTall.\n\n# Kitchen\n\n## Zucchini";
        let headings = Sections::headings(text);
        let names = |text: &str| Sections::headings(text).into_iter().map(|h| h.text).collect::<Vec<_>>();
        let children: Vec<&str> = Sections::children(0, &headings).iter().map(|i| headings[*i].text.as_str()).collect();
        assert_eq!(children, ["Tomato", "apple", "Bean 10", "Bean 9"]);
        let sorted = Sections::sort_children(text, 0).unwrap();
        assert_eq!(names(&sorted), ["Plants", "apple", "Bean 9", "Bean 10", "Tomato", "Cherry tomato", "Kitchen", "Zucchini"]);
        assert!(sorted.starts_with("# Plants\n\nIntro stays first.\n\n## apple\n\nGreen.\n"));
        assert!(sorted.contains("## Tomato\n\nRed.\n\n### Cherry tomato\n"));
        assert!(sorted.contains("## Bean 9\nTall.\n"));
        // Every line is still there, once.
        let lines = |text: &str| {
            let mut lines: Vec<String> = text.split('\n').filter(|l| !l.is_empty()).map(String::from).collect();
            lines.sort();
            lines
        };
        assert_eq!(lines(&sorted), lines(text));
        assert_eq!(Sections::sort_children(&sorted, 0), None);
        // One heading under it, or none: nothing to sort.
        assert_eq!(Sections::sort_children(text, 6), None);
        assert_eq!(Sections::sort_children(text, 7), None);
    }

    #[test]
    fn sorting_says_why_nothing_changed() {
        let text = "intro\n# Kitchen\n## Zucchini\n# Plants\n## b\n## a\n";
        assert_eq!(Sections::sort_headings(text, None, 0).unwrap_err(), "Put the cursor in a section to sort the headings under it.");
        assert_eq!(Sections::sort_headings(text, None, 1).unwrap_err(), "“Kitchen” has fewer than two headings right under it.");
        let sorted = Sections::sort_headings(text, Some(2), 0).unwrap();
        assert_eq!(sorted, "intro\n# Kitchen\n## Zucchini\n# Plants\n## a\n## b\n");
        assert_eq!(Sections::sort_headings(&sorted, None, 3).unwrap_err(), "The headings under “Plants” are in order already.");
    }

    #[test]
    fn a_section_moves_among_its_siblings() {
        let text = "# A\n\none\n\n# B\n\ntwo\n";
        let moved = Sections::move_section(text, 1, 0).unwrap();
        assert_eq!(moved.text, "# B\n\ntwo\n\n# A\n\none\n");
        assert_eq!((moved.from, moved.to), (4, 0));
        assert_eq!(Sections::move_section(text, 1, 1), None);
        assert_eq!(Sections::heading_at(2, &Sections::headings(text)), Some(0));
        assert_eq!(Sections::heading_at(0, &Sections::headings("text\n# A")), None);
    }
}
