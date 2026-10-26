//! What a click or the wheel does, by what is under it: the tree, the note
//! or the list over them, as the last draw left them in `App::areas`.

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

use super::app::{App, Areas, Focus, Overlay};

/// Rows the wheel moves at a time.
const WHEEL: isize = 3;

/// What is under a point of the screen, in rows from the top of each pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// A row of the tree on screen.
    Tree(usize),
    /// A row of the note on screen, and the column in it.
    Note(usize, u16),
    /// An item of the open list, and the column in its row.
    Item(usize, u16),
    /// Inside the open list, on no item.
    Picker,
    /// Outside the open list.
    Outside,
    Nothing,
}

impl Areas {
    pub fn hit(&self, x: u16, y: u16, overlay: bool) -> Hit {
        let at = Position::new(x, y);
        if overlay {
            if self.list.contains(at) {
                return match self.list_items.get((y - self.list.y) as usize) {
                    Some(&item) => Hit::Item(item, x - self.list.x),
                    None => Hit::Picker,
                };
            }
            return if self.picker.contains(at) { Hit::Picker } else { Hit::Outside };
        }
        if self.tree.contains(at) {
            Hit::Tree((y - self.tree.y) as usize)
        } else if self.note.contains(at) {
            Hit::Note((y - self.note.y) as usize, x - self.note.x)
        } else {
            Hit::Nothing
        }
    }
}

/// Whether [column] of [line] is a task's box: the cell after the spaces
/// that indent it, and the space after it.
pub fn on_checkbox(line: &Line, column: u16) -> bool {
    let at: usize = line.spans.iter().take_while(|span| span.content.trim().is_empty()).map(|span| span.content.width()).sum();
    (at..at + 2).contains(&(column as usize))
}

pub fn handle(app: &mut App, event: MouseEvent) {
    let hit = app.areas.hit(event.column, event.row, app.overlay.is_some());
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => click(app, hit),
        MouseEventKind::ScrollDown => scroll(app, hit, WHEEL),
        MouseEventKind::ScrollUp => scroll(app, hit, -WHEEL),
        _ => {}
    }
}

fn click(app: &mut App, hit: Hit) {
    if matches!(app.overlay, Some(Overlay::Help)) {
        app.overlay = None;
        return;
    }
    match hit {
        // A row of the tree opens, as Enter does.
        Hit::Tree(row) => {
            let at = app.tree_scroll + row;
            if at < app.tree.len() {
                app.message = None;
                app.focus = Focus::Sidebar;
                app.tree_selected = at;
                app.tree_activate();
            }
        }
        // The cursor to the row; a box ticks; the row under the cursor
        // clicked again follows its link.
        Hit::Note(row, column) => {
            let focused = app.focus == Focus::Note;
            let Some(note) = &mut app.note else { return };
            let at = note.scroll + row;
            let Some(shown) = note.rows.get(at) else { return };
            let block = &note.blocks[shown.block];
            let first = at == 0 || note.rows[at - 1].block != shown.block;
            let tick = first && block.task.is_some() && !block.rtl && on_checkbox(&shown.line, column);
            let follow = focused && note.cursor == at && !block.links.is_empty();
            note.cursor = at;
            app.focus = Focus::Note;
            app.message = None;
            if tick {
                app.toggle_here();
            } else if follow {
                app.follow();
            }
        }
        // An item opens; a task's box ticks it.
        Hit::Item(item, column) => {
            let Some(picker) = app.picker() else { return };
            let Some(entry) = picker.items.get(item) else { return };
            if !entry.selectable() {
                return;
            }
            // The list draws a mark before each row.
            let tick = entry.task.is_some() && column > 0 && on_checkbox(&entry.line, column - 1);
            picker.selected = item;
            if tick {
                app.picker_toggle();
            } else {
                app.picker_activate();
            }
        }
        Hit::Outside => app.overlay = None,
        Hit::Picker | Hit::Nothing => {}
    }
}

fn scroll(app: &mut App, hit: Hit, by: isize) {
    match hit {
        Hit::Item(..) | Hit::Picker | Hit::Outside => app.picker_move(by),
        Hit::Tree(_) => app.tree_scroll_by(by),
        Hit::Note(..) => app.note_scroll_by(by),
        Hit::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::text::Span;

    use super::*;

    fn areas() -> Areas {
        Areas {
            tree: Rect::new(2, 1, 20, 10),
            note: Rect::new(26, 1, 50, 10),
            picker: Rect::new(10, 3, 40, 8),
            list: Rect::new(12, 6, 36, 4),
            list_items: vec![0, 0, 2, 3],
        }
    }

    #[test]
    fn a_point_is_a_row_of_the_pane_under_it() {
        let areas = areas();
        assert_eq!(areas.hit(2, 1, false), Hit::Tree(0));
        assert_eq!(areas.hit(21, 10, false), Hit::Tree(9));
        assert_eq!(areas.hit(30, 4, false), Hit::Note(3, 4));
        assert_eq!(areas.hit(0, 0, false), Hit::Nothing);
        assert_eq!(areas.hit(23, 4, false), Hit::Nothing);
        assert_eq!(areas.hit(30, 11, false), Hit::Nothing);
    }

    #[test]
    fn over_a_list_a_point_is_an_item_or_the_list_or_outside() {
        let areas = areas();
        // An item with a detail line takes two rows.
        assert_eq!(areas.hit(12, 6, true), Hit::Item(0, 0));
        assert_eq!(areas.hit(15, 7, true), Hit::Item(0, 3));
        assert_eq!(areas.hit(12, 8, true), Hit::Item(2, 0));
        assert_eq!(areas.hit(12, 4, true), Hit::Picker);
        assert_eq!(areas.hit(1, 1, true), Hit::Outside);
        // Under the list: the tree is covered.
        assert_eq!(areas.hit(12, 10, true), Hit::Picker);
    }

    #[test]
    fn the_box_is_after_the_indent() {
        let line = Line::from(vec![Span::raw("  "), Span::raw("☐ "), Span::raw("Ship it")]);
        assert!(!on_checkbox(&line, 1));
        assert!(on_checkbox(&line, 2));
        assert!(on_checkbox(&line, 3));
        assert!(!on_checkbox(&line, 4));
        let flush = Line::from(vec![Span::raw(""), Span::raw("☑ "), Span::raw("Done")]);
        assert!(on_checkbox(&flush, 0));
        assert!(!on_checkbox(&flush, 2));
    }
}
