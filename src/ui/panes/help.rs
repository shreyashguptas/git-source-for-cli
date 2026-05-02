//! Modal help overlay listing all keybindings, grouped by context.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::theme::Theme;

pub fn render(area: Rect, frame: &mut Frame, theme: &Theme) {
    let modal = centered(area, 70, 80);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            " gsc — keybindings (? or Esc closes) ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = modal.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    frame.render_widget(block, modal);

    let lines = build_lines(theme);
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(para, inner);
}

fn build_lines(theme: &Theme) -> Vec<Line<'static>> {
    let head = |s: &'static str| {
        Line::from(Span::styled(
            s,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let row = |k: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(
                format!("  {k:14}"),
                Style::default()
                    .fg(theme.modified)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(d, Style::default().fg(theme.fg)),
        ])
    };
    vec![
        head("Global"),
        row("?", "toggle this help"),
        row("q  Ctrl-C", "quit"),
        row("Tab  Shift-Tab", "cycle pane focus forward / back"),
        row("1 / 2 / 3", "jump to Branches / Changes / Graph pane"),
        row("r", "force refresh"),
        row("j  k  ↑↓", "move selection"),
        row("g  G", "top / bottom"),
        row("PgUp PgDn", "jump 10 rows"),
        row("Enter", "context action (see below)"),
        Line::from(""),
        head("Mouse"),
        row("left-click", "focus pane and select item under cursor"),
        row("scroll wheel", "scroll the pane / preview under the cursor"),
        row("(macOS)", "hold Option while dragging to use the terminal's native text selection"),
        Line::from(""),
        head("Graph header (top of graph pane)"),
        row("↑N to push", "you have N commits not yet on origin/<branch>"),
        row("↓N to pull", "origin has N commits you don't yet have"),
        row("↑ <commit>", "this specific commit is unpushed (left-margin marker)"),
        Line::from(""),
        head("Branches pane"),
        row("Enter", "checkout selected branch (blocked if dirty)"),
        row("n", "new branch (type name, press Enter)"),
        row("d / D", "delete (safe / force) — confirms"),
        row("m", "merge selected branch into current — confirms"),
        row("p / P / f", "push / pull (--ff-only) / fetch --all"),
        row("o", "open PR in browser (if `gh` is connected)"),
        Line::from(""),
        head("Changes pane"),
        row("Enter", "view diff in overlay"),
        row("Space", "stage / unstage selected file"),
        row("a / A", "stage all / unstage all"),
        row("c", "start commit (type message, Enter to commit)"),
        row("C", "commit and push"),
        row("x", "discard local changes — confirms"),
        Line::from(""),
        head("Graph pane"),
        row("↑↓  j  k", "scroll commits — diff preview pane updates live"),
        row("Enter", "open the diff full-screen (more room to scroll)"),
        row("o", "open commit on github.com (origin remote)"),
        Line::from(""),
        head("Preview pane (right column, auto-updating)"),
        row("(none)", "automatically shows diff for selected commit/file"),
        row("(scroll)", "press Enter on Graph/Changes for full-screen scroll"),
        row("(width)", "preview hides automatically when terminal < 130 cols"),
        Line::from(""),
        head("Overlay (diff / commit details / help)"),
        row("Esc  q", "close overlay"),
        row("j  k  PgUp PgDn  g  G", "scroll"),
        Line::from(""),
        head("Commit input mode"),
        row("Enter", "submit commit (or create branch when prefixed with 'branch:')"),
        row("Esc", "cancel"),
        row("← →  Home End  Backspace", "edit input"),
    ]
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
