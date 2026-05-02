use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::{
    app::{App, Pane},
    graph,
    ui::theme::Theme,
};

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Graph;

    let items: Vec<ListItem<'_>> = if app.commits.is_empty() {
        vec![ListItem::new(Span::styled(
            " (loading history…)",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        // Recompute layout each render. Cheap relative to git fetch; if we
        // notice slowness in huge repos, cache by commits' first-hash.
        let rows = graph::layout(&app.commits);
        rows.iter()
            .map(|row| {
                let commit = &app.commits[row.commit_idx];
                let is_head = commit.refs.iter().any(|r| {
                    matches!(
                        r,
                        crate::git::RefName::HeadAt(_) | crate::git::RefName::Head
                    )
                });
                let spans = graph::row_spans(row, commit, theme, is_head);
                ListItem::new(Line::from(spans))
            })
            .collect()
    };

    let title = format!(" Graph ({} commits) ", app.commits.len());
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

    frame.render_stateful_widget(list, area, &mut app.graph_state);
}
