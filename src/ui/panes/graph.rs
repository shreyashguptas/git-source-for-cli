use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::{
    app::{App, Pane},
    graph,
    ui::theme::Theme,
};

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Graph;
    let title = build_title(app);

    // Render the bordered block; everything below paints inside `inner`.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.block_border(active))
        .title(Span::styled(title, theme.title(active)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.last_rects.graph_list = inner;

    if app.commits.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            " (loading history…)",
            Style::default().fg(theme.fg_dim),
        ));
        frame.render_widget(placeholder, inner);
        return;
    }

    // Lane layout is O(N) over commits; cache it across frames so scrolling
    // doesn't recompute on every keystroke. Invalidated when commits reload
    // (see `App::graph_layout = None` in app.rs).
    if app.graph_layout.is_none() {
        app.graph_layout = Some(graph::layout(&app.commits));
    }

    // Body width available to `row_spans` after subtracting the 2-cell
    // divergence marker prepended below. (Borders are already excluded by
    // `block.inner`.)
    const MARKER_WIDTH: usize = 2;
    let body_width = (inner.width as usize).saturating_sub(MARKER_WIDTH);

    let viewport = inner.height as usize;
    let total = app.commits.len();
    if viewport == 0 || total == 0 {
        return;
    }

    let selected = app
        .graph_state
        .selected()
        .unwrap_or(0)
        .min(total.saturating_sub(1));

    // Manage scroll offset ourselves: we don't pass items to `List`, so
    // ratatui's auto-scroll doesn't apply. Mirror its behavior — keep
    // `selected` inside [offset, offset+viewport).
    {
        let offset = app.graph_state.offset_mut();
        if *offset > selected {
            *offset = selected;
        }
        if selected >= *offset + viewport {
            *offset = selected + 1 - viewport;
        }
        // Avoid leaving blank rows at the bottom when total < offset+viewport
        // would scroll past the last row.
        let max_offset = total.saturating_sub(viewport);
        if *offset > max_offset {
            *offset = max_offset;
        }
    }
    let offset = app.graph_state.offset();
    let end = (offset + viewport).min(total);

    let rows = app.graph_layout.as_ref().expect("just populated");
    let max_lanes = rows.iter().map(graph::lane_count).max().unwrap_or(1);
    let commits = &app.commits;
    let ahead = &app.ahead_shas;
    let behind = &app.behind_shas;

    // Construct spans ONLY for the visible window — this is what makes the
    // pane scale to thousands of commits without per-frame O(N) span work.
    let highlight = Style::default()
        .bg(theme.selection_bg(active))
        .add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(end - offset);
    for i in offset..end {
        let row = &rows[i];
        let commit = &commits[row.commit_idx];
        let is_head = commit.refs.iter().any(|r| {
            matches!(
                r,
                crate::git::RefName::HeadAt(_) | crate::git::RefName::Head
            )
        });
        let mut spans = graph::row_spans(row, commit, theme, is_head, max_lanes, body_width);
        if let Some(m) = divergence_marker_static(&commit.hash, ahead, behind, theme) {
            spans.insert(0, m);
        }
        let mut line = Line::from(spans);
        if i == selected {
            line = line.style(highlight);
        }
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Build the graph title with the VS Code-style divergence summary:
/// `Graph · main · ↑3 to push · ↓0 to pull · 12 commits`
fn build_title(app: &App) -> String {
    let branch = graph_branch_label(app);
    let upstream = app.graph_upstream.as_deref();

    let total = app.commits.len();
    let ahead = app.ahead_shas.len();
    let behind = app.behind_shas.len();
    // `5000+` when we hit the load cap — disclose that there's more history.
    let count = if app.graph_truncated {
        format!("{total}+ commits")
    } else {
        format!("{total} commits")
    };

    match (branch.as_deref(), upstream) {
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
            format!(" Graph · {b} ↔ {up} · {sync_part} · {count} ")
        }
        (Some(b), None) => format!(" Graph · {b} (no upstream) · {count} "),
        (None, _) => format!(" Graph · {count} "),
    }
}

fn graph_branch_label(app: &App) -> Option<String> {
    let branch = app.graph_branch.as_deref()?;
    if app.graph_root != app.repo.root {
        let worktree = app
            .graph_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("worktree");
        return Some(format!("{branch} @ {worktree}"));
    }
    if matches!(&app.head, crate::git::HeadRef::Branch(current) if current == branch) {
        Some(branch.to_string())
    } else {
        Some(format!("{branch} (selected)"))
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
