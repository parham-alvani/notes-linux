//! A fuzzy match, as a command palette does it: every letter of the query
//! in order, anywhere in the text. A letter at the start of a word, and
//! letters in a row, score more; a gap scores less.

use crate::text::slugs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    /// The matched characters, as offsets into the text.
    pub positions: Vec<usize>,
}

pub struct Fuzzy;

impl Fuzzy {
    pub fn matches(query: &str, text: &str) -> Option<Match> {
        let needle: Vec<char> = slugs::fold(query).chars().filter(|c| !c.is_whitespace()).collect();
        if needle.is_empty() {
            return Some(Match { score: 0, positions: vec![] });
        }
        let haystack: Vec<char> = slugs::fold(text).chars().collect();
        let original: Vec<char> = slugs::normalize(text).chars().collect();
        if haystack.len() != original.len() {
            return None;
        }
        let (n, m) = (haystack.len(), needle.len());
        if m > n {
            return None;
        }
        // best[i][j]: the best score with letter i of the query at j.
        let none = i64::MIN / 2;
        let mut best = vec![vec![none; n]; m];
        let mut from = vec![vec![usize::MAX; n]; m];
        for i in 0..m {
            for j in i..n {
                if haystack[j] != needle[i] {
                    continue;
                }
                let own = 1 + if j == 0 {
                    8
                } else if is_boundary(&original, j) {
                    6
                } else {
                    0
                };
                if i == 0 {
                    best[i][j] = own;
                    continue;
                }
                for k in (i - 1)..j {
                    if best[i - 1][k] <= none {
                        continue;
                    }
                    let step = if k + 1 == j { 5 } else { -(4.min(j - k - 1) as i64) };
                    let score = best[i - 1][k] + own + step;
                    if score > best[i][j] {
                        best[i][j] = score;
                        from[i][j] = k;
                    }
                }
            }
        }
        // The first of equal bests, as on the Mac: `max_by_key` keeps the last.
        let last = (0..n).rev().max_by_key(|&j| best[m - 1][j])?;
        if best[m - 1][last] <= none {
            return None;
        }
        let mut positions = vec![last];
        for i in (1..m).rev() {
            positions.insert(0, from[i][positions[0]]);
        }
        // Shorter texts win a tie: "Sync" over "Sync Now and Push".
        Some(Match { score: best[m - 1][last] - (n / 12) as i64, positions })
    }
}

/// The start of a word: after a space or punctuation, or a capital after a small letter.
fn is_boundary(text: &[char], at: usize) -> bool {
    let before = text[at - 1];
    if before.is_whitespace() || "-_/.:›(".contains(before) {
        return true;
    }
    text[at].is_uppercase() && before.is_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order() {
        assert_eq!(Fuzzy::matches("nn", "New Note").unwrap().positions, vec![0, 4]);
        assert!(Fuzzy::matches("xz", "New Note").is_none());
        assert_eq!(Fuzzy::matches("", "Anything").unwrap().score, 0);
    }

    #[test]
    fn word_starts_and_runs_score_more() {
        let starts = Fuzzy::matches("tt", "Toggle Terminal").unwrap();
        let middle = Fuzzy::matches("tt", "Bottle").unwrap();
        assert!(starts.score > middle.score);
        let run = Fuzzy::matches("sync", "Sync Now").unwrap();
        let spread = Fuzzy::matches("sync", "Show your notes, cleanly").unwrap();
        assert!(run.score > spread.score);
    }

    #[test]
    fn picks_the_best_place() {
        // "rn" at the word starts of "Random Note", not the "r" in "Rename".
        assert_eq!(Fuzzy::matches("rn", "Open Random Note").unwrap().positions, vec![5, 12]);
    }

    #[test]
    fn folds_case_and_counts_capitals() {
        assert!(Fuzzy::matches("NEW", "new note").is_some());
        assert_eq!(Fuzzy::matches("qo", "QuickOpen").unwrap().positions, vec![0, 5]);
    }
}
