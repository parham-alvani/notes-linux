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
    let (picker, list, list_items) = match &mut app.overlay {
        Some(Overlay::Picker(picker)) => draw_picker(frame, &theme, picker, area),
        Some(Overlay::Help) => (draw_help(frame, &theme, area), Rect::default(), Vec::new()),
        None => (Rect::default(), Rect::default(), Vec::new()),
    };
    app.areas.picker = picker;
    app.areas.list = list;
    app.areas.list_items = list_items;
}

/// The spinner's frames, one every 80 ms.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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
    app.areas.tree = inner;
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
    if app.note.is_none() {
        block = block.title(Line::from(Span::styled(" Home ", theme.heading(1))));
    }
    if let Some(note) = &app.note {
        block = block
            .title(Line::from(Span::styled(format!(" {} ", Vault::title(&note.path)), theme.heading(1))))
            .title(Line::from(Span::styled(format!(" {} ", QuickSwitcher::folder(&note.path)), theme.faint())).alignment(Alignment::Right));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.areas.note = inner;
    let height = inner.height as usize;
    app.layout(inner.width, height);
    let Some(note) = &app.note else {
        draw_home(frame, app, inner, focused);
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

/// Each digit and the colon in blocks, five rows high, for the home page's clock.
const DIGITS: [[&str; 5]; 11] = [
    ["███", "█ █", "█ █", "█ █", "███"],
    ["  █", "  █", "  █", "  █", "  █"],
    ["███", "  █", "███", "█  ", "███"],
    ["███", "  █", "███", "  █", "███"],
    ["█ █", "█ █", "███", "  █", "  █"],
    ["███", "█  ", "███", "  █", "███"],
    ["███", "█  ", "███", "█ █", "███"],
    ["███", "  █", "  █", "  █", "  █"],
    ["███", "█ █", "███", "█ █", "███"],
    ["███", "█ █", "███", "  █", "███"],
    [" ", "█", " ", "█", " "],
];

/// [time], `7:52` or `19:52`, in five rows of blocks.
fn big_clock(time: &str) -> [String; 5] {
    let glyphs: Vec<&[&str; 5]> =
        time.chars().filter_map(|c| if c == ':' { Some(&DIGITS[10]) } else { c.to_digit(10).map(|d| &DIGITS[d as usize]) }).collect();
    std::array::from_fn(|row| glyphs.iter().map(|glyph| glyph[row]).collect::<Vec<_>>().join(" "))
}

/// The note pane with no note: the time, a greeting, the notes read last,
/// the bookmarks and a quote, as the app's home page shows them.
fn draw_home(frame: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let theme = app.theme;
    let home = &app.home;
    let settings = &home.settings;
    let now = chrono::Local::now();
    let twenty_four_hour = settings.twenty_four_hour.unwrap_or(true);
    let (time, half) = if twenty_four_hour {
        (now.format("%H:%M").to_string(), String::new())
    } else {
        (now.format("%-I:%M").to_string(), now.format("%p").to_string())
    };
    let centered = |line: Line<'static>| (line, None, true);
    let mut lines: Vec<(Line<'static>, Option<usize>, bool)> = Vec::new();
    if settings.show_time {
        let accent = Style::new().fg(theme.accent);
        let clock = big_clock(&time);
        // The clock in blocks when the pane is big enough, else in one line.
        let lists = home.notes().len() + home.lists.len() * 2;
        if area.height as usize >= lists + 16 && area.width as usize > clock[0].chars().count() + 4 {
            for (row, text) in clock.into_iter().enumerate() {
                let mut spans = vec![Span::styled(text, accent)];
                if row == 4 && !half.is_empty() {
                    spans.push(Span::styled(format!(" {half}"), theme.muted()));
                }
                lines.push(centered(Line::from(spans)));
            }
            lines.push(centered(Line::default()));
        } else {
            let text = if half.is_empty() { time } else { format!("{time} {half}") };
            lines.push(centered(Line::from(Span::styled(text, accent.add_modifier(Modifier::BOLD)))));
        }
        lines.push(centered(Line::from(Span::styled(now.format("%A, %B %-d").to_string(), theme.muted()))));
    }
    if settings.show_greeting {
        let greeting = daftar_core::home::greeting(&settings.greeting_text, chrono::Timelike::hour(&now));
        lines.push(centered(Line::from(Span::styled(greeting, theme.heading(1)))));
    }
    if settings.show_search {
        lines.push(centered(Line::default()));
        lines.push(centered(Line::from(vec![
            Span::styled(" ⌕ Find a note ", Style::new().fg(theme.muted).bg(theme.panel)),
            Span::styled(" p ", Style::new().fg(theme.accent).bg(theme.panel)),
        ])));
    }
    let mut at = 0;
    for (title, notes) in &home.lists {
        lines.push((Line::default(), None, false));
        let mark = if *title == "Recent" { "◷" } else { "★" };
        lines.push((
            Line::from(Span::styled(format!("{mark} {title}"), Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))),
            None,
            false,
        ));
        for path in notes {
            let selected = focused && at == home.selected;
            let mut spans = vec![
                Span::styled(if selected { "▌ " } else { "  " }, Style::new().fg(theme.accent)),
                Span::styled(Vault::title(path), Style::new().fg(theme.text).add_modifier(Modifier::BOLD)),
            ];
            let folder = QuickSwitcher::folder(path);
            if !folder.is_empty() {
                spans.push(Span::styled(format!("  {folder}"), theme.faint()));
            }
            let mut line = Line::from(spans);
            if selected {
                line = line.style(Style::new().bg(theme.panel));
            }
            lines.push((line, Some(at), false));
            at += 1;
        }
    }
    if let Some(quote) = &home.quote {
        lines.push(centered(Line::default()));
        lines.push(centered(Line::from(Span::styled(format!("“{}”", quote.text), theme.muted().add_modifier(Modifier::ITALIC)))));
        if !quote.author.is_empty() {
            lines.push(centered(Line::from(Span::styled(format!("— {}", quote.author), theme.faint()))));
        }
    }
    lines.push(centered(Line::default()));
    lines.push(centered(Line::from(vec![
        Span::styled("H ", Style::new().fg(theme.accent)),
        Span::styled("home  ", theme.faint()),
        Span::styled("? ", Style::new().fg(theme.accent)),
        Span::styled("every key", theme.faint()),
    ])));
    // The lists in a column in the middle, as wide as the widest of them.
    let column = lines
        .iter()
        .filter(|(_, _, centered)| !centered)
        .map(|(line, _, _)| line.width() as u16)
        .max()
        .unwrap_or(0)
        .clamp(24.min(area.width), area.width);
    let left = area.x + (area.width - column) / 2;
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    let mut rows = vec![None; area.height as usize];
    for (offset, (line, note, centered)) in lines.into_iter().enumerate() {
        let y = top + offset as u16;
        if y >= area.bottom() {
            break;
        }
        rows[(y - area.y) as usize] = note;
        let rect = if centered { Rect { y, height: 1, ..area } } else { Rect { x: left, y, width: column, height: 1 } };
        let alignment = if centered { Alignment::Center } else { Alignment::Left };
        frame.render_widget(Paragraph::new(line).alignment(alignment), rect);
    }
    app.areas.home_rows = rows;
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
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
    let mut left = vec![
        Span::styled(" daftar ", Style::new().fg(theme.background).bg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} ", app.vault_name()), Style::new().fg(theme.text).bg(theme.panel)),
        Span::raw(" "),
    ];
    if let Some((doing, since)) = &app.busy {
        let frame = SPINNER[(since.elapsed().as_millis() / 80) as usize % SPINNER.len()];
        left.push(Span::styled(format!("{frame} {doing}…"), Style::new().fg(theme.accent)));
        left.push(Span::raw("  "));
    }
    if let Some((text, error)) = &app.message {
        left.push(Span::styled(text.clone(), Style::new().fg(if *error { theme.red } else { theme.em })));
    } else {
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
        // Whole hints, as many as fit beside the git status; "? help" last.
        let room = (area.width as usize).saturating_sub(right.width() + Line::from(left.clone()).width() + 1);
        let cost = |first: bool, (key, what): (&str, &str)| if first { 0 } else { 3 } + key.len() + 1 + what.len();
        let help = ("?", "help");
        let mut shown = Vec::new();
        let mut used = 0;
        for hint in hints {
            let width = cost(shown.is_empty(), hint);
            if used + width + cost(false, help) > room {
                break;
            }
            used += width;
            shown.push(hint);
        }
        if used + cost(shown.is_empty(), help) <= room {
            shown.push(help);
        }
        for (n, (key, what)) in shown.into_iter().enumerate() {
            if n > 0 {
                left.push(Span::styled(" · ", theme.faint()));
            }
            left.push(Span::styled(key, Style::new().fg(theme.accent)));
            left.push(Span::styled(format!(" {what}"), theme.muted()));
        }
    }
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

/// Draws the list; returns its frame, its rows, and the item on each row.
fn draw_picker(frame: &mut Frame, theme: &Theme, picker: &mut Picker, area: Rect) -> (Rect, Rect, Vec<usize>) {
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
    let mut items = Vec::new();
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
        items.push(at);
        if let Some(detail) = &item.detail {
            let mut detail = detail.clone();
            detail.spans.insert(0, Span::raw(if selected { "▌ " } else { "  " }).style(Style::new().fg(theme.accent)));
            if selected {
                detail = detail.style(Style::new().bg(theme.panel));
            }
            lines.push(detail);
            items.push(at);
        }
    }
    items.truncate(height);
    frame.render_widget(Paragraph::new(lines), list);
    (rect, list, items)
}

pub const KEYS: [(&str, &str); 27] = [
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
    ("H", "home: the notes read last, the bookmarks"),
    ("Space", "tick the task, or open it again"),
    ("Click", "open; tick a box; again on a line: follow"),
    ("Wheel", "scroll the tree, the note or the list"),
    ("e", "edit the note in $EDITOR"),
    ("s", "sync: commit, pull, push"),
    ("r", "read the vault again"),
    (".", "show the hidden files, or hide them again"),
    ("?", "this help"),
    ("Esc", "close the list"),
    ("q  Ctrl+C", "quit"),
    ("", ""),
    ("", "A tick refuses when the line changed on disk."),
    ("", "A change on disk shows as it lands."),
];

fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect) -> Rect {
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
    rect
}
