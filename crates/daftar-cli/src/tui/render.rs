//! Draws the app: the tree on the left, the note, the status bar, and a
//! list or the help over them.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use daftar_core::{QuickSwitcher, Vault};

use super::app::{App, Focus, Overlay, Picker};
use super::theme::Theme;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let theme = app.theme;
    let area = frame.area();
    frame.render_widget(Block::new().style(theme.base()), area);
    let [main, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let sidebar_width = (main.width / 4).clamp(22, 36);
    let [sidebar, note] = Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(20)]).areas(main);
    draw_tree(frame, app, sidebar);
    draw_note(frame, app, note);
    draw_status(frame, app, status);
    match &mut app.overlay {
        Some(Overlay::Picker(picker)) => draw_picker(frame, &theme, picker, area),
        Some(Overlay::Help) => draw_help(frame, &theme, area),
        None => {}
    }
}

fn pane<'a>(theme: &Theme, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { theme.accent } else { theme.rule }))
        .padding(Padding::horizontal(1))
        .style(theme.base())
}

fn draw_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let focused = app.focus == Focus::Sidebar && app.overlay.is_none();
    let block = pane(&theme, focused).title(Line::from(vec![
        Span::styled(" ◆ ", Style::new().fg(theme.accent)),
        Span::styled(format!("{} ", app.vault_name()), Style::new().fg(theme.text).add_modifier(Modifier::BOLD)),
    ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height as usize;
    if app.tree_selected < app.tree_scroll {
        app.tree_scroll = app.tree_selected;
    } else if app.tree_selected >= app.tree_scroll + height {
        app.tree_scroll = app.tree_selected + 1 - height;
    }
    let open = app.note.as_ref().map(|n| n.path.as_str());
    let lines: Vec<Line> = app
        .tree
        .iter()
        .enumerate()
        .skip(app.tree_scroll)
        .take(height)
        .map(|(at, row)| {
            let indent = Span::raw("  ".repeat(row.depth));
            let current = open.is_some_and(|p| p == row.path || row.folder_note.as_deref() == Some(p));
            let (mark, style) = if row.folder {
                (if row.open { "▾ " } else { "▸ " }, Style::new().fg(theme.text).add_modifier(Modifier::BOLD))
            } else if Vault::is_note(&row.path) {
                ("  ", Style::new().fg(theme.text))
            } else {
                ("· ", theme.faint())
            };
            let name = if !row.folder && Vault::is_note(&row.path) { Vault::title(&row.path) } else { row.name.clone() };
            let style = if current { style.fg(theme.accent) } else { style };
            let mut line = Line::from(vec![indent, Span::styled(mark, Style::new().fg(theme.accent)), Span::styled(name, style)]);
            if at == app.tree_selected && (focused || app.note.is_none()) {
                line = line.style(Style::new().bg(theme.panel));
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_note(frame: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let focused = app.focus == Focus::Note && app.overlay.is_none();
    let mut block = pane(&theme, focused);
    if let Some(note) = &app.note {
        block = block
            .title(Line::from(Span::styled(format!(" {} ", Vault::title(&note.path)), theme.heading(1))))
            .title(Line::from(Span::styled(format!(" {} ", QuickSwitcher::folder(&note.path)), theme.faint())).alignment(Alignment::Right));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = inner.height as usize;
    app.layout(inner.width, height);
    let Some(note) = &app.note else {
        draw_welcome(frame, &theme, inner);
        return;
    };
    let lines: Vec<Line> = note
        .rows
        .iter()
        .enumerate()
        .skip(note.scroll)
        .take(height)
        .map(|(at, row)| {
            let mut line = row.line.clone();
            if focused && at == note.cursor {
                line = line.patch_style(Style::new().bg(theme.panel));
            }
            line
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
    // Where in the note: a thin bar on the right edge.
    if note.rows.len() > height && height > 2 {
        let total = note.rows.len();
        let top = note.scroll * height / total;
        let size = (height * height / total).max(1);
        for y in 0..height {
            let mark = if y >= top && y < top + size { "┃" } else { " " };
            let cell = Rect { x: area.right() - 1, y: inner.y + y as u16, width: 1, height: 1 };
            frame.render_widget(Paragraph::new(Span::styled(mark, Style::new().fg(theme.rule))), cell);
        }
    }
}

fn draw_welcome(frame: &mut Frame, theme: &Theme, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("دفتر", Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("Daftar", theme.heading(1))),
        Line::default(),
        Line::from(vec![Span::styled("Ctrl+P ", Style::new().fg(theme.accent)), Span::styled("open a note", theme.muted())]),
        Line::from(vec![Span::styled("/ ", Style::new().fg(theme.accent)), Span::styled("search the vault", theme.muted())]),
        Line::from(vec![Span::styled("t ", Style::new().fg(theme.accent)), Span::styled("open tasks", theme.muted())]),
        Line::from(vec![Span::styled("? ", Style::new().fg(theme.accent)), Span::styled("every key", theme.muted())]),
    ];
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let rect = Rect { y: area.y + top, height: area.height.saturating_sub(top), ..area };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), rect);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut left = vec![
        Span::styled(" daftar ", Style::new().fg(theme.background).bg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} ", app.vault_name()), Style::new().fg(theme.text).bg(theme.panel)),
        Span::raw(" "),
    ];
    match &app.message {
        Some((text, error)) => {
            left.push(Span::styled(text.clone(), Style::new().fg(if *error { theme.red } else { theme.em })));
        }
        None => {
            let here = app.note.as_ref().and_then(|n| n.block());
            let mut hints = vec![("^P", "open"), ("/", "search"), ("t", "tasks"), ("b", "links"), ("#", "tags")];
            if app.focus == Focus::Note {
                if here.is_some_and(|b| b.task.is_some()) {
                    hints.insert(0, ("space", "tick"));
                }
                if here.is_some_and(|b| !b.links.is_empty()) {
                    hints.insert(0, ("enter", "follow"));
                }
            }
            hints.push(("?", "help"));
            for (n, (key, what)) in hints.into_iter().enumerate() {
                if n > 0 {
                    left.push(Span::styled(" · ", theme.faint()));
                }
                left.push(Span::styled(key, Style::new().fg(theme.accent)));
                left.push(Span::styled(format!(" {what}"), theme.muted()));
            }
        }
    }
    let right = match &app.git_status {
        Some(status) => {
            let mut spans = vec![Span::styled(format!(" ⎇ {} ", status.branch), Style::new().fg(theme.text))];
            if status.ahead > 0 {
                spans.push(Span::styled(format!("↑{} ", status.ahead), Style::new().fg(theme.green)));
            }
            if status.behind > 0 {
                spans.push(Span::styled(format!("↓{} ", status.behind), Style::new().fg(theme.orange)));
            }
            if status.changed > 0 {
                spans.push(Span::styled(format!("●{} ", status.changed), Style::new().fg(theme.vivid_yellow)));
            } else {
                spans.push(Span::styled("✓ ", Style::new().fg(theme.green)));
            }
            Line::from(spans).style(Style::new().bg(theme.panel))
        }
        None => Line::from(Span::styled(" no git ", theme.faint())),
    };
    let right_width = right.width() as u16;
    let [left_area, right_area] = Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(left)), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 3, width, height }
}

fn draw_picker(frame: &mut Frame, theme: &Theme, picker: &mut Picker, area: Rect) {
    let rect = centered(area, (area.width * 7 / 10).max(50), (area.height * 7 / 10).max(12));
    frame.render_widget(Clear, rect);
    let count = picker.items.iter().filter(|i| i.selectable()).count();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .padding(Padding::horizontal(1))
        .style(theme.base())
        .title(Line::from(Span::styled(format!(" {} ", picker.title), Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))))
        .title_bottom(Line::from(Span::styled(format!(" {count} "), theme.faint())).alignment(Alignment::Right))
        .title_bottom(Line::from(Span::styled(" ↑↓ move · enter open · esc close ", theme.faint())));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut list = inner;
    if let Some(query) = &picker.query {
        let line = Line::from(vec![
            Span::styled("› ", Style::new().fg(theme.accent)),
            Span::styled(query.clone(), Style::new().fg(theme.text)),
            Span::styled("▏", Style::new().fg(theme.accent)),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
        let rule = Line::from(Span::styled("─".repeat(inner.width as usize), Style::new().fg(theme.rule)));
        frame.render_widget(Paragraph::new(rule), Rect { y: inner.y + 1, height: 1, ..inner });
        list = Rect { y: inner.y + 2, height: inner.height.saturating_sub(2), ..inner };
    }
    let height = list.height as usize;
    let rows = |at: usize| if picker.items[at].detail.is_some() { 2 } else { 1 };
    if picker.selected < picker.scroll {
        picker.scroll = picker.selected;
    }
    while picker.scroll < picker.selected && (picker.scroll..=picker.selected).map(rows).sum::<usize>() > height {
        picker.scroll += 1;
    }
    let mut lines = Vec::new();
    for (at, item) in picker.items.iter().enumerate().skip(picker.scroll) {
        if lines.len() >= height {
            break;
        }
        let selected = at == picker.selected && item.selectable();
        let mark = if selected { Span::styled("▌", Style::new().fg(theme.accent)) } else { Span::raw(" ") };
        let mut line = item.line.clone();
        line.spans.insert(0, mark.clone());
        if selected {
            line = line.style(Style::new().bg(theme.panel));
        }
        lines.push(line);
        if let Some(detail) = &item.detail {
            let mut detail = detail.clone();
            detail.spans.insert(0, Span::raw(if selected { "▌ " } else { "  " }).style(Style::new().fg(theme.accent)));
            if selected {
                detail = detail.style(Style::new().bg(theme.panel));
            }
            lines.push(detail);
        }
    }
    frame.render_widget(Paragraph::new(lines), list);
}

pub const KEYS: [(&str, &str); 22] = [
    ("Ctrl+P  p", "open a note by name"),
    ("/  Ctrl+F", "search the text of every note"),
    ("t", "tasks: the note's queries, else every open task"),
    ("b", "notes that link to this one"),
    ("#", "tags, then the notes with one"),
    ("Tab", "the tree or the note"),
    ("j k  ↓ ↑", "move"),
    ("Ctrl+D  Ctrl+U", "half a page down, up"),
    ("g  G", "top, bottom"),
    ("] [", "next, previous link or task"),
    ("Enter  l  →", "follow the link; open a folder or a note"),
    ("h  ←", "close the folder"),
    ("Backspace  Alt+←", "back"),
    ("Space", "tick the task, or open it again"),
    ("e", "edit the note in $EDITOR"),
    ("s", "sync: commit, pull, push"),
    ("r", "read the vault again"),
    ("?", "this help"),
    ("Esc", "close the list"),
    ("q  Ctrl+C", "quit"),
    ("", ""),
    ("", "A tick refuses when the line changed on disk."),
];

fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect) {
    let rect = centered(area, 64, KEYS.len() as u16 + 4);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .padding(Padding::new(2, 2, 1, 0))
        .style(theme.base())
        .title(Line::from(Span::styled(" Keys ", Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))));
    let lines: Vec<Line> = KEYS
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("{key:<18}"), Style::new().fg(theme.accent)),
                Span::styled(*what, Style::new().fg(theme.text)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}
