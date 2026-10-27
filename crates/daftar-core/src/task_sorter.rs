//! The vault's own Task Sorter plugin: in each section of a note, the tasks
//! that are done or cancelled go first and the open ones last, each with the
//! lines indented under it. Lines that are not tasks stay where they are.

use std::sync::LazyLock;

use regex::Regex;

static ROOT_TASK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- \[([ xX\-])\] ").unwrap());
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6} ").unwrap());

pub struct TaskSorter;

enum Element<'a> {
    Task { lines: Vec<&'a str>, done: bool },
    Other(Vec<&'a str>),
}

impl TaskSorter {
    /// [text] with its tasks sorted, or `None` when no section changes.
    pub fn sort(text: &str) -> Option<String> {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut result: Vec<&str> = Vec::with_capacity(lines.len());
        let mut changed = false;
        let mut start = 0;
        for index in 1..lines.len() {
            if HEADING.is_match(lines[index]) {
                changed |= append(&lines[start..index], &mut result);
                start = index;
            }
        }
        changed |= append(&lines[start..], &mut result);
        changed.then(|| result.join("\n"))
    }
}

fn append<'a>(section: &[&'a str], result: &mut Vec<&'a str>) -> bool {
    match sort_section(section) {
        Some(sorted) => {
            result.extend(sorted);
            true
        }
        None => {
            result.extend_from_slice(section);
            false
        }
    }
}

fn sort_section<'a>(lines: &[&'a str]) -> Option<Vec<&'a str>> {
    let mut elements = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if let Some(c) = ROOT_TASK.captures(lines[index]) {
            let done = &c[1] != " ";
            let mut block = vec![lines[index]];
            index += 1;
            while index < lines.len() && (lines[index].starts_with("    ") || lines[index].starts_with('\t')) {
                block.push(lines[index]);
                index += 1;
            }
            elements.push(Element::Task { lines: block, done });
        } else {
            let mut run = vec![lines[index]];
            index += 1;
            while index < lines.len() && !ROOT_TASK.is_match(lines[index]) {
                run.push(lines[index]);
                index += 1;
            }
            elements.push(Element::Other(run));
        }
    }
    let tasks: Vec<(&Vec<&str>, bool)> = elements
        .iter()
        .filter_map(|e| match e {
            Element::Task { lines, done } => Some((lines, *done)),
            Element::Other(_) => None,
        })
        .collect();
    let sorted: Vec<&Vec<&str>> = tasks.iter().filter(|t| t.1).map(|t| t.0).chain(tasks.iter().filter(|t| !t.1).map(|t| t.0)).collect();
    if sorted.iter().zip(&tasks).all(|(a, b)| *a == b.0) {
        return None;
    }
    let mut next = sorted.into_iter();
    let mut out = Vec::with_capacity(lines.len());
    for element in &elements {
        match element {
            Element::Other(lines) => out.extend_from_slice(lines),
            Element::Task { .. } => out.extend_from_slice(next.next().map(Vec::as_slice).unwrap_or_default()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_tasks_go_first_with_their_sub_items() {
        let text = "# Plan\n- [ ] Write the report\n    - [x] Outline\n- [x] Book the room\n- [-] Call the bank\n\tnote under the call\n- [ ] Send the slides";
        assert_eq!(
            TaskSorter::sort(text).as_deref(),
            Some(
                "# Plan\n- [x] Book the room\n- [-] Call the bank\n\tnote under the call\n- [ ] Write the report\n    - [x] Outline\n- [ ] Send the slides"
            )
        );
    }

    #[test]
    fn each_section_is_sorted_on_its_own_and_other_lines_stay() {
        let text = "- [ ] Open on top\n- [X] Done on top\n## Next\nSome words.\n- [ ] Open\nBetween the tasks.\n- [x] Done\n## Last\n- [x] Already first\n- [ ] Already last";
        assert_eq!(
            TaskSorter::sort(text).as_deref(),
            Some(
                "- [X] Done on top\n- [ ] Open on top\n## Next\nSome words.\n- [x] Done\nBetween the tasks.\n- [ ] Open\n## Last\n- [x] Already first\n- [ ] Already last"
            )
        );
    }

    #[test]
    fn a_sorted_note_is_left_alone() {
        assert_eq!(TaskSorter::sort("- [x] a\n- [ ] b\n"), None);
        assert_eq!(TaskSorter::sort(""), None);
        assert_eq!(TaskSorter::sort("# Only a heading"), None);
        // In progress, an indented task and a `*` bullet are not root tasks.
        assert_eq!(TaskSorter::sort("- [/] going\n- [x] done\n"), None);
        assert_eq!(TaskSorter::sort("  - [ ] a\n  - [x] b"), None);
        assert_eq!(TaskSorter::sort("* [ ] a\n* [x] b"), None);
    }

    #[test]
    fn sorting_twice_changes_nothing() {
        let once = TaskSorter::sort("- [ ] a\n- [x] b\n- [ ] c\n- [-] d");
        assert_eq!(once.as_deref(), Some("- [x] b\n- [-] d\n- [ ] a\n- [ ] c"));
        assert_eq!(once.and_then(|t| TaskSorter::sort(&t)), None);
    }
}
