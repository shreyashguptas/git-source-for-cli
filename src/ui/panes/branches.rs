use std::path::PathBuf;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::{
    app::{App, BranchAction, Pane},
    gh::{Pr, PrState},
    git::Branch,
    ui::theme::Theme,
};

/// Toolbar definition. Order is what the user sees left-to-right.
const BUTTONS: &[(&str, BranchAction)] = &[
    ("checkout", BranchAction::Checkout),
    ("+ new", BranchAction::NewBranch),
    ("push", BranchAction::Push),
    ("pull", BranchAction::Pull),
    ("fetch", BranchAction::Fetch),
    ("merge", BranchAction::Merge),
    ("delete", BranchAction::Delete),
];

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Branches;

    // Render the bordered Block first; we then place the toolbar + list inside.
    let title = format!(" Branches ({}) ", app.branches.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.block_border(active))
        .title(Span::styled(title, theme.title(active)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Carve inner: 1 row for the toolbar, rest for the branches list.
    let toolbar_h: u16 = 1;
    if inner.height < toolbar_h + 1 {
        // Pane too short — skip toolbar entirely so the list still renders.
        app.branch_button_rects.clear();
        render_list(app, inner, frame, theme);
        return;
    }
    let toolbar_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: toolbar_h,
    };
    let list_area = Rect {
        x: inner.x,
        y: inner.y + toolbar_h,
        width: inner.width,
        height: inner.height - toolbar_h,
    };

    render_button_bar(app, toolbar_area, frame, theme);
    render_list(app, list_area, frame, theme);
}

/// Render the clickable toolbar of branch actions and record each button's
/// rect on `app.branch_button_rects` so the click handler can hit-test.
fn render_button_bar(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let mut button_rects: Vec<(BranchAction, Rect)> = Vec::with_capacity(BUTTONS.len());
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(BUTTONS.len() * 2);
    let mut x = area.x;
    let area_right = area.x + area.width;

    for (label, action) in BUTTONS {
        let text = format!(" {label} ");
        let w = text.chars().count() as u16;
        if x + w > area_right {
            break; // ran out of horizontal space — show what fits
        }
        spans.push(Span::styled(text, button_style(*action)));
        button_rects.push((
            *action,
            Rect {
                x,
                y: area.y,
                width: w,
                height: 1,
            },
        ));
        x += w;
        // 1-col gap between buttons; skip if there's no room.
        if x + 1 < area_right {
            spans.push(Span::raw(" "));
            x += 1;
        }
    }

    // Pad remainder so background colour doesn't bleed past last button.
    if x < area_right {
        spans.push(Span::styled(
            " ".repeat((area_right - x) as usize),
            Style::default().bg(theme.bg).fg(theme.fg_dim),
        ));
    }

    app.branch_button_rects = button_rects;
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Per-action button colour. Same family as the rest of the pill palette so
/// it reads as part of the same UI.
fn button_style(action: BranchAction) -> Style {
    let ink = Color::Rgb(0x10, 0x14, 0x18);
    let bg = match action {
        BranchAction::Checkout => Color::Rgb(0x9E, 0xCB, 0xFF), // light blue
        BranchAction::NewBranch => Color::Rgb(0xA8, 0xE0, 0xB6), // mint
        BranchAction::Push => Color::Rgb(0xC8, 0xA2, 0xE2),     // soft purple
        BranchAction::Pull => Color::Rgb(0xFF, 0xD8, 0x8E),     // peach
        BranchAction::Fetch => Color::Rgb(0xB6, 0xC2, 0xE6),    // periwinkle
        BranchAction::Merge => Color::Rgb(0x9E, 0xDB, 0xDB),    // pale teal
        BranchAction::Delete => Color::Rgb(0xFF, 0xB0, 0xB0),   // soft coral (destructive cue)
    };
    Style::default().bg(bg).fg(ink).add_modifier(Modifier::BOLD)
}

fn render_list(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
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
                let wt = app
                    .worktrees
                    .get(&b.name)
                    .filter(|p| !b.is_current && **p != app.repo.root);
                ListItem::new(branch_line(b, pr, wt, area.width, theme))
            })
            .collect()
    };

    let list = List::new(items)
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

    let track = format_track(b);
    let pr_chip = pr.map(pr_chip_text);
    let wt_hint = worktree.map(short_path);

    let right_text_len: usize = [
        track.chars().count(),
        pr_chip.as_ref().map(|c| c.text.chars().count()).unwrap_or(0),
        wt_hint.as_ref().map(|s| s.chars().count() + 4).unwrap_or(0),
    ]
    .iter()
    .filter(|n| **n > 0)
    .map(|n| *n + 1)
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

/// "↑3 push · ↓2 pull" — spell it out so the meaning is obvious at a glance.
/// Empty when there's no upstream OR everything is in sync.
fn format_track(b: &Branch) -> String {
    if b.upstream.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::with_capacity(2);
    if b.ahead > 0 {
        parts.push(format!("↑{} push", b.ahead));
    }
    if b.behind > 0 {
        parts.push(format!("↓{} pull", b.behind));
    }
    parts.join(" · ")
}

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
