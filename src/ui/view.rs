use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::{App, InputMode, Pane, PaneRects},
    gh::Availability,
    git::HeadRef,
    ollama::Availability as OllamaAvailability,
    ui::{panes, theme},
};

/// Threshold below which the inline preview pane is hidden so the other panes
/// stay readable on narrow terminals.
const PREVIEW_MIN_WIDTH: u16 = 130;

/// Top-level render. Always called from the UI thread, never blocks.
pub fn render(app: &mut App, frame: &mut Frame) {
    let theme = theme::current();
    let area = frame.area();

    // The commit-input UI now lives inline inside the Changes pane (so it
    // sits next to the file list, like VS Code's source-control textbox). We
    // no longer reserve a strip at the bottom for it.
    let mut constraints = vec![Constraint::Min(1)];
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
    if app.model_picker.is_some() {
        let current = app.config.ollama.model.clone();
        if let Some(picker) = app.model_picker.as_mut() {
            panes::model_picker::render(picker, current.as_deref(), area, frame, &theme);
        }
    }
    if app.settings.is_some() {
        let current_model = app.config.ollama.model.clone();
        let base_url = app.config.ollama.base_url.clone();
        let system_prompt = app.config.ollama.system_prompt.clone();
        if let Some(state) = app.settings.as_mut() {
            panes::settings::render(
                state,
                current_model.as_deref(),
                &base_url,
                &system_prompt,
                area,
                frame,
                &theme,
            );
        }
    }
    // Info / error popup. Rendered above other modals so the message can't
    // be hidden by a settings/picker that happened to be open underneath.
    if let Some(d) = app.info_dialog.clone() {
        panes::info::render(&d, area, frame, &theme);
    }
    // Generation modal renders LAST so it sits on top of every other modal —
    // it owns the screen exclusively while AI is generating.
    if let Some(g) = app.generation.clone() {
        panes::generation::render(&g, area, frame, &theme);
    }
}

fn render_main(app: &mut App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
    let show_preview = area.width >= PREVIEW_MIN_WIDTH;
    let (left_rect, graph_rect, preview_rect) = compute_columns(area, app, show_preview);
    let (branches_rect, changes_rect) = compute_left_split(left_rect, app);

    // Capture rects for the mouse handler. Status-bar rects are filled in
    // later by `render_status_bar`; we preserve any prior values so they're
    // not nulled out between draws.
    let prev_status_ollama = app.last_rects.status_ollama;
    let prev_status_settings = app.last_rects.status_settings;
    // List rects start empty each frame; each pane fills its own when it
    // renders below. Defaulting to the pane rect keeps fall-through behavior
    // sane if a pane skips rendering for any reason.
    app.last_rects = PaneRects {
        main_area: area,
        branches: branches_rect,
        changes: changes_rect,
        graph: graph_rect,
        preview: preview_rect,
        status_ollama: prev_status_ollama,
        status_settings: prev_status_settings,
        branches_list: branches_rect,
        changes_list: changes_rect,
        graph_list: graph_rect,
    };

    panes::branches::render(app, branches_rect, frame, theme);
    panes::changes::render(app, changes_rect, frame, theme);
    panes::graph::render(app, graph_rect, frame, theme);
    if let Some(p) = preview_rect {
        panes::preview::render(app, p, frame, theme);
    }
}

/// Compute the three (or two) column rects honoring user resize overrides.
fn compute_columns(area: Rect, app: &App, show_preview: bool) -> (Rect, Rect, Option<Rect>) {
    if show_preview {
        // Defaults: 25 / 35 / 40 percent.
        let default_left = (area.width as u32 * 25 / 100) as u16;
        let default_graph = (area.width as u32 * 35 / 100) as u16;

        let left_w = app
            .layout_overrides
            .left_width
            .unwrap_or(default_left)
            .clamp(15, area.width.saturating_sub(40).max(15));

        let remaining = area.width.saturating_sub(left_w);
        let graph_w = app
            .layout_overrides
            .graph_width
            .unwrap_or(default_graph)
            .clamp(20, remaining.saturating_sub(20).max(20));

        let preview_w = remaining.saturating_sub(graph_w).max(20);

        let left = Rect {
            x: area.x,
            y: area.y,
            width: left_w,
            height: area.height,
        };
        let graph = Rect {
            x: area.x + left_w,
            y: area.y,
            width: graph_w,
            height: area.height,
        };
        let preview = Rect {
            x: area.x + left_w + graph_w,
            y: area.y,
            width: preview_w,
            height: area.height,
        };
        (left, graph, Some(preview))
    } else {
        // Two-column fallback for narrow terminals.
        let default_left = (area.width as u32 * 35 / 100) as u16;
        let left_w = app
            .layout_overrides
            .left_width
            .unwrap_or(default_left)
            .clamp(15, area.width.saturating_sub(20).max(15));
        let graph_w = area.width.saturating_sub(left_w);

        let left = Rect {
            x: area.x,
            y: area.y,
            width: left_w,
            height: area.height,
        };
        let graph = Rect {
            x: area.x + left_w,
            y: area.y,
            width: graph_w,
            height: area.height,
        };
        (left, graph, None)
    }
}

/// Compute Branches (top) + Changes (bottom) inside the left column.
fn compute_left_split(left: Rect, app: &App) -> (Rect, Rect) {
    let default_branches = (left.height as u32 * 60 / 100) as u16;
    let branches_h = app
        .layout_overrides
        .branches_height
        .unwrap_or(default_branches)
        .clamp(3, left.height.saturating_sub(4).max(3));

    let branches = Rect {
        x: left.x,
        y: left.y,
        width: left.width,
        height: branches_h,
    };
    let changes = Rect {
        x: left.x,
        y: left.y + branches_h,
        width: left.width,
        height: left.height.saturating_sub(branches_h),
    };
    (branches, changes)
}

fn render_status_bar(app: &mut App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
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
        Availability::Ready => "gh: ✓".to_string(),
        Availability::NotAuthed => "gh: not authed".to_string(),
        Availability::NotInstalled => "gh: ✗".to_string(),
    };
    let ollama_label = match &app.ollama {
        OllamaAvailability::Ready { .. } => match app.config.ollama.model.as_deref() {
            Some(m) => format!("ollama: ✓ {m}"),
            None => "ollama: ✓".to_string(),
        },
        OllamaAvailability::NoModels => "ollama: no models".to_string(),
        OllamaAvailability::NotRunning => "ollama: ✗".to_string(),
        OllamaAvailability::Unknown => "ollama: …".to_string(),
    };
    let hints = pane_hints(app);

    // We render the bar as a sequence of spans so we can hit-test specific
    // segments (the ollama chip and the settings chip) on click. Each span's
    // x range is tracked as we go so the click handler knows where they live.
    let base = theme.status_bar();
    // Clickable segments use a slightly punchier underline + bold so they
    // read as actionable to a user who's mousing over.
    let clickable = base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let prefix = format!(
        " gsc · {head_label}{track_label} · {} changes · {gh_label} · ",
        app.status.files.len(),
    );
    let mid = format!(" · {pane_label}{mode_label} · {hints} · ");
    let settings_chip = "⚙ settings";
    let tail = " · ? help · q quit ";

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(8);
    let mut cursor_x = area.x;
    let push_seg = |spans: &mut Vec<Span<'static>>,
                    cursor_x: &mut u16,
                    text: String,
                    style: Style|
     -> Rect {
        let w = text.chars().count() as u16;
        let r = Rect {
            x: *cursor_x,
            y: area.y,
            width: w.min(area.width.saturating_sub((*cursor_x).saturating_sub(area.x))),
            height: 1,
        };
        spans.push(Span::styled(text, style));
        *cursor_x = (*cursor_x).saturating_add(w);
        r
    };

    let _ = push_seg(&mut spans, &mut cursor_x, prefix, base);
    let ollama_rect = push_seg(&mut spans, &mut cursor_x, ollama_label, clickable);
    let _ = push_seg(&mut spans, &mut cursor_x, mid, base);
    let settings_rect = push_seg(&mut spans, &mut cursor_x, settings_chip.to_string(), clickable);
    let _ = push_seg(&mut spans, &mut cursor_x, tail.to_string(), base);

    // Persist the rects so the click handler can hit-test them.
    app.last_rects.status_ollama = Some(ollama_rect);
    app.last_rects.status_settings = Some(settings_rect);

    let bar = Paragraph::new(Line::from(spans)).style(base);
    frame.render_widget(bar, area);
}

/// Pane-specific keybinding hints shown in the status bar — discoverability
/// for the most common actions without opening the help overlay.
fn pane_hints(app: &App) -> &'static str {
    if app.input_mode == InputMode::Commit {
        if app.input.generating {
            return "streaming… · Esc cancel";
        }
        return "Enter commit · ^G regenerate · Esc cancel";
    }
    match app.active_pane {
        Pane::Branches => "↑↓ graph preview · Enter checkout · n new · p push · P pull · m merge · d del",
        Pane::Changes => "Space stage · a all · c commit · ^G generate message · U uncommit · x discard",
        Pane::Graph => "↑↓ live preview · Enter full · o github",
    }
}

fn render_toast(app: &App, frame: &mut Frame, area: Rect, theme: &theme::Theme) {
    let Some(msg) = &app.toast else { return };
    let bar = Paragraph::new(Span::raw(format!(" ⓘ {msg} "))).style(theme.toast_info());
    frame.render_widget(bar, area);
}
