//! Modal overlay for diff/commit details viewing.
//! Renders a full-screen panel with a scrollable colored diff.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use crate::{git::diff::{classify, DiffLineKind}, ui::theme::Theme};

/// What's being shown in the overlay.
#[derive(Debug, Clone)]
pub enum DetailsContent {
    /// Loading state — the body fetch is in flight.
    Loading { title: String },
    /// Loaded diff/show output.
    Body { title: String, body: String },
    /// Error during load.
    Error { title: String, message: String },
}

impl DetailsContent {
    pub fn title(&self) -> &str {
        match self {
            DetailsContent::Loading { title }
            | DetailsContent::Body { title, .. }
            | DetailsContent::Error { title, .. } => title,
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            DetailsContent::Body { body, .. } => body.lines().count(),
            _ => 1,
        }
    }
}

pub fn render(
    content: &DetailsContent,
    scroll: u16,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
) {
    // Carve the modal: 90% width, 90% height, centered.
    let modal = centered(area, 90, 90);

    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            format!(" {} (Esc close · j/k scroll · g/G top/bottom) ", content.title()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = modal.inner(Margin { vertical: 1, horizontal: 2 });
    frame.render_widget(block, modal);

    let lines: Vec<Line<'_>> = match content {
        DetailsContent::Loading { .. } => vec![Line::from(Span::styled(
            "loading…",
            Style::default().fg(theme.fg_dim),
        ))],
        DetailsContent::Error { message, .. } => vec![Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(theme.error),
        ))],
        DetailsContent::Body { body, .. } => body
            .lines()
            .map(|l| {
                let kind = classify(l);
                Line::from(Span::styled(l, line_style(kind, theme)))
            })
            .collect(),
    };

    let total = lines.len();
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(para, inner);

    // Scrollbar
    if total > inner.height as usize {
        let mut sb_state = ScrollbarState::new(total).position(scroll as usize);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme.fg_dim));
        frame.render_stateful_widget(
            sb,
            modal.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut sb_state,
        );
    }
}

fn line_style(kind: DiffLineKind, theme: &Theme) -> Style {
    match kind {
        DiffLineKind::Add => Style::default().fg(theme.added),
        DiffLineKind::Del => Style::default().fg(theme.deleted),
        DiffLineKind::Hunk => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::Header => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        DiffLineKind::Meta => Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::ITALIC),
        DiffLineKind::CommitMeta => Style::default()
            .fg(theme.modified)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::Stat => Style::default().fg(theme.fg_dim),
        DiffLineKind::Context => Style::default().fg(theme.fg),
        DiffLineKind::Other => Style::default().fg(theme.fg),
    }
}

fn centered(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let h = (area.height as u32 * percent_y as u32 / 100) as u16;
    let w = (area.width as u32 * percent_x as u32 / 100) as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}
