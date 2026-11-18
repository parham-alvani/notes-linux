//! `daftar-cli mentions`: the places in the vault where a note's title or
//! one of its aliases is written without a link, as the inspector's
//! Unlinked Mentions lists them for one note (`UnlinkedMentions`), here for
//! every note at once, and with `--fix` the first mention of each note in
//! each other note made a link.
//!
//! A name is left out when it is shorter than `--min-length`, has no
//! letter (a date, a number), or is the title or alias of more than one
//! note: a link would have to guess. The hidden files and the week notes
//! in `Calendar/` (the phone writes them whole) are neither looked in nor
//! looked for, a note is never its own mention, and a note that links to a
//! note already lists none of its mentions. A vault titles notes with
//! common words (Code, Cloud, Order), so two more rules keep the list to
//! names: a one-word name with a capital is not mentioned by the word
//! written all in small letters ("code"), and a name more than
//! `--max-notes` notes write is a word, not a name, and is left out.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use rayon::prelude::*;

use daftar_core::{CalendarWeek, Mention, MentionFinder, UnlinkedMentions, Vault, VaultEdits, VaultIndex};

/// What [find] looks for and where.
pub struct Options {
    /// Only the mentions of this note.
    pub note: Option<String>,
    /// Only the mentions in these notes.
    pub only: Option<HashSet<String>>,
    /// The fewest characters a name has to be looked for.
    pub min_length: usize,
    /// The most notes a name may be written in and still be looked for; 0
    /// is no limit.
    pub max_notes: usize,
    /// Only the mentions of notes no note links to and that link to none.
    pub orphans: bool,
    /// A one-word name written in small letters counts too.
    pub any_case: bool,
    /// Every mention, not only the first of each note in each other note.
    pub every: bool,
}

impl Options {
    /// Names shorter than this are mostly common words.
    pub const MIN_LENGTH: usize = 4;
    /// Names more notes write than this are words; tuned on a vault of
    /// 2,800 notes, where a product or a place is written in 30 at most.
    pub const MAX_NOTES: usize = 30;
}

/// A name written in [path] without a link to [target].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: String,
    pub target: String,
    /// What the link names: the title when it goes to [target] from
    /// [path], else the shortest path that does (`check --fix`'s).
    pub link: String,
    pub mention: Mention,
}

impl Found {
    /// `path:line`, the link a fix makes, and the line around the name.
    pub fn line(&self) -> String {
        format!("{}:{}\t[[{}]]\t{}", self.path, self.mention.line + 1, self.link, self.mention.excerpt.replace('\t', " "))
    }

    /// `{"path", "line", "column", "target", "link", "text", "excerpt"}`,
    /// the line and column from 1, the column in characters.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "line": self.mention.line + 1,
            "column": self.mention.column + 1,
            "target": self.target,
            "link": self.link,
            "text": self.mention.text,
            "excerpt": self.mention.excerpt,
        })
    }
}

/// Whether [path] is a note this command reads or links to.
fn counted(path: &str, hidden: &impl Fn(&str) -> bool) -> bool {
    Vault::is_note(path) && !hidden(path) && !path.starts_with(&format!("{}/", CalendarWeek::FOLDER))
}

/// The unlinked mentions of the vault, by path, line and column.
pub fn find(root: &Path, index: &VaultIndex, hidden: impl Fn(&str) -> bool + Sync, options: &Options) -> Vec<Found> {
    // Every note's names count toward a name being ambiguous, hidden ones too.
    let mut owners: BTreeMap<String, (String, BTreeSet<&str>)> = BTreeMap::new();
    for (path, note) in &index.notes {
        for name in std::iter::once(Vault::title(path)).chain(note.parsed.aliases.iter().cloned()) {
            if name.chars().count() >= options.min_length && name.chars().any(char::is_alphabetic) {
                owners.entry(UnlinkedMentions::folded(&name)).or_insert_with(|| (name, BTreeSet::new())).1.insert(path);
            }
        }
    }
    let wanted = |path: &str| {
        counted(path, &hidden)
            && options.note.as_deref().is_none_or(|note| note == path)
            && (!options.orphans || crate::check::is_orphan(index, path, &hidden))
    };
    let mut names = Vec::new();
    let mut targets: Vec<(&str, bool)> = Vec::new();
    for (folded, (name, paths)) in owners {
        let mut paths = paths.into_iter();
        if let (Some(path), None) = (paths.next(), paths.next())
            && wanted(path)
        {
            // A word with a capital, as a product or a person is written.
            let proper = !options.any_case && !name.contains(char::is_whitespace) && name.chars().any(char::is_uppercase);
            names.push(folded);
            targets.push((path, proper));
        }
    }
    if names.is_empty() {
        return vec![];
    }
    let finder = MentionFinder::new(&names);
    let mut sources: Vec<&String> = index.notes.keys().filter(|p| counted(p, &hidden)).collect();
    sources.sort();
    let found: Vec<Found> = sources
        .par_iter()
        .flat_map_iter(|path| {
            let text = fs::read_to_string(root.join(path.as_str())).unwrap_or_default();
            let linked: HashSet<String> = index.resolved_links(path).into_values().collect();
            let mut links: HashMap<&str, Option<String>> = HashMap::new();
            let mut out = Vec::new();
            for (name, mention) in finder.find(&text) {
                let (target, proper) = targets[name];
                if target == path.as_str() || linked.contains(target) || (proper && !mention.text.chars().any(char::is_uppercase)) {
                    continue;
                }
                let link = links.entry(target).or_insert_with(|| link_to(index, target, path)).clone();
                if let Some(link) = link {
                    out.push(Found { path: (*path).clone(), target: target.to_string(), link, mention });
                }
            }
            out
        })
        .collect();
    // How many notes write each name, over the whole vault whatever the
    // notes asked about, so a word stays a word.
    let mut writers: HashMap<&str, HashSet<&str>> = HashMap::new();
    for item in &found {
        writers.entry(&item.target).or_default().insert(&item.path);
    }
    let common: HashSet<&str> = writers
        .into_iter()
        .filter(|(_, notes)| options.max_notes > 0 && notes.len() > options.max_notes)
        .map(|(target, _)| target)
        .collect();
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut kept = Vec::new();
    for item in &found {
        if common.contains(item.target.as_str())
            || options.only.as_ref().is_some_and(|only| !only.contains(&item.path))
            || (!options.every && !seen.insert((&item.path, &item.target)))
        {
            continue;
        }
        kept.push(item.clone());
    }
    kept
}

/// What a link from [source] to [target] names: its title when that goes
/// there, else the shortest path that does.
fn link_to(index: &VaultIndex, target: &str, source: &str) -> Option<String> {
    let title = Vault::title(target);
    if index.resolve(&title, source).as_deref() == Some(target) {
        return Some(title);
    }
    crate::check::unique_target(index, target, source)
}

/// Each note's first mention of each target in [found], made a link
/// through `VaultEdits`, which reads the file again: a note whose text is
/// not where it was found any more is left as it is and reported. Prints
/// each link made (or that would be, with [dry_run]); the number made, and
/// the notes refused.
pub fn fix(root: &Path, found: &[Found], dry_run: bool, print: &mut impl FnMut(String)) -> (usize, Vec<String>) {
    let mut firsts: BTreeMap<&str, BTreeMap<&str, &Found>> = BTreeMap::new();
    for item in found {
        let first = firsts.entry(&item.path).or_default().entry(&item.target).or_insert(item);
        if (item.mention.line, item.mention.column) < (first.mention.line, first.mention.column) {
            *first = item;
        }
    }
    let (mut made, mut refused) = (0, Vec::new());
    for (path, by_target) in firsts {
        let mut items: Vec<&Found> = by_target.into_values().collect();
        // From the end, so a link made does not move the next one.
        items.sort_by_key(|f| std::cmp::Reverse((f.mention.line, f.mention.column)));
        let edit = VaultEdits::apply(path, root, |text| {
            let mut changed = text.to_string();
            for item in &items {
                changed = UnlinkedMentions::link(&item.mention, &item.link, &changed)?;
            }
            Ok(if dry_run { text.to_string() } else { changed })
        });
        if let Err(error) = edit {
            eprintln!("daftar-cli: {path}: {error}");
            refused.push(path.to_string());
            continue;
        }
        items.reverse();
        for item in items {
            made += 1;
            let shown = if item.mention.text == item.link { String::new() } else { format!("|{}", item.mention.text) };
            print(format!("{}:{}\tlinked\t{} → [[{}{shown}]]", item.path, item.mention.line + 1, item.mention.text, item.link));
        }
    }
    (made, refused)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn options() -> Options {
        Options { note: None, only: None, min_length: Options::MIN_LENGTH, max_notes: 0, orphans: false, any_case: false, every: true }
    }

    fn lines(dir: &Path, options: &Options) -> Vec<String> {
        let index = Vault::index(dir, None);
        let mut found: Vec<String> = find(dir, &index, |p| p.starts_with("Hidden/"), options).iter().map(Found::line).collect();
        found.sort();
        found
    }

    #[test]
    fn lists_the_names_written_without_a_link() {
        let dir = vault(&[
            ("Companies/Globex.md", "---\naliases: [Globex Corp]\n---\nA made-up company.\n"),
            ("People/Hank.md", "Works at globex corp since spring.\n"),
            ("Notes/Plan.md", "Call Globex, then Hank.\nGlobex again.\n"),
            ("Notes/Linked.md", "See [[Globex]]. Globex is fine here.\n"),
            ("Calendar/2026-W47.md", "## Monday\n\n- 10:00 Globex call\n"),
            ("Hidden/Agent.md", "Globex in a hidden note.\n"),
            ("Notes/Twice.md", "Hank and Hank.\n"),
        ]);
        assert_eq!(
            lines(dir.path(), &options()),
            [
                "Notes/Plan.md:1\t[[Globex]]\tCall Globex, then Hank.",
                "Notes/Plan.md:1\t[[Hank]]\tCall Globex, then Hank.",
                "Notes/Plan.md:2\t[[Globex]]\tGlobex again.",
                "Notes/Twice.md:1\t[[Hank]]\tHank and Hank.",
                "Notes/Twice.md:1\t[[Hank]]\tHank and Hank.",
                "People/Hank.md:1\t[[Globex]]\tWorks at globex corp since spring.",
            ]
        );
        let only_globex = Options { note: Some("Companies/Globex.md".into()), min_length: 5, ..options() };
        assert_eq!(lines(dir.path(), &only_globex).len(), 3);
        let only_in = Options { only: Some(HashSet::from(["People/Hank.md".to_string()])), ..options() };
        assert_eq!(lines(dir.path(), &only_in), ["People/Hank.md:1\t[[Globex]]\tWorks at globex corp since spring."]);
    }

    #[test]
    fn a_short_ambiguous_or_letterless_name_is_left_out() {
        let dir = vault(&[
            ("Work/Plan.md", "Work's plan.\n"),
            ("Home/Plan.md", "Home's plan.\n"),
            ("Journal/2026-11-18.md", "A day.\n"),
            ("Tags/Go.md", "A language.\n"),
            ("Notes/Today.md", "On 2026-11-18 the Plan, in Go.\n"),
        ]);
        assert!(lines(dir.path(), &Options { min_length: 2, ..options() }).is_empty());
    }

    #[test]
    fn a_common_word_is_not_a_name() {
        let dir = vault(&[
            ("Topics/Cloud.md", "x\n"),
            ("Topics/Valkey.md", "y\n"),
            ("A.md", "The cloud is Valkey's; Valkey again.\n"),
            ("B.md", "Cloud bills and cloud costs.\n"),
            ("C.md", "Cloud, again.\n"),
        ]);
        // "cloud" in small letters is the word; written with its capital it is the name.
        assert_eq!(
            lines(dir.path(), &Options { every: false, ..options() }),
            [
                "A.md:1\t[[Valkey]]\tThe cloud is Valkey's; Valkey again.",
                "B.md:1\t[[Cloud]]\tCloud bills and cloud costs.",
                "C.md:1\t[[Cloud]]\tCloud, again."
            ]
        );
        assert_eq!(lines(dir.path(), &Options { any_case: true, ..options() }).len(), 6);
        // Written in more notes than --max-notes, it is a word, even in the notes asked about.
        let few = Options { max_notes: 1, only: Some(HashSet::from(["C.md".to_string()])), ..options() };
        assert!(lines(dir.path(), &few).is_empty());
        assert_eq!(lines(dir.path(), &Options { max_notes: 1, ..options() }).len(), 2);
    }

    #[test]
    fn orphans_restricts_the_notes_looked_for() {
        let dir = vault(&[
            ("Lonely.md", "Nothing links here.\n"),
            ("Known.md", "Linked from Index.\n"),
            ("Index.md", "[[Known]]\n"),
            ("Other.md", "Lonely and Known.\n"),
        ]);
        assert_eq!(lines(dir.path(), &Options { orphans: true, ..options() }), ["Other.md:1\t[[Lonely]]\tLonely and Known."]);
    }

    #[test]
    fn fix_links_the_first_mention_with_the_shortest_path_and_the_text_as_written() {
        let dir = vault(&[
            ("Work/Plan Notes.md", "x\n"),
            ("Home/Plan Notes.md", "y\n"),
            ("Companies/Globex.md", "---\naliases: [Globex Corp]\n---\nz\n"),
            ("Notes/Day.md", "Met globex corp.\nGlobex again, and Globex.\n| Who |\n| --- |\n"),
        ]);
        let index = Vault::index(dir.path(), None);
        let found = find(dir.path(), &index, |_| false, &options());
        let mut printed = Vec::new();
        let (made, refused) = fix(dir.path(), &found, true, &mut |l| printed.push(l));
        assert_eq!((made, refused.len()), (1, 0));
        assert_eq!(printed, ["Notes/Day.md:1\tlinked\tglobex corp → [[Globex|globex corp]]"]);
        // A dry run writes nothing.
        assert!(fs::read_to_string(dir.path().join("Notes/Day.md")).unwrap().starts_with("Met globex corp."));
        fix(dir.path(), &found, false, &mut |_| {});
        assert_eq!(
            fs::read_to_string(dir.path().join("Notes/Day.md")).unwrap(),
            "Met [[Globex|globex corp]].\nGlobex again, and Globex.\n| Who |\n| --- |\n"
        );
    }

    #[test]
    fn a_title_another_note_shares_links_by_its_folder() {
        let dir = vault(&[("Work/Acme.md", "x\n"), ("Home/Acme.md", "y\n"), ("Day.md", "Acme today.\n")]);
        let index = Vault::index(dir.path(), None);
        // The name is ambiguous, so nothing is found; the link it would make names the folder.
        assert!(find(dir.path(), &index, |_| false, &options()).is_empty());
        assert_eq!(link_to(&index, "Work/Acme.md", "Day.md").as_deref(), Some("Work/Acme"));
    }

    #[test]
    fn fix_refuses_a_note_that_changed_since() {
        let dir = vault(&[("Globex.md", "z\n"), ("Day.md", "Lunch.\nGlobex called.\n"), ("Other.md", "Globex wrote.\n")]);
        let index = Vault::index(dir.path(), None);
        let found = find(dir.path(), &index, |_| false, &options());
        fs::write(dir.path().join("Day.md"), "Globex called.\n").unwrap();
        let (made, refused) = fix(dir.path(), &found, false, &mut |_| {});
        assert_eq!((made, refused), (1, vec!["Day.md".to_string()]));
        assert_eq!(fs::read_to_string(dir.path().join("Day.md")).unwrap(), "Globex called.\n");
        assert_eq!(fs::read_to_string(dir.path().join("Other.md")).unwrap(), "[[Globex]] wrote.\n");
    }
}
