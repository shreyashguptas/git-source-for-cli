//! Modal listing the locally-installed Ollama models. Up/down + Enter picks one,
//! Esc cancels. The selected model is persisted to the user config so future
//! sessions remember the choice.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub struct ModelPickerState {
    pub models: Vec<String>,
    pub state: ListState,
}

impl ModelPickerState {
    pub fn new(models: Vec<String>, current: Option<&str>) -> Self {
        let mut state = ListState::default();
        // Pre-select the current model if it's still in the list, else the first.
        let initial = current
            .and_then(|c| models.iter().position(|m| m == c))
            .unwrap_or(0);
        if !models.is_empty() {
            state.select(Some(initial));
        }
        Self { models, state }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.models.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.models.len() as isize - 1) as usize;
        self.state.select(Some(next));
    }

    pub fn selected(&self) -> Option<&str> {
        self.state
            .selected()
            .and_then(|i| self.models.get(i))
            .map(|s| s.as_str())
    }
}

pub fn render(
    picker: &mut ModelPickerState,
    current: Option<&str>,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
) {
    let modal = centered(area, 50, 60);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            " Select Ollama model (↑↓ · Enter pick · Esc cancel) ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(block, modal);

    let inner = modal.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    let items: Vec<ListItem<'_>> = picker
        .models
        .iter()
        .map(|name| {
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
    frame.render_stateful_widget(list, inner, &mut picker.state);
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
