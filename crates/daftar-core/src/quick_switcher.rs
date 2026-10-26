//! The quick switcher's ranking, as Obsidian's quick switcher and VS Code's
//! Quick Open work: every word of the query, in any order, matches the
//! file's name, one of its aliases or its folders, fuzzily. A word with a
//! `/` in it matches the whole path, so `comp/acme` and `Companies/` find
//! the files in that folder. The name counts most and the folders least;
//! an exact name, and a name that starts with the query, come first, and a
//! note comes before an attachment that matches as well.

use std::collections::{BTreeSet, HashMap};

use crate::fuzzy::Fuzzy;
use crate::text::slugs;
use crate::vault::Vault;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    /// Folded once, when the index changes, not on each keystroke.
    title: String,
    aliases: Vec<String>,
    folders: String,
    /// The folders and the name, as a word with a `/` is matched.
    full: String,
    note: bool,
}

impl Candidate {
    pub fn new(path: &str, title: &str, aliases: &[String]) -> Candidate {
        let title = slugs::fold(title);
        let folders = slugs::fold(&QuickSwitcher::folder(path));
        let full = if folders.is_empty() { title.clone() } else { format!("{folders}/{title}") };
        Candidate {
            path: path.to_string(),
            title,
            aliases: aliases.iter().map(|a| slugs::fold(a)).collect(),
            folders,
            full,
            note: Vault::opens_in_app(path),
        }
    }
}

pub struct QuickSwitcher;

impl QuickSwitcher {
    /// The folders a path is in, as the switcher shows them: `A/B`.
    pub fn folder(path: &str) -> String {
        path.rsplit_once('/')
            .map(|(folder, _)| folder.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/"))
            .unwrap_or_default()
    }

    pub fn rank(query: &str, candidates: &[Candidate], recent: &[String], limit: usize) -> Vec<String> {
        let needle = slugs::fold(query.trim_matches(' '));
        let words: Vec<&str> = needle.split_whitespace().collect();
        if words.is_empty() {
            return recent.iter().take(limit).cloned().collect();
        }
        let mut recency: HashMap<&str, usize> = HashMap::new();
        for (at, path) in recent.iter().enumerate() {
            recency.entry(path.as_str()).or_insert(at);
        }
        let mut scored: Vec<(&str, i64, i64)> = Vec::with_capacity(64);
        for candidate in candidates {
            let Some(matched) = score(&words, candidate) else { continue };
            let mut total = matched;
            if candidate.title == needle || candidate.aliases.contains(&needle) || candidate.full == needle {
                total += 1000;
            } else if candidate.title.starts_with(&needle) {
                total += 300;
            } else if candidate.title.contains(&needle) {
                total += 120;
            }
            if candidate.note {
                total += 8;
            }
            // A note opened lately wins a close race, not a clear one.
            if let Some(&at) = recency.get(candidate.path.as_str()) {
                total += 1.max(12 - at as i64);
            }
            scored.push((&candidate.path, total, matched));
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.chars().count().cmp(&b.0.chars().count())).then(a.0.cmp(b.0)));
        // Letters spread over a path match nearly anything: once something
        // matches well, what matches under half as well is noise, not a
        // result. The words alone are compared, not the bonus of an exact
        // name, which would leave that one note alone in the list.
        let Some(best) = scored.iter().map(|s| s.2).max() else { return vec![] };
        scored.into_iter().filter(|s| s.2 * 2 >= best).take(limit).map(|s| s.0.to_string()).collect()
    }

    /// The letters of [title] and of [folder] that [query] matched, as
    /// offsets in characters, for the switcher to mark them.
    pub fn highlights(query: &str, title: &str, folder: &str) -> (BTreeSet<usize>, BTreeSet<usize>) {
        let (mut in_title, mut in_folder) = (BTreeSet::new(), BTreeSet::new());
        let whole = if folder.is_empty() { title.to_string() } else { format!("{folder}/{title}") };
        let start = if folder.is_empty() { 0 } else { folder.chars().count() + 1 };
        for word in query.split_whitespace() {
            if word.contains('/') {
                for at in Fuzzy::matches(word, &whole).map(|m| m.positions).unwrap_or_default() {
                    if at >= start {
                        in_title.insert(at - start);
                    } else {
                        in_folder.insert(at);
                    }
                }
            } else if let Some(found) = Fuzzy::matches(word, title) {
                in_title.extend(found.positions);
            } else if let Some(found) = Fuzzy::matches(word, folder) {
                in_folder.extend(found.positions);
            }
        }
        (in_title, in_folder)
    }
}

/// The sum of each word's best match, or `None` when a word matches nowhere.
fn score(words: &[&str], candidate: &Candidate) -> Option<i64> {
    let mut total = 0;
    for word in words {
        let mut best: Option<i64> = None;
        let mut consider = |text: &str, weight: i64| {
            if !may_match(word, text) {
                return;
            }
            let Some(found) = Fuzzy::matches(word, text) else { return };
            let mut value = found.score * 4 + weight;
            // The word as written, and at the start of a word, beats letters spread out.
            if text.starts_with(word) {
                value += 40;
            } else if text.contains(&format!(" {word}")) || text.contains(&format!("-{word}")) || text.contains(&format!("/{word}")) {
                value += 25;
            } else if text.contains(word) {
                value += 12;
            }
            // Letters spread far apart are barely a match.
            if let (Some(first), Some(last)) = (found.positions.first(), found.positions.last())
                && last - first + 1 > word.chars().count() * 3
            {
                value -= 30;
            }
            if best.is_none_or(|b| value > b) {
                best = Some(value);
            }
        };
        if word.contains('/') {
            consider(&candidate.full, 30);
        } else {
            consider(&candidate.title, 60);
            for alias in &candidate.aliases {
                consider(alias, 45);
            }
            consider(&candidate.folders, 0);
        }
        total += best?;
    }
    Some(total)
}

/// Every letter of [word] in [text] in order: the quick test that spares
/// the full match for the many notes that cannot match at all.
fn may_match(word: &str, text: &str) -> bool {
    let mut rest = word.chars().peekable();
    for character in text.chars() {
        if rest.peek() == Some(&character) {
            rest.next();
            if rest.peek().is_none() {
                return true;
            }
        }
    }
    rest.peek().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every fixture here is made up. The repository is public; the vault is not.

    fn notes() -> Vec<Candidate> {
        vec![
            Candidate::new("Learning/Infrastructure/Kubernetes/Kubernetes.md", "Kubernetes", &[]),
            Candidate::new("Learning/Infrastructure/Kubernetes/Firewall rules for Kubernetes.md", "Firewall rules for Kubernetes", &[]),
            Candidate::new("Learning/Infrastructure/Kubernetes/Kubectl.md", "Kubectl", &[]),
            Candidate::new("Companies/Docker/Docker.md", "Docker", &[]),
            Candidate::new("Companies/Docker/Interview.md", "Interview", &[]),
            Candidate::new("Companies/Acme/Interview.md", "Interview", &[]),
            Candidate::new("Notes/Inbox.md", "Inbox", &["In tray".to_string()]),
            Candidate::new("Notes/Bookkeeping.md", "Bookkeeping", &[]),
            Candidate::new("Notes/Kebab recipes.md", "Kebab recipes", &[]),
        ]
    }

    fn rank(query: &str, recent: &[&str]) -> Vec<String> {
        let recent: Vec<String> = recent.iter().map(|s| s.to_string()).collect();
        QuickSwitcher::rank(query, &notes(), &recent, 50)
    }

    fn top(query: &str, recent: &[&str]) -> Vec<String> {
        rank(query, recent).iter().map(|p| Vault::title(p)).collect()
    }

    #[test]
    fn the_exact_name_comes_first() {
        assert_eq!(top("kubernetes", &[])[0], "Kubernetes");
        // The exact name first, and the other notes about it still listed.
        assert!(top("kubernetes", &[]).contains(&"Firewall rules for Kubernetes".to_string()));
        assert_eq!(top("Docker", &[])[0], "Docker");
    }

    #[test]
    fn a_name_that_starts_with_the_query_beats_one_that_holds_it() {
        let found = top("kub", &[]);
        let mut two = found[..2].to_vec();
        two.sort();
        assert_eq!(two, ["Kubectl", "Kubernetes"]);
        assert!(found.iter().position(|t| t == "Firewall rules for Kubernetes").unwrap_or(99) > 1);
    }

    #[test]
    fn words_match_in_any_order() {
        assert_eq!(top("kub fire", &[])[0], "Firewall rules for Kubernetes");
        assert_eq!(top("fire kube", &[])[0], "Firewall rules for Kubernetes");
        assert!(top("kub zzz", &[]).is_empty());
    }

    #[test]
    fn a_folder_name_narrows_notes_with_the_same_name() {
        // Two notes called Interview: the folder word picks one.
        assert_eq!(rank("docker interview", &[])[0], "Companies/Docker/Interview.md");
        assert_eq!(rank("acme interv", &[])[0], "Companies/Acme/Interview.md");
    }

    #[test]
    fn an_alias_finds_its_note() {
        assert_eq!(top("in tray", &[])[0], "Inbox");
        assert!(top("tray", &[]).contains(&"Inbox".to_string()));
    }

    #[test]
    fn letters_that_skip_still_match_but_rank_low() {
        assert_eq!(top("kbrnts", &[])[0], "Kubernetes");
        assert_eq!(top("kebab", &[])[0], "Kebab recipes");
    }

    #[test]
    fn a_recent_note_wins_a_tie_not_a_clear_race() {
        assert_eq!(rank("interview", &["Companies/Acme/Interview.md"])[0], "Companies/Acme/Interview.md");
        // Recent, but a weak match, stays below the exact name.
        assert_eq!(top("kubernetes", &["Learning/Infrastructure/Kubernetes/Firewall rules for Kubernetes.md"])[0], "Kubernetes");
    }

    #[test]
    fn letters_spread_over_a_path_are_not_results() {
        assert_eq!(top("kebab", &[]), ["Kebab recipes"]);
        assert!(!top("docker", &[]).contains(&"Kubernetes".to_string()));
    }

    #[test]
    fn an_empty_query_lists_the_recent_notes() {
        assert_eq!(rank("  ", &["Notes/Inbox.md"]), ["Notes/Inbox.md"]);
    }

    #[test]
    fn a_path_finds_its_file() {
        assert_eq!(rank("Companies/Docker/Interview", &[])[0], "Companies/Docker/Interview.md");
        assert_eq!(rank("acme/interview", &[])[0], "Companies/Acme/Interview.md");
        // Folders shortened as in VS Code, and a path word with a name word.
        assert_eq!(rank("comp/dock int", &[])[0], "Companies/Docker/Interview.md");
        assert_eq!(rank("kube/fire", &[])[0], "Learning/Infrastructure/Kubernetes/Firewall rules for Kubernetes.md");
    }

    #[test]
    fn a_folder_lists_what_is_in_it() {
        let docker: BTreeSet<String> = rank("Companies/Docker/", &[]).into_iter().collect();
        assert_eq!(docker, ["Companies/Docker/Docker.md", "Companies/Docker/Interview.md"].map(String::from).into());
        let notes: BTreeSet<String> = rank("notes/", &[]).into_iter().collect();
        assert_eq!(notes, ["Notes/Inbox.md", "Notes/Bookkeeping.md", "Notes/Kebab recipes.md"].map(String::from).into());
    }

    #[test]
    fn attachments_are_found_after_notes() {
        let mut files = notes();
        files.push(Candidate::new("Companies/Docker/docker.png", "docker.png", &[]));
        assert_eq!(QuickSwitcher::rank("docker", &files, &[], 50)[..2], ["Companies/Docker/Docker.md", "Companies/Docker/docker.png"]);
        assert_eq!(QuickSwitcher::rank("docker png", &files, &[], 50)[0], "Companies/Docker/docker.png");
    }

    #[test]
    fn the_matched_letters_are_marked() {
        let (title, folder) = QuickSwitcher::highlights("dock/int", "Interview", "Companies/Docker");
        assert_eq!(folder, [10, 11, 12, 13, 16].into());
        assert_eq!(title, [0, 1, 2].into());
        let (title, folder) = QuickSwitcher::highlights("acme int", "Interview", "Companies/Acme");
        assert_eq!(title, [0, 1, 2].into());
        assert_eq!(folder, [10, 11, 12, 13].into());
    }
}
