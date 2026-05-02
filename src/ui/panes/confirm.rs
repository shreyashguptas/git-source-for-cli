//! Modal yes/no confirmation dialog.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    /// Tag identifying which action to perform on confirmation.
    pub action: ConfirmAction,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    DeleteBranch { name: String, force: bool },
    DiscardFile { path: String },
    MergeBranch { name: String },
}

pub fn render(dialog: &ConfirmDialog, area: Rect, frame: &mut Frame, theme: &Theme) {
    let modal = centered(area, 50, 30);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.error))
        .title(Span::styled(
            format!(" ⚠ {} ", dialog.title),
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = modal.inner(Margin { vertical: 1, horizontal: 2 });
    frame.render_widget(block, modal);

    let lines = vec![
        Line::from(Span::styled(
            dialog.message.as_str(),
            Style::default().fg(theme.fg),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [y] confirm  ",
                Style::default()
                    .bg(theme.error)
                    .fg(ratatui::style::Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                "  [n / Esc] cancel  ",
                Style::default()
                    .bg(theme.fg_dim)
                    .fg(ratatui::style::Color::Black),
            ),
        ]),
    ];

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(para, inner);
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
