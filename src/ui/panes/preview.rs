//! Inline live-diff preview pane (third column).
//!
//! Auto-updates as the user moves the selection in the Graph or Changes pane.
//! Shares diff line styling with [`super::details`].

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use crate::{
    app::{App, Pane},
    ui::{panes::details::lines_from_content_padded, theme::Theme},
};

pub fn render(app: &App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let title_text = match (&app.preview, app.active_pane) {
        (Some(c), _) => format!(" Preview · {} ", truncate(c.title(), 60)),
        (None, Pane::Branches) => " Preview · select a commit or file ".to_string(),
        (None, Pane::Changes) => " Preview · (no file selected) ".to_string(),
        (None, Pane::Graph) => " Preview · (no commit selected) ".to_string(),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            title_text,
            Style::default().fg(theme.fg_dim).add_modifier(Modifier::BOLD),
        ));
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line<'_>> = match &app.preview {
        Some(c) => lines_from_content_padded(c, theme, inner_area.width),
        None => vec![Line::from(Span::styled(
            hint_text(app.active_pane),
            Style::default().fg(theme.fg_dim),
        ))],
    };

    let total = lines.len();
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));
    frame.render_widget(para, inner_area);

    // Scrollbar on the right edge of the inner area.
    if total > inner_area.height as usize {
        let mut sb_state = ScrollbarState::new(total).position(app.preview_scroll as usize);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme.fg_dim));
        frame.render_stateful_widget(sb, inner_area, &mut sb_state);
    }
}

fn hint_text(pane: Pane) -> &'static str {
    match pane {
        Pane::Graph => "Use ↑↓/jk to scroll commits — preview updates automatically.\nPress Enter for the full-screen view.",
        Pane::Changes => "Use ↑↓/jk to browse files — preview updates automatically.\nSpace stages, c starts a commit.",
        Pane::Branches => "Branch selection updates the Graph pane. Switch to Graph or Changes for diff previews.",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
