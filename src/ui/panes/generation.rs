//! Modal that owns the screen during an Ollama commit-message generation.
//!
//! The modal goes through three phases:
//!   1. Streaming — tokens stream in live, screen is blocked for clicks/keys.
//!   2. Done — the generated message is presented for review with explicit
//!      "use it / regenerate / discard" actions.
//!   3. Error — Ollama failed; the modal explains why and offers next steps
//!      so failures never silently disappear into a toast.

use ratatui::{
    layout::{Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub enum GenerationPhase {
    Streaming { partial: String },
    Done { message: String },
    Error { message: String },
}

#[derive(Debug, Clone)]
pub struct GenerationDialog {
    pub phase: GenerationPhase,
    /// Frame counter for the spinner — bumped on every AppEvent::Tick while
    /// the modal is active. Wraps every full rotation, so we don't care about
    /// overflow.
    pub spinner: u32,
    /// Model name being used — surfaced in the title so the user knows
    /// what's running and can tell when the wrong model is loaded.
    pub model: String,
}

impl GenerationDialog {
    pub fn streaming(model: String) -> Self {
        Self {
            phase: GenerationPhase::Streaming {
                partial: String::new(),
            },
            spinner: 0,
            model,
        }
    }

    pub fn append_partial(&mut self, token: &str) {
        if let GenerationPhase::Streaming { partial } = &mut self.phase {
            partial.push_str(token);
        }
    }

    pub fn finish_done(&mut self, message: String) {
        self.phase = GenerationPhase::Done { message };
    }

    pub fn finish_error(&mut self, message: String) {
        self.phase = GenerationPhase::Error { message };
    }

    pub fn is_streaming(&self) -> bool {
        matches!(self.phase, GenerationPhase::Streaming { .. })
    }
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn render(dialog: &GenerationDialog, area: Rect, frame: &mut Frame, theme: &Theme) {
    let modal = centered(area, 60, 50);
    frame.render_widget(Clear, modal);

    let (title, accent) = match &dialog.phase {
        GenerationPhase::Streaming { .. } => {
            let s = SPINNER_FRAMES[dialog.spinner as usize % SPINNER_FRAMES.len()];
            (
                format!(" {s}  Generating commit message · {} ", dialog.model),
                theme.accent,
            )
        }
        GenerationPhase::Done { .. } => (
            format!(" ✓  Commit message ready · {} ", dialog.model),
            theme.added,
        ),
        GenerationPhase::Error { .. } => (
            format!(" ✗  Generation failed · {} ", dialog.model),
            theme.error,
        ),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(block, modal);

    let inner = modal.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    match &dialog.phase {
        GenerationPhase::Streaming { partial } => render_streaming(partial, inner, frame, theme),
        GenerationPhase::Done { message } => render_done(message, inner, frame, theme),
        GenerationPhase::Error { message } => render_error(message, inner, frame, theme),
    }
}

fn render_streaming(partial: &str, area: Rect, frame: &mut Frame, theme: &Theme) {
    // Two stripes inside: a 1-line note + the streamed text wrapped below it.
    let note = Paragraph::new(Line::from(vec![
        Span::styled(
            "Streaming tokens from Ollama — input is locked until done.  ",
            Style::default().fg(theme.fg_dim),
        ),
        Span::styled(
            "Esc cancel",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let note_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(note, note_area);

    let body_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(2),
    };
    let display = if partial.is_empty() {
        "(waiting for first token…)".to_string()
    } else {
        format!("{partial}▌")
    };
    let style = if partial.is_empty() {
        Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(theme.fg)
    };
    let body = Paragraph::new(Span::styled(display, style)).wrap(Wrap { trim: false });
    frame.render_widget(body, body_area);
}

fn render_done(message: &str, area: Rect, frame: &mut Frame, theme: &Theme) {
    let header = Paragraph::new(Line::from(Span::styled(
        "Generated message — review before committing:",
        Style::default().fg(theme.fg_dim),
    )));
    frame.render_widget(
        header,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    // Highlighted message box in the middle.
    let msg_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(4),
    };
    let inner_msg = msg_area.inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    let msg_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.added));
    frame.render_widget(msg_block, msg_area);
    let body = Paragraph::new(Span::styled(
        message.to_string(),
        Style::default()
            .fg(theme.fg)
            .add_modifier(Modifier::BOLD),
    ))
    .wrap(Wrap { trim: false });
    frame.render_widget(body, inner_msg);

    // Footer: action hints.
    let footer_y = area.y + area.height.saturating_sub(1);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(theme.added)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" use it  ·  ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            "r",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" regenerate  ·  ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" discard", Style::default().fg(theme.fg_dim)),
    ]));
    frame.render_widget(
        footer,
        Rect {
            x: area.x,
            y: footer_y,
            width: area.width,
            height: 1,
        },
    );
}

fn render_error(message: &str, area: Rect, frame: &mut Frame, theme: &Theme) {
    let header = Paragraph::new(Line::from(Span::styled(
        "Ollama returned an error:",
        Style::default().fg(theme.fg_dim),
    )));
    frame.render_widget(
        header,
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );

    let msg_area = Rect {
        x: area.x,
        y: area.y + 2,
        width: area.width,
        height: area.height.saturating_sub(5),
    };
    let body = Paragraph::new(Span::styled(
        message.to_string(),
        Style::default()
            .fg(theme.error)
            .add_modifier(Modifier::BOLD),
    ))
    .wrap(Wrap { trim: false });
    frame.render_widget(body, msg_area);

    // Hint section: suggest the most likely fix based on the error string.
    let hint = suggest_fix(message);
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3),
        width: area.width,
        height: 2,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            hint,
            Style::default()
                .fg(theme.fg_dim)
                .add_modifier(Modifier::ITALIC),
        ))
        .wrap(Wrap { trim: false }),
        hint_area,
    );

    let footer_y = area.y + area.height.saturating_sub(1);
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "r",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" retry  ·  ", Style::default().fg(theme.fg_dim)),
        Span::styled(
            "Esc",
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" dismiss", Style::default().fg(theme.fg_dim)),
    ]));
    frame.render_widget(
        footer,
        Rect {
            x: area.x,
            y: footer_y,
            width: area.width,
            height: 1,
        },
    );
}

/// Best-effort hint based on common Ollama error strings. Keeping this list
/// short and specific so it never drowns the actual error.
fn suggest_fix(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    if lower.contains("connection refused") || lower.contains("timed out connecting") {
        "Hint: the local Ollama daemon doesn't seem to be running. Start it with `ollama serve` in another terminal.".to_string()
    } else if lower.contains("model") && (lower.contains("not found") || lower.contains("missing")) {
        "Hint: the selected model isn't installed. Pull it with `ollama pull <model>` or pick another via M / settings.".to_string()
    } else if lower.contains("no models") {
        "Hint: Ollama has no models installed yet. Try `ollama pull qwen2.5-coder` to get started.".to_string()
    } else if lower.contains("nothing staged") {
        "Hint: stage at least one file (Space, or the + chip on a row) before generating a commit message.".to_string()
    } else {
        "Press r to retry, or Esc to dismiss. Check `ollama serve` is running and the selected model is available.".to_string()
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
