//! The vault's tags as a tree, as Obsidian's tag pane shows them.

use std::collections::{HashMap, HashSet};

use crate::text::{slugs, standard_compare};

/// `#a/b` is under `#a`, and a tag counts the notes of the tags under it too.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagNode {
    /// The last part: `b` of `a/b`.
    pub name: String,
    /// The whole tag, `a/b`, which a search uses.
    pub tag: String,
    /// Notes with the tag or a tag under it.
    pub count: usize,
    pub children: Option<Vec<TagNode>>,
}

#[derive(Default)]
struct Node {
    count: usize,
    spelled: HashMap<String, usize>,
    children: HashMap<String, usize>,
}

/// The boxes of the tree in one arena, so that a note can mark the ones it
/// already counted by their index.
#[derive(Default)]
struct Arena {
    nodes: Vec<Node>,
}

impl Arena {
    fn new() -> Arena {
        Arena { nodes: vec![Node::default()] }
    }

    /// The nodes from the top down to [tag], made as needed, each with the
    /// part of the tag it is for.
    fn path(&mut self, tag: &str) -> Vec<(usize, String)> {
        let mut at = 0;
        let mut out = Vec::new();
        for part in tag.split('/').filter(|p| !p.is_empty()) {
            let key = slugs::fold(part);
            let next = if let Some(&next) = self.nodes[at].children.get(&key) {
                next
            } else {
                self.nodes.push(Node::default());
                let next = self.nodes.len() - 1;
                self.nodes[at].children.insert(key, next);
                next
            };
            at = next;
            out.push((at, part.to_string()));
        }
        out
    }

    fn build(&self, at: usize, prefix: &str) -> Vec<TagNode> {
        let mut nodes: Vec<TagNode> = self.nodes[at]
            .children
            .values()
            .map(|&child| {
                let node = &self.nodes[child];
                // The spelling with more notes; on a tie, the first by code point.
                let name = node
                    .spelled
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                let tag = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                let kids = self.build(child, &tag);
                TagNode { name, tag, count: node.count, children: (!kids.is_empty()).then_some(kids) }
            })
            .collect();
        nodes.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| standard_compare(&a.name, &b.name)));
        nodes
    }
}

impl TagNode {
    /// [counts] is the index's notes per tag. Tags that differ only in case
    /// are one, as in Obsidian; the spelling with more notes wins. A note
    /// under two tags of one branch counts twice here; `tree_of_notes`
    /// counts it once.
    pub fn tree(counts: &HashMap<String, usize>) -> Vec<TagNode> {
        let mut arena = Arena::new();
        for (tag, &count) in counts {
            for (at, part) in arena.path(tag) {
                *arena.nodes[at].spelled.entry(part).or_default() += count;
                arena.nodes[at].count += count;
            }
        }
        arena.build(0, "")
    }

    /// The tree from each note's tags: a note counts once under a tag, however
    /// many of the tags under it it has, as Obsidian counts.
    pub fn tree_of_notes<'a>(notes: impl IntoIterator<Item = &'a [String]>) -> Vec<TagNode> {
        let mut arena = Arena::new();
        for tags in notes {
            let mut counted = HashSet::new();
            for tag in tags {
                for (at, part) in arena.path(tag) {
                    *arena.nodes[at].spelled.entry(part).or_default() += 1;
                    if counted.insert(at) {
                        arena.nodes[at].count += 1;
                    }
                }
            }
        }
        arena.build(0, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn nests_and_counts() {
        let tree =
            TagNode::tree(&counts(&[("work", 3), ("work/meeting", 2), ("work/plan/q4", 1), ("home", 4), ("Work", 1), ("ideas/new", 1)]));
        let tags: Vec<&str> = tree.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["work", "home", "ideas"]);
        let work = &tree[0];
        // 3 + 1 of its own (two spellings, one tag), 2 + 1 under it.
        assert_eq!(work.count, 7);
        let kids = work.children.as_ref().unwrap();
        assert_eq!(kids.iter().map(|t| t.tag.as_str()).collect::<Vec<_>>(), ["work/meeting", "work/plan"]);
        assert_eq!(kids[1].children.as_ref().unwrap()[0].tag, "work/plan/q4");
        assert_eq!(tree[1].children, None);
        // A parent with no notes of its own still shows, for its children.
        assert!(tree[2].count == 1 && tree[2].children.as_ref().unwrap()[0].name == "new");
    }

    #[test]
    fn the_commoner_spelling_names_the_tag() {
        assert_eq!(TagNode::tree(&counts(&[("Kubernetes", 5), ("kubernetes", 2)]))[0].name, "Kubernetes");
    }
}
