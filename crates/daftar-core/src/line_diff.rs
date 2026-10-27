//! The lines that one text of a note adds and removes from another, as
//! `git diff` shows them, for a note's history. The port of `LineDiff.swift`;
//! Swift's `difference(from:)` is a Myers diff, and so is this one.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    /// The line's number, from 1, in the old text and in the new one.
    pub old: Option<usize>,
    pub new: Option<usize>,
}

pub struct LineDiff;

impl LineDiff {
    /// Every line of [old] and [new], each removed, added or the same.
    pub fn lines(old: &str, new: &str) -> Vec<Line> {
        let (a, b) = (split(old), split(new));
        let same = common(&a, &b);
        let mut result = Vec::with_capacity(a.len().max(b.len()));
        let (mut i, mut j) = (0, 0);
        for &(x, y) in same.iter().chain(std::iter::once(&(a.len(), b.len()))) {
            while i < x {
                result.push(Line { kind: Kind::Removed, text: a[i].to_string(), old: Some(i + 1), new: None });
                i += 1;
            }
            while j < y {
                result.push(Line { kind: Kind::Added, text: b[j].to_string(), old: None, new: Some(j + 1) });
                j += 1;
            }
            if x < a.len() && y < b.len() {
                result.push(Line { kind: Kind::Same, text: a[x].to_string(), old: Some(x + 1), new: Some(y + 1) });
                i += 1;
                j += 1;
            }
        }
        result
    }

    /// The changed lines with [context] lines around them, a run of changes
    /// apart from the next when more than twice [context] lines lie between.
    pub fn hunks(lines: &[Line], context: usize) -> Vec<Vec<Line>> {
        let changed: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].kind != Kind::Same).collect();
        let Some(&first) = changed.first() else { return Vec::new() };
        let range = |index: usize| (index.saturating_sub(context), (index + context).min(lines.len() - 1));
        let mut ranges = Vec::new();
        let mut current = range(first);
        for &index in &changed[1..] {
            let next = range(index);
            if next.0 <= current.1 + 1 {
                current.1 = next.1;
            } else {
                ranges.push(current);
                current = next;
            }
        }
        ranges.push(current);
        ranges.into_iter().map(|(from, to)| lines[from..=to].to_vec()).collect()
    }

    /// How many lines [lines] adds and removes.
    pub fn counts(lines: &[Line]) -> (usize, usize) {
        (lines.iter().filter(|l| l.kind == Kind::Added).count(), lines.iter().filter(|l| l.kind == Kind::Removed).count())
    }
}

/// The lines of [text], without their ends; a last empty line is none.
fn split(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// The pairs of lines the two texts keep, in order: a longest common
/// subsequence, by Myers' greedy walk, after the common start and end.
fn common(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let start = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (middle_a, middle_b) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    let mut pairs: Vec<(usize, usize)> = (0..start).map(|i| (i, i)).collect();
    pairs.extend(myers(middle_a, middle_b).into_iter().map(|(x, y)| (x + start, y + start)));
    pairs.extend((0..end).map(|k| (a.len() - end + k, b.len() - end + k)));
    pairs
}

fn myers(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    if max == 0 {
        return Vec::new();
    }
    let offset = max as isize;
    let mut v = vec![0isize; 2 * max + 2];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    'search: for d in 0..=max as isize {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) { v[index + 1] } else { v[index - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                break 'search;
            }
        }
    }
    // Back from the end, the diagonal runs are the kept lines.
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let previous = if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) { k + 1 } else { k - 1 };
        let start_x = v[(previous + offset) as usize];
        let start_y = start_x - previous;
        while x > start_x && y > start_y {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        x = start_x;
        y = start_y;
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        pairs.push((x as usize, y as usize));
    }
    pairs.reverse();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[Line]) -> Vec<Kind> {
        lines.iter().map(|l| l.kind).collect()
    }

    #[test]
    fn lines_are_added_removed_or_the_same() {
        let lines = LineDiff::lines("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(kinds(&lines), [Kind::Same, Kind::Removed, Kind::Added, Kind::Same, Kind::Added]);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["a", "b", "B", "c", "d"]);
        assert!(lines[1].old == Some(2) && lines[2].new == Some(2) && lines[4].new == Some(4));
        assert_eq!(LineDiff::counts(&lines), (2, 1));
        assert_eq!(kinds(&LineDiff::lines("", "x")), [Kind::Added]);
        assert!(LineDiff::lines("x\r\ny", "x\ny").iter().all(|l| l.kind == Kind::Same));
        assert!(LineDiff::lines("", "").is_empty());
    }

    #[test]
    fn hunks_keep_three_lines_around() {
        let old = (1..=20).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
        let new = old.replace("\n2\n", "\ntwo\n").replace("\n18\n", "\neighteen\n");
        let hunks = LineDiff::hunks(&LineDiff::lines(&old, &new), 3);
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].first().unwrap().text == "1" && hunks[0].last().unwrap().text == "5");
        assert!(hunks[1].first().unwrap().text == "15" && hunks[1].last().unwrap().text == "20");
        let near = old.replace("\n5\n", "\nfive\n").replace("\n9\n", "\nnine\n");
        assert_eq!(LineDiff::hunks(&LineDiff::lines(&old, &near), 3).len(), 1);
        assert!(LineDiff::hunks(&LineDiff::lines(&old, &old), 3).is_empty());
    }

    /// The robustness line: a diff takes the old text to the new, and is as
    /// short as an edit can be.
    #[test]
    fn a_diff_takes_the_old_text_to_the_new() {
        let words = ["a", "b", "c", "", "d"];
        let mut state: u64 = 7;
        let mut next = move |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..500 {
            let old: Vec<&str> = (0..next(10)).map(|_| words[next(words.len())]).collect();
            let new: Vec<&str> = (0..next(10)).map(|_| words[next(words.len())]).collect();
            let (old, new) = (old.join("\n"), new.join("\n"));
            let lines = LineDiff::lines(&old, &new);
            let kept_old: Vec<&str> = lines.iter().filter(|l| l.kind != Kind::Added).map(|l| l.text.as_str()).collect();
            let kept_new: Vec<&str> = lines.iter().filter(|l| l.kind != Kind::Removed).map(|l| l.text.as_str()).collect();
            let (a, b) = (split(&old), split(&new));
            assert_eq!(kept_old, a);
            assert_eq!(kept_new, b);
            // The same lines are a longest common subsequence.
            let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
            for i in (0..a.len()).rev() {
                for j in (0..b.len()).rev() {
                    table[i][j] = if a[i] == b[j] { table[i + 1][j + 1] + 1 } else { table[i + 1][j].max(table[i][j + 1]) };
                }
            }
            assert_eq!(lines.iter().filter(|l| l.kind == Kind::Same).count(), table[0][0]);
        }
    }
}
