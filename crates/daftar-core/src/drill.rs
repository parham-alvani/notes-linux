//! Spaced-repetition drills from the notes as they are: a card is a heading
//! of a note, its section is the answer, and the note's title and the
//! headings above it are the context. Drilling never writes a note; what
//! was rated, and when each card is due, is Daftar's own state, kept per
//! vault in `$XDG_STATE_HOME/daftar/drill-<hash>.json`.
//!
//! Which headings are cards, on notes of questions and answers (a `###`
//! question under a `##` part, a `##` problem whose `### The Problem`,
//! `### Test Cases` and `### Solution` belong to it, a `####` example):
//!
//! - A **part** of an answer is a heading named as one: Example(s) (and one
//!   starting "Example" or "Worked example"), Problem, The Problem, Test
//!   Cases, Tests, Solution(s) (and any ending in "Solution(s)", as "Two
//!   Solutions"), Answer, Code, Output, Explanation, Hint(s). A part is never
//!   a card: it is the answer of the heading above it, with its heading.
//! - A **reference** heading (Related notes, Related, See also, Sources,
//!   References, Links, Further reading) is never a card and is left out of
//!   every answer.
//! - Every other heading is a card when its answer has text: its own lines,
//!   and the parts under it, but not the headings under it that are cards
//!   themselves, nor references. So `## Part 1: Indexes` with only
//!   questions under it is context, not a card, and a part with an
//!   introduction is a card of that introduction.
//! - The text above the first heading is no card, and a folder note
//!   (`Interview-Preparation/Interview-Preparation.md`) is left out of a
//!   folder's deck: it is the folder's index.
//!
//! A card is known by its note and its heading path, written as Obsidian
//! links to a nested heading: `Notes/Go.md#Part 1#Q1. What is a goroutine?`.
//! Changing the answer keeps the card's history; renaming a heading above
//! it starts the card again.
//!
//! The schedule is SuperMemo's SM-2 (Woźniak, 1990), with its quality
//! grades on four buttons: Again is 2 (forgotten), Hard 3, Good 4, Easy 5.
//! A forgotten card starts its repetitions again at one day and keeps its
//! ease; any other grade changes the ease by
//! `0.1 - (5 - q) * (0.08 + (5 - q) * 0.02)`, never under 1.3, and the
//! interval is 1 day, then 6, then the last one times the ease.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::day::Day;
use crate::note_parser::NoteParser;
use crate::reading_places;
use crate::sections::Sections;
use crate::vault::{Vault, VaultIndex};

/// One question: a heading and its section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// `path#Heading#Under it`, the card's name in the state file.
    pub id: String,
    pub path: String,
    /// The heading's line in the file, from 0.
    pub line: usize,
    /// The heading's text.
    pub question: String,
    /// The note's title, then the headings above this one, outermost first.
    pub context: Vec<String>,
    /// The section's Markdown, without the cards and references under it.
    pub answer: String,
}

/// How well a card was remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Again,
    Hard,
    Good,
    Easy,
}

impl Rating {
    /// `1`–`4`, or the rating's name in any case.
    pub fn parse(text: &str) -> Option<Rating> {
        match text.trim().to_lowercase().as_str() {
            "1" | "again" => Some(Rating::Again),
            "2" | "hard" => Some(Rating::Hard),
            "3" | "good" => Some(Rating::Good),
            "4" | "easy" => Some(Rating::Easy),
            _ => None,
        }
    }

    /// SM-2's quality grade, 0 to 5.
    fn quality(self) -> u8 {
        match self {
            Rating::Again => 2,
            Rating::Hard => 3,
            Rating::Good => 4,
            Rating::Easy => 5,
        }
    }
}

/// What is known of a card that was rated.
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    /// SM-2's E-Factor, from 1.3 up; 2.5 for a new card.
    pub ease: f64,
    /// Days from the last rating to the next.
    pub interval: i64,
    /// Ratings in a row that were not Again.
    pub reps: u32,
    /// How many times it was forgotten.
    pub lapses: u32,
    pub due: Day,
    pub last: Day,
}

impl Review {
    const START_EASE: f64 = 2.5;
    const LEAST_EASE: f64 = 1.3;

    /// The card after [rating] on [today], from what was known of it.
    pub fn rate(previous: Option<&Review>, rating: Rating, today: Day) -> Review {
        let (ease, interval, reps, lapses) = previous.map_or((Self::START_EASE, 0, 0, 0), |r| (r.ease, r.interval, r.reps, r.lapses));
        let q = f64::from(rating.quality());
        if rating == Rating::Again {
            return Review { ease, interval: 1, reps: 0, lapses: lapses + 1, due: today.adding_days(1), last: today };
        }
        let ease = (ease + (0.1 - (5.0 - q) * (0.08 + (5.0 - q) * 0.02))).max(Self::LEAST_EASE);
        // Two places are enough, and keep the file's numbers short.
        let ease = (ease * 100.0).round() / 100.0;
        let reps = reps + 1;
        let interval = match reps {
            1 => 1,
            2 => 6,
            #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss, reason = "an interval of days, far under 2^52")]
            _ => ((interval.max(1) as f64) * ease).round() as i64,
        };
        Review { ease, interval, reps, lapses, due: today.adding_days(interval), last: today }
    }

    fn json(&self) -> Value {
        json!({
            "ease": self.ease,
            "interval": self.interval,
            "reps": self.reps,
            "lapses": self.lapses,
            "due": self.due.to_string(),
            "last": self.last.to_string(),
        })
    }

    fn from_json(value: &Value) -> Option<Review> {
        let day = |key: &str| value.get(key).and_then(Value::as_str).and_then(Day::parse);
        let count = |key: &str| value.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
        Some(Review {
            ease: value.get("ease").and_then(Value::as_f64)?.max(Self::LEAST_EASE),
            interval: value.get("interval").and_then(Value::as_i64)?,
            reps: count("reps")?,
            lapses: count("lapses").unwrap_or(0),
            due: day("due")?,
            last: day("last")?,
        })
    }
}

/// Every card's review, by its id: the state file's content.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrillState {
    pub reviews: BTreeMap<String, Review>,
}

impl DrillState {
    /// The state file of the vault at [root]: `drill-<hash>.json` in
    /// Daftar's state folder, beside the reading places.
    pub fn file(root: &Path) -> PathBuf {
        reading_places::state_file(root, "drill")
    }

    /// The state in [file]; empty when there is none or it cannot be read.
    pub fn load(file: &Path) -> DrillState {
        fs::read_to_string(file).map(|data| DrillState::from_data(&data)).unwrap_or_default()
    }

    pub fn from_data(data: &str) -> DrillState {
        let Ok(value) = serde_json::from_str::<Value>(data) else { return DrillState::default() };
        let reviews = value
            .get("cards")
            .and_then(Value::as_object)
            .map(|cards| cards.iter().filter_map(|(id, review)| Some((id.clone(), Review::from_json(review)?))).collect())
            .unwrap_or_default();
        DrillState { reviews }
    }

    pub fn data(&self) -> String {
        let cards: Map<String, Value> = self.reviews.iter().map(|(id, review)| (id.clone(), review.json())).collect();
        serde_json::to_string_pretty(&json!({ "version": 1, "cards": cards })).unwrap_or_default()
    }

    /// Writes the state to [file] through a temporary file and a rename.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(folder) = file.parent() {
            fs::create_dir_all(folder)?;
        }
        let name = file.file_name().map_or_else(|| "drill.json".into(), |n| n.to_string_lossy().into_owned());
        let temporary = file.with_file_name(format!(".{name}.daftar-{}", std::process::id()));
        fs::write(&temporary, self.data())?;
        fs::rename(&temporary, file)
    }

    /// Rates the card [id] on [today], and its review after.
    pub fn rate(&mut self, id: &str, rating: Rating, today: Day) -> Review {
        let review = Review::rate(self.reviews.get(id), rating, today);
        self.reviews.insert(id.to_string(), review.clone());
        review
    }

    /// The day [card] is due, or `None` for a card never rated.
    pub fn due(&self, card: &Card) -> Option<Day> {
        self.reviews.get(&card.id).map(|r| r.due)
    }
}

pub struct Drill;

/// The heading as the rules compare it: lowercase, without emphasis marks
/// or a closing colon.
fn plain(heading: &str) -> String {
    let text: String = heading.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
    text.trim().trim_end_matches(':').trim().to_lowercase()
}

impl Drill {
    /// The folder [Drill::deck] reads when given none.
    pub const FOLDER: &'static str = "Learning/Interview-Preparation";
    /// New cards a session takes at most.
    pub const NEW_CARDS: usize = 10;

    /// A part of the answer of the heading above it.
    pub fn is_part(heading: &str) -> bool {
        const PARTS: [&str; 16] = [
            "example",
            "examples",
            "problem",
            "the problem",
            "test cases",
            "tests",
            "solution",
            "solutions",
            "answer",
            "the answer",
            "code",
            "output",
            "explanation",
            "hint",
            "hints",
            "the solution",
        ];
        let text = plain(heading);
        PARTS.contains(&text.as_str())
            || text.starts_with("example")
            || text.starts_with("worked example")
            || text.ends_with(" solution")
            || text.ends_with(" solutions")
    }

    /// A list of links or sources: no card, and no part of an answer.
    pub fn is_reference(heading: &str) -> bool {
        const REFERENCES: [&str; 7] = ["related notes", "related", "see also", "sources", "references", "links", "further reading"];
        REFERENCES.contains(&plain(heading).as_str())
    }

    /// The cards of the note at [path] whose text is [text], in its order.
    pub fn cards(path: &str, text: &str) -> Vec<Card> {
        let headings = NoteParser::parse(text).headings;
        let (lines, _) = Sections::lines(text);
        let count = headings.len();
        let mut parent: Vec<Option<usize>> = vec![None; count];
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut open: Vec<usize> = Vec::new();
        for (index, heading) in headings.iter().enumerate() {
            while open.last().is_some_and(|&top| headings[top].level >= heading.level) {
                open.pop();
            }
            if let Some(&top) = open.last() {
                parent[index] = Some(top);
                children[top].push(index);
            }
            open.push(index);
        }
        let has_text = |kept: &[usize]| kept.iter().any(|&l| lines.get(l).is_some_and(|line| !line.trim().is_empty()));
        // Each heading's answer lines and whether it is a card, from the
        // last heading up, since a heading's answer depends on its children.
        let mut kept: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut card = vec![false; count];
        for index in (0..count).rev() {
            let heading = &headings[index];
            let end = Sections::end(index, &headings, lines.len());
            let first_child = children[index].first().map_or(end, |&c| headings[c].line);
            let mut answer: Vec<usize> = (heading.line + 1..first_child).collect();
            for &child in &children[index] {
                if card[child] || Self::is_reference(&headings[child].text) || !has_text(&kept[child]) {
                    continue;
                }
                answer.push(headings[child].line);
                answer.extend_from_slice(&kept[child]);
            }
            card[index] = !Self::is_part(&heading.text) && !Self::is_reference(&heading.text) && has_text(&answer);
            kept[index] = answer;
        }
        let title = Vault::title(path);
        let mut seen = HashSet::new();
        let mut cards = Vec::new();
        for index in (0..count).filter(|&i| card[i]) {
            let mut above = Vec::new();
            let mut at = parent[index];
            while let Some(up) = at {
                above.push(headings[up].text.clone());
                at = parent[up];
            }
            above.reverse();
            let mut names = above.clone();
            names.push(headings[index].text.clone());
            let id = format!("{path}#{}", names.join("#"));
            // Two headings of the same path are one card: the first.
            if !seen.insert(id.clone()) {
                continue;
            }
            let answer: Vec<&str> = kept[index].iter().filter_map(|&l| lines.get(l).copied()).collect();
            let answer = answer.join("\n");
            let mut context = vec![title.clone()];
            context.extend(above);
            cards.push(Card {
                id,
                path: path.to_string(),
                line: headings[index].line,
                question: headings[index].text.clone(),
                context,
                answer: answer.trim_matches('\n').trim_end().to_string(),
            });
        }
        cards
    }

    /// A folder note: `X/X.md`, the index of its folder.
    pub fn is_folder_note(path: &str) -> bool {
        let mut parts = path.rsplit('/');
        let name = parts.next().unwrap_or_default();
        parts.next().is_some_and(|folder| Vault::title(name) == folder)
    }

    /// The notes of [folder] at any depth, by path, without its folder notes
    /// and what [hidden] leaves out.
    pub fn notes_in(index: &VaultIndex, folder: &str, hidden: impl Fn(&str) -> bool) -> Vec<String> {
        let folder = folder.trim_matches('/');
        let prefix = format!("{folder}/");
        let mut notes: Vec<String> = index
            .notes
            .keys()
            .filter(|p| folder.is_empty() || p.starts_with(&prefix))
            .filter(|p| !Self::is_folder_note(p) && !hidden(p))
            .cloned()
            .collect();
        notes.sort();
        notes
    }

    /// The cards of the notes at [paths] under [root], read now.
    pub fn deck(root: &Path, paths: &[String]) -> Vec<Card> {
        paths.iter().filter_map(|path| Some(Self::cards(path, &fs::read_to_string(root.join(path)).ok()?))).flatten().collect()
    }

    /// The cards to drill on [today]: the ones due, the longest overdue
    /// first, then at most [new] never rated, in the deck's order; at most
    /// [limit] in all.
    pub fn due<'a>(cards: &'a [Card], state: &DrillState, today: Day, new: usize, limit: usize) -> Vec<&'a Card> {
        let mut due: Vec<(Day, &Card)> = cards.iter().filter_map(|c| state.due(c).filter(|&d| d <= today).map(|d| (d, c))).collect();
        due.sort_by_key(|(day, _)| *day);
        let fresh = cards.iter().filter(|c| state.due(c).is_none()).take(new);
        due.into_iter().map(|(_, c)| c).chain(fresh).take(limit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        Day::parse(text).unwrap()
    }

    const QUESTIONS: &str = "\
- [A source](https://example.com)

Every question has a short answer.

## The Table to Remember

| Problem | Tool |
| --- | --- |
| Slow | Plan |

## Part 1: Indexes

### Q1. What does an index change?

**Answer:** reads get faster, writes slower.

#### Example

```sql
## not a heading
CREATE INDEX ON users (email);
```

### Q2. When is an index not used?

When a function wraps the column.

## Part 2: Writing SQL

Joins and groups.

### Q3. WHERE or HAVING?

WHERE filters rows, HAVING groups.

## Related notes

- [[Elsewhere]]
";

    #[test]
    fn a_question_under_a_part_is_a_card_and_its_example_is_its_answer() {
        let cards = Drill::cards("Prep/Database Questions.md", QUESTIONS);
        let questions: Vec<&str> = cards.iter().map(|c| c.question.as_str()).collect();
        assert_eq!(
            questions,
            [
                "The Table to Remember",
                "Q1. What does an index change?",
                "Q2. When is an index not used?",
                "Part 2: Writing SQL",
                "Q3. WHERE or HAVING?"
            ]
        );
        let q1 = &cards[1];
        assert_eq!(q1.id, "Prep/Database Questions.md#Part 1: Indexes#Q1. What does an index change?");
        assert_eq!(q1.context, ["Database Questions", "Part 1: Indexes"]);
        assert_eq!(q1.line, 12);
        assert!(q1.answer.starts_with("**Answer:** reads get faster"));
        assert!(q1.answer.contains("#### Example"));
        assert!(q1.answer.contains("## not a heading"));
        assert!(!q1.answer.contains("Q2"));
        // A part with an introduction is a card of the introduction alone.
        assert_eq!(cards[3].answer, "Joins and groups.");
        assert!(!cards.iter().any(|c| c.answer.contains("Elsewhere")));
    }

    #[test]
    fn a_problem_is_one_card_with_its_parts() {
        let text = "\
# Coding

## Most Frequent Item

### The Problem

Return the item seen most.

### Test Cases

| In | Out |
| --- | --- |
| [] | None |

### What to Ask First

Ties?

### Two Solutions

Count in a map.

#### Example

```python
pass
```

## Sources

- a book
";
        let cards = Drill::cards("Coding.md", text);
        let questions: Vec<&str> = cards.iter().map(|c| c.question.as_str()).collect();
        assert_eq!(questions, ["Most Frequent Item", "What to Ask First"]);
        let item = &cards[0];
        assert!(item.answer.starts_with("### The Problem\n\nReturn the item seen most."));
        assert!(item.answer.contains("### Test Cases"));
        assert!(item.answer.contains("### Two Solutions\n\nCount in a map."));
        assert!(item.answer.contains("#### Example"));
        assert!(!item.answer.contains("Ties?"));
        assert_eq!(cards[1].context, ["Coding", "Coding", "Most Frequent Item"]);
    }

    #[test]
    fn headings_with_nothing_under_them_are_no_cards() {
        assert!(Drill::cards("A.md", "# A\n\n## B\n\n### C\n").is_empty());
        assert!(Drill::cards("A.md", "Text with no heading.\n").is_empty());
        assert!(Drill::cards("A.md", "## Example\n\nAlone.\n").is_empty());
        assert_eq!(Drill::cards("A.md", "## Same\n\nOne.\n\n## Same\n\nTwo.\n").len(), 1);
    }

    #[test]
    fn parts_and_references_are_named_so() {
        for part in ["Example", "**Examples**", "Worked Example: URL Shortener", "The Problem", "Test Cases", "Two Solutions", "Solution:"]
        {
            assert!(Drill::is_part(part), "{part}");
        }
        for card in ["Q1. What is an example of a race?", "Problems", "How to Run the Round"] {
            assert!(!Drill::is_part(card), "{card}");
        }
        assert!(Drill::is_reference("Related Notes"));
        assert!(!Drill::is_reference("Related work"));
    }

    #[test]
    fn sm2_spaces_good_answers_out_and_starts_a_forgotten_card_again() {
        let today = day("2026-11-08");
        let first = Review::rate(None, Rating::Good, today);
        assert_eq!((first.interval, first.reps, first.due), (1, 1, day("2026-11-09")));
        assert!((first.ease - 2.5).abs() < 1e-9);
        let second = Review::rate(Some(&first), Rating::Good, first.due);
        assert_eq!((second.interval, second.due), (6, day("2026-11-15")));
        let third = Review::rate(Some(&second), Rating::Good, second.due);
        assert_eq!(third.interval, 15);
        let easy = Review::rate(Some(&second), Rating::Easy, second.due);
        assert!((easy.ease - 2.6).abs() < 1e-9);
        assert_eq!(easy.interval, 16);
        let hard = Review::rate(Some(&second), Rating::Hard, second.due);
        assert!((hard.ease - 2.36).abs() < 1e-9);
        assert_eq!(hard.interval, 14);
        let forgotten = Review::rate(Some(&third), Rating::Again, third.due);
        assert_eq!((forgotten.interval, forgotten.reps, forgotten.lapses), (1, 0, 1));
        assert!((forgotten.ease - third.ease).abs() < 1e-9);
        let back = Review::rate(Some(&forgotten), Rating::Good, forgotten.due);
        assert_eq!(back.interval, 1);
    }

    #[test]
    fn the_ease_never_falls_under_1_3() {
        let mut review = Review::rate(None, Rating::Hard, day("2026-01-01"));
        for _ in 0..20 {
            review = Review::rate(Some(&review), Rating::Hard, review.due);
        }
        assert!((review.ease - 1.3).abs() < 1e-9);
    }

    #[test]
    fn the_due_cards_come_first_then_a_few_new_ones() {
        let text = "## A\n\na\n\n## B\n\nb\n\n## C\n\nc\n\n## D\n\nd\n";
        let cards = Drill::cards("N.md", text);
        let today = day("2026-11-08");
        let mut state = DrillState::default();
        state.reviews.insert("N.md#C".into(), Review::rate(None, Rating::Good, day("2026-11-01")));
        state.reviews.insert("N.md#D".into(), Review::rate(None, Rating::Good, day("2026-11-05")));
        state.reviews.insert("N.md#B".into(), Review::rate(None, Rating::Good, today));
        let due: Vec<&str> = Drill::due(&cards, &state, today, 10, 100).iter().map(|c| c.question.as_str()).collect();
        assert_eq!(due, ["C", "D", "A"]);
        assert_eq!(Drill::due(&cards, &state, today, 0, 100).len(), 2);
        assert_eq!(Drill::due(&cards, &state, today, 10, 1).len(), 1);
    }

    #[test]
    fn the_state_comes_back_from_its_file() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("state/drill.json");
        let mut state = DrillState::default();
        state.rate("N.md#A", Rating::Good, day("2026-11-08"));
        state.rate("N.md#A", Rating::Easy, day("2026-11-09"));
        state.save(&file).unwrap();
        let back = DrillState::load(&file);
        assert_eq!(back, state);
        assert_eq!(back.reviews["N.md#A"].due, day("2026-11-15"));
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        assert_eq!(DrillState::from_data("not json"), DrillState::default());
    }

    #[test]
    fn a_folders_deck_leaves_its_folder_note_out() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::create_dir_all(root.join("Prep/Go")).unwrap();
        fs::write(root.join("Prep/Prep.md"), "## Notes in This Category\n\n- [[Go]]\n").unwrap();
        fs::write(root.join("Prep/Go/Go Questions.md"), "## Q1. What is a goroutine?\n\nA light thread.\n").unwrap();
        fs::write(root.join("Other.md"), "## Elsewhere\n\nText.\n").unwrap();
        let index = Vault::index(root, None);
        let notes = Drill::notes_in(&index, "Prep/", |_| false);
        assert_eq!(notes, ["Prep/Go/Go Questions.md"]);
        let deck = Drill::deck(root, &notes);
        assert_eq!(deck.len(), 1);
        assert_eq!(deck[0].answer, "A light thread.");
        assert!(Drill::notes_in(&index, "Prep", |p| p.contains("Go")).is_empty());
    }
}
