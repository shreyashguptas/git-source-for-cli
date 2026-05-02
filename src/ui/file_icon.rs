//! Shared mapping from a file path to its language/Devicons-style glyph and
//! the brand color we paint it with. Used by the Changes pane file rows AND
//! by the diff Preview pane's file-header bar so they stay visually consistent.

use ratatui::style::{Color, Style};

use crate::ui::theme::Theme;

/// Pick a Nerd-Font glyph + color for `path`. The matching is `lowercase
/// basename → extension`, falling back to a generic "" file glyph.
pub fn file_icon(path: &str, theme: &Theme) -> (&'static str, Style) {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");

    let (icon, color) = match name {
        "cargo.toml" | "cargo.lock" => ("", Color::Rgb(0xDE, 0xA5, 0x84)),
        "dockerfile" => ("", Color::Rgb(0x24, 0x96, 0xED)),
        "makefile" => ("", Color::Rgb(0xB8, 0xB8, 0xB8)),
        _ => match ext {
            "rs" => ("", Color::Rgb(0xDE, 0xA5, 0x84)),
            "ts" => ("", Color::Rgb(0x31, 0x78, 0xC6)),
            "tsx" => ("", Color::Rgb(0x61, 0xDA, 0xFB)),
            "js" | "mjs" | "cjs" => ("", Color::Rgb(0xF7, 0xDF, 0x1E)),
            "jsx" => ("", Color::Rgb(0x61, 0xDA, 0xFB)),
            "json" => ("", Color::Rgb(0xF7, 0xDF, 0x1E)),
            "html" | "htm" => ("", Color::Rgb(0xE3, 0x4C, 0x26)),
            "css" => ("", Color::Rgb(0x56, 0x9C, 0xD6)),
            "scss" | "sass" => ("", Color::Rgb(0xC6, 0x53, 0x8C)),
            "md" | "markdown" => ("", Color::Rgb(0x56, 0x9C, 0xD6)),
            "py" => ("", Color::Rgb(0xFF, 0xD4, 0x3B)),
            "go" => ("", Color::Rgb(0x00, 0xAD, 0xD8)),
            "java" => ("", Color::Rgb(0xF8, 0x98, 0x20)),
            "kt" | "kts" => ("", Color::Rgb(0xB1, 0x25, 0xEA)),
            "rb" => ("", Color::Rgb(0xCC, 0x34, 0x2D)),
            "php" => ("", Color::Rgb(0x77, 0x7B, 0xB4)),
            "sh" | "bash" | "zsh" | "fish" => ("", Color::Rgb(0x89, 0xE0, 0x51)),
            "yml" | "yaml" | "toml" | "ini" => ("", theme.fg_dim),
            "c" | "h" => ("", Color::Rgb(0x59, 0x9E, 0xD8)),
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => ("", Color::Rgb(0x00, 0x59, 0x9C)),
            "cs" => ("󰌛", Color::Rgb(0x68, 0x2A, 0xD7)),
            "swift" => ("", Color::Rgb(0xF0, 0x51, 0x38)),
            _ => ("", theme.fg_dim),
        },
    };

    (icon, Style::default().fg(color))
}
