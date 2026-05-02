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

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    git::diff::{classify, DiffLineKind},
    ui::{file_icon::file_icon, theme::Theme},
};

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
            if let Some((o_start, o_len, n_start, n_len, _ctx)) = parse_hunk_full(line) {
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
            if let Some((o_start, _o_len, n_start, _n_len, _ctx)) = parse_hunk_full(line) {
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
                    out.extend(diff_rows(
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
                    out.extend(diff_rows(
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
                    out.extend(diff_rows(
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

/// Render a file separator bar — language icon + bold filename + dim parent
/// directory on a muted background, padded across the full width.
fn file_header_bar<'a>(path: &'a str, theme: &Theme, width: u16) -> Line<'a> {
    let bar_bg = theme.selection_bg_inactive;
    let (parent, name) = match path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", path),
    };
    // Pull the language icon + brand color from the shared mapping that the
    // Changes pane file rows use, so the diff header matches them visually.
    let (icon, icon_style) = file_icon(path, theme);
    let icon_style = icon_style.bg(bar_bg);

    let mut spans: Vec<Span<'a>> = Vec::with_capacity(7);
    spans.push(Span::styled(" ", Style::default().bg(bar_bg)));
    spans.push(Span::styled(icon, icon_style));
    spans.push(Span::styled("  ", Style::default().bg(bar_bg)));
    spans.push(Span::styled(
        name.to_string(),
        Style::default()
            .fg(theme.accent)
            .bg(bar_bg)
            .add_modifier(Modifier::BOLD),
    ));
    if !parent.is_empty() {
        spans.push(Span::styled("  ", Style::default().bg(bar_bg)));
        spans.push(Span::styled(
            parent.to_string(),
            Style::default().fg(theme.fg_dim).bg(bar_bg),
        ));
    }
    // Display-width-correct padding so the bar fills the row even when the
    // path contains wide glyphs.
    let used: usize = spans.iter().map(|s| s.content.as_ref().width()).sum();
    if (width as usize) > used {
        spans.push(Span::styled(
            " ".repeat(width as usize - used),
            Style::default().bg(bar_bg),
        ));
    }
    Line::from(spans)
}

/// Hunk header — replace the cryptic `@@ -X,Y +A,B @@` with a human-friendly
/// `Line N · context` (the N matches the gutter's NEW-side number, and the
/// context is the trailing function/scope hint git already emits).
fn hunk_line<'a>(line: &'a str, theme: &Theme, gutter_chars: u16) -> Line<'a> {
    let label_style = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let context_style = Style::default()
        .fg(theme.fg_dim)
        .add_modifier(Modifier::ITALIC);

    let (label, context) = match parse_hunk_full(line) {
        Some((_, _, ns, _, ctx)) => (format!("Line {ns}"), ctx.to_string()),
        // Couldn't parse — fall back to the raw line so we don't lose info.
        None => (line.to_string(), String::new()),
    };

    let mut spans = Vec::with_capacity(4);
    spans.push(Span::raw(" ".repeat(gutter_chars as usize)));
    spans.push(Span::styled(label, label_style));
    if !context.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(context, context_style));
    }
    Line::from(spans)
}

/// One diff content row with a two-column line-number gutter, manually wrapped
/// so the gutter stays aligned even when content overflows the pane width.
///
/// Without this, ratatui's `Wrap` would break the content at column 0 of the
/// next visual row — the wrapped text bleeds past the gutter and the bg color
/// from the previous row carries through to spaces it shouldn't. By emitting
/// one `Line` per chunk (each exactly `width` cells, including padding) we
/// guarantee the wrapper never has to wrap us.
fn diff_rows<'a>(
    line: &'a str,
    kind: DiffLineKind,
    old_n: Option<u32>,
    new_n: Option<u32>,
    digits: u16,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'a>> {
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
    let gutter_chars = (digits * 2 + 3) as usize;
    let blank_gutter = " ".repeat(gutter_chars);

    let content_w = (width as usize).saturating_sub(gutter_chars);
    let needs_pad = matches!(kind, DiffLineKind::Add | DiffLineKind::Del);

    // Degenerate widths: emit a single row so we never return zero lines and
    // never panic on subtraction.
    if width == 0 || content_w == 0 {
        return vec![Line::from(vec![
            Span::styled(old_str, gutter_style),
            Span::styled(new_str, gutter_style),
            Span::styled(line, style),
        ])];
    }

    let chunks = split_by_display_width(line, content_w);
    let mut out: Vec<Line<'a>> = Vec::with_capacity(chunks.len().max(1));
    for (i, chunk) in chunks.iter().enumerate() {
        let mut spans: Vec<Span<'a>> = Vec::with_capacity(4);
        if i == 0 {
            spans.push(Span::styled(old_str.clone(), gutter_style));
            spans.push(Span::styled(new_str.clone(), gutter_style));
        } else {
            // Continuation rows: blank gutter so the content under add/del
            // bars stays aligned with the first chunk.
            spans.push(Span::styled(blank_gutter.clone(), gutter_style));
        }
        spans.push(Span::styled(chunk.clone(), style));
        if needs_pad {
            let cw = chunk.width();
            if cw < content_w {
                spans.push(Span::styled(" ".repeat(content_w - cw), style));
            }
        }
        out.push(Line::from(spans));
    }
    if out.is_empty() {
        // Empty diff line (rare — usually a `+` with no content). Still emit
        // a row so the gutter line numbers are present.
        let mut spans: Vec<Span<'a>> = vec![
            Span::styled(old_str, gutter_style),
            Span::styled(new_str, gutter_style),
        ];
        if needs_pad {
            spans.push(Span::styled(" ".repeat(content_w), style));
        }
        out.push(Line::from(spans));
    }
    out
}

/// Split a string into pieces whose display width is ≤ `max_w`. Used to
/// pre-wrap diff content so ratatui's `Wrap` never has to.
fn split_by_display_width(s: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if cur_w + cw > max_w && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            cur_w = 0;
        }
        current.push(ch);
        cur_w += cw;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
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

/// Parse a hunk header and return `(old_start, old_len, new_start, new_len,
/// trailing_context)`. Accepts both `@@ -X,Y +A,B @@` and `@@ -X +A @@`
/// (length defaults to 1). The trailing context is the function/scope hint
/// git emits after the second `@@` — we surface it in the friendly hunk
/// label.
fn parse_hunk_full(line: &str) -> Option<(u32, u32, u32, u32, &str)> {
    let rest = line.strip_prefix("@@")?.trim_start();
    let close = rest.find("@@")?;
    let ranges = rest[..close].trim();
    let context = rest[close + 2..].trim();
    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (os, ol) = split_range(old);
    let (ns, nl) = split_range(new);
    Some((os, ol, ns, nl, context))
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
        assert_eq!(
            parse_hunk_full("@@ -1,3 +1,4 @@"),
            Some((1, 3, 1, 4, ""))
        );
        assert_eq!(
            parse_hunk_full("@@ -10,0 +11,5 @@ fn foo()"),
            Some((10, 0, 11, 5, "fn foo()"))
        );
        assert_eq!(parse_hunk_full("@@ -1 +1 @@"), Some((1, 1, 1, 1, "")));
        assert_eq!(parse_hunk_full("not a hunk"), None);
    }

    #[test]
    fn splits_by_display_width_keeps_chunks_under_limit() {
        let chunks = split_by_display_width("abcdefghij", 4);
        assert_eq!(chunks, vec!["abcd", "efgh", "ij"]);
        assert!(split_by_display_width("", 4).is_empty());
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
