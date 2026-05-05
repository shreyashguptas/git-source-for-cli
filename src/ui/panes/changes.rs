use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{is_fully_staged, App, ChangeAction, InputMode, Pane},
    git::{ChangeKind, FileChange},
    ui::{file_icon::file_icon, theme::Theme, toolbar},
};

/// Toolbar definition. Order is what the user sees left-to-right.
// Grouped left-to-right: commit actions · staging actions · inspect/recovery · utility.
const BUTTONS: &[(&str, ChangeAction)] = &[
    ("✓ commit", ChangeAction::Commit),
    ("⇡ commit & push", ChangeAction::CommitAndPush),
    ("✨ generate message", ChangeAction::AiMessage),
    ("+ stage all", ChangeAction::StageAll),
    ("− unstage all", ChangeAction::UnstageAll),
    ("≣ view all", ChangeAction::ViewAll),
    ("↶ uncommit", ChangeAction::Uncommit),
    ("↻ refresh", ChangeAction::Refresh),
];

pub fn render(app: &mut App, area: Rect, frame: &mut Frame, theme: &Theme) {
    let active = app.active_pane == Pane::Changes;
    let title = format!(" Changes ({}) ", app.status.files.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.block_border(active))
        .title(Span::styled(title, theme.title(active)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Pack toolbar — always shows every button, wrapping to new rows when narrow.
    let packed = toolbar::pack(BUTTONS, inner);
    let toolbar_h = toolbar::rows_used(&packed, inner);

    if inner.height < toolbar_h + 1 {
        // Pane too short — skip toolbar + per-row buttons; render the bare list.
        app.change_button_rects.clear();
        app.change_file_button_rects.clear();
        app.last_rects.changes_list = inner;
        render_list(app, inner, frame, theme, /* with_buttons */ false);
        return;
    }

    let toolbar_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: toolbar_h,
    };
    let mut remaining = Rect {
        x: inner.x,
        y: inner.y + toolbar_h,
        width: inner.width,
        height: inner.height - toolbar_h,
    };

    let bg_style = Style::default().bg(theme.bg).fg(theme.fg_dim);
    toolbar::render(&packed, button_style, bg_style, toolbar_area, frame);
    app.change_button_rects = packed.iter().map(|p| (p.action, p.rect)).collect();

    // Inline commit-message section, between the toolbar and the file list,
    // mirrors VS Code's source-control text box. Only rendered while the
    // user is actually composing (input_mode == Commit) so it doesn't take
    // permanent vertical space when not in use.
    if app.input_mode == InputMode::Commit && remaining.height >= 4 {
        let commit_h: u16 = 3; // top border + content + bottom border
        let commit_area = Rect {
            x: remaining.x,
            y: remaining.y,
            width: remaining.width,
            height: commit_h,
        };
        render_commit_box(app, commit_area, frame, theme);
        remaining = Rect {
            x: remaining.x,
            y: remaining.y + commit_h,
            width: remaining.width,
            height: remaining.height - commit_h,
        };
    }

    app.last_rects.changes_list = remaining;
    render_list(app, remaining, frame, theme, /* with_buttons */ true);
    // After the list draws (and applies its highlight to the selected row),
    // overlay the per-row +/− chips so they keep their own bg color even on
    // the highlighted row. This is what "make the green chip stay green when
    // the row is selected" looks like in code.
    overlay_file_chips(app, frame);
}

fn render_commit_box(app: &App, area: Rect, frame: &mut Frame, theme: &Theme) {
    // Background panel (single-line content surrounded by a thin border).
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(
            if app.input.generating {
                " ✨ generating commit message — Esc to cancel "
            } else if app.input.push_after {
                " commit (then push) — Enter to apply · Esc to cancel "
            } else {
                " commit — Enter to apply · Esc to cancel · ^G regenerate "
            },
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Compose the inline message line with caret.
    let buf = &app.input.buf;
    let cursor = app.input.cursor.min(buf.len());
    let (left, right) = buf.split_at(cursor);
    let body = if app.input.generating {
        format!(" {buf}▌ ")
    } else {
        format!(" {left}▏{right} ")
    };
    let style = Style::default().fg(theme.fg).bg(theme.selection_bg(true));

    // Reserve a 12-col tail for an "✨ regen" hint chip (only when not actively
    // generating). The visual cue that the message is editable-and-AI-aware.
    let hint = if app.input.generating {
        ""
    } else if buf.is_empty() {
        "  ^G ✨ ai"
    } else {
        ""
    };
    let hint_w = hint.width() as u16;

    let body_w = inner.width.saturating_sub(hint_w);
    let body_area = Rect {
        x: inner.x,
        y: inner.y,
        width: body_w,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(body, style)).style(style),
        body_area,
    );

    if hint_w > 0 {
        let hint_area = Rect {
            x: inner.x + body_w,
            y: inner.y,
            width: hint_w,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                hint,
                Style::default().fg(theme.fg_dim).bg(theme.selection_bg(false)),
            )),
            hint_area,
        );
    }
}

/// Paint the +/− stage-toggle chips on top of the just-rendered list, so the
/// row highlight from `List::highlight_style` doesn't bleed through their
/// colored backgrounds.
fn overlay_file_chips(app: &mut App, frame: &mut Frame) {
    let files = app.status.files.clone();
    for (idx, rect) in app.change_file_button_rects.clone() {
        let Some(f) = files.get(idx) else { continue };
        let staged = is_fully_staged(f);
        let glyph = if staged { " − " } else { " + " };
        let style = if staged {
            Style::default()
                .fg(Color::Rgb(0x10, 0x14, 0x18))
                .bg(Color::Rgb(0xE2, 0xA8, 0xA8))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Rgb(0x10, 0x14, 0x18))
                .bg(Color::Rgb(0xA8, 0xE0, 0xB6))
                .add_modifier(Modifier::BOLD)
        };
        // Force the bg by setting both the Span style AND the Paragraph style;
        // the latter fills any non-styled cells (defensive for narrow rects).
        frame.render_widget(
            Paragraph::new(Span::styled(glyph, style)).style(style),
            rect,
        );
    }
}

fn button_style(action: ChangeAction) -> Style {
    let ink = Color::Rgb(0x10, 0x14, 0x18);
    let bg = match action {
        ChangeAction::Commit => Color::Rgb(0x9E, 0xCB, 0xFF),       // light blue
        ChangeAction::CommitAndPush => Color::Rgb(0xC8, 0xA2, 0xE2), // soft purple
        ChangeAction::AiMessage => Color::Rgb(0xFF, 0xD8, 0x8E),    // peach (sparkles)
        ChangeAction::StageAll => Color::Rgb(0xA8, 0xE0, 0xB6),     // mint
        ChangeAction::UnstageAll => Color::Rgb(0xE2, 0xA8, 0xA8),   // pale red
        ChangeAction::Uncommit => Color::Rgb(0xFF, 0xB0, 0xB0),     // soft coral (destructive cue)
        ChangeAction::Refresh => Color::Rgb(0xB6, 0xC2, 0xE6),      // periwinkle
        ChangeAction::ViewAll => Color::Rgb(0xE5, 0xD0, 0x95),      // soft gold
    };
    Style::default().fg(ink).bg(bg).add_modifier(Modifier::BOLD)
}

/// Render the file list. When `with_buttons` is set, each row reserves a
/// trailing zone for the `+`/`−` toggle button (drawn later as an overlay,
/// not inline, so the row highlight doesn't override the chip color).
fn render_list(
    app: &mut App,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
    with_buttons: bool,
) {
    let active = app.active_pane == Pane::Changes;
    let files = &app.status.files;

    // Width budget for the chip column: 1-col gap + 3-col chip = 4 cols.
    const BTN_GAP: u16 = 1;
    const BTN_W: u16 = 3;
    const RESERVED: u16 = BTN_GAP + BTN_W;
    let content_width = area.width.saturating_sub(2) as usize; // -2 for left gutter + scroll padding
    let row_width_for_text = if with_buttons && !files.is_empty() {
        content_width.saturating_sub(RESERVED as usize)
    } else {
        content_width
    };

    let items: Vec<ListItem<'_>> = if files.is_empty() {
        vec![ListItem::new(Span::styled(
            " (working tree clean)",
            Style::default().fg(theme.fg_dim),
        ))]
    } else {
        files
            .iter()
            .map(|f| ListItem::new(file_line(f, row_width_for_text, theme)))
            .collect()
    };

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(theme.selection_bg(active))
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, area, &mut app.changes_state);

    // Compute per-row chip rects from the visible window. The List's offset
    // tells us which file index renders on the first visible row.
    app.change_file_button_rects.clear();
    if with_buttons && !files.is_empty() {
        let offset = app.changes_state.offset();
        let visible = (area.height as usize).min(files.len().saturating_sub(offset));
        for i in 0..visible {
            let idx = offset + i;
            let rect = Rect {
                x: area.x + area.width.saturating_sub(BTN_W),
                y: area.y + i as u16,
                width: BTN_W,
                height: 1,
            };
            app.change_file_button_rects.push((idx, rect));
        }
    }
}

fn file_line<'a>(
    f: &'a FileChange,
    content_width: usize,
    theme: &Theme,
) -> Line<'a> {
    let deleted = matches!(f.kind, ChangeKind::Deleted);
    let (name, parent) = split_path(&f.path);
    let (icon, icon_style) = file_icon(&f.path, theme);
    let badge = status_badge(f.kind, theme);

    let name_style = if deleted {
        Style::default()
            .fg(theme.deleted)
            .add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::default().fg(Color::Rgb(0xFF, 0xFF, 0xFF))
    };
    let parent_style = if deleted {
        Style::default()
            .fg(theme.deleted)
            .add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::default().fg(theme.fg_dim)
    };

    let icon_width = icon.width();
    let badge_width = badge.text.width();
    let max_left_width = content_width.saturating_sub(badge_width + 1);
    let icon_gap_width = if max_left_width > icon_width { 1 } else { 0 };
    let max_name_width = max_left_width.saturating_sub(icon_width + icon_gap_width);
    let shown_name = truncate_end(name, max_name_width);
    let mut left_width = icon_width + icon_gap_width + shown_name.width();

    let shown_meta = file_meta(parent, f.from.as_deref()).and_then(|meta| {
        let remaining = max_left_width.saturating_sub(left_width);
        if remaining == 0 {
            None
        } else {
            Some(truncate_start(&meta, remaining))
        }
    });
    if let Some(meta) = &shown_meta {
        left_width += meta.width();
    }

    let mut spans = vec![
        Span::styled(icon, icon_style),
        Span::raw(" ".repeat(icon_gap_width)),
        Span::styled(shown_name, name_style),
    ];

    if let Some(meta) = shown_meta {
        spans.push(Span::styled(meta, parent_style));
    }

    let pad = content_width
        .saturating_sub(left_width + badge_width)
        .max(1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(badge.text, badge.style));

    Line::from(spans)
}

fn split_path(path: &str) -> (&str, Option<&str>) {
    path.rsplit_once('/')
        .map(|(parent, name)| (name, Some(parent)))
        .unwrap_or((path, None))
}

fn file_meta(parent: Option<&str>, from: Option<&str>) -> Option<String> {
    match (parent, from) {
        (Some(parent), Some(from)) => Some(format!(" {parent} ← {from}")),
        (Some(parent), None) => Some(format!(" {parent}")),
        (None, Some(from)) => Some(format!(" ← {from}")),
        (None, None) => None,
    }
}

struct Badge {
    text: &'static str,
    style: Style,
}

fn status_badge(kind: ChangeKind, theme: &Theme) -> Badge {
    let (text, style) = match kind {
        ChangeKind::Modified => ("M", Style::default().fg(theme.modified)),
        ChangeKind::Added => ("A", Style::default().fg(theme.added)),
        ChangeKind::Deleted => (
            "D",
            Style::default()
                .fg(theme.deleted)
                .add_modifier(Modifier::BOLD | Modifier::CROSSED_OUT),
        ),
        ChangeKind::Renamed => ("R", Style::default().fg(theme.renamed)),
        ChangeKind::Copied => ("C", Style::default().fg(theme.renamed)),
        ChangeKind::Untracked => ("U", Style::default().fg(theme.added)),
        ChangeKind::Ignored => ("I", Style::default().fg(theme.fg_dim)),
        ChangeKind::Conflicted => (
            "!",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        ChangeKind::TypeChanged => ("T", Style::default().fg(theme.modified)),
        ChangeKind::Unknown => ("?", Style::default().fg(theme.fg_dim)),
    };
    Badge { text, style }
}

fn truncate_end(value: &str, max_width: usize) -> String {
    if value.width() <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    let limit = max_width - 1;
    for ch in value.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > limit {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('…');
    out
}

fn truncate_start(value: &str, max_width: usize) -> String {
    if value.width() <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut tail = Vec::new();
    let mut width = 0;
    let limit = max_width - 1;
    for ch in value.chars().rev() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > limit {
            break;
        }
        tail.push(ch);
        width += ch_width;
    }

    let mut out = String::from("…");
    out.extend(tail.into_iter().rev());
    out
}
