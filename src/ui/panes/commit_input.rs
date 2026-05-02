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

    // The inline bar is one row tall, so when the buffer carries an AI-
    // generated body (subject\n\nbody) we render only the subject and
    // append a "(+N body lines)" hint. The body is committed; it just
    // doesn't fit in the bar. Cursor stays inside the subject portion
    // (parked there by OllamaDone) so backspace/insert behave intuitively.
    let subject_end = state.buf.find('\n').unwrap_or(state.buf.len());
    let subject = &state.buf[..subject_end];
    let body_lines: usize = if subject_end == state.buf.len() {
        0
    } else {
        // Count non-empty body lines after the blank-line separator.
        state.buf[subject_end..]
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    };
    let cursor_in_subject = state.cursor.min(subject.len());

    let body_with_caret = if state.generating {
        // Animated-ish caret to signal "writing now".
        format!(" {subject}▌ ")
    } else if cursor_in_subject == subject.len() {
        format!(" {subject} ▏ ")
    } else {
        format!(
            " {}▏{} ",
            &subject[..cursor_in_subject],
            &subject[cursor_in_subject..]
        )
    };

    // Render the label and body as paragraphs side by side, with an optional
    // "+N body lines" tail chip on the right.
    let prompt_w = prompt.chars().count() as u16;
    let hint = if body_lines > 0 && !state.generating {
        format!(" +{body_lines} body lines ")
    } else {
        String::new()
    };
    let hint_w = hint.chars().count() as u16;
    let label_area = Rect {
        x: area.x,
        y: area.y,
        width: prompt_w.min(area.width),
        height: 1,
    };
    let body_w = area
        .width
        .saturating_sub(prompt_w)
        .saturating_sub(hint_w);
    let body_area = Rect {
        x: area.x + prompt_w.min(area.width),
        y: area.y,
        width: body_w,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Span::raw(prompt)).style(label_style), label_area);
    frame.render_widget(
        Paragraph::new(Span::raw(body_with_caret)).style(body_style),
        body_area,
    );
    if hint_w > 0 {
        let hint_area = Rect {
            x: area.x + prompt_w + body_w,
            y: area.y,
            width: hint_w,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                hint,
                Style::default()
                    .bg(theme.selection_bg(false))
                    .fg(theme.fg_dim)
                    .add_modifier(Modifier::ITALIC),
            )),
            hint_area,
        );
    }
}
