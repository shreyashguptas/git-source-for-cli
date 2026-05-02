use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::Span,
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::{App, InputMode, Pane},
    gh::Availability,
    git::HeadRef,
    ui::{panes, theme},
};

/// Top-level render. Always called from the UI thread, never blocks.
pub fn render(app: &mut App, frame: &mut Frame) {
    let theme = theme::current();
    let area = frame.area();

    let mut constraints = vec![Constraint::Min(1)];
    if app.input_mode == InputMode::Commit {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1)); // status bar
    if app.toast.is_some() {
        constraints.push(Constraint::Length(1));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    render_main(app, frame, chunks[idx], &theme);
    idx += 1;
    if app.input_mode == InputMode::Commit {
        panes::commit_input::render(&app.input, chunks[idx], frame, &theme);
        idx += 1;
    }
    render_status_bar(app, frame, chunks[idx], &theme);
    idx += 1;
    if app.toast.is_some() {
        render_toast(app, frame, chunks[idx], &theme);
    }

    // Modal overlays draw on top of everything.
    if let Some(content) = app.overlay.clone() {
        panes::details::render(&content, app.overlay_scroll, area, frame, &theme);
    }
    if app.show_help {
        panes::help::render(area, frame, &theme);
    }
    if let Some(dialog) = app.confirm.clone() {
        panes::confirm::render(&dialog, area, frame, &theme);
    }
}

fn render_main(app: &mut App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(cols[0]);

    panes::branches::render(app, left[0], frame, theme);
    panes::changes::render(app, left[1], frame, theme);
    panes::graph::render(app, cols[1], frame, theme);
}

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
    let head_label = match &app.head {
        HeadRef::Branch(b) => b.clone(),
        HeadRef::Detached(sha) => format!("detached@{sha}"),
        HeadRef::Unborn => "unborn".to_string(),
    };
    let track_label = if app.status.ahead > 0 || app.status.behind > 0 {
        format!(" ↑{} ↓{}", app.status.ahead, app.status.behind)
    } else {
        String::new()
    };
    let pane_label = match app.active_pane {
        Pane::Branches => "branches",
        Pane::Changes => "changes",
        Pane::Graph => "graph",
    };
    let mode_label = if app.input_mode == InputMode::Commit {
        " · INPUT"
    } else {
        ""
    };
    let gh_label = match app.gh {
        Availability::Ready => "gh: ✓",
        Availability::NotAuthed => "gh: not authed",
        Availability::NotInstalled => "gh: ✗",
    };
    let text = format!(
        " gsc · {head_label}{track_label} · {} changes · {gh_label} · pane: {pane_label}{mode_label} · ? help · q quit ",
        app.status.files.len(),
    );
    let bar = Paragraph::new(Span::raw(text)).style(theme.status_bar());
    frame.render_widget(bar, area);
}

fn render_toast(app: &App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
    let Some(msg) = &app.toast else { return };
    let bar = Paragraph::new(Span::raw(format!(" ⓘ {msg} "))).style(theme.toast_info());
    frame.render_widget(bar, area);
}
