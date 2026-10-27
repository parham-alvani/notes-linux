//! What each key does, by where the keyboard is: a list, the help, the
//! tree, the note or the home page.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Focus, Overlay, PickerKind};

pub fn handle(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl && key.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    match &app.overlay {
        Some(Overlay::Help) => {
            app.overlay = None;
            return;
        }
        Some(Overlay::Picker(picker)) => {
            let typing = picker.query.is_some();
            let tasks = picker.kind == PickerKind::Tasks;
            picker_key(app, key, ctrl, typing, tasks);
            return;
        }
        None => {}
    }
    // A message says what the last key did; the next one clears it.
    app.message = None;
    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('p') if ctrl => app.open_switcher(),
        KeyCode::Char('p') => app.open_switcher(),
        KeyCode::Char('f') if ctrl => app.open_search(),
        KeyCode::Char('/') => app.open_search(),
        KeyCode::Char('t') => app.open_tasks(),
        KeyCode::Char('b') => app.open_backlinks(),
        KeyCode::Char('#') => app.open_tags(),
        KeyCode::Char('?') => app.overlay = Some(Overlay::Help),
        KeyCode::Char('e') => app.edit(),
        KeyCode::Char('s') => app.start_sync(),
        KeyCode::Char('r') => app.reindex(),
        KeyCode::Char('H') => app.go_home(),
        KeyCode::Char('.') => app.toggle_hidden(),
        KeyCode::Tab | KeyCode::BackTab => {
            let pane = app.note.is_some() || !app.home.notes().is_empty();
            app.focus = if app.focus == Focus::Sidebar && pane { Focus::Note } else { Focus::Sidebar };
        }
        KeyCode::Backspace => app.back(),
        KeyCode::Left if alt => app.back(),
        _ => match app.focus {
            Focus::Sidebar => tree_key(app, key, ctrl),
            Focus::Note if app.note.is_none() => home_key(app, key),
            Focus::Note => note_key(app, key, ctrl),
        },
    }
}

fn tree_key(app: &mut App, key: KeyEvent, ctrl: bool) {
    let half = (app.page / 2).max(1) as isize;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.tree_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.tree_move(-1),
        KeyCode::Char('d') if ctrl => app.tree_move(half),
        KeyCode::Char('u') if ctrl => app.tree_move(-half),
        KeyCode::PageDown => app.tree_move(half * 2),
        KeyCode::PageUp => app.tree_move(-half * 2),
        KeyCode::Char('g') | KeyCode::Home => app.tree_selected = 0,
        KeyCode::Char('G') | KeyCode::End => app.tree_selected = app.tree.len().saturating_sub(1),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.tree_activate(),
        KeyCode::Char('h') | KeyCode::Left => app.tree_collapse(),
        _ => {}
    }
}

fn note_key(app: &mut App, key: KeyEvent, ctrl: bool) {
    let half = (app.page / 2).max(1) as isize;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.note_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.note_move(-1),
        KeyCode::Char('d') if ctrl => app.note_move(half),
        KeyCode::Char('u') if ctrl => app.note_move(-half),
        KeyCode::PageDown => app.note_move(half * 2),
        KeyCode::PageUp => app.note_move(-half * 2),
        KeyCode::Char('g') | KeyCode::Home => app.note_move(isize::MIN / 2),
        KeyCode::Char('G') | KeyCode::End => app.note_move(isize::MAX / 2),
        KeyCode::Char(']') => app.note_jump(true),
        KeyCode::Char('[') => app.note_jump(false),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.follow(),
        KeyCode::Char(' ') => app.toggle_here(),
        KeyCode::Char('h') | KeyCode::Left => app.focus = Focus::Sidebar,
        _ => {}
    }
}

/// The home page's lists: the notes read last and the bookmarks.
fn home_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.home_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.home_move(-1),
        KeyCode::Char('g') | KeyCode::Home => app.home.selected = 0,
        KeyCode::Char('G') | KeyCode::End => app.home_move(isize::MAX / 2),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.home_activate(),
        KeyCode::Char('h') | KeyCode::Left => app.focus = Focus::Sidebar,
        _ => {}
    }
}

fn picker_key(app: &mut App, key: KeyEvent, ctrl: bool, typing: bool, tasks: bool) {
    match key.code {
        KeyCode::Esc => app.overlay = None,
        KeyCode::Enter => app.picker_activate(),
        KeyCode::Down | KeyCode::Tab => app.picker_move(1),
        KeyCode::Up | KeyCode::BackTab => app.picker_move(-1),
        KeyCode::Char('n') if ctrl => app.picker_move(1),
        KeyCode::Char('p') if ctrl => app.picker_move(-1),
        KeyCode::PageDown => app.picker_move(10),
        KeyCode::PageUp => app.picker_move(-10),
        KeyCode::Char('u') if ctrl && typing => {
            if let Some(picker) = app.picker() {
                picker.query = Some(String::new());
            }
            app.refilter();
        }
        KeyCode::Backspace if typing => {
            if let Some(query) = app.picker().and_then(|p| p.query.as_mut()) {
                query.pop();
            }
            app.refilter();
        }
        KeyCode::Char(c) if typing && !ctrl => {
            if let Some(query) = app.picker().and_then(|p| p.query.as_mut()) {
                query.push(c);
            }
            app.refilter();
        }
        KeyCode::Char(' ') if tasks => app.picker_toggle(),
        KeyCode::Char('j') => app.picker_move(1),
        KeyCode::Char('k') => app.picker_move(-1),
        KeyCode::Char('q') | KeyCode::Backspace => app.overlay = None,
        _ => {}
    }
}
