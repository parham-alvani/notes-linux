//! A `.canvas` file: Obsidian's board, in the JSON Canvas 1.0 format
//! (jsoncanvas.org). Cards -- text, a file of the vault, a web link, or a
//! group around others -- at a place and a size, and arrows between them.
//!
//! The app only reads a canvas, and reads what it can: a key it does not know
//! is skipped, a node of an unknown type or without an id is left out, an edge
//! to a node that is not there is left out, and a file that is not JSON is an
//! empty board. Nothing in the file can stop the app.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::note_parser::NoteParser;
use crate::vault::VaultIndex;

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasNode {
    pub id: String,
    /// `text`, `file`, `link` or `group`.
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// `"1"` to `"6"`, a preset colour, or `#rrggbb`.
    pub color: Option<String>,
    /// A text card's Markdown.
    pub text: Option<String>,
    /// A file card's vault path, and a heading or block in it (`#Part`).
    pub file: Option<String>,
    pub subpath: Option<String>,
    /// A link card's address.
    pub url: Option<String>,
    /// A group's title, and its background picture.
    pub label: Option<String>,
    pub background: Option<String>,
    pub background_style: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasEdge {
    pub id: String,
    pub from_node: String,
    pub to_node: String,
    /// `top`, `right`, `bottom` or `left`; `None` lets the app choose.
    pub from_side: Option<String>,
    pub to_side: Option<String>,
    /// `none` or `arrow`. An edge has an arrow at its end, not its start,
    /// unless it says otherwise.
    pub from_end: String,
    pub to_end: String,
    pub color: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Canvas {
    pub nodes: Vec<CanvasNode>,
    pub edges: Vec<CanvasEdge>,
}

const TYPES: [&str; 4] = ["text", "file", "link", "group"];
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
const ENDS: [&str; 2] = ["none", "arrow"];
/// Far past any real board; a bigger number is a broken file.
const LARGEST: f64 = 1_000_000.0;

static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$").unwrap());
/// A number too large for a double, as `1e400`: JSON allows it, the parser does not.
static HUGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-?\d+(?:\.\d+)?[eE]\+?\d{3,}").unwrap());

/// A string, or a number written where a string was meant (`"id": 3`).
fn lossy_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => {
            let n = number.as_f64()?;
            if !n.is_finite() {
                return None;
            }
            Some(if n == n.round() && n.abs() < 1e15 { format!("{}", n as i64) } else { n.to_string() })
        }
        _ => None,
    }
}

/// A finite number, or a string that is one (`"x": "120"`).
fn lossy_number(value: Option<&Value>) -> Option<f64> {
    let number = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    number.is_finite().then(|| number.clamp(-LARGEST, LARGEST))
}

impl CanvasNode {
    fn new(value: &Value) -> Option<CanvasNode> {
        let get = |key: &str| value.get(key);
        let id = lossy_string(get("id")).filter(|id| !id.is_empty())?;
        let kind = lossy_string(get("type")).filter(|t| TYPES.contains(&t.as_str()))?;
        // A card is never too small to see or too large to draw.
        let size = |key: &str, default: f64| lossy_number(get(key)).unwrap_or(default).clamp(10.0, LARGEST);
        Some(CanvasNode {
            id,
            kind,
            x: lossy_number(get("x")).unwrap_or(0.0),
            y: lossy_number(get("y")).unwrap_or(0.0),
            width: size("width", 250.0),
            height: size("height", 60.0),
            color: lossy_string(get("color")),
            text: lossy_string(get("text")),
            file: lossy_string(get("file")),
            subpath: lossy_string(get("subpath")),
            url: lossy_string(get("url")),
            label: lossy_string(get("label")),
            background: lossy_string(get("background")),
            background_style: lossy_string(get("backgroundStyle")),
        })
    }
}

impl CanvasEdge {
    fn new(value: &Value) -> Option<CanvasEdge> {
        let get = |key: &str| lossy_string(value.get(key));
        let (from, to) = (get("fromNode")?, get("toNode")?);
        let one_of = |key: &str, allowed: &[&str]| get(key).filter(|v| allowed.contains(&v.as_str()));
        Some(CanvasEdge {
            id: get("id").unwrap_or_else(|| format!("{from}->{to}")),
            from_side: one_of("fromSide", &SIDES),
            to_side: one_of("toSide", &SIDES),
            from_end: one_of("fromEnd", &ENDS).unwrap_or_else(|| "none".into()),
            to_end: one_of("toEnd", &ENDS).unwrap_or_else(|| "arrow".into()),
            color: get("color"),
            label: get("label"),
            from_node: from,
            to_node: to,
        })
    }
}

impl Canvas {
    /// A canvas from a file's text; an empty one when it is not JSON.
    pub fn parse(text: &str) -> Canvas {
        let file = serde_json::from_str::<Value>(text).or_else(|_| {
            // A number no double holds is no number: the default, as for a
            // value that is not one.
            serde_json::from_str::<Value>(&HUGE.replace_all(text, "null"))
        });
        let Ok(Value::Object(file)) = file else { return Canvas::default() };
        let mut seen = HashSet::new();
        // The first node with an id wins; a second one is a mistake.
        let nodes: Vec<CanvasNode> = file
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(CanvasNode::new)
            .filter(|node| seen.insert(node.id.clone()))
            .collect();
        let edges = file
            .get("edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(CanvasEdge::new)
            .filter(|edge| seen.contains(&edge.from_node) && seen.contains(&edge.to_node))
            .collect();
        Canvas { nodes, edges }
    }

    pub fn is_canvas(path: &str) -> bool {
        path.to_lowercase().ends_with(".canvas")
    }

    /// A node's or an edge's colour as the page draws it: Obsidian's six
    /// presets by name (`1` red, `2` orange, `3` yellow, `4` green, `5` cyan,
    /// `6` purple), each in the palette's own colour, or a `#hex`. `None` for
    /// no colour, or one the app cannot read.
    pub fn color(value: Option<&str>) -> Option<String> {
        let value = value?.trim();
        let preset = match value {
            "1" => "red",
            "2" => "orange",
            "3" => "yellow",
            "4" => "green",
            "5" => "cyan",
            "6" => "purple",
            _ => return HEX.is_match(value).then(|| value.to_string()),
        };
        Some(preset.into())
    }

    /// The vault file each file card shows, by node id: the path as written
    /// when the vault has it, else what a link to it resolves to.
    pub fn files(&self, index: &VaultIndex, from: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for node in self.nodes.iter().filter(|n| n.kind == "file") {
            let Some(file) = node.file.as_deref().filter(|f| !f.is_empty()) else { continue };
            if index.files.contains(file) {
                out.insert(node.id.clone(), file.to_string());
            } else if let Some(found) = index.resolve(file, from) {
                out.insert(node.id.clone(), found);
            }
        }
        out
    }

    /// The links in the text cards, resolved as a note's are.
    pub fn links(&self, index: &VaultIndex, from: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for text in self.nodes.iter().filter(|n| n.kind == "text").filter_map(|n| n.text.as_deref()) {
            for (target, path) in index.resolved_links_in(&NoteParser::parse(text), from) {
                out.entry(target).or_insert(path);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{NoteEntry, Vault};

    // Every fixture here is made up. The repository is public; the vault is not.

    const BOARD: &str = r##"
        {
          "nodes": [
            {"id": "t1", "type": "text", "text": "# Plan\nSee [[Inbox]].", "x": -100, "y": 20, "width": 300, "height": 140, "color": "1"},
            {"id": "f1", "type": "file", "file": "Notes/Inbox.md", "subpath": "#Today", "x": 300, "y": 0, "width": 400, "height": 300},
            {"id": "f2", "type": "file", "file": "map.png", "x": 300, "y": 400, "width": 200, "height": 200, "color": "#00AAFF"},
            {"id": "l1", "type": "link", "url": "https://example.com", "x": 0, "y": 400, "width": 250, "height": 60},
            {"id": "g1", "type": "group", "label": "Week", "x": -150, "y": -60, "width": 900, "height": 700, "background": "uploads/map.png", "backgroundStyle": "cover"},
            {"id": "w1", "type": "widget", "x": 0, "y": 0, "width": 10, "height": 10},
            {"type": "text", "text": "no id"},
            {"id": "t1", "type": "text", "text": "the same id again"},
            {"id": "n1", "type": "text", "x": "12", "y": null, "width": -5, "height": 1e400, "plugin": {"deep": [1, 2]}}
          ],
          "edges": [
            {"id": "e1", "fromNode": "t1", "fromSide": "right", "toNode": "f1", "toSide": "left", "label": "next", "color": "4"},
            {"id": "e2", "fromNode": "f1", "toNode": "f2", "fromEnd": "arrow", "toEnd": "none", "fromSide": "middle"},
            {"id": "e3", "fromNode": "t1", "toNode": "gone"},
            {"id": "e4", "fromNode": "t1"},
            "not an edge"
          ],
          "extra": true
        }
        "##;

    #[test]
    fn reads_every_kind_of_card_and_edge() {
        let canvas = Canvas::parse(BOARD);
        assert_eq!(canvas.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["t1", "f1", "f2", "l1", "g1", "n1"]);
        let text = &canvas.nodes[0];
        assert!(text.kind == "text" && text.text.as_deref() == Some("# Plan\nSee [[Inbox]]."));
        assert!(text.x == -100.0 && text.width == 300.0 && text.color.as_deref() == Some("1"));
        assert!(canvas.nodes[1].file.as_deref() == Some("Notes/Inbox.md") && canvas.nodes[1].subpath.as_deref() == Some("#Today"));
        assert_eq!(canvas.nodes[3].url.as_deref(), Some("https://example.com"));
        assert!(canvas.nodes[4].label.as_deref() == Some("Week") && canvas.nodes[4].background_style.as_deref() == Some("cover"));
        // A number written as text is read; a size too small is the smallest,
        // and one that is not a number is the default.
        let odd = &canvas.nodes[5];
        assert!(odd.x == 12.0 && odd.y == 0.0 && odd.width == 10.0 && odd.height == 60.0);

        assert_eq!(canvas.edges.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["e1", "e2"]);
        let first = &canvas.edges[0];
        assert!(first.from_side.as_deref() == Some("right") && first.to_side.as_deref() == Some("left"));
        assert!(first.label.as_deref() == Some("next") && first.from_end == "none" && first.to_end == "arrow");
        let second = &canvas.edges[1];
        assert!(second.from_side.is_none() && second.from_end == "arrow" && second.to_end == "none");
    }

    #[test]
    fn not_json_is_an_empty_board() {
        for text in ["", "{", "[]", "null", "{\"nodes\": 3}", "{\"nodes\": [1, \"a\", null]}", "not json"] {
            assert_eq!(Canvas::parse(text), Canvas::default());
        }
        assert_eq!(Canvas::parse("{\"nodes\": [{\"id\": \"a\", \"type\": \"text\"}]}").nodes[0].width, 250.0);
    }

    #[test]
    fn colours_are_presets_or_hex() {
        let presets: Vec<Option<String>> = ["1", "2", "3", "4", "5", "6"].iter().map(|c| Canvas::color(Some(c))).collect();
        assert_eq!(presets, ["red", "orange", "yellow", "green", "cyan", "purple"].map(|c| Some(c.to_string())));
        assert_eq!(Canvas::color(Some("#00AAFF")).as_deref(), Some("#00AAFF"));
        assert_eq!(Canvas::color(Some("#abc")).as_deref(), Some("#abc"));
        assert_eq!(Canvas::color(Some("7")), None);
        assert_eq!(Canvas::color(Some("red; x")), None);
        assert_eq!(Canvas::color(None), None);
    }

    #[test]
    fn files_and_links_resolve_in_the_vault() {
        let mut index = VaultIndex::default();
        for path in ["Notes/Inbox.md", "Home.md"] {
            index.notes.insert(
                path.into(),
                NoteEntry { path: path.into(), modified: std::time::SystemTime::now(), parsed: NoteParser::parse("") },
            );
        }
        for file in ["Notes/Inbox.md", "Home.md", "uploads/map.png", "Boards/Plan.canvas"] {
            index.files.insert(file.into());
        }
        index.derive();
        let canvas = Canvas::parse(BOARD);
        let files = canvas.files(&index, "Boards/Plan.canvas");
        assert_eq!(
            files,
            BTreeMap::from([("f1".to_string(), "Notes/Inbox.md".to_string()), ("f2".to_string(), "uploads/map.png".to_string())])
        );
        assert_eq!(canvas.links(&index, "Boards/Plan.canvas"), BTreeMap::from([("Inbox".to_string(), "Notes/Inbox.md".to_string())]));
        assert!(Canvas::is_canvas("Boards/Plan.canvas") && !Canvas::is_canvas("Plan.md"));
        assert!(Vault::opens_in_app("Boards/Plan.canvas"));
    }
}
