//! `daftar-cli inbox`: the triage of the capture inbox (`Scratchpad.md`,
//! `daftar_core::Inbox`). It lists each item with the notes it likely
//! belongs to: the notes it links to, a note named as one of its tags, then
//! search by meaning fused with the words (the word index alone when the
//! model is not downloaded; it is never downloaded here). On a terminal it
//! walks the items one at a time: 1–3 moves one to a suggestion, `/` names
//! another note, `s` skips, `q` stops. A move adds the item's lines to the
//! other note first and then takes them out of the inbox; each write reads
//! its file again, and the second refuses when the item is not what was
//! listed, which leaves it in both notes, never in neither.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use clap::Subcommand;

use daftar_core::semantic;
use daftar_core::text::slugs;
use daftar_core::{Candidate, ExcludedFiles, Hit, Inbox, InboxItem, NoteParser, QuickSwitcher, SearchIndex, Vault, VaultEdits, VaultIndex};

use crate::meaning::Meaning;

#[derive(Subcommand)]
pub enum Action {
    /// Moves the item on a line of the inbox to a note, as a choice in the
    /// walk does: `daftar-cli inbox move 5 Tasks/Work.md --heading Later`.
    Move {
        /// The item's first line, from 1, as `inbox` prints it.
        line: usize,
        /// The note it goes to; never a week note of `Calendar/`.
        note: String,
        /// The heading to add it under (its text, or its line from 1), at
        /// the end of its section; a name the note lacks is made at its end.
        /// With none, the end of the note.
        #[arg(long)]
        heading: Option<String>,
        /// The item's text, or its first line, as `inbox` printed it: the
        /// move is refused when the line holds another item now.
        #[arg(long)]
        expect: Option<String>,
    },
}

/// A note an item likely belongs to, and why: `link`, `tag`, `name` (the
/// item writes its name or an alias), `meaning` (by meaning and words,
/// fused) or `words`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub path: String,
    pub by: &'static str,
}

/// What suggests notes for the items of one inbox.
pub struct Suggest<'a> {
    index: &'a VaultIndex,
    words: &'a SearchIndex,
    hidden: &'a ExcludedFiles,
    meaning: Option<&'a Meaning>,
    inbox: &'a str,
    /// Each name of four letters or more, as [words] makes it, and its
    /// note; none when two notes have it.
    names: HashMap<String, Option<String>>,
}

impl<'a> Suggest<'a> {
    pub fn new(
        index: &'a VaultIndex,
        words: &'a SearchIndex,
        hidden: &'a ExcludedFiles,
        meaning: Option<&'a Meaning>,
        inbox: &'a str,
    ) -> Suggest<'a> {
        let mut names: HashMap<String, Option<String>> = HashMap::new();
        for (path, note) in &index.notes {
            if hidden.contains(path) || !Inbox::may_receive(path, inbox) {
                continue;
            }
            for name in std::iter::once(Vault::title(path)).chain(note.parsed.aliases.iter().cloned()) {
                let name = spoken(&name).join(" ");
                if name.chars().count() < 4 {
                    continue;
                }
                names.entry(name).and_modify(|p| *p = p.take().filter(|p| p == path)).or_insert_with(|| Some(path.clone()));
            }
        }
        Suggest { index, words, hidden, meaning, inbox, names }
    }

    /// Up to [limit] notes for [item], best first: those it links to, a
    /// note named as one of its tags, a note whose name it writes (the
    /// longest first), then what search finds for its text.
    pub fn of(&self, item: &InboxItem, limit: usize) -> Vec<Suggestion> {
        let mut found: Vec<Suggestion> = Vec::new();
        let put = |found: &mut Vec<Suggestion>, path: String, by| {
            if found.len() < limit
                && Vault::is_note(&path)
                && Inbox::may_receive(&path, self.inbox)
                && !self.hidden.contains(&path)
                && !found.iter().any(|s| s.path == path)
            {
                found.push(Suggestion { path, by });
            }
        };
        let parsed = NoteParser::parse(&item.lines.join("\n"));
        for link in parsed.links.iter().filter(|l| !l.target.is_empty() && !l.target.contains("://")) {
            if let Some(path) = self.index.resolve(&link.target, self.inbox) {
                put(&mut found, path, "link");
            }
        }
        for tag in &parsed.tags {
            let name = tag.rsplit('/').next().unwrap_or(tag);
            if let Some(path) = self.index.resolve(name, self.inbox) {
                put(&mut found, path, "tag");
            }
        }
        let said = spoken(&item.text);
        let mut named: Vec<(usize, usize, &String)> = Vec::new();
        for start in 0..said.len() {
            for length in (1..=4.min(said.len() - start)).rev() {
                if let Some(Some(path)) = self.names.get(&said[start..start + length].join(" ")) {
                    named.push((length, start, path));
                }
            }
        }
        named.sort_by_key(|&(length, start, _)| (std::cmp::Reverse(length), start));
        for (_, _, path) in named {
            put(&mut found, path.clone(), "name");
        }
        let depth = 30;
        let hits: Vec<Hit> =
            self.words.search_any(&item.text, depth + 20).into_iter().filter(|h| !self.hidden.contains(&h.path)).take(depth).collect();
        match self.meaning {
            Some(meaning) => {
                let near = meaning.search(self.hidden, &item.text, depth);
                for fused in semantic::fused(&hits, &near, depth) {
                    put(&mut found, fused.hit.path, "meaning");
                }
            }
            None => {
                for hit in hits {
                    put(&mut found, hit.path, "words");
                }
            }
        }
        found
    }
}

/// [text]'s words, folded, as a name is matched: `Ansible Vault` and
/// `ansible-vault` alike.
fn spoken(text: &str) -> Vec<String> {
    slugs::fold(text).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// The inbox's path in the vault: [path] as the vault spells it, so
/// `scratchpad.md` finds `Scratchpad.md`.
pub fn locate(index: &VaultIndex, path: &str) -> String {
    let folded = path.to_lowercase();
    index.files.iter().find(|f| f.to_lowercase() == folded).cloned().unwrap_or_else(|| path.to_string())
}

/// An item as a line: `path:line`, a tab, its text, then a tab before each
/// suggestion.
pub fn line(inbox: &str, item: &InboxItem, suggestions: &[Suggestion]) -> String {
    let mut line = format!("{inbox}:{}\t{}", item.line + 1, item.text);
    for suggestion in suggestions {
        line.push('\t');
        line.push_str(&suggestion.path);
    }
    line
}

/// An item as `--json` prints it: `{"path", "line" (from 1), "heading",
/// "text", "lines", "suggestions": [{"path", "by"}]}`.
pub fn json(inbox: &str, item: &InboxItem, suggestions: &[Suggestion]) -> serde_json::Value {
    let suggestions: Vec<serde_json::Value> = suggestions.iter().map(|s| serde_json::json!({ "path": s.path, "by": s.by })).collect();
    serde_json::json!({
        "path": inbox,
        "line": item.line + 1,
        "heading": item.heading,
        "text": item.text,
        "lines": item.lines,
        "suggestions": suggestions,
    })
}

/// Where a move put the item.
#[derive(Debug, PartialEq, Eq)]
pub struct Moved {
    pub to: String,
    pub lines: usize,
}

/// Moves the item on [line] (from 0) of [inbox] to [target], at the end of
/// it or under [heading] (a heading's text, made when missing, or its line
/// from 1). [expected] is the item as it was listed, when known.
pub fn move_item(
    root: &Path,
    inbox: &str,
    line: usize,
    expected: Option<&str>,
    target: &str,
    heading: Option<&str>,
) -> Result<Moved, String> {
    if !Inbox::may_receive(target, inbox) {
        return Err(format!("{target} takes no item from the inbox: it is the inbox, or a week note of Calendar/, which are read-only"));
    }
    let text = fs::read_to_string(root.join(inbox)).map_err(|e| format!("{inbox}: {e}"))?;
    let item = Inbox::item_at(&text, line).ok_or_else(|| format!("{inbox}:{} is not the start of an item", line + 1))?;
    if let Some(expected) = expected
        && !Inbox::holds(&item, expected)
    {
        return Err(format!("{inbox}:{} is now “{}”, not “{}”", line + 1, item.text, expected.trim()));
    }
    let heading = heading.map(str::trim).filter(|h| !h.is_empty());
    VaultEdits::apply(target, root, |text| match heading {
        None => Ok(Inbox::added(text, &item.lines, None)),
        Some(heading) => match heading.parse::<usize>() {
            Ok(at) => {
                let at = at.checked_sub(1).filter(|&at| NoteParser::parse(text).headings.iter().any(|h| h.line == at));
                at.map(|at| Inbox::added(text, &item.lines, Some(at))).ok_or(daftar_core::Failure::TextMoved)
            }
            Err(_) => Ok(Inbox::added_under(text, &item.lines, heading)),
        },
    })
    .map_err(|e| match (e, heading) {
        (daftar_core::Failure::TextMoved, Some(heading)) => format!("{target} has no heading on line {heading}; nothing moved"),
        (e, _) => format!("{target}: {e}; nothing moved"),
    })?;
    VaultEdits::apply(inbox, root, |text| Inbox::removed(text, line, &item.lines)).map_err(|e| {
        format!("“{}” was added to {target} but not taken out of {inbox} ({e}): it is in both now, take it out by hand", item.text)
    })?;
    Ok(Moved { to: target.to_string(), lines: item.lines.len() })
}

/// The notes `/` may name: every note but the hidden ones, the week notes
/// and the inbox.
pub fn candidates(index: &VaultIndex, hidden: &ExcludedFiles, inbox: &str) -> Vec<Candidate> {
    index
        .notes
        .keys()
        .filter(|path| !hidden.contains(path) && Inbox::may_receive(path, inbox))
        .map(|path| {
            let aliases = index.notes.get(path).map_or_default(|n| n.parsed.aliases.clone());
            Candidate::new(path, &Vault::title(path), &aliases)
        })
        .collect()
}

/// How a walk went.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub moved: usize,
    pub skipped: usize,
    pub left: usize,
}

/// Walks the items of [inbox] one by one on [input] and [output], each
/// with [suggest]'s notes. The inbox is read again after each move, and
/// the items still to come are found there by their lines.
pub fn session(
    root: &Path,
    inbox: &str,
    candidates: &[Candidate],
    suggest: &dyn Fn(&InboxItem) -> Vec<Suggestion>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Outcome, String> {
    let failed = |e: std::io::Error| e.to_string();
    let text = fs::read_to_string(root.join(inbox)).map_err(|e| format!("{inbox}: {e}"))?;
    let mut queue: VecDeque<InboxItem> = Inbox::items(&text).into();
    let total = queue.len();
    let mut outcome = Outcome::default();
    let mut asked = 0;
    while let Some(item) = queue.pop_front() {
        asked += 1;
        let place = item.heading.as_deref().map_or_else(String::new, |h| format!(" · {h}"));
        writeln!(output, "\n\x1b[2m[{asked}/{total}] {inbox}:{}{place}\x1b[0m", item.line + 1).map_err(failed)?;
        for line in &item.lines {
            writeln!(output, "\x1b[1m{line}\x1b[0m").map_err(failed)?;
        }
        let suggestions = suggest(&item);
        for (at, suggestion) in suggestions.iter().enumerate() {
            writeln!(output, "  {} {} \x1b[2m({})\x1b[0m", at + 1, suggestion.path, suggestion.by).map_err(failed)?;
        }
        let target = loop {
            let numbers = match suggestions.len() {
                0 => String::new(),
                1 => "1 moves it there, ".to_string(),
                n => format!("1-{n} moves it there, "),
            };
            write!(output, "{numbers}/ another note, s skips, q stops: ").map_err(failed)?;
            output.flush().map_err(failed)?;
            let Some(answer) = read(input)? else { break None };
            match answer.to_lowercase().as_str() {
                "q" => break None,
                "s" | "" => break Some(None),
                "/" => {
                    if let Some(picked) = pick(candidates, input, output)? {
                        break Some(Some(picked));
                    }
                }
                choice => {
                    if let Some(s) = choice.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|n| suggestions.get(n)) {
                        break Some(Some((s.path.clone(), None)));
                    }
                }
            }
        };
        let Some(target) = target else {
            outcome.left = queue.len() + 1;
            break;
        };
        let Some((to, heading)) = target else {
            outcome.skipped += 1;
            continue;
        };
        match move_item(root, inbox, item.line, Some(&item.text), &to, heading.as_deref()) {
            Ok(moved) => {
                outcome.moved += 1;
                writeln!(output, "\x1b[2mmoved to {}\x1b[0m", moved.to).map_err(failed)?;
                let text = fs::read_to_string(root.join(inbox)).map_err(|e| format!("{inbox}: {e}"))?;
                let before = queue.len();
                queue = follow(queue, &text);
                if queue.len() < before {
                    writeln!(output, "\x1b[2m{} items changed meanwhile and are left\x1b[0m", before - queue.len()).map_err(failed)?;
                }
            }
            Err(message) => writeln!(output, "\x1b[31m{message}\x1b[0m").map_err(failed)?,
        }
    }
    Ok(outcome)
}

/// [queue]'s items as [text] has them now, in order, found by their
/// lines; an item no longer there is left out.
fn follow(queue: VecDeque<InboxItem>, text: &str) -> VecDeque<InboxItem> {
    let now = Inbox::items(text);
    let mut from = 0;
    let mut found = VecDeque::new();
    for item in queue {
        if let Some(at) = now[from..].iter().position(|n| n.lines == item.lines) {
            found.push_back(now[from + at].clone());
            from += at + 1;
        }
    }
    found
}

/// A note named at a prompt (`Name`, or `Name#Heading`), as the quick
/// switcher ranks it: Enter takes the best, 1–3 another, anything else
/// goes back.
fn pick(candidates: &[Candidate], input: &mut impl BufRead, output: &mut impl Write) -> Result<Option<(String, Option<String>)>, String> {
    let failed = |e: std::io::Error| e.to_string();
    write!(output, "note (Name or Name#Heading): ").map_err(failed)?;
    output.flush().map_err(failed)?;
    let Some(typed) = read(input)? else { return Ok(None) };
    let (name, heading) = match typed.split_once('#') {
        Some((name, heading)) => (name.trim(), Some(heading.trim().to_string()).filter(|h| !h.is_empty())),
        None => (typed.as_str(), None),
    };
    let ranked = QuickSwitcher::rank(name, candidates, &[], 3);
    if ranked.is_empty() {
        writeln!(output, "\x1b[2mno note like “{name}”\x1b[0m").map_err(failed)?;
        return Ok(None);
    }
    for (at, path) in ranked.iter().enumerate() {
        writeln!(output, "  {} {path}", at + 1).map_err(failed)?;
    }
    write!(output, "Enter takes 1, a number another, anything else goes back: ").map_err(failed)?;
    output.flush().map_err(failed)?;
    let Some(answer) = read(input)? else { return Ok(None) };
    let chosen = if answer.is_empty() { Some(0) } else { answer.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) };
    Ok(chosen.and_then(|n| ranked.get(n)).map(|path| (path.clone(), heading)))
}

/// A line of [input], trimmed; `None` at its end.
fn read(input: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    let read = input.read_line(&mut line).map_err(|e| e.to_string())?;
    Ok((read > 0).then(|| line.trim().to_string()))
}

/// The command's flags.
pub struct Options {
    pub json: bool,
    pub list: bool,
    pub interactive: bool,
    pub limit: usize,
}

/// `inbox`: the items listed (`--json`, or not on a terminal) or walked,
/// or `inbox move`.
pub fn run(root: &Path, hidden: &ExcludedFiles, setting: Option<&str>, action: Option<Action>, options: &Options) -> Result<(), String> {
    let Options { json: json_out, list, interactive, limit } = *options;
    let words = SearchIndex::new();
    let index = Vault::index(root, Some(&words));
    let inbox = locate(&index, &Inbox::path(setting));
    if let Some(Action::Move { line, note, heading, expect }) = action {
        let target = crate::note_path(&index, &note)?;
        let at = line.checked_sub(1).ok_or("lines count from 1")?;
        let moved = move_item(root, &inbox, at, expect.as_deref(), &target, heading.as_deref())?;
        out!("{}\t{} lines from {inbox}:{line}", moved.to, moved.lines);
        return Ok(());
    }
    let Ok(text) = fs::read_to_string(root.join(&inbox)) else {
        eprintln!("daftar-cli: there is no {inbox}, so nothing to triage");
        return Ok(());
    };
    let items = Inbox::items(&text);
    if items.is_empty() {
        if json_out {
            out!("[]");
        } else {
            eprintln!("daftar-cli: {inbox} is empty");
        }
        return Ok(());
    }
    // Search by meaning only with the model there; never downloaded here.
    let meaning = if daftar_core::semantic_model::is_downloaded() {
        let mut meaning = Meaning::open(root)?;
        meaning.update(root, &index, true)?;
        Some(meaning)
    } else {
        None
    };
    let suggest = Suggest::new(&index, &words, hidden, meaning.as_ref(), &inbox);
    let walking = !json_out && !list && (interactive || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal()));
    if walking {
        let candidates = candidates(&index, hidden, &inbox);
        let outcome =
            session(root, &inbox, &candidates, &|item| suggest.of(item, limit), &mut std::io::stdin().lock(), &mut std::io::stdout())?;
        eprintln!("\ndaftar-cli: {} moved, {} skipped, {} left", outcome.moved, outcome.skipped, outcome.left);
        return Ok(());
    }
    if json_out {
        let rows: Vec<serde_json::Value> = items.iter().map(|item| json(&inbox, item, &suggest.of(item, limit))).collect();
        out!("{}", serde_json::Value::Array(rows));
        return Ok(());
    }
    for item in &items {
        out!("{}", line(&inbox, item, &suggest.of(item, limit)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INBOX: &str = "## 2026-10-29\n\n- Ask the broker team about consumer lag\n- [ ] Water the basil #garden\n- Call about [[Bank]] money\n    and the card\n\n## 2026-10-30\n\n- Plant tomato seeds\n";

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        };
        write("Scratchpad.md", INBOX);
        write("Tech/Queues.md", "# Queues\n\nThe broker and its consumer lag.\n");
        write("Garden.md", "# Garden\n\n## Plants\n\n- basil\n- tomato seeds\n\n## Later\n");
        write("Bank.md", "# Bank\n\nAccounts and money.\n");
        write("Calendar/2026-W44.md", "# Week 44\n\nbroker consumer lag tomato basil money\n");
        root
    }

    fn suggestions(root: &Path, meaning: Option<&Meaning>) -> Vec<(usize, Vec<Suggestion>)> {
        let words = SearchIndex::new();
        let index = Vault::index(root, Some(&words));
        let hidden = ExcludedFiles::default();
        let suggest = Suggest::new(&index, &words, &hidden, meaning, "Scratchpad.md");
        Inbox::items(INBOX).iter().map(|item| (item.line + 1, suggest.of(item, 3))).collect()
    }

    fn s(path: &str, by: &'static str) -> Suggestion {
        Suggestion { path: path.into(), by }
    }

    #[test]
    fn links_and_tags_come_first_then_the_words() {
        let root = vault();
        let found = suggestions(root.path(), None);
        assert_eq!(found[0], (3, vec![s("Tech/Queues.md", "words")]));
        assert_eq!(found[1], (4, vec![s("Garden.md", "tag")]));
        assert_eq!(found[2], (5, vec![s("Bank.md", "link")]));
        assert_eq!(found[3], (10, vec![s("Garden.md", "words")]));
    }

    #[test]
    fn a_note_named_in_the_item_comes_before_the_words() {
        let root = vault();
        fs::write(root.path().join("Tech/Consumer Lag.md"), "---\naliases: [Lag Alerts]\n---\nWhen to page.\n").unwrap();
        fs::create_dir_all(root.path().join("Old")).unwrap();
        fs::write(root.path().join("Old/Garden.md"), "Another garden.\n").unwrap();
        let words = SearchIndex::new();
        let index = Vault::index(root.path(), Some(&words));
        let hidden = ExcludedFiles::default();
        let suggest = Suggest::new(&index, &words, &hidden, None, "Scratchpad.md");
        let item = |text: &str| Inbox::items(text).remove(0);
        let found = suggest.of(&item("- Ask about consumer-lag and the lag alerts at the queues\n"), 3);
        assert_eq!(found[..2], [s("Tech/Consumer Lag.md", "name"), s("Tech/Queues.md", "name")]);
        // Two notes named Garden: the name says neither.
        let found = suggest.of(&item("- The garden\n"), 3);
        assert!(found.iter().all(|s| s.by != "name"), "{found:?}");
    }

    #[test]
    fn by_meaning_when_the_model_is_there() {
        let root = vault();
        let words = SearchIndex::new();
        let index = Vault::index(root.path(), Some(&words));
        let mut meaning = Meaning::with(crate::meaning::fake::load, &root.path().join(".cache/semantic.sqlite")).unwrap();
        meaning.update(root.path(), &index, false).unwrap();
        let found = suggestions(root.path(), Some(&meaning));
        assert_eq!(found[0].1[0], s("Tech/Queues.md", "meaning"));
        // Never the week notes, nor the inbox, though they match.
        for (_, list) in &found {
            assert!(list.iter().all(|s| !s.path.starts_with("Calendar/") && s.path != "Scratchpad.md"), "{list:?}");
        }
    }

    #[test]
    fn an_item_as_a_line_and_as_json() {
        let item = &Inbox::items(INBOX)[2];
        let found = [s("Bank.md", "link"), s("Garden.md", "words")];
        assert_eq!(line("Scratchpad.md", item, &found), "Scratchpad.md:5\tCall about [[Bank]] money and the card\tBank.md\tGarden.md");
        let row = json("Scratchpad.md", item, &found);
        assert_eq!(row["line"], 5);
        assert_eq!(row["heading"], "2026-10-29");
        assert_eq!(row["lines"][1], "    and the card");
        assert_eq!(row["suggestions"][0]["by"], "link");
    }

    #[test]
    fn a_move_adds_the_lines_then_takes_them_out() {
        let root = vault();
        let moved = move_item(root.path(), "Scratchpad.md", 4, Some("Call about [[Bank]] money and the card"), "Bank.md", None).unwrap();
        assert_eq!(moved, Moved { to: "Bank.md".into(), lines: 2 });
        let bank = fs::read_to_string(root.path().join("Bank.md")).unwrap();
        assert_eq!(bank, "# Bank\n\nAccounts and money.\n\n- Call about [[Bank]] money\n    and the card\n");
        let inbox = fs::read_to_string(root.path().join("Scratchpad.md")).unwrap();
        assert!(!inbox.contains("Bank"), "{inbox}");
        // The last item of a day takes its heading along.
        move_item(root.path(), "Scratchpad.md", 7, None, "Garden.md", Some("Plants")).unwrap();
        let garden = fs::read_to_string(root.path().join("Garden.md")).unwrap();
        assert_eq!(garden, "# Garden\n\n## Plants\n\n- basil\n- tomato seeds\n- Plant tomato seeds\n\n## Later\n");
        let inbox = fs::read_to_string(root.path().join("Scratchpad.md")).unwrap();
        assert_eq!(inbox, "## 2026-10-29\n\n- Ask the broker team about consumer lag\n- [ ] Water the basil #garden\n");
        // Under a heading by its line.
        move_item(root.path(), "Scratchpad.md", 3, None, "Garden.md", Some("9")).unwrap();
        let garden = fs::read_to_string(root.path().join("Garden.md")).unwrap();
        assert!(garden.ends_with("## Later\n\n- [ ] Water the basil #garden\n"), "{garden}");
    }

    #[test]
    fn a_move_is_refused_when_the_item_changed_or_the_note_is_a_week_note() {
        let root = vault();
        let refused = move_item(root.path(), "Scratchpad.md", 2, Some("Plant tomato seeds"), "Garden.md", None).unwrap_err();
        assert!(refused.contains("is now “Ask the broker team about consumer lag”"), "{refused}");
        assert!(move_item(root.path(), "Scratchpad.md", 1, None, "Garden.md", None).unwrap_err().contains("not the start of an item"));
        assert!(move_item(root.path(), "Scratchpad.md", 2, None, "Calendar/2026-W44.md", None).unwrap_err().contains("read-only"));
        assert!(move_item(root.path(), "Scratchpad.md", 2, None, "Garden.md", Some("2")).unwrap_err().contains("no heading on line 2"));
        // Nothing was written.
        assert_eq!(fs::read_to_string(root.path().join("Scratchpad.md")).unwrap(), INBOX);
        assert!(!fs::read_to_string(root.path().join("Garden.md")).unwrap().contains("broker"));
    }

    #[test]
    fn the_walk_moves_skips_names_a_note_and_stops() {
        let root = vault();
        let words = SearchIndex::new();
        let index = Vault::index(root.path(), Some(&words));
        let hidden = ExcludedFiles::default();
        let suggest = Suggest::new(&index, &words, &hidden, None, "Scratchpad.md");
        let candidates = candidates(&index, &hidden, "Scratchpad.md");
        assert!(candidates.iter().all(|c| !c.path.starts_with("Calendar/") && c.path != "Scratchpad.md"));
        // The first to its suggestion, the second skipped, the third to a
        // note named at the prompt, under a heading made there; then stop.
        let mut input = "1\ns\n/\nqueues#From the inbox\n\nq\n".as_bytes();
        let mut output = Vec::new();
        let outcome = session(root.path(), "Scratchpad.md", &candidates, &|item| suggest.of(item, 3), &mut input, &mut output).unwrap();
        assert_eq!(outcome, Outcome { moved: 2, skipped: 1, left: 1 });
        let shown = String::from_utf8(output).unwrap();
        assert!(shown.contains("[1/4] Scratchpad.md:3 · 2026-10-29"), "{shown}");
        assert!(shown.contains("1 Tech/Queues.md"), "{shown}");
        let queues = fs::read_to_string(root.path().join("Tech/Queues.md")).unwrap();
        assert!(
            queues.contains(
                "lag.\n\n- Ask the broker team about consumer lag\n\n## From the inbox\n\n- Call about [[Bank]] money\n    and the card\n"
            ),
            "{queues}"
        );
        let inbox = fs::read_to_string(root.path().join("Scratchpad.md")).unwrap();
        assert_eq!(inbox, "## 2026-10-29\n\n- [ ] Water the basil #garden\n\n## 2026-10-30\n\n- Plant tomato seeds\n");
    }
}
