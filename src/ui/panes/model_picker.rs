//! Modal listing the locally-installed Ollama models with a type-to-filter
//! search input at the top (OpenCode-style). Up/down + Enter picks one,
//! Esc cancels. The selected model is persisted to the user config so
//! future sessions remember the choice.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub struct ModelPickerState {
    /// All models known from `/api/tags` — the unfiltered source of truth.
    pub all_models: Vec<String>,
    /// Current filter query (lowercased substring match).
    pub filter: String,
    /// Indices into `all_models` that match `filter`, in sort order.
    pub filtered: Vec<usize>,
    /// Selection within `filtered` (NOT `all_models`).
    pub state: ListState,
    /// True while a fresh `/api/tags` probe is in flight. The picker shows a
    /// small "refreshing…" hint in the title until it lands. Cleared by
    /// `replace_models`.
    pub refreshing: bool,
}

impl ModelPickerState {
    pub fn new(models: Vec<String>, current: Option<&str>) -> Self {
        let mut s = Self {
            all_models: models,
            filter: String::new(),
            filtered: Vec::new(),
            state: ListState::default(),
            refreshing: false,
        };
        s.recompute_filter();
        // Pre-select the current model if visible, else first.
        if let Some(cur) = current {
            if let Some(pos) = s
                .filtered
                .iter()
                .position(|&i| s.all_models[i].as_str() == cur)
            {
                s.state.select(Some(pos));
            }
        }
        if s.state.selected().is_none() && !s.filtered.is_empty() {
            s.state.select(Some(0));
        }
        s
    }

    /// Replace the underlying model list (e.g. after a fresh `/api/tags`
    /// probe lands). Re-applies the current filter and tries to keep the
    /// previously-selected model highlighted; falls back to the first match.
    pub fn replace_models(&mut self, models: Vec<String>, current: Option<&str>) {
        let prev_selected: Option<String> = self
            .state
            .selected()
            .and_then(|row| self.filtered.get(row).copied())
            .and_then(|idx| self.all_models.get(idx).cloned());
        self.all_models = models;
        self.refreshing = false;
        self.recompute_filter();
        // Try to keep the user's previous highlight; if it's gone, fall back
        // to the configured current model; failing that, row 0.
        let target = prev_selected.as_deref().or(current);
        if let Some(want) = target {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| self.all_models[i].as_str() == want)
            {
                self.state.select(Some(pos));
            }
        }
        if self.state.selected().is_none() && !self.filtered.is_empty() {
            self.state.select(Some(0));
        }
    }

    /// Rebuild `filtered` from `all_models` honoring the current `filter`.
    /// Resets selection to row 0 (so the highlight follows the search).
    fn recompute_filter(&mut self) {
        let q = self.filter.to_lowercase();
        self.filtered = self
            .all_models
            .iter()
            .enumerate()
            .filter(|(_, name)| q.is_empty() || name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if self.filtered.is_empty() {
            self.state.select(None);
        } else {
            self.state.select(Some(0));
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.filtered.len() as isize - 1) as usize;
        self.state.select(Some(next));
    }

    pub fn selected(&self) -> Option<&str> {
        let row = self.state.selected()?;
        let idx = *self.filtered.get(row)?;
        self.all_models.get(idx).map(|s| s.as_str())
    }

    pub fn push_char(&mut self, c: char) {
        self.filter.push(c);
        self.recompute_filter();
    }

    pub fn pop_char(&mut self) {
        self.filter.pop();
        self.recompute_filter();
    }

    pub fn clear_filter(&mut self) {
        if !self.filter.is_empty() {
            self.filter.clear();
            self.recompute_filter();
        }
    }
}

pub fn render(
    picker: &mut ModelPickerState,
    current: Option<&str>,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
) {
    let modal = centered(area, 60, 70);
    frame.render_widget(Clear, modal);

    let title = if picker.refreshing {
        " Select Ollama model · ⟳ refreshing… (type to filter · ↑↓ · Enter pick · Esc cancel) "
    } else {
        " Select Ollama model (type to filter · ↑↓ · Enter pick · Esc cancel) "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(block, modal);

    let inner = modal.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    // Two stripes inside: a 1-row search box + the list below.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    let search_area = chunks[0];
    let list_area = chunks[1];

    // Search input — single line. Cursor caret rendered as ▏.
    let search_line = Line::from(vec![
        Span::styled(
            " 🔎  ",
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            picker.filter.clone(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("▏", Style::default().fg(theme.accent)),
        Span::styled(
            if picker.filter.is_empty() {
                "  filter…"
            } else {
                ""
            },
            Style::default().fg(theme.fg_dim),
        ),
    ]);
    let search = Paragraph::new(search_line).style(
        Style::default().bg(theme.selection_bg(false)),
    );
    frame.render_widget(search, Rect { height: 1, ..search_area });

    // Subtle separator underline.
    let sep = Rect {
        x: search_area.x,
        y: search_area.y + 1,
        width: search_area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(sep.width as usize),
            Style::default().fg(theme.border),
        )),
        sep,
    );

    if picker.filtered.is_empty() {
        let msg = if picker.all_models.is_empty() {
            "no models installed"
        } else {
            "no matches"
        };
        let p = Paragraph::new(Span::styled(
            format!("  {msg}"),
            Style::default().fg(theme.fg_dim).add_modifier(Modifier::ITALIC),
        ));
        frame.render_widget(p, list_area);
        return;
    }

    let items: Vec<ListItem<'_>> = picker
        .filtered
        .iter()
        .map(|&i| {
            let name = &picker.all_models[i];
            let is_current = current == Some(name.as_str());
            let marker = if is_current { "● " } else { "  " };
            let marker_style = if is_current {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.fg_dim)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(name.as_str(), Style::default().fg(theme.fg)),
            ]))
        })
        .collect();

    let list = List::new(items).highlight_style(
        Style::default()
            .bg(theme.selection_bg(true))
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(list, list_area, &mut picker.state);
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
