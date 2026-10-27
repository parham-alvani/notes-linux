//! New, renamed, moved and deleted files, and the links that follow a file
//! when it moves: every `[[link]]` and `[text](link)` that went to the old
//! path goes to the new one, as Obsidian's "always update links" does.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use rayon::prelude::*;
use regex::Regex;

use crate::link_resolver::LinkResolver;
use crate::note_parser::Wikilink;
use crate::text::slugs;
use crate::vault::{Vault, VaultIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveFailure {
    BadName(String),
    Exists(String),
    IntoItself,
    /// The disk refused: the file is gone, or it cannot be moved or written.
    Io(String),
}

impl fmt::Display for MoveFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveFailure::BadName(name) => write!(f, "“{name}” cannot be a file name: it is empty or has one of / \\ : # ^ [ ] |"),
            MoveFailure::Exists(path) => write!(f, "“{path}” is already in the vault"),
            MoveFailure::IntoItself => write!(f, "A folder cannot go into itself"),
            MoveFailure::Io(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for MoveFailure {}

impl From<std::io::Error> for MoveFailure {
    fn from(error: std::io::Error) -> MoveFailure {
        MoveFailure::Io(error.to_string())
    }
}

static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\]\n]+?)\]\]").unwrap());
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?\[[^\]\n]*\]\()(<?)([^)>\s]+)(>?\))").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]*`").unwrap());
static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:>\s*)*(```+|~~~+)").unwrap());

/// A path in a markdown link: what a URL path may hold, but `(` and `)`
/// encoded too, or the link ends early; `#` kept for the heading.
const URL_ENCODED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b':')
    .remove(b'@')
    .remove(b'/')
    .remove(b'#');

pub struct VaultMoves;

impl VaultMoves {
    /// A name Obsidian can link to: no path separators, no link syntax.
    pub fn is_valid_name(name: &str) -> bool {
        let trimmed = name.trim();
        !trimmed.is_empty() && !trimmed.starts_with('.') && !trimmed.contains(['/', '\\', ':', '#', '^', '[', ']', '|'])
    }

    /// The name with `.md` added when it names no other kind of file.
    pub fn note_name(name: &str) -> String {
        let trimmed = name.trim();
        let extension = trimmed.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        if ["md", "base", "canvas"].contains(&extension.as_str()) { trimmed.into() } else { format!("{trimmed}.md") }
    }

    /// `Untitled.md`, else `Untitled 1.md`, and so on: a name in [folder]
    /// that no file has yet. An empty [extension] names a folder.
    pub fn free_name(base: &str, extension: &str, folder: &str, files: &BTreeSet<String>) -> String {
        let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
        let taken: BTreeSet<String> = files.iter().map(|f| slugs::fold(f)).collect();
        let folders: BTreeSet<String> = files
            .iter()
            .flat_map(|path| {
                let parts: Vec<&str> = path.split('/').collect();
                (1..parts.len()).map(move |n| slugs::fold(&parts[..n].join("/")))
            })
            .collect();
        (0..)
            .map(|number| {
                let name = if number == 0 { base.to_string() } else { format!("{base} {number}") };
                if extension.is_empty() { format!("{prefix}{name}") } else { format!("{prefix}{name}.{extension}") }
            })
            .find(|path| {
                let folded = slugs::fold(path);
                !taken.contains(&folded) && !folders.contains(&folded)
            })
            .unwrap()
    }

    /// Where each file goes when [from] -- a file or a folder -- becomes
    /// [to]. A folder note `X/X.md` takes the folder's new name, as the
    /// folder-notes plugin does.
    pub fn plan(from: &str, to: &str, files: &BTreeSet<String>) -> Result<BTreeMap<String, String>, MoveFailure> {
        if from == to {
            return Ok(BTreeMap::new());
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(MoveFailure::IntoItself);
        }
        let folded_to = slugs::fold(to);
        let case_only = slugs::fold(from) == folded_to;
        if !case_only
            && files.iter().any(|f| {
                let folded = slugs::fold(f);
                folded == folded_to || folded.starts_with(&format!("{folded_to}/"))
            })
        {
            return Err(MoveFailure::Exists(to.into()));
        }
        if files.contains(from) {
            return Ok(BTreeMap::from([(from.to_string(), to.to_string())]));
        }
        let (old_name, new_name) = (last(from), last(to));
        let folder_note = format!("{from}/{old_name}.md");
        let mut moves = BTreeMap::new();
        for path in files.iter().filter(|p| p.starts_with(&format!("{from}/"))) {
            let moved = if *path == folder_note { format!("{to}/{new_name}.md") } else { format!("{to}{}", &path[from.len()..]) };
            moves.insert(path.clone(), moved);
        }
        // Two files to one place: `X/Y.md` beside the folder note of `X`, renamed `Y`.
        let mut taken: HashMap<String, &str> = HashMap::new();
        for (path, moved) in &moves {
            let folded = slugs::fold(moved);
            if let Some(other) = taken.get(&folded)
                && slugs::fold(other) != slugs::fold(path)
            {
                return Err(MoveFailure::Exists(moved.clone()));
            }
            taken.insert(folded, path);
        }
        Ok(moves)
    }

    /// The notes whose links change with [moves], each with its new text,
    /// by the path it has after the moves.
    pub fn relinked(
        index: &VaultIndex,
        moves: &BTreeMap<String, String>,
        read: impl Fn(&str) -> Option<String> + Sync,
    ) -> BTreeMap<String, String> {
        if moves.is_empty() {
            return BTreeMap::new();
        }
        let mut after = index.files.clone();
        for (old, new) in moves {
            after.remove(old);
            after.insert(new.clone());
        }
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        for note in index.notes.values().filter(|n| !n.parsed.aliases.is_empty()) {
            aliases.insert(moves.get(&note.path).unwrap_or(&note.path).clone(), note.parsed.aliases.clone());
        }
        let resolver = LinkResolver::new(after.iter(), &aliases);
        let mut names: HashMap<String, usize> = HashMap::new();
        for path in &after {
            *names.entry(slugs::fold(&short_name(path))).or_default() += 1;
        }
        let unique = |name: &str| names.get(&slugs::fold(name)) == Some(&1);
        let mut sources: BTreeSet<String> = BTreeSet::new();
        for old in moves.keys() {
            sources.extend(index.backlinks(old).iter().map(|b| b.source.clone()));
        }
        // A moved note's links to itself, and its relative links, change too.
        sources.extend(moves.keys().filter(|p| Vault::is_note(p)).cloned());
        // Each note is read and relinked on its own, several at a time: a
        // much-linked note has thousands of notes that name it.
        let sources: Vec<String> = sources.into_iter().collect();
        sources
            .par_iter()
            .filter_map(|source| {
                let text = read(source)?;
                let target = moves.get(source).unwrap_or(source);
                let new = relink(&text, source, target, index, moves, &resolver, &unique);
                (new != text).then(|| (target.clone(), new))
            })
            .collect()
    }

    /// Moves the files on disk, then writes the notes whose links changed.
    /// Returns every vault path that changed, old and new.
    pub fn perform(from: &str, to: &str, index: &VaultIndex, root: &Path) -> Result<BTreeSet<String>, MoveFailure> {
        let moves = Self::plan(from, to, &index.files)?;
        let (source, destination) = (root.join(from), root.join(to));
        let make_parent = || -> Result<(), MoveFailure> {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            Ok(())
        };
        let differs = slugs::fold(from) != slugs::fold(to);
        // A folder with no file in it moves alone: no link can name it.
        if moves.is_empty() {
            if !index.empty_folders.contains(from) {
                return Ok(BTreeSet::new());
            }
            if differs && destination.exists() {
                return Err(MoveFailure::Exists(to.into()));
            }
            make_parent()?;
            move_item(&source, &destination)?;
            return Ok(BTreeSet::from([from.to_string(), to.to_string()]));
        }
        let (old_name, new_name) = (last(from), last(to));
        let folder_note = moves.get(&format!("{from}/{old_name}.md")) == Some(&format!("{to}/{new_name}.md")) && old_name != new_name;
        // What the disk could refuse is asked before the first file moves,
        // so a move does not stop half done.
        if !source.exists() {
            return Err(MoveFailure::Io(format!("{} is not there", source.display())));
        }
        if differs && destination.exists() {
            return Err(MoveFailure::Exists(to.into()));
        }
        if folder_note && slugs::fold(old_name) != slugs::fold(new_name) && source.join(format!("{new_name}.md")).exists() {
            return Err(MoveFailure::Exists(format!("{to}/{new_name}.md")));
        }
        let edits = Self::relinked(index, &moves, |path| fs::read_to_string(root.join(path)).ok());
        make_parent()?;
        move_item(&source, &destination)?;
        // The folder note, renamed with its folder.
        if folder_note {
            move_item(&destination.join(format!("{old_name}.md")), &destination.join(format!("{new_name}.md")))?;
        }
        for (path, text) in &edits {
            let file = root.join(path);
            let temporary = file.with_file_name(format!(".{}.daftar-{}", last(path), std::process::id()));
            fs::write(&temporary, text)?;
            fs::rename(&temporary, &file)?;
        }
        let mut changed: BTreeSet<String> = moves.keys().chain(moves.values()).cloned().collect();
        changed.insert(from.into());
        changed.insert(to.into());
        Ok(changed)
    }
}

fn last(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_parts(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts
}

/// A change of case only goes through a free name, for a disk that folds case.
fn move_item(from: &Path, to: &Path) -> Result<(), MoveFailure> {
    if slugs::fold(&from.to_string_lossy()) == slugs::fold(&to.to_string_lossy()) {
        let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let step = to.with_file_name(format!("{name}.daftar-{}", std::process::id()));
        fs::rename(from, &step)?;
        fs::rename(&step, to)?;
    } else {
        fs::rename(from, to)?;
    }
    Ok(())
}

fn relink(
    text: &str,
    source: &str,
    new_source: &str,
    index: &VaultIndex,
    moves: &BTreeMap<String, String>,
    resolver: &LinkResolver,
    unique: &dyn Fn(&str) -> bool,
) -> String {
    let mut open: Option<String> = None;
    // A note in another folder now: its relative links are written again.
    let rebased = parent_parts(source) != parent_parts(new_source);
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        // The patterns run only on a line with the marks they need.
        if (line.contains("```") || line.contains("~~~"))
            && let Some(caps) = FENCE.captures(line)
        {
            let marker = caps[1].to_string();
            match &open {
                None => open = Some(marker),
                Some(first) if marker.starts_with(&first[..1]) && marker.len() >= first.len() => open = None,
                _ => {}
            }
            out.push(line.into());
            continue;
        }
        if open.is_some() || (!line.contains("[[") && !line.contains("](")) {
            out.push(line.into());
            continue;
        }
        let code: Vec<Range<usize>> = INLINE_CODE.find_iter(line).map(|m| m.range()).collect();
        let in_code = |r: &Range<usize>, code: &[Range<usize>]| code.iter().any(|c| c.start < r.end && r.start < c.end);
        let mut result = WIKILINK
            .replace_all(line, |caps: &regex::Captures| {
                let all = caps.get(0).unwrap();
                if in_code(&all.range(), &code) {
                    return all.as_str().to_string();
                }
                let link = Wikilink::new(&caps[2]);
                let Some(old) = (!link.target.is_empty()).then(|| index.resolver().resolve_with(&link.target, source, false)).flatten()
                else {
                    return all.as_str().to_string();
                };
                let Some(new) = moves.get(&old).cloned().or_else(|| (is_relative(&link.target) && rebased).then(|| old.clone())) else {
                    return all.as_str().to_string();
                };
                let mut inner = if is_relative(&link.target) {
                    let mut inner = relative(&new, new_source);
                    if Vault::is_note(&new) && !link.target.to_lowercase().ends_with(".md") {
                        inner.truncate(inner.len() - 3);
                    }
                    inner
                } else {
                    spelling(&link.target, &new, new_source, resolver, unique)
                };
                if let Some(anchor) = &link.anchor {
                    inner.push('#');
                    inner.push_str(anchor);
                }
                if let Some(alias) = &link.alias {
                    inner.push_str(if link.escaped { "\\|" } else { "|" });
                    inner.push_str(alias);
                }
                format!("{}[[{inner}]]", &caps[1])
            })
            .into_owned();
        if result.contains("](") {
            let code: Vec<Range<usize>> = INLINE_CODE.find_iter(&result).map(|m| m.range()).collect();
            result = MARKDOWN_LINK
                .replace_all(&result, |caps: &regex::Captures| {
                    let all = caps.get(0).unwrap();
                    let keep = all.as_str().to_string();
                    if in_code(&all.range(), &code) {
                        return keep;
                    }
                    let url = &caps[3];
                    if url.contains("://") || url.starts_with("mailto:") || url.starts_with('#') {
                        return keep;
                    }
                    let decoded = percent_decode_str(url).decode_utf8().map(|d| d.into_owned()).unwrap_or_else(|_| url.to_string());
                    let (target, anchor) = match decoded.find('#') {
                        Some(hash) => (&decoded[..hash], &decoded[hash..]),
                        None => (decoded.as_str(), ""),
                    };
                    let Some(old) = index.resolver().resolve_with(target, source, false) else { return keep };
                    let Some(moved) = moves.get(&old).cloned().or_else(|| (is_relative(target) && rebased).then(|| old.clone())) else {
                        return keep;
                    };
                    let new = if is_relative(target) { relative(&moved, new_source) } else { moved };
                    let written = if caps[2].is_empty() {
                        utf8_percent_encode(&format!("{new}{anchor}"), URL_ENCODED).to_string()
                    } else {
                        format!("{new}{anchor}")
                    };
                    format!("{}{}{written}{}", &caps[1], &caps[2], &caps[4])
                })
                .into_owned();
        }
        out.push(result);
    }
    out.join("\n")
}

fn is_relative(target: &str) -> bool {
    target.starts_with("./") || target.starts_with("../")
}

/// [path] as a link from [source] that starts `./` or `../`.
fn relative(path: &str, source: &str) -> String {
    let dirs = parent_parts(source);
    let parts: Vec<&str> = path.split('/').collect();
    let shared = dirs.iter().zip(&parts[..parts.len() - 1]).take_while(|(a, b)| a == b).count();
    let mut out: Vec<&str> = vec![".."; dirs.len() - shared];
    if out.is_empty() {
        out.push(".");
    }
    out.extend(&parts[shared..]);
    out.join("/")
}

/// A note's name without `.md`, any other file's name with its extension.
fn short_name(path: &str) -> String {
    let name = last(path);
    if Vault::is_note(name) { name[..name.len() - 3].into() } else { name.into() }
}

/// How a link names [path] after a move, in the style it was written: a
/// bare name stays bare while no other file has it, a path stays a path.
fn spelling(written: &str, path: &str, source: &str, resolver: &LinkResolver, unique: &dyn Fn(&str) -> bool) -> String {
    let note = Vault::is_note(path);
    let full = if note { &path[..path.len() - 3] } else { path };
    let name = short_name(path);
    let bare = !written.contains('/') && unique(&name) && resolver.resolve(&name, source).as_deref() == Some(path);
    let mut spelled = if bare { name } else { full.to_string() };
    if note && written.to_lowercase().ends_with(".md") {
        spelled.push_str(".md");
    }
    spelled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_parser::NoteParser;

    // Every fixture here is made up. The repository is public; the vault is not.

    const HOME: &str = "[[Alpha]], [[Alpha#Plan|the plan]], ![[Alpha]], [[Projects/Alpha]] and [[Alpha.md]].
A [markdown link](Projects/Alpha/Alpha.md) and `[[Alpha]]` in code.
```
[[Alpha]]
```
![[diagram.png|300]] and [[Other]].";

    fn files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("Home.md", HOME),
            ("Projects/Alpha/Alpha.md", "The folder note. See [[Plan]]."),
            ("Projects/Alpha/Plan.md", "Plan for [[Alpha]], from [[Home]]."),
            ("Work/Beta.md", "Beta, see [[Plan]]."),
            ("Other.md", "Nothing to change."),
            ("uploads/diagram.png", "png"),
        ]
    }

    fn make_vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        root
    }

    fn read(path: &str, root: &tempfile::TempDir) -> String {
        fs::read_to_string(root.path().join(path)).unwrap()
    }

    fn perform(from: &str, to: &str, root: &tempfile::TempDir) -> BTreeSet<String> {
        VaultMoves::perform(from, to, &Vault::index(root.path(), None), root.path()).unwrap()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn names() {
        assert!(VaultMoves::is_valid_name("Meeting notes"));
        assert!(!VaultMoves::is_valid_name("a/b"));
        assert!(!VaultMoves::is_valid_name("x#y"));
        assert!(!VaultMoves::is_valid_name("  "));
        assert!(!VaultMoves::is_valid_name(".hidden"));
        assert_eq!(VaultMoves::note_name("Plan"), "Plan.md");
        assert_eq!(VaultMoves::note_name("Shops.base"), "Shops.base");
        let taken = set(&["Notes/Untitled.md", "Notes/Untitled 1.md", "Notes/New/x.md"]);
        assert_eq!(VaultMoves::free_name("Untitled", "md", "Notes", &taken), "Notes/Untitled 2.md");
        assert_eq!(VaultMoves::free_name("New", "", "Notes", &taken), "Notes/New 1");
        assert_eq!(VaultMoves::free_name("Untitled", "md", "", &taken), "Untitled.md");
    }

    #[test]
    fn plans() {
        let paths: BTreeSet<String> = files().iter().map(|(p, _)| p.to_string()).collect();
        assert_eq!(VaultMoves::plan("Other.md", "Archive/Other.md", &paths), Ok(map(&[("Other.md", "Archive/Other.md")])));
        // The folder note takes the folder's new name.
        assert_eq!(
            VaultMoves::plan("Projects/Alpha", "Projects/Gamma", &paths),
            Ok(map(&[("Projects/Alpha/Alpha.md", "Projects/Gamma/Gamma.md"), ("Projects/Alpha/Plan.md", "Projects/Gamma/Plan.md")]))
        );
        assert_eq!(VaultMoves::plan("Other.md", "Home.md", &paths), Err(MoveFailure::Exists("Home.md".into())));
        assert_eq!(VaultMoves::plan("Projects", "Projects/Alpha/Projects", &paths), Err(MoveFailure::IntoItself));
        // Only the case changes: not a clash with itself.
        assert_eq!(VaultMoves::plan("Other.md", "other.md", &paths), Ok(map(&[("Other.md", "other.md")])));
    }

    #[test]
    fn renaming_a_note_rewrites_every_link_in_its_style() {
        let root = make_vault(&files());
        let changed = perform("Projects/Alpha", "Projects/Gamma", &root);
        assert!(changed.contains("Projects/Gamma/Gamma.md"));
        assert_eq!(
            read("Home.md", &root),
            "[[Gamma]], [[Gamma#Plan|the plan]], ![[Gamma]], [[Projects/Gamma/Gamma]] and [[Gamma.md]].
A [markdown link](Projects/Gamma/Gamma.md) and `[[Alpha]]` in code.
```
[[Alpha]]
```
![[diagram.png|300]] and [[Other]]."
        );
        assert_eq!(read("Projects/Gamma/Plan.md", &root), "Plan for [[Gamma]], from [[Home]].");
        assert!(root.path().join("Projects/Gamma/Gamma.md").exists());
        assert!(!root.path().join("Projects/Alpha").exists());
        // Untouched notes stay as they were.
        assert_eq!(read("Other.md", &root), "Nothing to change.");
    }

    #[test]
    fn a_name_that_is_no_longer_unique_becomes_a_path() {
        let root = make_vault(&files());
        perform("Other.md", "Work/Plan.md", &root);
        assert!(read("Home.md", &root).ends_with("![[diagram.png|300]] and [[Work/Plan]]."));
    }

    #[test]
    fn moving_an_attachment_follows_its_embeds() {
        let root = make_vault(&files());
        perform("uploads/diagram.png", "uploads/flow.png", &root);
        assert!(read("Home.md", &root).contains("![[flow.png|300]]"));
    }

    #[test]
    fn changing_only_the_case_works() {
        let root = make_vault(&files());
        perform("Other.md", "other.md", &root);
        assert!(Vault::scan(root.path()).contains("other.md"));
        assert!(read("Home.md", &root).ends_with("[[other]]."));
    }

    #[test]
    fn self_links_front_matter_and_tables_follow() {
        let root = make_vault(&[
            ("Parent.md", "# Top\nSee [[Parent#Top]] and [[#Top]]."),
            ("Child.md", "---\nup: \"[[Parent]]\"\n---\n| a | [[Parent\\|p]] |"),
            ("Kin.md", "---\nup: \"[[Parent]]\"\n---\nNo other link."),
        ]);
        perform("Parent.md", "Mother.md", &root);
        assert_eq!(read("Mother.md", &root), "# Top\nSee [[Mother#Top]] and [[#Top]].");
        assert_eq!(read("Child.md", &root), "---\nup: \"[[Mother]]\"\n---\n| a | [[Mother\\|p]] |");
        assert_eq!(read("Kin.md", &root), "---\nup: \"[[Mother]]\"\n---\nNo other link.");
    }

    #[test]
    fn a_moved_note_keeps_its_relative_links() {
        let root =
            make_vault(&[("Other.md", "x"), ("Notes/sib.md", "y"), ("Notes/x.md", "[o](../Other.md), [s](./sib.md) and [[../Other]].")]);
        perform("Notes/x.md", "Deep/More/x.md", &root);
        assert_eq!(read("Deep/More/x.md", &root), "[o](../../Other.md), [s](../../Notes/sib.md) and [[../../Other]].");
    }

    #[test]
    fn an_alias_is_not_rewritten() {
        let root = make_vault(&[("Bar.md", "---\naliases: [Foo]\n---\n"), ("Home.md", "[[Foo]] and [[Bar]]")]);
        perform("Bar.md", "Baz.md", &root);
        assert_eq!(read("Home.md", &root), "[[Foo]] and [[Baz]]");
    }

    #[test]
    fn a_folder_and_its_note_change_case() {
        let root = make_vault(&[("Foo/Foo.md", "folder note"), ("Foo/x.md", "[[Foo]]")]);
        perform("Foo", "foo", &root);
        assert_eq!(Vault::scan(root.path()), set(&["foo/foo.md", "foo/x.md"]));
        assert_eq!(read("foo/x.md", &root), "[[foo]]");
    }

    #[test]
    fn two_notes_cannot_become_the_folder_note() {
        assert_eq!(VaultMoves::plan("X", "Y", &set(&["X/X.md", "X/Y.md"])), Err(MoveFailure::Exists("Y/Y.md".into())));
    }

    #[test]
    fn markdown_links_encode_parentheses() {
        let root = make_vault(&[("Old/a.md", "a"), ("Home.md", "[a](Old/a.md)")]);
        perform("Old", "Notes (old)", &root);
        assert_eq!(read("Home.md", &root), "[a](Notes%20%28old%29/a.md)");
        assert_eq!(NoteParser::parse(&read("Home.md", &root)).links[0].target, "Notes (old)/a.md");
    }
}
