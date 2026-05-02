use std::path::PathBuf;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::{
    app::{App, Pane},
    gh::{Pr, PrState},
    git::Branch,
    ui::theme::Theme,
};

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Branches;

    let items: Vec<ListItem<'_>> = if app.branches.is_empty() {
        vec![ListItem::new(Span::styled(
            " (loading branches…)",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        app.branches
            .iter()
            .map(|b| {
                let pr = app.prs.get(&b.name);
                // Worktree info: which worktree owns this branch (if any).
                // Skip the current repo root because that's just `is_current`.
                let wt = app
                    .worktrees
                    .get(&b.name)
                    .filter(|p| !b.is_current && **p != app.repo.root);
                ListItem::new(branch_line(b, pr, wt, area.width, theme))
            })
            .collect()
    };

    let title = format!(" Branches ({}) ", app.branches.len());
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
        )
        .highlight_symbol(">");

    frame.render_stateful_widget(list, area, &mut app.branches_state);
}

fn branch_line<'a>(
    b: &'a Branch,
    pr: Option<&'a Pr>,
    worktree: Option<&'a PathBuf>,
    width: u16,
    theme: &Theme,
) -> Line<'a> {
    // Prefix glyph: `*` current, `⎘` checked out in another worktree, ` ` else.
    let (marker, marker_style) = if b.is_current {
        (
            "* ",
            Style::default()
                .fg(theme.branch_current)
                .add_modifier(Modifier::BOLD),
        )
    } else if worktree.is_some() {
        (
            "⎘ ",
            Style::default()
                .fg(theme.modified)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("  ", Style::default().fg(theme.fg_dim))
    };

    let name_style = if b.is_current {
        Style::default()
            .fg(theme.branch_current)
            .add_modifier(Modifier::BOLD)
    } else if worktree.is_some() {
        // Branches locked to another worktree: dim and italic so they read as
        // "you can't check this out from here".
        Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(theme.fg)
    };

    let mut spans = vec![
        Span::styled(marker, marker_style),
        Span::styled(b.name.as_str(), name_style),
    ];

    // Compose right-side text: track + PR chip + worktree hint
    let track = format_track(b);
    let pr_chip = pr.map(pr_chip_text);
    let wt_hint = worktree.map(|p| short_path(p));

    let right_text_len: usize = [
        track.chars().count(),
        pr_chip.as_ref().map(|c| c.text.chars().count()).unwrap_or(0),
        wt_hint.as_ref().map(|s| s.chars().count() + 4).unwrap_or(0), // " wt:"
    ]
    .iter()
    .filter(|n| **n > 0)
    .map(|n| *n + 1) // separators
    .sum();

    let used = 2 + b.name.chars().count() + right_text_len;
    let pad = (width as usize).saturating_sub(used + 3);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    } else {
        spans.push(Span::raw(" "));
    }

    if let Some(s) = wt_hint {
        spans.push(Span::styled(
            format!("⎘ {s}"),
            Style::default()
                .fg(theme.modified)
                .add_modifier(Modifier::DIM),
        ));
        spans.push(Span::raw(" "));
    }

    if !track.is_empty() {
        spans.push(Span::styled(track, Style::default().fg(theme.accent)));
        if pr_chip.is_some() {
            spans.push(Span::raw(" "));
        }
    }
    if let Some(chip) = pr_chip {
        spans.push(Span::styled(chip.text, chip.style));
    }
    Line::from(spans)
}

fn format_track(b: &Branch) -> String {
    let mut s = String::new();
    if b.upstream.is_none() {
        return s;
    }
    if b.ahead > 0 {
        s.push_str(&format!("↑{}", b.ahead));
    }
    if b.behind > 0 {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("↓{}", b.behind));
    }
    s
}

/// Turn an absolute worktree path into a compact form for the branches pane.
/// Show last 2 path components: `…/repo-foo/wt-bar`. Avoids leaking $HOME.
fn short_path(p: &PathBuf) -> String {
    let comps: Vec<_> = p.components().collect();
    let n = comps.len();
    if n <= 2 {
        return p.display().to_string();
    }
    let last_two = comps[n - 2..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    format!("…/{last_two}")
}

struct Chip {
    text: String,
    style: Style,
}

fn pr_chip_text(pr: &Pr) -> Chip {
    use ratatui::style::Color;
    let (label_prefix, fg) = match pr.state {
        PrState::Open => ("PR", Color::Rgb(0x73, 0xC9, 0x91)),
        PrState::Draft => ("PR", Color::Rgb(0x80, 0x80, 0x80)),
        PrState::Merged => ("PR", Color::Rgb(0xC5, 0x86, 0xC0)),
        PrState::Closed => ("PR", Color::Rgb(0x80, 0x80, 0x80)),
        PrState::ChangesRequested => ("PR", Color::Rgb(0xF4, 0x47, 0x47)),
    };
    Chip {
        text: format!("{label_prefix}#{}", pr.number),
        style: Style::default().fg(fg).add_modifier(Modifier::BOLD),
    }
}
