use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::{
    app::{App, Pane},
    git::HeadRef,
    graph,
    ui::theme::Theme,
};

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Graph;

    // Build title + items first so the immutable borrow of `app` ends before
    // we render the stateful widget (which needs a mutable borrow of
    // `app.graph_state`).
    let title = build_title(app);
    let items: Vec<ListItem<'_>> = if app.commits.is_empty() {
        vec![ListItem::new(Span::styled(
            " (loading history…)",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        let rows = graph::layout(&app.commits);
        let commits = &app.commits;
        let ahead = &app.ahead_shas;
        let behind = &app.behind_shas;
        rows.iter()
            .map(|row| {
                let commit = &commits[row.commit_idx];
                let is_head = commit.refs.iter().any(|r| {
                    matches!(
                        r,
                        crate::git::RefName::HeadAt(_) | crate::git::RefName::Head
                    )
                });
                let mut spans = graph::row_spans(row, commit, theme, is_head);
                if let Some(m) = divergence_marker_static(&commit.hash, ahead, behind, theme) {
                    spans.insert(0, m);
                }
                ListItem::new(Line::from(spans))
            })
            .collect()
    };
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

/// Build the graph title with the VS Code-style divergence summary:
/// `Graph · main · ↑3 to push · ↓0 to pull · 12 commits`
fn build_title(app: &App) -> String {
    let branch = match &app.head {
        HeadRef::Branch(b) => Some(b.as_str()),
        HeadRef::Detached(_) => Some("(detached)"),
        HeadRef::Unborn => None,
    };
    let upstream = app
        .branches
        .iter()
        .find(|b| matches!(&app.head, HeadRef::Branch(name) if name == &b.name))
        .and_then(|b| b.upstream.as_deref());

    let total = app.commits.len();
    let ahead = app.ahead_shas.len();
    let behind = app.behind_shas.len();

    match (branch, upstream) {
        (Some(b), Some(up)) => {
            let mut sync_part = String::new();
            if ahead == 0 && behind == 0 {
                sync_part.push_str("in sync");
            } else {
                if ahead > 0 {
                    sync_part.push_str(&format!("↑{ahead} to push"));
                }
                if behind > 0 {
                    if !sync_part.is_empty() {
                        sync_part.push_str(" · ");
                    }
                    sync_part.push_str(&format!("↓{behind} to pull"));
                }
            }
            format!(" Graph · {b} ↔ {up} · {sync_part} · {total} commits ")
        }
        (Some(b), None) => format!(" Graph · {b} (no upstream) · {total} commits "),
        (None, _) => format!(" Graph · {total} commits "),
    }
}

/// Marker glyph shown to the LEFT of each commit's lane glyphs:
///   ↑  this commit is local-only (ahead of origin)
///   ↓  this commit exists on origin only (behind — usually only after fetch)
///   space otherwise
fn divergence_marker_static<'a>(
    sha: &str,
    ahead: &std::collections::HashSet<String>,
    behind: &std::collections::HashSet<String>,
    theme: &Theme,
) -> Option<Span<'a>> {
    if ahead.contains(sha) {
        return Some(Span::styled(
            "↑ ".to_string(),
            Style::default()
                .fg(theme.added)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if behind.contains(sha) {
        return Some(Span::styled(
            "↓ ".to_string(),
            Style::default()
                .fg(theme.modified)
                .add_modifier(Modifier::BOLD),
        ));
    }
    // 2-char filler so all commit rows align horizontally.
    Some(Span::styled("  ".to_string(), Style::default()))
}
