//! `daftar-cli drill`: spaced-repetition drills from the notes' headings
//! (`daftar_core::Drill`). In a terminal it asks each card due: the
//! context and the question, Enter shows the answer, 1–4 rates it, `q`
//! stops. Otherwise, or with `--list`, it prints the cards due. Each rating
//! is saved at once in Daftar's state file; no note is ever written.

use std::io::{BufRead, Write};
use std::path::Path;

use daftar_core::{Card, Day, Drill, DrillState, Rating, VaultIndex};

/// The notes [targets] name, each a note or a folder of the vault; the
/// interview preparation folder when none is named.
pub fn notes(index: &VaultIndex, targets: &[String], hidden: impl Fn(&str) -> bool) -> Result<Vec<String>, String> {
    if targets.is_empty() {
        return Ok(Drill::notes_in(index, Drill::FOLDER, hidden));
    }
    let mut notes = Vec::new();
    for target in targets {
        if let Ok(path) = crate::note_path(index, target) {
            notes.push(path);
            continue;
        }
        let found = Drill::notes_in(index, target, &hidden);
        if found.is_empty() {
            return Err(format!("{target} is neither a note nor a folder of notes"));
        }
        notes.extend(found);
    }
    let mut seen = std::collections::HashSet::new();
    notes.retain(|p| seen.insert(p.clone()));
    Ok(notes)
}

/// A card as a line: `path:line`, a tab, when it is due (`new` for one
/// never rated), a tab, and the heading.
pub fn line(card: &Card, state: &DrillState) -> String {
    let due = state.due(card).map_or_else(|| "new".to_string(), |d| d.to_string());
    format!("{}:{}\t{due}\t{}", card.path, card.line + 1, card.question)
}

/// The cards' context: the note and the headings above, with `›` between.
pub fn context(card: &Card) -> String {
    card.context.join(" › ")
}

/// How a session went.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub rated: usize,
    pub again: usize,
    pub left: usize,
}

/// Asks [cards] one by one on [input] and [output]. A card rated Again
/// comes back at the end of the session, as SM-2 repeats what was
/// forgotten, but only its first rating is kept. The state is saved to
/// [file] after each rating.
pub fn session(
    cards: &[&Card],
    state: &mut DrillState,
    file: &Path,
    today: Day,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Outcome, String> {
    let failed = |e: std::io::Error| e.to_string();
    let mut queue: std::collections::VecDeque<(&Card, bool)> = cards.iter().map(|&c| (c, false)).collect();
    let mut outcome = Outcome::default();
    let total = queue.len();
    let mut asked = 0;
    while let Some((card, again)) = queue.pop_front() {
        asked += usize::from(!again);
        let count = if again { "again".to_string() } else { format!("{asked}/{total}") };
        writeln!(output, "\n\x1b[2m[{count}] {}\x1b[0m", context(card)).map_err(failed)?;
        writeln!(output, "\x1b[1m{}\x1b[0m", card.question).map_err(failed)?;
        write!(output, "\x1b[2mEnter shows the answer, q stops: \x1b[0m").map_err(failed)?;
        output.flush().map_err(failed)?;
        if read(input)?.is_none_or(|l| l == "q") {
            outcome.left = queue.len() + 1;
            break;
        }
        writeln!(output, "\n{}\n", card.answer).map_err(failed)?;
        writeln!(output, "\x1b[2m{}:{}\x1b[0m", card.path, card.line + 1).map_err(failed)?;
        let rating = loop {
            write!(output, "1 Again  2 Hard  3 Good  4 Easy  q stop: ").map_err(failed)?;
            output.flush().map_err(failed)?;
            match read(input)? {
                None => break None,
                Some(text) if text == "q" => break None,
                Some(text) => {
                    if let Some(rating) = Rating::parse(&text) {
                        break Some(rating);
                    }
                }
            }
        };
        let Some(rating) = rating else {
            outcome.left = queue.len() + 1;
            break;
        };
        if !again {
            let review = state.rate(&card.id, rating, today);
            state.save(file).map_err(|e| format!("{}: {e}", file.display()))?;
            outcome.rated += 1;
            writeln!(output, "\x1b[2mnext on {}\x1b[0m", review.due).map_err(failed)?;
        }
        if rating == Rating::Again {
            outcome.again += usize::from(!again);
            queue.push_back((card, true));
        }
    }
    Ok(outcome)
}

/// A line of [input], trimmed and lowercased; `None` at its end.
fn read(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    let read = input.read_line(&mut line).map_err(|e| e.to_string())?;
    Ok((read > 0).then(|| line.trim().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use daftar_core::Vault;

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Learning/Interview-Preparation");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("Interview-Preparation.md"), "## Notes in This Category\n\n- [[Go]]\n").unwrap();
        std::fs::write(
            path.join("Go.md"),
            "## Part 1\n\n### Q1. What is a goroutine?\n\nA function run on its own.\n\n### Q2. What is a channel?\n\nA typed pipe.\n",
        )
        .unwrap();
        std::fs::write(root.path().join("Other.md"), "## Elsewhere\n\nText.\n").unwrap();
        root
    }

    #[test]
    fn the_preparation_folder_is_the_deck_unless_another_is_named() {
        let root = vault();
        let index = Vault::index(root.path(), None);
        assert_eq!(notes(&index, &[], |_| false).unwrap(), ["Learning/Interview-Preparation/Go.md"]);
        assert_eq!(notes(&index, &["Other".into()], |_| false).unwrap(), ["Other.md"]);
        assert_eq!(notes(&index, &["Learning/".into(), "Go".into()], |_| false).unwrap(), ["Learning/Interview-Preparation/Go.md"]);
        assert!(notes(&index, &["Nowhere".into()], |_| false).is_err());
    }

    #[test]
    fn a_session_rates_each_card_and_asks_a_forgotten_one_again() {
        let root = vault();
        let index = Vault::index(root.path(), None);
        let deck = Drill::deck(root.path(), &notes(&index, &[], |_| false).unwrap());
        let file = root.path().join("state/drill.json");
        let today = Day::parse("2026-11-08").unwrap();
        let mut state = DrillState::default();
        let due = Drill::due(&deck, &state, today, 10, 100);
        assert_eq!(line(due[0], &state), "Learning/Interview-Preparation/Go.md:3\tnew\tQ1. What is a goroutine?");
        // Q1 forgotten, Q2 good, Q1 again and good: two ratings kept.
        let mut input = "\n1\n\n3\n\n3\n".as_bytes();
        let mut output = Vec::new();
        let outcome = session(&due, &mut state, &file, today, &mut input, &mut output).unwrap();
        assert_eq!(outcome, Outcome { rated: 2, again: 1, left: 0 });
        let shown = String::from_utf8(output).unwrap();
        assert!(shown.contains("Go › Part 1"));
        assert!(shown.contains("A typed pipe."));
        assert!(shown.contains("[again]"));
        let saved = DrillState::load(&file);
        assert_eq!(saved.reviews["Learning/Interview-Preparation/Go.md#Part 1#Q1. What is a goroutine?"].lapses, 1);
        assert_eq!(saved.reviews["Learning/Interview-Preparation/Go.md#Part 1#Q2. What is a channel?"].due.to_string(), "2026-11-09");
        assert!(Drill::due(&deck, &saved, today, 10, 100).is_empty());
        let note = std::fs::read_to_string(root.path().join("Learning/Interview-Preparation/Go.md")).unwrap();
        assert!(note.contains("A typed pipe."));
    }

    #[test]
    fn q_stops_and_keeps_what_was_rated() {
        let root = vault();
        let index = Vault::index(root.path(), None);
        let deck = Drill::deck(root.path(), &notes(&index, &[], |_| false).unwrap());
        let file = root.path().join("drill.json");
        let today = Day::parse("2026-11-08").unwrap();
        let mut state = DrillState::default();
        let due = Drill::due(&deck, &state, today, 10, 100);
        let mut input = "\n4\nq\n".as_bytes();
        let outcome = session(&due, &mut state, &file, today, &mut input, &mut Vec::new()).unwrap();
        assert_eq!(outcome, Outcome { rated: 1, again: 0, left: 1 });
        assert_eq!(DrillState::load(&file).reviews.len(), 1);
    }
}
