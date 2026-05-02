//! Generic info / error popup. Used wherever an op outcome is important
//! enough that the user shouldn't risk missing it (e.g. "branch can't be
//! deleted because it's checked out at a worktree"). The modal owns the
//! screen so the message stays visible until explicitly dismissed.
//!
//! This replaces toast-bar errors for any case where the message has
//! diagnostic value — toasts are reserved for routine, fire-and-forget
//! confirmations.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoSeverity {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct InfoDialog {
    pub title: String,
    pub message: String,
    /// Optional second paragraph offering a likely next step. Rendered
    /// dimmed beneath the main message so the user has somewhere to go.
    pub hint: Option<String>,
    pub severity: InfoSeverity,
}

impl InfoDialog {
    pub fn error(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            hint: None,
            severity: InfoSeverity::Error,
        }
    }

    pub fn info(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            hint: None,
            severity: InfoSeverity::Info,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

pub fn render(dialog: &InfoDialog, area: Rect, frame: &mut Frame, theme: &Theme) {
    let modal = centered(area, 60, 50);
    frame.render_widget(Clear, modal);

    let (accent, glyph) = match dialog.severity {
        InfoSeverity::Error => (theme.error, "✗"),
        InfoSeverity::Info => (theme.accent, "ⓘ"),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            format!(" {glyph}  {} ", dialog.title),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let inner = inner.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    // Reserve the bottom row for the dismiss hint; everything above is content.
    let footer_h: u16 = 1;
    let body_h = inner.height.saturating_sub(footer_h + 1);
    let body_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: body_h,
    };

    // Main message — wrapped, primary color, never truncated.
    let mut lines: Vec<Line<'_>> = Vec::new();
    for line in dialog.message.lines() {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )));
    }

    if let Some(hint) = &dialog.hint {
        // Blank separator + hint paragraph.
        lines.push(Line::from(Span::raw("")));
        for line in hint.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(theme.fg_dim)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(body, body_area);

    let footer_y = inner.y + inner.height.saturating_sub(1);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "Esc",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            "Enter",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" dismiss", Style::default().fg(theme.fg_dim)),
    ]));
    frame.render_widget(
        footer,
        Rect {
            x: inner.x,
            y: footer_y,
            width: inner.width,
            height: 1,
        },
    );
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
