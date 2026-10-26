//! The notes and the links between them, for the graph view: around one
//! note, or the whole vault. And a force layout to draw them with.

use std::collections::{HashMap, HashSet};

use crate::vault::{ExcludedFiles, Vault, VaultIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub path: String,
    pub title: String,
    /// The top folder, which gives the node its colour.
    pub group: String,
    /// How many links touch it, which gives its size.
    pub degree: usize,
    /// Steps from the centre note; 0 is the centre, and the whole vault has none.
    pub distance: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl NoteGraph {
    /// Every note and the links between notes, without the excluded ones.
    pub fn vault(index: &VaultIndex, excluded: Option<&ExcludedFiles>) -> NoteGraph {
        let mut paths: Vec<String> = index.notes.keys().filter(|p| excluded.is_none_or(|e| !e.contains(p))).cloned().collect();
        paths.sort();
        Self::build(index, &paths, &HashMap::new())
    }

    /// [center] and the notes [depth] links away, in either direction.
    pub fn local(index: &VaultIndex, center: &str, depth: usize) -> NoteGraph {
        let mut distances: HashMap<String, usize> = HashMap::from([(center.to_string(), 0)]);
        let mut frontier = vec![center.to_string()];
        for step in 1..=depth.max(1) {
            let mut next = Vec::new();
            for path in &frontier {
                for neighbour in Self::neighbours(path, index) {
                    if !distances.contains_key(&neighbour) {
                        distances.insert(neighbour.clone(), step);
                        next.push(neighbour);
                    }
                }
            }
            frontier = next;
        }
        let mut paths: Vec<String> = distances.keys().cloned().collect();
        paths.sort_by(|a, b| (distances[a], a).cmp(&(distances[b], b)));
        Self::build(index, &paths, &distances)
    }

    /// Notes this one links to, and notes that link to it.
    fn neighbours(path: &str, index: &VaultIndex) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for link in index.notes.get(path).map(|n| n.parsed.links.as_slice()).unwrap_or(&[]) {
            if let Some(target) = index.resolve(&link.target, path)
                && Vault::is_note(&target)
                && target != path
                && !out.contains(&target)
            {
                out.push(target);
            }
        }
        for back in index.backlinks(path) {
            if !out.contains(&back.source) {
                out.push(back.source.clone());
            }
        }
        out
    }

    fn build(index: &VaultIndex, paths: &[String], distances: &HashMap<String, usize>) -> NoteGraph {
        let mut graph = NoteGraph::default();
        let mut position: HashMap<&str, usize> = HashMap::new();
        for path in paths {
            position.insert(path, graph.nodes.len());
            let group = if path.contains('/') { path.split('/').next().unwrap_or("").to_string() } else { String::new() };
            graph.nodes.push(GraphNode {
                path: path.clone(),
                title: Vault::title(path),
                group,
                degree: 0,
                distance: distances.get(path).copied(),
            });
        }
        let mut seen = HashSet::new();
        for path in paths {
            let from = position[path.as_str()];
            for link in index.notes.get(path).map(|n| n.parsed.links.as_slice()).unwrap_or(&[]) {
                let Some(to) = index.resolve(&link.target, path).and_then(|t| position.get(t.as_str()).copied()) else { continue };
                if to == from {
                    continue;
                }
                let edge = GraphEdge { from: from.min(to), to: from.max(to) };
                if seen.insert(edge) {
                    graph.edges.push(edge);
                }
            }
        }
        for edge in graph.edges.clone() {
            graph.nodes[edge.from].degree += 1;
            graph.nodes[edge.to].degree += 1;
        }
        graph
    }
}

/// A force layout: links pull, all nodes push, and a weak pull keeps the
/// whole near the middle. The push only counts nodes in nearby cells of a
/// grid, so a vault of thousands of notes lays out in a moment.
#[derive(Debug, Clone)]
pub struct GraphLayout {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    edges: Vec<GraphEdge>,
    pinned: Option<usize>,
}

impl GraphLayout {
    /// Nodes start on a spiral, so the same graph lays out the same way.
    pub fn new(graph: &NoteGraph, pin: Option<usize>) -> GraphLayout {
        let count = graph.nodes.len();
        let mut x: Vec<f64> = (0..count).map(|i| (i as f64 * 2.399963).cos() * (i as f64 + 0.5).sqrt() * 30.0).collect();
        let mut y: Vec<f64> = (0..count).map(|i| (i as f64 * 2.399963).sin() * (i as f64 + 0.5).sqrt() * 30.0).collect();
        if let Some(center) = pin.filter(|c| *c < count) {
            x[center] = 0.0;
            y[center] = 0.0;
        }
        GraphLayout { x, y, edges: graph.edges.clone(), pinned: pin }
    }

    pub fn run(&mut self, steps: usize) {
        for step in 0..steps {
            self.step(1.0 - step as f64 / steps as f64);
        }
    }

    pub fn step(&mut self, temperature: f64) {
        let count = self.x.len();
        if count <= 1 {
            return;
        }
        let spacing = 60.0;
        let cell = spacing * 3.0;
        let (mut dx, mut dy) = (vec![0.0; count], vec![0.0; count]);
        let key = |i: usize| ((self.x[i] / cell).floor() as i64, (self.y[i] / cell).floor() as i64);
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for i in 0..count {
            grid.entry(key(i)).or_default().push(i);
        }
        for i in 0..count {
            let (cx, cy) = key(i);
            for ox in -1..=1 {
                for oy in -1..=1 {
                    for &j in grid.get(&(cx + ox, cy + oy)).map(Vec::as_slice).unwrap_or(&[]) {
                        if j == i {
                            continue;
                        }
                        let (mut ddx, mut ddy) = (self.x[i] - self.x[j], self.y[i] - self.y[j]);
                        let mut distance = (ddx * ddx + ddy * ddy).sqrt();
                        if distance < 0.01 {
                            ddx = ((i * 7919 + j) % 13) as f64 - 6.0;
                            ddy = ((i * 104_729 + j) % 11) as f64 - 5.0;
                            distance = 1.0;
                        }
                        let push = spacing * spacing / distance;
                        dx[i] += ddx / distance * push;
                        dy[i] += ddy / distance * push;
                    }
                }
            }
        }
        for edge in &self.edges {
            let (ddx, ddy) = (self.x[edge.to] - self.x[edge.from], self.y[edge.to] - self.y[edge.from]);
            let distance = (ddx * ddx + ddy * ddy).sqrt().max(0.01);
            let pull = distance * distance / (spacing * 2.0);
            dx[edge.from] += ddx / distance * pull;
            dy[edge.from] += ddy / distance * pull;
            dx[edge.to] -= ddx / distance * pull;
            dy[edge.to] -= ddy / distance * pull;
        }
        let limit = (40.0 * temperature).max(2.0);
        for i in 0..count {
            if Some(i) == self.pinned {
                continue;
            }
            // A weak pull to the middle keeps parts that do not link apart, but near.
            dx[i] -= self.x[i] * 0.02;
            dy[i] -= self.y[i] * 0.02;
            let length = (dx[i] * dx[i] + dy[i] * dy[i]).sqrt();
            if length <= 0.0 || !length.is_finite() {
                continue;
            }
            let step = length.min(limit);
            self.x[i] += dx[i] / length * step;
            self.y[i] += dy[i] / length * step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_parser::NoteParser;
    use crate::vault::NoteEntry;
    use std::collections::BTreeSet;
    use std::time::{Instant, SystemTime};

    // Every fixture here is made up. The repository is public; the vault is not.

    fn index_of(notes: &[(String, String)], others: &[&str]) -> VaultIndex {
        let mut index = VaultIndex::default();
        for (path, text) in notes {
            index
                .notes
                .insert(path.clone(), NoteEntry { path: path.clone(), modified: SystemTime::now(), parsed: NoteParser::parse(text) });
        }
        index.files = notes.iter().map(|(p, _)| p.clone()).chain(others.iter().map(|s| s.to_string())).collect::<BTreeSet<_>>();
        index.derive();
        index
    }

    fn make_index() -> VaultIndex {
        let notes = [
            ("Home.md", "[[Alpha]] and [[Beta]]"),
            ("Projects/Alpha.md", "[[Plan]] and ![[diagram.png]]"),
            ("Projects/Plan.md", "Plan, back to [[Alpha]]."),
            ("Work/Beta.md", "Beta."),
            ("Work/Far.md", "[[Plan]]"),
            ("Lonely.md", "Nothing."),
        ];
        index_of(&notes.map(|(p, t)| (p.to_string(), t.to_string())), &["uploads/diagram.png"])
    }

    #[test]
    fn the_whole_vault() {
        let graph = NoteGraph::vault(&make_index(), None);
        assert_eq!(graph.nodes.len(), 6);
        // Attachments are not nodes; a link both ways is one edge.
        assert_eq!(graph.edges.len(), 4);
        let alpha = graph.nodes.iter().find(|n| n.path == "Projects/Alpha.md").unwrap();
        assert_eq!(alpha.degree, 2);
        assert_eq!(alpha.group, "Projects");
        assert_eq!(graph.nodes.iter().find(|n| n.path == "Lonely.md").unwrap().degree, 0);
    }

    #[test]
    fn around_one_note() {
        let index = make_index();
        let one = NoteGraph::local(&index, "Projects/Alpha.md", 1);
        assert_eq!(one.nodes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), ["Projects/Alpha.md", "Home.md", "Projects/Plan.md"]);
        assert_eq!(one.nodes.iter().map(|n| n.distance).collect::<Vec<_>>(), [Some(0), Some(1), Some(1)]);
        let two = NoteGraph::local(&index, "Projects/Alpha.md", 2);
        let paths: HashSet<&str> = two.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, HashSet::from(["Projects/Alpha.md", "Home.md", "Projects/Plan.md", "Work/Beta.md", "Work/Far.md"]));
    }

    #[test]
    fn linked_notes_end_up_closer() {
        let graph = NoteGraph::vault(&make_index(), None);
        let mut layout = GraphLayout::new(&graph, None);
        layout.run(300);
        assert!(layout.x.iter().chain(&layout.y).all(|v| v.is_finite()));
        let at = |path: &str| graph.nodes.iter().position(|n| n.path == path).unwrap();
        let distance = |a: usize, b: usize| (layout.x[a] - layout.x[b]).hypot(layout.y[a] - layout.y[b]);
        assert!(distance(at("Projects/Alpha.md"), at("Projects/Plan.md")) < distance(at("Projects/Alpha.md"), at("Lonely.md")));
        // Nothing sits on top of anything else.
        for i in 0..graph.nodes.len() {
            for j in i + 1..graph.nodes.len() {
                assert!(distance(i, j) > 10.0);
            }
        }
    }

    #[test]
    fn a_thousand_notes_lay_out_quickly() {
        let notes: Vec<(String, String)> =
            (0..1000).map(|n| (format!("Notes/N{n}.md"), format!("[[N{}]] [[N{}]]", (n * 7 + 1) % 1000, n / 2))).collect();
        let graph = NoteGraph::vault(&index_of(&notes, &[]), None);
        let mut layout = GraphLayout::new(&graph, None);
        let start = Instant::now();
        layout.run(200);
        // A guard against a layout that never ends, not a speed test.
        assert!(start.elapsed().as_secs() < 30);
        assert!(layout.x.iter().all(|v| v.is_finite()));
    }
}
