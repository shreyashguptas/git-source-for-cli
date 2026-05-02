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
                ListItem::new(branch_line(b, pr, area.width, theme))
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

fn branch_line<'a>(b: &'a Branch, pr: Option<&'a Pr>, width: u16, theme: &Theme) -> Line<'a> {
    let marker = if b.is_current { "* " } else { "  " };
    let marker_style = if b.is_current {
        Style::default()
            .fg(theme.branch_current)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    let name_style = if b.is_current {
        Style::default()
            .fg(theme.branch_current)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg)
    };

    let mut spans = vec![
        Span::styled(marker, marker_style),
        Span::styled(b.name.as_str(), name_style),
    ];

    let track = format_track(b);
    let pr_chip = pr.map(pr_chip_text);
    let right_text = match (track.as_str(), pr_chip.as_ref()) {
        ("", None) => String::new(),
        ("", Some(c)) => c.text.clone(),
        (t, None) => t.to_string(),
        (t, Some(c)) => format!("{t} {}", c.text),
    };

    let used = 2 + b.name.chars().count() + right_text.chars().count();
    let pad = (width as usize).saturating_sub(used + 3);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    } else {
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
