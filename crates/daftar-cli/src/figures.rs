//! `check --figures`: the text of a hand-built SVG figure that a reader
//! cannot read. usvg lays each `<text>` out with real fonts (the system's,
//! and the ones this binary carries), so every run of text has the box its
//! glyphs' ink covers, in the SVG's own units. A run is reported when it
//! covers another (`svg-text-overlap`), runs off the canvas
//! (`svg-text-outside`), or is set in a box, a pill or a chip and runs out
//! of it (`svg-text-spill`): the smallest one that holds its anchor and the
//! most of it. A figure usvg cannot read is one `svg-parse`. The `<style>`
//! of a figure is read as a browser reads it: its classes give the sizes,
//! weights and families, and a `@media` block (the dark theme) is skipped.

use std::sync::{Arc, LazyLock};

use usvg::tiny_skia_path::Point;
use usvg::{Group, Node, Rect, fontdb};

/// How deep two runs must go into each other to overlap, along every side
/// of either: lines set close, or a descender over the next line's
/// capitals, touch by a pixel or two and read well.
const OVERLAP: f32 = 2.0;
/// How far a run may go past the canvas or its shape: antialiasing and a
/// stroke's half width are about this.
const SPILL: f32 = 1.0;
/// The least fill opacity of a shape with no outline that text is set in:
/// a fainter one is a band shading part of a chart, which a note on the
/// chart may cross.
const TINT: f32 = 0.25;
/// How much of a run's width a box must hold for the run to be set in it.
const HELD: f32 = 0.4;
/// The longest text a finding quotes.
const QUOTE: usize = 40;

/// The fonts a figure is laid out with: the system's, and the ones built
/// in, so a family the computer lacks still has its own widths.
pub(crate) static FONTS: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    let mut fonts = fontdb::Database::new();
    fonts.load_system_fonts();
    for data in crate::export::FONTS {
        fonts.load_font_data(data.to_vec());
    }
    Arc::new(fonts)
});

/// usvg's fallback for a letter the font lacks, with an emoji taken from a
/// color emoji font first, as a browser does. usvg's own takes the first
/// font that has the letter, and when that font has every letter of the
/// run it sets the whole run in it: a label with a 🐼 came out as wide again
/// as the browser draws it.
fn emoji_first() -> usvg::FallbackSelectionFn<'static> {
    let fallback = usvg::FontResolver::default_fallback_selector();
    Box::new(move |c, used, fonts| {
        let emoji = matches!(c as u32, 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF | 0xFE0F | 0x200D);
        // One the letter is missing from is tried, and then left out: usvg
        // asks again without it.
        let colored =
            || fonts.faces().find(|f| !used.contains(&f.id) && f.families.iter().any(|(name, _)| name.contains("Emoji"))).map(|f| f.id);
        if emoji && let Some(id) = colored() { Some(id) } else { fallback(c, used, fonts) }
    })
}

/// A run of text: what it says, its ink and where it is anchored, the
/// `x` and `y` of its first chunk on the canvas, which a finding names.
#[derive(Clone)]
struct Run {
    text: String,
    ink: Rect,
    /// The corners of the ink box in the run's own frame, on the canvas: a
    /// turned label covers far less than its upright bounds.
    corners: [Point; 4],
    anchor: Point,
}

/// What is wrong with the text of the SVG [data], each a kind and what it
/// found, with [fonts].
pub(crate) fn problems(data: &[u8], fonts: &Arc<fontdb::Database>) -> Vec<(&'static str, String)> {
    let options = usvg::Options {
        fontdb: Arc::clone(fonts),
        font_resolver: usvg::FontResolver { select_fallback: emoji_first(), ..usvg::FontResolver::default() },
        ..usvg::Options::default()
    };
    let tree = match usvg::Tree::from_data(data, &options) {
        Ok(tree) => tree,
        Err(error) => return vec![("svg-parse", format!("usvg cannot read it: {error}"))],
    };
    let (mut runs, mut shapes) = (Vec::new(), Vec::new());
    gather(tree.root(), &mut runs, &mut shapes);
    // The same words drawn twice in one place are a halo under the text,
    // not two runs: the one on top is kept.
    let halo = |a: &Run, b: &Run| a.text == b.text && (a.anchor.x - b.anchor.x).abs() < SPILL && (a.anchor.y - b.anchor.y).abs() < SPILL;
    let runs: Vec<Run> = (0..runs.len()).filter(|&i| !runs[i + 1..].iter().any(|b| halo(&runs[i], b))).map(|i| runs[i].clone()).collect();
    let size = tree.size();
    let mut found = Vec::new();
    for (i, a) in runs.iter().enumerate() {
        for b in &runs[i + 1..] {
            if depth(&a.corners, &b.corners) > OVERLAP {
                found.push((
                    "svg-text-overlap",
                    format!("{} at {} overlaps {} at {}", quote(&a.text), spot(a.anchor), quote(&b.text), spot(b.anchor)),
                ));
            }
        }
    }
    for run in &runs {
        let past = [
            ("left", -run.ink.left()),
            ("right", run.ink.right() - size.width()),
            ("top", -run.ink.top()),
            ("bottom", run.ink.bottom() - size.height()),
        ];
        if let Some((side, by)) = past.into_iter().filter(|(_, by)| *by > SPILL).max_by(|a, b| a.1.total_cmp(&b.1)) {
            found.push((
                "svg-text-outside",
                format!("{} at {} runs {by:.0} past the canvas's {side} edge", quote(&run.text), spot(run.anchor)),
            ));
            continue;
        }
        // A box the run is set in holds its anchor, the most of its width
        // and nearly all its height: a legend's swatch beside a label, or a
        // band under a heading, holds the anchor alone.
        let inside = |s: &&Rect| {
            let (across, down) = cover(**s, run.ink);
            s.left() < run.anchor.x
                && run.anchor.x < s.right()
                && s.top() < run.anchor.y
                && run.anchor.y < s.bottom()
                && across >= run.ink.width() * HELD
                && down >= run.ink.height() - SPILL
        };
        let Some(shape) = shapes.iter().filter(inside).min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height()))) else {
            continue;
        };
        let past = [
            ("left", shape.left() - run.ink.left()),
            ("right", run.ink.right() - shape.right()),
            ("top", shape.top() - run.ink.top()),
            ("bottom", run.ink.bottom() - shape.bottom()),
        ];
        if let Some((side, by)) = past.into_iter().filter(|(_, by)| *by > SPILL).max_by(|a, b| a.1.total_cmp(&b.1)) {
            found.push((
                "svg-text-spill",
                format!(
                    "{} at {} runs {by:.0} past the {side} of its shape at {} ({:.0}×{:.0})",
                    quote(&run.text),
                    spot(run.anchor),
                    at(*shape),
                    shape.width(),
                    shape.height()
                ),
            ));
        }
    }
    found
}

/// The runs of text and the filled shapes under [group], in canvas units.
fn gather(group: &Group, runs: &mut Vec<Run>, shapes: &mut Vec<Rect>) {
    for node in group.children() {
        match node {
            Node::Group(group) if group.opacity().get() > 0.0 => gather(group, runs, shapes),
            Node::Text(text) => {
                let said: Vec<&str> = text.chunks().iter().map(|c| c.text().trim()).filter(|t| !t.is_empty()).collect();
                if said.is_empty() || !text.chunks().iter().flat_map(usvg::TextChunk::spans).any(usvg::TextSpan::is_visible) {
                    continue;
                }
                let ink = text.abs_stroke_bounding_box();
                let first = &text.chunks()[0];
                let anchor = match (first.x(), first.y()) {
                    (Some(x), Some(y)) => {
                        let mut point = Point::from_xy(x, y);
                        text.abs_transform().map_point(&mut point);
                        point
                    }
                    _ => Point::from_xy(ink.left() + ink.width() / 2.0, ink.top() + ink.height() / 2.0),
                };
                let own = text.stroke_bounding_box();
                let mut corners = [
                    Point::from_xy(own.left(), own.top()),
                    Point::from_xy(own.right(), own.top()),
                    Point::from_xy(own.right(), own.bottom()),
                    Point::from_xy(own.left(), own.bottom()),
                ];
                text.abs_transform().map_points(&mut corners);
                runs.push(Run { text: said.join(" "), ink, corners, anchor });
            }
            Node::Path(path)
                if path.is_visible()
                    && is_box(path)
                    && path.fill().is_some_and(|f| f.opacity().get() >= TINT || path.stroke().is_some()) =>
            {
                let shape = path.abs_bounding_box();
                if shape.width() > 1.0 && shape.height() > 1.0 {
                    shapes.push(shape);
                }
            }
            _ => {}
        }
    }
}

/// Whether [path] is a box text can sit in: a rectangle, a rounded one, a
/// pill, a circle or a diamond, whose every point (control points too) is
/// on the edge of its bounds. The coast of a map or the area under a curve
/// is not: its bounds hold much that it does not.
fn is_box(path: &usvg::Path) -> bool {
    let bounds = path.bounding_box();
    let on_edge = |p: &Point| {
        let near = |a: f32, b: f32| (a - b).abs() < 0.5;
        near(p.x, bounds.left()) || near(p.x, bounds.right()) || near(p.y, bounds.top()) || near(p.y, bounds.bottom())
    };
    path.data().points().iter().all(on_edge)
}

/// How deep the boxes with corners [a] and [b] go into each other: the
/// least overlap of their shadows on the sides of either (the separating
/// axis test), negative when they are apart. Two upright boxes go as deep
/// as the lesser of how far they cover each other across and down.
fn depth(a: &[Point; 4], b: &[Point; 4]) -> f32 {
    let mut least = f32::INFINITY;
    for corners in [a, b] {
        for i in 0..2 {
            let (from, to) = (corners[i], corners[i + 1]);
            let (x, y) = (to.x - from.x, to.y - from.y);
            let length = x.hypot(y);
            if length < f32::EPSILON {
                continue;
            }
            let normal = (-y / length, x / length);
            let shadow = |c: &[Point; 4]| {
                let along = c.iter().map(|p| p.x * normal.0 + p.y * normal.1);
                along.clone().fold(f32::INFINITY, f32::min)..along.fold(f32::NEG_INFINITY, f32::max)
            };
            let (p, q) = (shadow(a), shadow(b));
            least = least.min(p.end.min(q.end) - p.start.max(q.start));
        }
    }
    least
}

/// How far [a] and [b] cover each other, across and down; negative when
/// they are apart.
fn cover(a: Rect, b: Rect) -> (f32, f32) {
    (a.right().min(b.right()) - a.left().max(b.left()), a.bottom().min(b.bottom()) - a.top().max(b.top()))
}

/// The top left corner of [r], in whole units.
fn at(r: Rect) -> String {
    format!("{:.0},{:.0}", r.left(), r.top())
}

/// [p] in whole units: a run's anchor is its `x` and `y` as written.
fn spot(p: Point) -> String {
    format!("{:.0},{:.0}", p.x, p.y)
}

/// [text] in quotes, cut short with an ellipsis.
fn quote(text: &str) -> String {
    if text.chars().count() > QUOTE {
        format!("“{}…”", text.chars().take(QUOTE - 1).collect::<String>())
    } else {
        format!("“{text}”")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A figure of [width] by 100 with [body], styled as the vault's are,
    /// in a family built into this binary.
    fn figure(width: u32, body: &str) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width} 100\" width=\"100%\"><style>\n\
             svg{{font-family:\"DejaVu Sans Mono\",monospace;font-size:10px}}\n.t{{fill:#555}}.big{{font-size:20px;font-weight:700}}\n\
             @media (prefers-color-scheme:dark){{.t{{fill:#ccc}}.big{{font-size:40px}}}}\n</style>{body}</svg>"
        )
    }

    fn kinds(svg: &str) -> Vec<&'static str> {
        problems(svg.as_bytes(), &FONTS).into_iter().map(|(kind, _)| kind).collect()
    }

    #[test]
    fn reads_the_style_and_skips_the_dark_theme() {
        let svg = figure(200, "<text x=\"10\" y=\"40\" class=\"t big\">Wide</text>");
        let options = usvg::Options { fontdb: Arc::clone(&FONTS), ..usvg::Options::default() };
        let tree = usvg::Tree::from_data(svg.as_bytes(), &options).unwrap();
        let Node::Text(text) = &tree.root().children()[0] else { panic!("no text") };
        let span = &text.chunks()[0].spans()[0];
        assert_eq!(span.font_size().get(), 20.0);
        assert_eq!(span.font().weight(), 700);
        // DejaVu Sans Mono is 0.6 em a letter: four letters of 20 px.
        let ink = text.abs_stroke_bounding_box();
        assert!((40.0..=48.0).contains(&ink.width()), "{ink:?}");
    }

    #[test]
    fn a_clean_figure_has_nothing() {
        let svg = figure(
            200,
            "<rect x=\"5\" y=\"5\" width=\"120\" height=\"30\" rx=\"6\" fill=\"#eee\"/>\
             <text x=\"10\" y=\"24\" class=\"t\">In its box</text>\
             <text x=\"10\" y=\"60\" class=\"t\">One line</text><text x=\"10\" y=\"72\" class=\"t\">The next line</text>\
             <text x=\"190\" y=\"90\" text-anchor=\"end\" class=\"t\">corner</text>",
        );
        assert_eq!(kinds(&svg), [] as [&str; 0]);
    }

    #[test]
    fn finds_text_over_text() {
        let svg = figure(200, "<text x=\"10\" y=\"50\" class=\"t\">First label</text><text x=\"30\" y=\"54\" class=\"t\">Second</text>");
        let found = problems(svg.as_bytes(), &FONTS);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "svg-text-overlap");
        assert!(found[0].1.starts_with("“First label” at 10,50 overlaps “Second” at 30,54"), "{}", found[0].1);
    }

    #[test]
    fn finds_text_off_the_canvas() {
        let svg = figure(100, "<text x=\"60\" y=\"50\" class=\"t\">Far too long a label</text>");
        let found = problems(svg.as_bytes(), &FONTS);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "svg-text-outside");
        assert!(found[0].1.contains("past the canvas's right edge"), "{}", found[0].1);
    }

    #[test]
    fn finds_text_out_of_its_box() {
        let svg = figure(
            200,
            "<rect x=\"0\" y=\"0\" width=\"200\" height=\"100\" fill=\"#fff\"/>\
             <rect x=\"5\" y=\"5\" width=\"80\" height=\"30\" rx=\"6\" fill=\"#eee\"/>\
             <text x=\"10\" y=\"24\" class=\"t\">Spills out of it</text>",
        );
        let found = problems(svg.as_bytes(), &FONTS);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "svg-text-spill");
        assert!(found[0].1.contains("past the right of its shape at 5,5 (80×30)"), "{}", found[0].1);
    }

    #[test]
    fn what_was_calibrated_away_stays_away() {
        let svg = figure(
            340,
            // Two turned labels whose upright bounds cross and whose ink does not.
            "<text transform=\"translate(20 90) rotate(-30)\" class=\"t\">a long runway label</text>\
             <text transform=\"translate(60 95) rotate(-30)\" class=\"t\">another one below</text>\
             <rect x=\"200\" y=\"10\" width=\"10\" height=\"10\" fill=\"#c00\"/><text x=\"204\" y=\"19\" class=\"t\">swatch label</text>\
             <rect x=\"200\" y=\"30\" width=\"40\" height=\"60\" fill=\"#00c\" fill-opacity=\".08\"/><text x=\"204\" y=\"50\" class=\"t\">a note over a band</text>",
        );
        let found = problems(svg.as_bytes(), &FONTS);
        assert!(found.is_empty(), "{found:?}");
        // A label turned onto another is still one.
        let svg = figure(
            200,
            "<text x=\"20\" y=\"50\" class=\"t\">Across here</text><text transform=\"translate(40 80) rotate(-90)\" class=\"t\">Upward label</text>",
        );
        assert_eq!(kinds(&svg), ["svg-text-overlap"]);
    }

    #[test]
    fn a_halo_is_not_an_overlap_and_a_broken_file_is_one_finding() {
        let svg = figure(200, "<text x=\"10\" y=\"50\" stroke=\"#fff\" stroke-width=\"3\">Halo</text><text x=\"10\" y=\"50\">Halo</text>");
        assert_eq!(kinds(&svg), [] as [&str; 0]);
        assert_eq!(kinds("<svg><text>"), ["svg-parse"]);
        assert_eq!(quote(&"x".repeat(50)).chars().count(), QUOTE + 2);
    }
}
