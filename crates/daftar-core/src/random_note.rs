//! Obsidian's "Open random note": one note of the vault, any of them alike,
//! or one in a folder or with a tag. The index holds only the files the app
//! reads (`Vault::counts`); the caller leaves the hidden ones out too.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use crate::tag_rename::TagRename;
use crate::vault::{Vault, VaultIndex};

pub struct RandomNote;

impl RandomNote {
    /// The notes to pick from, by path: in [folder] at any depth when one is
    /// given, with [tag] or a tag under it when one is given, and none that
    /// [hidden] leaves out.
    pub fn candidates(index: &VaultIndex, folder: Option<&str>, tag: Option<&str>, hidden: impl Fn(&str) -> bool) -> Vec<String> {
        let folder = folder.map(|f| f.trim_matches('/')).filter(|f| !f.is_empty()).map(|f| format!("{f}/"));
        let tag = tag.map(str::trim).filter(|t| !t.is_empty() && *t != "#");
        let mut notes: Vec<String> = index
            .notes
            .values()
            .filter(|n| Vault::is_note(&n.path) && !hidden(&n.path))
            .filter(|n| folder.as_deref().is_none_or(|f| n.path.starts_with(f)))
            .filter(|n| tag.is_none_or(|t| TagRename::has(&n.parsed.tags, t)))
            .map(|n| n.path.clone())
            .collect();
        // In one order, so that the same number picks the same note.
        notes.sort();
        notes
    }

    /// One of [candidates]: the one [random] picks given how many there
    /// are (a number under it), or `None` when there is none.
    pub fn pick(candidates: &[String], random: impl FnOnce(usize) -> usize) -> Option<String> {
        if candidates.is_empty() {
            return None;
        }
        candidates.get(random(candidates.len()) % candidates.len()).cloned()
    }

    /// A number under [count] (0 for 0) from the system's randomness: the
    /// random keys the standard library seeds its hash maps with.
    pub fn number(count: usize) -> usize {
        let seed = RandomState::new().hash_one(std::time::SystemTime::now());
        #[expect(clippy::cast_possible_truncation, reason = "the remainder is under count, a usize")]
        let number = (seed % (count.max(1) as u64)) as usize;
        number
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::create_dir_all(path.join("Garden")).unwrap();
        std::fs::create_dir_all(path.join(".hidden")).unwrap();
        std::fs::write(path.join("Garden/Basil.md"), "#herb/green\n").unwrap();
        std::fs::write(path.join("Garden/Rose.md"), "#flower\n").unwrap();
        std::fs::write(path.join("Inbox.md"), "#herb\n").unwrap();
        std::fs::write(path.join("Garden/photo.png"), "").unwrap();
        std::fs::write(path.join(".hidden/Secret.md"), "#herb\n").unwrap();
        root
    }

    #[test]
    fn any_note_may_come_but_hidden_ones_and_other_files() {
        let root = vault();
        let index = Vault::index(root.path(), None);
        let all = RandomNote::candidates(&index, None, None, |_| false);
        assert_eq!(all, ["Garden/Basil.md", "Garden/Rose.md", "Inbox.md"]);
        assert_eq!(RandomNote::candidates(&index, None, None, |p| p == "Inbox.md").len(), 2);
        assert_eq!(RandomNote::pick(&all, |_| 0).as_deref(), Some("Garden/Basil.md"));
        assert_eq!(RandomNote::pick(&all, |count| count - 1).as_deref(), Some("Inbox.md"));
        assert_eq!(RandomNote::pick(&[], |_| 0), None);
    }

    #[test]
    fn a_folder_or_a_tag_narrows_the_pick() {
        let root = vault();
        let index = Vault::index(root.path(), None);
        assert_eq!(RandomNote::candidates(&index, Some("Garden/"), None, |_| false), ["Garden/Basil.md", "Garden/Rose.md"]);
        assert_eq!(RandomNote::candidates(&index, None, Some("#herb"), |_| false), ["Garden/Basil.md", "Inbox.md"]);
        assert_eq!(RandomNote::candidates(&index, Some("Garden"), Some("herb"), |_| false), ["Garden/Basil.md"]);
        assert!(RandomNote::candidates(&index, Some("Gard"), None, |_| false).is_empty());
    }

    #[test]
    fn the_number_is_under_the_count() {
        for count in [1, 2, 7, 1000] {
            assert!(RandomNote::number(count) < count);
        }
        assert_eq!(RandomNote::number(0), 0);
    }
}
