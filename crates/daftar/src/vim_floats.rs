//! Neovim's floating windows in Neovim mode: Telescope, blink.cmp, an LSP
//! hover. The editor shows Neovim's buffer, not its screen, so a plugin that
//! draws a window of its own would not show. With `ext_multigrid` each
//! window is a grid of its own; this keeps the grids, the highlights and
//! where each float stands, on the thread that reads Neovim, and at each
//! `flush` hands the window the floats as rows of Pango markup to draw over
//! the note.

use std::collections::HashMap;

use daftar_core::MsgValue;
use gtk::glib;

/// One float, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatView {
    pub grid: i64,
    /// Placed on the editor's screen (`relative = "editor"`), or on a
    /// window, near its cursor.
    pub on_editor: bool,
    /// `NW`, `NE`, `SW` or `SE`: the float's corner at the anchor.
    pub corner: String,
    pub row: f64,
    pub column: f64,
    pub width: usize,
    pub height: usize,
    pub zindex: i64,
    /// Each row as Pango markup, in the float's colors.
    pub rows: Vec<String>,
    /// The float's own background, for the box around the rows.
    pub background: Option<String>,
}

/// The floats after a flush, and where the cursor is in its window's grid,
/// which places a float anchored to that window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Floats {
    pub views: Vec<FloatView>,
    pub window_cursor: Option<(i64, i64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Attr {
    foreground: Option<u32>,
    background: Option<u32>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    reverse: bool,
}

#[derive(Debug, Clone, Default)]
struct Grid {
    width: usize,
    /// Each cell's text (one character, or more for a wide one) and highlight.
    cells: Vec<Vec<(String, i64)>>,
}

impl Grid {
    fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.cells.resize(height, Vec::new());
        for row in &mut self.cells {
            row.resize(width, (" ".into(), 0));
        }
    }

    fn clear(&mut self) {
        for row in &mut self.cells {
            for cell in row.iter_mut() {
                *cell = (" ".into(), 0);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Place {
    anchor_grid: i64,
    corner: String,
    row: f64,
    column: f64,
    zindex: i64,
}

#[derive(Default)]
pub struct FloatState {
    attrs: HashMap<i64, Attr>,
    foreground: Option<u32>,
    background: Option<u32>,
    grids: HashMap<i64, Grid>,
    floats: HashMap<i64, Place>,
    cursor: Option<(i64, i64, i64)>,
    dirty: bool,
}

fn number(value: Option<&MsgValue>) -> Option<f64> {
    match value? {
        MsgValue::Double(d) => Some(*d),
        other => other.as_int().map(|i| i as f64),
    }
}

impl FloatState {
    /// One `redraw` notification's batches; the floats when a flush ends a
    /// change to them.
    pub fn apply(&mut self, batches: &[MsgValue]) -> Option<Floats> {
        let mut flushed = None;
        for batch in batches {
            let Some(items) = batch.as_array() else { continue };
            let Some(name) = items.first().and_then(MsgValue::as_str) else { continue };
            for args in items.iter().skip(1).filter_map(MsgValue::as_array) {
                let int = |at: usize| args.get(at).and_then(MsgValue::as_int);
                match name {
                    "default_colors_set" => {
                        self.foreground = int(0).filter(|c| *c >= 0).map(|c| c as u32);
                        self.background = int(1).filter(|c| *c >= 0).map(|c| c as u32);
                        self.dirty = true;
                    }
                    "hl_attr_define" => {
                        let Some(id) = int(0) else { continue };
                        let mut attr = Attr::default();
                        if let Some(MsgValue::Map(pairs)) = args.get(1) {
                            for (key, value) in pairs {
                                match key.as_str().unwrap_or("") {
                                    "foreground" => attr.foreground = value.as_int().map(|c| c as u32),
                                    "background" => attr.background = value.as_int().map(|c| c as u32),
                                    "bold" => attr.bold = value.as_bool().unwrap_or(false),
                                    "italic" => attr.italic = value.as_bool().unwrap_or(false),
                                    "underline" | "undercurl" => attr.underline = value.as_bool().unwrap_or(false),
                                    "strikethrough" => attr.strike = value.as_bool().unwrap_or(false),
                                    "reverse" => attr.reverse = value.as_bool().unwrap_or(false),
                                    _ => {}
                                }
                            }
                        }
                        self.attrs.insert(id, attr);
                    }
                    "grid_resize" => {
                        let (Some(grid), Some(width), Some(height)) = (int(0), int(1), int(2)) else { continue };
                        self.grids.entry(grid).or_default().resize(width.max(0) as usize, height.max(0) as usize);
                        self.touch(grid);
                    }
                    "grid_clear" => {
                        let Some(grid) = int(0) else { continue };
                        if let Some(cells) = self.grids.get_mut(&grid) {
                            cells.clear();
                        }
                        self.touch(grid);
                    }
                    "grid_destroy" | "win_close" => {
                        let Some(grid) = int(0) else { continue };
                        self.grids.remove(&grid);
                        if self.floats.remove(&grid).is_some() {
                            self.dirty = true;
                        }
                    }
                    "win_hide" => {
                        if let Some(grid) = int(0)
                            && self.floats.remove(&grid).is_some()
                        {
                            self.dirty = true;
                        }
                    }
                    "win_pos" => {
                        // A window of the editor's layout: not a float, or no longer one.
                        if let Some(grid) = int(0)
                            && self.floats.remove(&grid).is_some()
                        {
                            self.dirty = true;
                        }
                    }
                    "win_float_pos" => {
                        let Some(grid) = int(0) else { continue };
                        let place = Place {
                            corner: args.get(2).and_then(MsgValue::as_str).unwrap_or("NW").to_string(),
                            anchor_grid: int(3).unwrap_or(1),
                            row: number(args.get(4)).unwrap_or(0.0),
                            column: number(args.get(5)).unwrap_or(0.0),
                            zindex: int(7).unwrap_or(50),
                        };
                        self.floats.insert(grid, place);
                        self.dirty = true;
                    }
                    "grid_cursor_goto" => {
                        let (Some(grid), Some(row), Some(column)) = (int(0), int(1), int(2)) else { continue };
                        let before = self.cursor.replace((grid, row, column));
                        if self.floats.contains_key(&grid) || before.is_some_and(|(g, _, _)| self.floats.contains_key(&g)) {
                            self.dirty = true;
                        }
                    }
                    "grid_scroll" => {
                        let (Some(grid), Some(top), Some(bottom), Some(rows)) = (int(0), int(1), int(2), int(5)) else { continue };
                        if let Some(cells) = self.grids.get_mut(&grid) {
                            scroll(&mut cells.cells, top.max(0) as usize, bottom.max(0) as usize, rows);
                        }
                        self.touch(grid);
                    }
                    "grid_line" => {
                        let (Some(grid), Some(row), Some(column)) = (int(0), int(1), int(2)) else { continue };
                        let Some(cells) = args.get(3).and_then(MsgValue::as_array) else { continue };
                        if let Some(target) = self.grids.get_mut(&grid) {
                            line(target, row.max(0) as usize, column.max(0) as usize, cells);
                        }
                        self.touch(grid);
                    }
                    "flush" if std::mem::take(&mut self.dirty) => flushed = Some(self.floats()),
                    _ => {}
                }
            }
        }
        flushed
    }

    fn touch(&mut self, grid: i64) {
        if self.floats.contains_key(&grid) {
            self.dirty = true;
        }
    }

    fn floats(&self) -> Floats {
        let mut views: Vec<FloatView> = self
            .floats
            .iter()
            .filter_map(|(&grid, place)| {
                let cells = self.grids.get(&grid)?;
                let cursor = self.cursor.filter(|(g, _, _)| *g == grid).map(|(_, r, c)| (r as usize, c as usize));
                let rows = cells
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(at, row)| self.markup(row, cursor.filter(|(r, _)| *r == at).map(|(_, c)| c)))
                    .collect();
                Some(FloatView {
                    grid,
                    on_editor: place.anchor_grid == 1,
                    corner: place.corner.clone(),
                    row: place.row,
                    column: place.column,
                    width: cells.width,
                    height: cells.cells.len(),
                    zindex: place.zindex,
                    rows,
                    background: self.background.map(hex),
                })
            })
            .filter(|view| view.width > 0 && view.height > 0)
            .collect();
        views.sort_by_key(|view| (view.zindex, view.grid));
        let window_cursor = self.cursor.filter(|(grid, _, _)| !self.floats.contains_key(grid)).map(|(_, row, column)| (row, column));
        Floats { views, window_cursor }
    }

    /// A row of cells as Pango markup: a span for each run of one highlight,
    /// and the cursor's cell reversed when the cursor is in the float.
    fn markup(&self, row: &[(String, i64)], cursor: Option<usize>) -> String {
        let mut out = String::new();
        let mut at = 0;
        while at < row.len() {
            let id = row[at].1;
            let mut end = at + 1;
            while end < row.len() && row[end].1 == id && Some(end) != cursor && Some(at) != cursor {
                end += 1;
            }
            let text: String = row[at..end].iter().map(|(text, _)| text.as_str()).collect();
            let mut attr = self.attrs.get(&id).cloned().unwrap_or_default();
            if Some(at) == cursor {
                attr.reverse = !attr.reverse;
            }
            out.push_str(&self.span(&attr, &text));
            at = end;
        }
        out
    }

    fn span(&self, attr: &Attr, text: &str) -> String {
        let (mut fg, mut bg) = (attr.foreground.or(self.foreground), attr.background.or(self.background));
        if attr.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        let mut open = String::from("<span");
        if let Some(fg) = fg {
            open.push_str(&format!(" foreground=\"{}\"", hex(fg)));
        }
        if let Some(bg) = bg {
            open.push_str(&format!(" background=\"{}\"", hex(bg)));
        }
        if attr.bold {
            open.push_str(" weight=\"bold\"");
        }
        if attr.italic {
            open.push_str(" style=\"italic\"");
        }
        if attr.underline {
            open.push_str(" underline=\"single\"");
        }
        if attr.strike {
            open.push_str(" strikethrough=\"true\"");
        }
        format!("{open}>{}</span>", glib::markup_escape_text(text))
    }
}

fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0xFF_FFFF)
}

/// `grid_line`'s cells from [column]: `[text, hl_id?, repeat?]`, a missing
/// highlight being the one before.
fn line(grid: &mut Grid, row: usize, column: usize, cells: &[MsgValue]) {
    let Some(target) = grid.cells.get_mut(row) else { return };
    let (mut at, mut id) = (column, 0);
    for cell in cells.iter().filter_map(MsgValue::as_array) {
        let text = cell.first().and_then(MsgValue::as_str).unwrap_or(" ");
        if let Some(next) = cell.get(1).and_then(MsgValue::as_int) {
            id = next;
        }
        let repeat = cell.get(2).and_then(MsgValue::as_int).unwrap_or(1).max(1) as usize;
        for _ in 0..repeat {
            if let Some(slot) = target.get_mut(at) {
                *slot = (text.to_string(), id);
            }
            at += 1;
        }
    }
}

/// `grid_scroll`: rows [top, bottom) move up by [rows] (down when negative).
fn scroll(cells: &mut [Vec<(String, i64)>], top: usize, bottom: usize, rows: i64) {
    let bottom = bottom.min(cells.len());
    if top >= bottom {
        return;
    }
    if rows > 0 {
        for at in top..bottom.saturating_sub(rows as usize) {
            cells[at] = cells[at + rows as usize].clone();
        }
    } else if rows < 0 {
        let by = (-rows) as usize;
        for at in (top + by..bottom).rev() {
            cells[at] = cells[at - by].clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> MsgValue {
        MsgValue::String(text.into())
    }

    fn i(n: i64) -> MsgValue {
        MsgValue::Int(n)
    }

    fn a(items: Vec<MsgValue>) -> MsgValue {
        MsgValue::Array(items)
    }

    #[test]
    fn a_float_is_drawn_with_its_colors_at_the_flush() {
        let mut state = FloatState::default();
        let batches = vec![
            a(vec![s("default_colors_set"), a(vec![i(0xffffff), i(0x000000), i(-1)])]),
            a(vec![
                s("hl_attr_define"),
                a(vec![i(3), MsgValue::Map(vec![(s("foreground"), i(0xff0000)), (s("bold"), MsgValue::Bool(true))])]),
            ]),
            a(vec![s("grid_resize"), a(vec![i(4), i(5), i(2)])]),
            a(vec![
                s("win_float_pos"),
                a(vec![i(4), i(1000), s("NW"), i(1), MsgValue::Double(3.0), MsgValue::Double(10.0), MsgValue::Bool(true), i(50)]),
            ]),
            a(vec![
                s("grid_line"),
                a(vec![i(4), i(0), i(0), a(vec![a(vec![s("a"), i(3), i(2)]), a(vec![s("<")])]), MsgValue::Bool(false)]),
            ]),
            a(vec![s("flush"), a(vec![])]),
        ];
        let floats = state.apply(&batches).expect("a flush with a float");
        assert_eq!(floats.views.len(), 1);
        let view = &floats.views[0];
        assert!(view.on_editor);
        assert_eq!((view.row, view.column, view.width, view.height), (3.0, 10.0, 5, 2));
        assert_eq!(
            view.rows[0],
            "<span foreground=\"#ff0000\" background=\"#000000\" weight=\"bold\">aa&lt;</span><span foreground=\"#ffffff\" background=\"#000000\">  </span>"
        );
        // Nothing changed: no new floats at the next flush.
        assert_eq!(state.apply(&[a(vec![s("flush"), a(vec![])])]), None);
        // Closed: none.
        let closed = state.apply(&[a(vec![s("win_close"), a(vec![i(4)])]), a(vec![s("flush"), a(vec![])])]).unwrap();
        assert!(closed.views.is_empty());
    }

    #[test]
    fn a_scroll_moves_the_rows() {
        let mut rows: Vec<Vec<(String, i64)>> = ["a", "b", "c"].iter().map(|t| vec![(t.to_string(), 0)]).collect();
        scroll(&mut rows, 0, 3, 1);
        assert_eq!(rows.iter().map(|r| r[0].0.as_str()).collect::<Vec<_>>(), ["b", "c", "c"]);
        scroll(&mut rows, 0, 3, -1);
        assert_eq!(rows.iter().map(|r| r[0].0.as_str()).collect::<Vec<_>>(), ["b", "b", "c"]);
    }
}
