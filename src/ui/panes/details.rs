//! Modal overlay for diff/commit details viewing.
//! Renders a full-screen panel with a scrollable colored diff.
//!
//! This module also exposes `lines_from_content_padded` and `line_style` so
//! the inline `preview` pane can share the diff line styling.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};

use crate::{git::diff::{classify, DiffLineKind}, ui::theme::Theme};

/// What's being shown in the overlay (or preview pane).
#[derive(Debug, Clone)]
pub enum DetailsContent {
    /// Loading state — the body fetch is in flight.
    Loading { title: String },
    /// Loaded diff/show output.
    Body { title: String, body: String },
    /// Error during load.
    Error { title: String, message: String },
}

impl DetailsContent {
    pub fn title(&self) -> &str {
        match self {
            DetailsContent::Loading { title }
            | DetailsContent::Body { title, .. }
            | DetailsContent::Error { title, .. } => title,
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            DetailsContent::Body { body, .. } => body.lines().count(),
            _ => 1,
        }
    }
}

pub fn render(
    content: &DetailsContent,
    scroll: u16,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
) {
    // Carve the modal: 90% width, 90% height, centered.
    let modal = centered(area, 90, 90);

    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            format!(" {} (Esc close · j/k scroll · g/G top/bottom) ", content.title()),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = modal.inner(Margin { vertical: 1, horizontal: 2 });
    frame.render_widget(block, modal);

    let lines = lines_from_content_padded(content, theme, inner.width);
    let total = lines.len();
    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(para, inner);

    // Scrollbar
    if total > inner.height as usize {
        let mut sb_state = ScrollbarState::new(total).position(scroll as usize);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme.fg_dim));
        frame.render_stateful_widget(
            sb,
            modal.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut sb_state,
        );
    }
}

/// Convert a `DetailsContent` into colored Lines. Reused by the inline preview pane.
/// Renders each file in the diff with a VS Code-style filename bar and adds a
/// two-column line-number gutter to each diff row. Add/del rows are padded so
/// the highlight covers the full content width past the gutter.
pub fn lines_from_content_padded<'a>(
    content: &'a DetailsContent,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'a>> {
    match content {
        DetailsContent::Loading { .. } => vec![Line::from(Span::styled(
            "loading…",
            Style::default().fg(theme.fg_dim),
        ))],
        DetailsContent::Error { message, .. } => vec![Line::from(Span::styled(
            message.as_str(),
            Style::default().fg(theme.error),
        ))],
        DetailsContent::Body { body, .. } => render_body(body, theme, width),
    }
}

/// Walks the diff body and emits structured lines with file headers and
/// line-number gutters. Pre-diff content (commit metadata, --stat block) is
/// passed through with simple styling.
fn render_body<'a>(body: &'a str, theme: &Theme, width: u16) -> Vec<Line<'a>> {
    // First pass: find the largest line number in any hunk header so the gutter
    // is wide enough for both columns. Falls back to 3 digits.
    let mut max_num: u32 = 0;
    for line in body.lines() {
        if line.starts_with("@@") {
            if let Some((o_start, o_len, n_start, n_len)) = parse_hunk(line) {
                max_num = max_num
                    .max(o_start.saturating_add(o_len))
                    .max(n_start.saturating_add(n_len));
            }
        }
    }
    let digits = digit_count(max_num).max(3);
    // Each line: " OLD NEW " — single space pad between cols and one trailing.
    let gutter_chars = digits * 2 + 3;

    let mut out: Vec<Line<'a>> = Vec::new();
    let mut in_file = false;
    let mut first_file = true;
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;

    for line in body.lines() {
        // New file section.
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = parse_diff_git_path(rest);
            if !first_file {
                out.push(Line::from(""));
            }
            first_file = false;
            out.push(file_header_bar(path, theme, width));
            in_file = true;
            old_no = 0;
            new_no = 0;
            continue;
        }

        // Inside a file section, hide the noisy meta the header already implies,
        // but keep informative state lines as a small badge under the bar.
        if in_file {
            if line.starts_with("index ")
                || line.starts_with("--- ")
                || line.starts_with("+++ ")
            {
                continue;
            }
            if line.starts_with("new file")
                || line.starts_with("deleted file")
                || line.starts_with("similarity")
                || line.starts_with("rename ")
                || line.starts_with("Binary files")
            {
                out.push(Line::from(Span::styled(
                    format!("  {line}"),
                    Style::default()
                        .fg(theme.fg_dim)
                        .add_modifier(Modifier::ITALIC),
                )));
                continue;
            }
        }

        if line.starts_with("@@") {
            if let Some((o_start, _o_len, n_start, _n_len)) = parse_hunk(line) {
                old_no = o_start;
                new_no = n_start;
            }
            out.push(hunk_line(line, theme, gutter_chars));
            continue;
        }

        if in_file {
            let kind = classify(line);
            match kind {
                DiffLineKind::Context => {
                    out.push(diff_row(
                        line,
                        kind,
                        Some(old_no),
                        Some(new_no),
                        digits,
                        theme,
                        width,
                    ));
                    old_no = old_no.saturating_add(1);
                    new_no = new_no.saturating_add(1);
                }
                DiffLineKind::Add => {
                    out.push(diff_row(
                        line,
                        kind,
                        None,
                        Some(new_no),
                        digits,
                        theme,
                        width,
                    ));
                    new_no = new_no.saturating_add(1);
                }
                DiffLineKind::Del => {
                    out.push(diff_row(
                        line,
                        kind,
                        Some(old_no),
                        None,
                        digits,
                        theme,
                        width,
                    ));
                    old_no = old_no.saturating_add(1);
                }
                _ => {
                    // "\ No newline at end of file" and similar oddities.
                    out.push(Line::from(Span::styled(
                        format!("{empty} {line}", empty = " ".repeat(gutter_chars as usize)),
                        Style::default()
                            .fg(theme.fg_dim)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
            }
        } else {
            // Pre-diff content (commit headers, --stat block, blank lines).
            out.push(simple_line(line, theme));
        }
    }

    out
}

/// Render a file separator bar — bold filename + dim parent directory on a
/// muted background, padded across the full width.
fn file_header_bar<'a>(path: &'a str, theme: &Theme, width: u16) -> Line<'a> {
    let bar_bg = theme.selection_bg_inactive;
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", path),
    };

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(5);
    spans.push(Span::styled(
        " ",
        Style::default().bg(bar_bg),
    ));
    spans.push(Span::styled(
        name.to_string(),
        Style::default()
            .fg(theme.accent)
            .bg(bar_bg)
            .add_modifier(Modifier::BOLD),
    ));
    if !parent.is_empty() {
        spans.push(Span::styled(
            "  ",
            Style::default().bg(bar_bg),
        ));
        spans.push(Span::styled(
            parent.to_string(),
            Style::default().fg(theme.fg_dim).bg(bar_bg),
        ));
    }
    let used = 1 + name.chars().count() + if parent.is_empty() { 0 } else { 2 + parent.chars().count() };
    if (width as usize) > used {
        spans.push(Span::styled(
            " ".repeat(width as usize - used),
            Style::default().bg(bar_bg),
        ));
    }
    Line::from(spans)
}

/// Hunk header — pass through the raw `@@ -X,Y +A,B @@ context` line with the
/// hunk style, indented past the gutter so it lines up under the file bar.
fn hunk_line<'a>(line: &'a str, theme: &Theme, gutter_chars: u16) -> Line<'a> {
    let style = line_style(DiffLineKind::Hunk, theme);
    Line::from(vec![
        Span::raw(" ".repeat(gutter_chars as usize)),
        Span::styled(line, style),
    ])
}

/// One diff content row with a two-column line-number gutter.
/// `old_n` / `new_n` are `None` for the column that doesn't apply (None on the
/// old side for additions, None on the new side for deletions). For add/del
/// rows the content area is padded out to `width` so the bg highlight fills.
fn diff_row<'a>(
    line: &'a str,
    kind: DiffLineKind,
    old_n: Option<u32>,
    new_n: Option<u32>,
    digits: u16,
    theme: &Theme,
    width: u16,
) -> Line<'a> {
    let style = line_style(kind, theme);
    let gutter_style = Style::default().fg(theme.fg_dim);
    let d = digits as usize;

    let old_str = match old_n {
        Some(n) => format!(" {:>w$} ", n, w = d),
        None => " ".repeat(d + 2),
    };
    let new_str = match new_n {
        Some(n) => format!("{:>w$} ", n, w = d),
        None => " ".repeat(d + 1),
    };

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(4);
    spans.push(Span::styled(old_str, gutter_style));
    spans.push(Span::styled(new_str, gutter_style));
    spans.push(Span::styled(line, style));

    // Pad add/del rows so the bg highlight covers the rest of the content area.
    if matches!(kind, DiffLineKind::Add | DiffLineKind::Del) && width > 0 {
        let gutter_chars = digits * 2 + 3;
        let content_w = (width as usize).saturating_sub(gutter_chars as usize);
        let len = line.chars().count();
        if len < content_w {
            spans.push(Span::styled(" ".repeat(content_w - len), style));
        }
    }
    Line::from(spans)
}

/// Pre-diff lines (commit headers, --stat block) — same classification as
/// before, no gutter.
fn simple_line<'a>(text: &'a str, theme: &Theme) -> Line<'a> {
    let kind = classify(text);
    Line::from(Span::styled(text, line_style(kind, theme)))
}

/// Parse `a/<path> b/<path>` from a `diff --git` line and return the b-path
/// (the new name). Falls back to the raw rest if parsing fails.
fn parse_diff_git_path(rest: &str) -> &str {
    // Common, simple form. Rename diffs use the same `a/x b/y` shape too.
    if let Some(b_idx) = rest.find(" b/") {
        let b = &rest[b_idx + 3..];
        return b.trim_end();
    }
    rest.trim_end()
}

/// Parse a hunk header and return `(old_start, old_len, new_start, new_len)`.
/// Accepts both `@@ -X,Y +A,B @@` and `@@ -X +A @@` (length defaults to 1).
fn parse_hunk(line: &str) -> Option<(u32, u32, u32, u32)> {
    // Strip leading "@@ "
    let rest = line.strip_prefix("@@")?.trim_start();
    // Split off the trailing "@@ ..." — only keep the ranges section.
    let close = rest.find("@@")?;
    let ranges = rest[..close].trim();
    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (os, ol) = split_range(old);
    let (ns, nl) = split_range(new);
    Some((os, ol, ns, nl))
}

fn split_range(s: &str) -> (u32, u32) {
    match s.split_once(',') {
        Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
        None => (s.parse().unwrap_or(0), 1),
    }
}

fn digit_count(n: u32) -> u16 {
    let mut n = n;
    let mut d: u16 = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

pub fn line_style(kind: DiffLineKind, theme: &Theme) -> Style {
    match kind {
        DiffLineKind::Add => Style::default().fg(theme.added_fg).bg(theme.added_bg),
        DiffLineKind::Del => Style::default().fg(theme.deleted_fg).bg(theme.deleted_bg),
        DiffLineKind::Hunk => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::Header => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        DiffLineKind::Meta => Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::ITALIC),
        DiffLineKind::CommitMeta => Style::default()
            .fg(theme.modified)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::Stat => Style::default().fg(theme.fg_dim),
        DiffLineKind::Context => Style::default().fg(theme.fg),
        DiffLineKind::Other => Style::default().fg(theme.fg),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunk_headers() {
        assert_eq!(parse_hunk("@@ -1,3 +1,4 @@"), Some((1, 3, 1, 4)));
        assert_eq!(parse_hunk("@@ -10,0 +11,5 @@ context"), Some((10, 0, 11, 5)));
        assert_eq!(parse_hunk("@@ -1 +1 @@"), Some((1, 1, 1, 1)));
        assert_eq!(parse_hunk("not a hunk"), None);
    }

    #[test]
    fn parses_diff_git_path() {
        assert_eq!(parse_diff_git_path("a/src/foo.rs b/src/foo.rs"), "src/foo.rs");
        assert_eq!(parse_diff_git_path("a/x b/y"), "y");
    }

    #[test]
    fn digit_count_works() {
        assert_eq!(digit_count(0), 1);
        assert_eq!(digit_count(9), 1);
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(999), 3);
        assert_eq!(digit_count(1000), 4);
    }
}
