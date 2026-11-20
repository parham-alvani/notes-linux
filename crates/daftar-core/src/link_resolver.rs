//! Resolves `[[wikilink]]` targets to vault paths, the way Obsidian does.

use std::collections::HashMap;

use crate::text::{slugs, trim};

/// The same rules as Daftar on Android (`LinkResolver.kt`) and macOS
/// (`LinkResolver.swift`): a target is a suffix of the file's vault path,
/// aligned on `/`, with `.md` optional on the last segment. A folder note
/// `X/X.md` also answers to `X`. Case is folded only when the exact match
/// misses, and an alias is asked last, because Obsidian gives a real name
/// priority over an alias.
#[derive(Debug, Clone, Default)]
pub struct LinkResolver {
    exact: HashMap<String, Vec<String>>,
    folded: HashMap<String, Vec<String>>,
    aliased: HashMap<String, Vec<String>>,
    /// The folders whose `index.md` has `okf_version`: a `/` path in a note
    /// under one starts there (`docs/specs/open-knowledge-format.md`).
    roots: std::collections::BTreeSet<String>,
}

impl LinkResolver {
    pub fn new<'a>(paths: impl IntoIterator<Item = &'a String>, aliases: &HashMap<String, Vec<String>>) -> Self {
        let mut resolver = Self::default();
        for path in paths {
            resolver.register(&slugs::normalize(path));
        }
        for (path, names) in aliases {
            let normalized = slugs::normalize(path);
            for name in names {
                let key = slugs::fold(trim(name));
                if key.is_empty() {
                    continue;
                }
                let list = resolver.aliased.entry(key).or_default();
                if !list.contains(&normalized) {
                    list.push(normalized.clone());
                }
            }
        }
        resolver
    }

    fn register(&mut self, path: &str) {
        let segments: Vec<&str> = path.split('/').collect();
        self.index_suffixes(&segments, path);
        let last = segments[segments.len() - 1];
        let name = last.strip_suffix(".md").unwrap_or(last);
        if segments.len() >= 2 && segments[segments.len() - 2] == name {
            self.index_suffixes(&segments[..segments.len() - 1], path);
        }
    }

    fn index_suffixes(&mut self, segments: &[&str], path: &str) {
        for from in 0..segments.len() {
            let slice = &segments[from..];
            self.put(slice.join("/"), path);
            let last = slice[slice.len() - 1];
            if let Some(bare) = last.strip_suffix(".md") {
                let mut parts = slice[..slice.len() - 1].to_vec();
                parts.push(bare);
                self.put(parts.join("/"), path);
            }
        }
    }

    fn put(&mut self, key: String, path: &str) {
        let lower = slugs::fold(&key);
        let list = self.exact.entry(key).or_default();
        if !list.iter().any(|p| p == path) {
            list.push(path.to_string());
        }
        let list = self.folded.entry(lower).or_default();
        if !list.iter().any(|p| p == path) {
            list.push(path.to_string());
        }
    }

    /// The vault path [target] refers to, seen from [source], or `None` for
    /// a broken link. With [aliases] false, only a file's name or path answers.
    pub fn resolve(&self, target: &str, source: &str) -> Option<String> {
        self.resolve_with(target, source, true)
    }

    /// The bundle roots of the vault, for the `/` paths of Markdown links.
    pub fn set_roots(&mut self, roots: std::collections::BTreeSet<String>) {
        self.roots = roots;
    }

    pub fn roots(&self) -> &std::collections::BTreeSet<String> {
        &self.roots
    }

    /// The file at exactly [path], or its note.
    fn file(&self, path: &str) -> Option<String> {
        let own = |found: &String| slugs::normalize(found) == path || slugs::normalize(found) == format!("{path}.md");
        self.exact
            .get(path)
            .or_else(|| self.exact.get(&format!("{path}.md")))
            .and_then(|list| list.iter().find(|found| own(found)).cloned())
    }

    pub fn resolve_with(&self, target: &str, source: &str, aliases: bool) -> Option<String> {
        if target.is_empty() {
            return Some(source.to_string());
        }
        // A path as OKF reads it: `/` from the bundle's root, `./` and `../`
        // from the note's folder, with no other way to find it; one that
        // climbs out of the vault names nothing. `name/` is `name/index.md`.
        let written = slugs::normalize(target);
        let written = written.trim_matches([' ', '\t']);
        let folder = written.ends_with('/');
        let at = |path: String| if folder { self.file(&format!("{path}/index.md")) } else { self.file(&path) };
        if written.starts_with('/') || written.starts_with("./") || written.starts_with("../") {
            let inner = if written.trim_matches('/').is_empty() { "/" } else { written.trim_end_matches('/') };
            let found = crate::Okf::path(inner, source, &self.roots).and_then(at);
            // A wikilink to a folder names its folder note, as before.
            if found.is_some() || !folder {
                return found;
            }
        } else if written.contains('/') || written.contains('.') {
            // A path with no `/` first is from the note's folder, when a file is there;
            // else it is found as a wikilink is, so the links the vault has keep working.
            if let Some(found) = crate::Okf::path(written.trim_end_matches('/'), source, &self.roots).and_then(at) {
                return Some(found);
            }
        }
        let normalized = slugs::normalize(target);
        let cleaned = trim(&normalized).trim_end_matches('/');
        if cleaned.is_empty() {
            return Some(source.to_string());
        }
        if (cleaned.starts_with("./") || cleaned.starts_with("../"))
            && let Some(found) = self.relative(cleaned, source)
        {
            return Some(found);
        }
        let folded = slugs::fold(cleaned);
        let candidates = self
            .exact
            .get(cleaned)
            .or_else(|| self.folded.get(&folded))
            .or_else(|| if aliases { self.aliased.get(&folded) } else { None })?;
        Some(if candidates.len() == 1 { candidates[0].clone() } else { disambiguate(candidates, source) })
    }

    fn relative(&self, target: &str, source: &str) -> Option<String> {
        let mut base: Vec<&str> = source.split('/').filter(|s| !s.is_empty()).collect();
        base.pop();
        for segment in target.split('/').filter(|s| !s.is_empty()) {
            match segment {
                "." => {}
                ".." => {
                    base.pop()?;
                }
                other => base.push(other),
            }
        }
        let joined = base.join("/");
        self.exact.get(&joined).or_else(|| self.exact.get(&(joined.clone() + ".md"))).and_then(|list| list.first().cloned())
    }
}

/// The candidate that shares the longest folder prefix with the source, then
/// the shallowest, the shortest, and the first by name, so that the guess is
/// at least stable.
fn disambiguate(candidates: &[String], source: &str) -> String {
    let dirs = |path: &str| -> Vec<String> {
        let mut parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
        parts.pop();
        parts
    };
    let source_dirs = dirs(source);
    let shared = |path: &str| dirs(path).iter().zip(&source_dirs).take_while(|(a, b)| a == b).count();
    let depth = |path: &str| path.matches('/').count();
    candidates
        .iter()
        .min_by(|a, b| {
            shared(b)
                .cmp(&shared(a))
                .then_with(|| depth(a).cmp(&depth(b)))
                .then_with(|| a.chars().count().cmp(&b.chars().count()))
                .then_with(|| a.cmp(b))
        })
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn resolver() -> LinkResolver {
        let paths: Vec<String> =
            ["Learning/Kubernetes.md", "Work/Kubernetes.md", "Projects/Alpha/Alpha.md", "uploads/diagram.png", "Notes/Inbox.md"]
                .map(String::from)
                .to_vec();
        let aliases = HashMap::from([("Notes/Inbox.md".to_string(), vec!["In tray".to_string()])]);
        LinkResolver::new(&paths, &aliases)
    }

    fn resolve(target: &str, source: &str) -> Option<String> {
        resolver().resolve(target, source)
    }

    #[test]
    fn bare_names_and_partial_paths() {
        assert_eq!(resolve("Inbox", "A.md").as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(resolve("Notes/Inbox", "A.md").as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(resolve("Notes/Inbox.md", "A.md").as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(resolve("diagram.png", "A.md").as_deref(), Some("uploads/diagram.png"));
    }

    #[test]
    fn folder_note_answers_to_its_folder() {
        assert_eq!(resolve("Projects/Alpha", "A.md").as_deref(), Some("Projects/Alpha/Alpha.md"));
    }

    #[test]
    fn case_is_folded_only_when_exact_misses() {
        assert_eq!(resolve("inbox", "A.md").as_deref(), Some("Notes/Inbox.md"));
    }

    #[test]
    fn alias_is_asked_last() {
        assert_eq!(resolve("In tray", "A.md").as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(resolver().resolve_with("In tray", "A.md", false), None);
        assert_eq!(resolve("Nowhere", "A.md"), None);
    }

    #[test]
    fn ambiguous_name_prefers_the_sources_folder() {
        assert_eq!(resolve("Kubernetes", "Work/Deploy.md").as_deref(), Some("Work/Kubernetes.md"));
        assert_eq!(resolve("Kubernetes", "Learning/Pods.md").as_deref(), Some("Learning/Kubernetes.md"));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(resolve("../Notes/Inbox", "Projects/Plan.md").as_deref(), Some("Notes/Inbox.md"));
        assert_eq!(resolve("", "A.md").as_deref(), Some("A.md"));
    }
}
