use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
        let content_width = area.width.saturating_sub(2) as usize;
        files
            .iter()
            .map(|f| ListItem::new(file_line(f, content_width, theme)))
            .collect()
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

fn file_line<'a>(f: &'a FileChange, content_width: usize, theme: &Theme) -> Line<'a> {
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

fn file_icon(path: &str, theme: &Theme) -> (&'static str, Style) {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");

    let (icon, color) = match name {
        "cargo.toml" | "cargo.lock" => ("", Color::Rgb(0xDE, 0xA5, 0x84)),
        "dockerfile" => ("", Color::Rgb(0x24, 0x96, 0xED)),
        "makefile" => ("", Color::Rgb(0xB8, 0xB8, 0xB8)),
        _ => match ext {
            "rs" => ("", Color::Rgb(0xDE, 0xA5, 0x84)),
            "ts" => ("", Color::Rgb(0x31, 0x78, 0xC6)),
            "tsx" => ("", Color::Rgb(0x61, 0xDA, 0xFB)),
            "js" | "mjs" | "cjs" => ("", Color::Rgb(0xF7, 0xDF, 0x1E)),
            "jsx" => ("", Color::Rgb(0x61, 0xDA, 0xFB)),
            "json" => ("", Color::Rgb(0xF7, 0xDF, 0x1E)),
            "html" | "htm" => ("", Color::Rgb(0xE3, 0x4C, 0x26)),
            "css" => ("", Color::Rgb(0x56, 0x9C, 0xD6)),
            "scss" | "sass" => ("", Color::Rgb(0xC6, 0x53, 0x8C)),
            "md" | "markdown" => ("", Color::Rgb(0x56, 0x9C, 0xD6)),
            "py" => ("", Color::Rgb(0xFF, 0xD4, 0x3B)),
            "go" => ("", Color::Rgb(0x00, 0xAD, 0xD8)),
            "java" => ("", Color::Rgb(0xF8, 0x98, 0x20)),
            "kt" | "kts" => ("", Color::Rgb(0xB1, 0x25, 0xEA)),
            "rb" => ("", Color::Rgb(0xCC, 0x34, 0x2D)),
            "php" => ("", Color::Rgb(0x77, 0x7B, 0xB4)),
            "sh" | "bash" | "zsh" | "fish" => ("", Color::Rgb(0x89, 0xE0, 0x51)),
            "yml" | "yaml" | "toml" | "ini" => ("", theme.fg_dim),
            "c" | "h" => ("", Color::Rgb(0x59, 0x9E, 0xD8)),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => ("", Color::Rgb(0x00, 0x59, 0x9C)),
            "cs" => ("󰌛", Color::Rgb(0x68, 0x2A, 0xD7)),
            "swift" => ("", Color::Rgb(0xF0, 0x51, 0x38)),
            _ => ("", theme.fg_dim),
        },
    };

    (icon, Style::default().fg(color))
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
