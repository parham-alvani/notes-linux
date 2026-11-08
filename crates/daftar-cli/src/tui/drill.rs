//! `D`: a drill over the TUI, the same cards and schedule as `daftar-cli
//! drill` (`daftar_core::Drill`). The context and the question, Enter or
//! Space shows the answer, 1–4 rates it; a card rated Again comes back at
//! the end, its first rating kept. Each rating is saved at once.

use std::collections::VecDeque;
use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use daftar_core::{Card, Day, DrillState, Rating};

use super::markdown;
use super::theme::Theme;

pub struct DrillView {
    /// What is drilled: a note's title, or a folder.
    pub title: String,
    /// The card asked now, and whether it is asked again.
    pub card: Option<(Card, bool)>,
    queue: VecDeque<(Card, bool)>,
    pub revealed: bool,
    /// The answer's first row on screen.
    pub scroll: usize,
    pub state: DrillState,
    /// Where the state is saved: Daftar's state file.
    pub file: PathBuf,
    pub total: usize,
    pub asked: usize,
    pub rated: usize,
    /// What the last rating did, or why it was not saved.
    pub said: Option<String>,
}

impl DrillView {
    pub fn new(title: String, cards: Vec<Card>, state: DrillState, file: PathBuf) -> DrillView {
        let total = cards.len();
        let mut view = DrillView {
            title,
            card: None,
            queue: cards.into_iter().map(|c| (c, false)).collect(),
            revealed: false,
            scroll: 0,
            state,
            file,
            total,
            asked: 0,
            rated: 0,
            said: None,
        };
        view.next();
        view
    }

    fn next(&mut self) {
        self.card = self.queue.pop_front();
        self.revealed = false;
        self.scroll = 0;
        if let Some((_, false)) = self.card {
            self.asked += 1;
        }
    }

    pub fn reveal(&mut self) {
        self.revealed = self.card.is_some();
    }

    /// Rates the card shown, once its answer is.
    pub fn rate(&mut self, rating: Rating, today: Day) {
        if !self.revealed {
            return;
        }
        let Some((card, again)) = self.card.take() else { return };
        if !again {
            let review = self.state.rate(&card.id, rating, today);
            self.said = Some(match self.state.save(&self.file) {
                Ok(()) => format!("next on {}", review.due),
                Err(error) => format!("not saved: {error}"),
            });
            self.rated += 1;
        }
        if rating == Rating::Again {
            self.queue.push_back((card, true));
        }
        self.next();
    }

    pub fn scroll_by(&mut self, by: isize) {
        self.scroll = self.scroll.saturating_add_signed(by);
    }
}

/// Draws [view] over [area]; returns its frame.
pub fn draw(frame: &mut Frame, theme: &Theme, view: &mut DrillView, area: Rect) -> Rect {
    let width = (area.width * 8 / 10).max(50).min(area.width.saturating_sub(4));
    let height = (area.height * 8 / 10).max(12).min(area.height.saturating_sub(2));
    let rect = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 3, width, height };
    frame.render_widget(Clear, rect);
    let count = match &view.card {
        Some((_, true)) => "again".to_string(),
        Some(_) => format!("{}/{}", view.asked, view.total),
        None => format!("{} rated", view.rated),
    };
    let keys = match (&view.card, view.revealed) {
        (None, _) => " esc close ",
        (Some(_), false) => " enter show the answer · esc stop ",
        (Some(_), true) => " 1 again · 2 hard · 3 good · 4 easy · j k scroll · esc stop ",
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.accent))
        .padding(Padding::horizontal(1))
        .style(theme.base())
        .title(Line::from(Span::styled(format!(" Drill · {} ", view.title), Style::new().fg(theme.accent).add_modifier(Modifier::BOLD))))
        .title_bottom(Line::from(Span::styled(format!(" {count} "), theme.faint())).alignment(Alignment::Right))
        .title_bottom(Line::from(Span::styled(keys, theme.faint())));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    let Some((card, _)) = &view.card else {
        let done = if view.total == 0 { "Nothing is due.".to_string() } else { format!("Done: {} cards rated.", view.rated) };
        lines.push(Line::from(Span::styled(done, Style::new().fg(theme.text))));
        if let Some(said) = &view.said {
            lines.push(Line::from(Span::styled(said.clone(), theme.faint())));
        }
        frame.render_widget(Paragraph::new(lines), inner);
        return rect;
    };
    lines.push(Line::from(Span::styled(card.context.join(" › "), theme.muted())));
    lines.push(Line::from(Span::styled(card.question.clone(), theme.heading(1))));
    lines.push(Line::from(Span::styled(format!("{}:{}", card.path, card.line + 1), theme.faint())));
    lines.push(Line::default());
    if view.revealed {
        let blocks = markdown::blocks(&card.answer, &[], theme, &Day::today().to_string());
        let rows = markdown::wrap(&blocks, inner.width, theme);
        let room = (inner.height as usize).saturating_sub(lines.len());
        view.scroll = view.scroll.min(rows.len().saturating_sub(room));
        lines.extend(rows.into_iter().skip(view.scroll).take(room).map(|row| row.line));
    } else if let Some(said) = &view.said {
        lines.push(Line::from(Span::styled(said.clone(), theme.faint())));
    }
    frame.render_widget(Paragraph::new(lines), inner);
    rect
}
