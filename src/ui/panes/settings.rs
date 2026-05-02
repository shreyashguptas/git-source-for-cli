//! Settings modal — OpenCode-style settings page rendered as an overlay.
//! Lists every user-configurable option with its current value; Enter (or a
//! mouse click) drills into a per-row action: open the model picker, edit
//! the base URL, or edit the system prompt inline.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::theme::Theme;

/// Which row inside the settings list is being focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    Model,
    BaseUrl,
    SystemPrompt,
}

impl SettingsRow {
    pub const ALL: [SettingsRow; 3] = [
        SettingsRow::Model,
        SettingsRow::BaseUrl,
        SettingsRow::SystemPrompt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsRow::Model => "Ollama model",
            SettingsRow::BaseUrl => "Ollama base URL",
            SettingsRow::SystemPrompt => "System prompt",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            SettingsRow::Model => "the model used to write commit messages",
            SettingsRow::BaseUrl => "where the local Ollama daemon is listening",
            SettingsRow::SystemPrompt => "instructions sent to the model before each diff",
        }
    }
}

/// State of the inline editor when the user is editing a text field.
#[derive(Debug, Clone)]
pub struct FieldEditor {
    pub row: SettingsRow,
    pub buf: String,
    pub cursor: usize,
    /// Scroll offset for multi-line fields (system prompt). Single-line fields ignore this.
    pub scroll: u16,
}

impl FieldEditor {
    pub fn new(row: SettingsRow, initial: String) -> Self {
        let cursor = initial.len();
        Self {
            row,
            buf: initial,
            cursor,
            scroll: 0,
        }
    }

    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut nc = self.cursor - 1;
        while nc > 0 && !self.buf.is_char_boundary(nc) {
            nc -= 1;
        }
        self.buf.replace_range(nc..self.cursor, "");
        self.cursor = nc;
    }

    pub fn left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut nc = self.cursor - 1;
        while nc > 0 && !self.buf.is_char_boundary(nc) {
            nc -= 1;
        }
        self.cursor = nc;
    }

    pub fn right(&mut self) {
        if self.cursor >= self.buf.len() {
            return;
        }
        let mut nc = self.cursor + 1;
        while nc < self.buf.len() && !self.buf.is_char_boundary(nc) {
            nc += 1;
        }
        self.cursor = nc;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buf.len();
    }
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub selected: SettingsRow,
    /// When `Some`, an inline editor is consuming key events and the row's
    /// value preview is replaced by the editor box.
    pub editor: Option<FieldEditor>,
    /// On-screen rect of each row, written each render — click handler reads
    /// this to know which row was clicked.
    pub row_rects: Vec<(SettingsRow, Rect)>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            selected: SettingsRow::Model,
            editor: None,
            row_rects: Vec::new(),
        }
    }
}

impl SettingsState {
    pub fn move_selection(&mut self, delta: isize) {
        if self.editor.is_some() {
            return;
        }
        let cur = SettingsRow::ALL
            .iter()
            .position(|r| *r == self.selected)
            .unwrap_or(0) as isize;
        let n = SettingsRow::ALL.len() as isize;
        let next = (cur + delta).clamp(0, n - 1) as usize;
        self.selected = SettingsRow::ALL[next];
    }
}

pub fn render(
    state: &mut SettingsState,
    current_model: Option<&str>,
    base_url: &str,
    system_prompt: &str,
    area: Rect,
    frame: &mut Frame,
    theme: &Theme,
) {
    let modal = centered(area, 70, 80);
    frame.render_widget(Clear, modal);

    let title_text = if state.editor.is_some() {
        " Settings · editing (Enter save · Esc cancel) "
    } else {
        " Settings (↑↓ · Enter edit · click rows · Esc close) "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(Span::styled(
            title_text,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let inner = inner.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    // Section header.
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "OLLAMA",
            Style::default()
                .fg(theme.fg_dim)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  · local AI commit messages",
            Style::default().fg(theme.fg_dim),
        ),
    ]));
    let header_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(header, header_area);

    let body = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };

    // Each row: 3 lines (label / value / blank). Editor expands the value
    // area for the editing row.
    state.row_rects.clear();

    let mut y = body.y;
    for row in SettingsRow::ALL {
        let value = match row {
            SettingsRow::Model => current_model.unwrap_or("(none — Ollama unreachable)").to_string(),
            SettingsRow::BaseUrl => base_url.to_string(),
            SettingsRow::SystemPrompt => system_prompt.to_string(),
        };
        let editing_this = state
            .editor
            .as_ref()
            .map(|e| e.row == row)
            .unwrap_or(false);
        // System prompt editor wants a multi-line box; everything else is single line.
        let value_h: u16 = if editing_this && row == SettingsRow::SystemPrompt {
            6
        } else if editing_this {
            1
        } else if row == SettingsRow::SystemPrompt {
            // Show ~3 lines of preview when not editing.
            3
        } else {
            1
        };
        let row_h = 1 + value_h + 1; // label + value + spacer
        if y + row_h > body.y + body.height {
            break;
        }

        let row_rect = Rect {
            x: body.x,
            y,
            width: body.width,
            height: row_h,
        };
        state.row_rects.push((row, row_rect));

        let is_selected = state.selected == row && state.editor.is_none();
        let is_editing = editing_this;

        // Label line.
        let chevron = if is_selected || is_editing { "▸ " } else { "  " };
        let label_line = Line::from(vec![
            Span::styled(
                chevron,
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                row.label(),
                Style::default()
                    .fg(if is_selected || is_editing {
                        theme.accent
                    } else {
                        theme.fg
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                row.description(),
                Style::default().fg(theme.fg_dim),
            ),
        ]);
        let label_area = Rect {
            x: row_rect.x,
            y: row_rect.y,
            width: row_rect.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(label_line), label_area);

        // Value line(s).
        let value_area = Rect {
            x: row_rect.x + 2,
            y: row_rect.y + 1,
            width: row_rect.width.saturating_sub(2),
            height: value_h,
        };

        if is_editing {
            render_editor(
                state.editor.as_ref().unwrap(),
                value_area,
                frame,
                theme,
            );
        } else {
            let style = Style::default()
                .fg(theme.fg)
                .bg(if is_selected {
                    theme.selection_bg(true)
                } else {
                    theme.selection_bg(false)
                });
            let display = if value.is_empty() {
                "(empty)".to_string()
            } else {
                value.clone()
            };
            let para = Paragraph::new(Span::styled(format!(" {display} "), style))
                .wrap(Wrap { trim: false });
            frame.render_widget(para, value_area);
        }

        y += row_h;
    }

    // Footer hint.
    let footer_y = inner.y + inner.height.saturating_sub(1);
    if footer_y > body.y {
        let hint = if state.editor.is_some() {
            "Enter save · Esc cancel · ←→ Home End Backspace"
        } else {
            "Enter edit · click rows · M direct model picker · Esc close"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(theme.fg_dim))),
            Rect {
                x: inner.x,
                y: footer_y,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn render_editor(editor: &FieldEditor, area: Rect, frame: &mut Frame, theme: &Theme) {
    let style = Style::default()
        .fg(theme.fg)
        .bg(theme.selection_bg(true));

    if editor.row == SettingsRow::SystemPrompt {
        // Multi-line area: just render the buffer with wrap. Append caret at
        // cursor position; for now we show it at the end of the buffer slice
        // up to cursor (good-enough since most edits append).
        let (left, right) = editor.buf.split_at(editor.cursor.min(editor.buf.len()));
        let text = format!("{left}▏{right}");
        let para = Paragraph::new(Span::styled(text, style)).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    } else {
        let (left, right) = editor.buf.split_at(editor.cursor.min(editor.buf.len()));
        let text = format!(" {left}▏{right} ");
        let para = Paragraph::new(Span::styled(text, style));
        frame.render_widget(para, area);
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
