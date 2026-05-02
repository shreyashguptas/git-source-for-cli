//! Single-line commit message input bar (rendered above the status bar when active).
//!
//! Multi-line / body editing can be added later via tui-textarea; for v1 a
//! single-line bar is enough — `git commit -m "..."` honors it directly.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
    widgets::Paragraph,
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct InputState {
    pub buf: String,
    pub cursor: usize,
    /// Optional flag set by app to drive `commit && push`.
    pub push_after: bool,
    /// True while an Ollama generation is streaming into `buf`. While set, the
    /// input ignores keystrokes (except Esc to cancel) so the user doesn't
    /// fight the streamed text mid-flight.
    pub generating: bool,
}

impl InputState {
    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.push_after = false;
        self.generating = false;
    }

    pub fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Append text to the end of the buffer (used by streaming generation).
    /// Always moves the cursor to the new end.
    pub fn append(&mut self, s: &str) {
        self.buf.push_str(s);
        self.cursor = self.buf.len();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // step back one char boundary
        let mut new_cursor = self.cursor - 1;
        while new_cursor > 0 && !self.buf.is_char_boundary(new_cursor) {
            new_cursor -= 1;
        }
        self.buf.replace_range(new_cursor..self.cursor, "");
        self.cursor = new_cursor;
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

pub fn render(state: &InputState, area: Rect, frame: &mut Frame, theme: &Theme) {
    let prompt = if state.generating {
        " ✨ generating (Esc cancel) > "
    } else if state.push_after {
        " commit (then push) > "
    } else {
        " commit > "
    };
    let label_style = Style::default()
        .bg(theme.accent)
        .fg(ratatui::style::Color::Black)
        .add_modifier(Modifier::BOLD);
    let body_style = Style::default()
        .bg(theme.selection_bg(true))
        .fg(theme.fg);

    // Compose: label + buffer + cursor caret + hint
    let body_with_caret = if state.generating {
        // Animated-ish caret to signal "writing now".
        format!(" {}▌ ", state.buf)
    } else if state.cursor == state.buf.len() {
        format!(" {} ▏ ", state.buf)
    } else {
        format!(" {}▏{} ", &state.buf[..state.cursor], &state.buf[state.cursor..])
    };

    // Render the label and body as two paragraphs side by side.
    let prompt_w = prompt.chars().count() as u16;
    let label_area = Rect {
        x: area.x,
        y: area.y,
        width: prompt_w.min(area.width),
        height: 1,
    };
    let body_area = Rect {
        x: area.x + prompt_w.min(area.width),
        y: area.y,
        width: area.width.saturating_sub(prompt_w),
        height: 1,
    };
    frame.render_widget(Paragraph::new(Span::raw(prompt)).style(label_style), label_area);
    frame.render_widget(
        Paragraph::new(Span::raw(body_with_caret)).style(body_style),
        body_area,
    );
}
