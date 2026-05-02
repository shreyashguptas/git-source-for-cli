//! Centralized colors + styles. One file so a future "theme" config can swap it.

use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub border: Color,
    pub border_active: Color,
    pub accent: Color,
    pub error: Color,
    pub branch_current: Color,
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub selection_bg_active: Color,
    pub selection_bg_inactive: Color,
    pub modified: Color,
    pub added: Color,
    pub deleted: Color,
    pub renamed: Color,
    pub toast_bg: Color,
}

impl Theme {
    pub const fn default_dark() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Rgb(0xCC, 0xCC, 0xCC),
            fg_dim: Color::Rgb(0x80, 0x80, 0x80),
            border: Color::Rgb(0x44, 0x44, 0x44),
            border_active: Color::Rgb(0x4F, 0xC1, 0xFF),
            accent: Color::Rgb(0x4F, 0xC1, 0xFF),
            error: Color::Rgb(0xF4, 0x47, 0x47),
            branch_current: Color::Rgb(0x73, 0xC9, 0x91),
            status_bar_bg: Color::Rgb(0x00, 0x7A, 0xCC),
            status_bar_fg: Color::Rgb(0xFF, 0xFF, 0xFF),
            selection_bg_active: Color::Rgb(0x09, 0x4A, 0x77),
            selection_bg_inactive: Color::Rgb(0x2A, 0x2A, 0x2A),
            modified: Color::Rgb(0xE2, 0xC0, 0x8D),
            added: Color::Rgb(0x73, 0xC9, 0x91),
            deleted: Color::Rgb(0xF4, 0x47, 0x47),
            renamed: Color::Rgb(0x4F, 0xC1, 0xFF),
            toast_bg: Color::Rgb(0x5A, 0x1D, 0x1D),
        }
    }

    pub fn block_border(&self, active: bool) -> Style {
        Style::default().fg(if active {
            self.border_active
        } else {
            self.border
        })
    }

    pub fn title(&self, active: bool) -> Style {
        let base = Style::default().fg(if active { self.accent } else { self.fg_dim });
        if active {
            base.add_modifier(Modifier::BOLD)
        } else {
            base
        }
    }

    pub fn status_bar(&self) -> Style {
        Style::default()
            .bg(self.status_bar_bg)
            .fg(self.status_bar_fg)
    }

    pub fn selection_bg(&self, active: bool) -> Color {
        if active {
            self.selection_bg_active
        } else {
            self.selection_bg_inactive
        }
    }

    pub fn toast_error(&self) -> Style {
        Style::default()
            .bg(self.toast_bg)
            .fg(Color::Rgb(0xFF, 0xFF, 0xFF))
            .add_modifier(Modifier::BOLD)
    }

    /// Neutral info-style toast (used for op results, both success and benign failures).
    pub fn toast_info(&self) -> Style {
        Style::default()
            .bg(Color::Rgb(0x33, 0x33, 0x33))
            .fg(Color::Rgb(0xFF, 0xFF, 0xFF))
    }
}

pub fn current() -> Theme {
    Theme::default_dark()
}
