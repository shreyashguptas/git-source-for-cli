use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::{
    app::{App, Pane},
    git::{ChangeKind, FileChange},
    ui::theme::Theme,
};

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Changes;
    let files = &app.status.files;

    let items: Vec<ListItem<'_>> = if files.is_empty() {
        vec![ListItem::new(Span::styled(
            " (working tree clean)",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        files.iter().map(|f| ListItem::new(file_line(f, theme))).collect()
    };

    let title = format!(" Changes ({}) ", files.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.block_border(active))
                .title(Span::styled(title, theme.title(active))),
        )
        .highlight_style(
            Style::default()
                .bg(theme.selection_bg(active))
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut app.changes_state);
}

fn file_line<'a>(f: &'a FileChange, theme: &Theme) -> Line<'a> {
    // Indicator dot: filled if anything is staged, hollow otherwise.
    let staged = f.staged.is_some() && !matches!(f.kind, ChangeKind::Untracked | ChangeKind::Ignored);
    let dot = if staged { "● " } else { "○ " };
    let dot_style = if staged {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.fg_dim)
    };

    let glyph = f.kind.glyph();
    let glyph_style = match f.kind {
        ChangeKind::Modified => Style::default().fg(theme.modified),
        ChangeKind::Added | ChangeKind::Untracked => {
            Style::default().fg(theme.added)
        }
        ChangeKind::Deleted => Style::default().fg(theme.deleted),
        ChangeKind::Renamed | ChangeKind::Copied => {
            Style::default().fg(theme.renamed)
        }
        ChangeKind::Conflicted => Style::default().fg(theme.error).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(theme.fg_dim),
    };

    let mut spans = vec![
        Span::styled(dot, dot_style),
        Span::styled(format!("{glyph} "), glyph_style),
        Span::styled(f.path.as_str(), Style::default().fg(theme.fg)),
    ];

    if let Some(from) = &f.from {
        spans.push(Span::styled(
            format!("  ← {from}"),
            Style::default().fg(theme.fg_dim),
        ));
    }

    Line::from(spans)
}
