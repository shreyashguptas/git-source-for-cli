//! Wrapping button-bar shared by the Branches and Changes panes.
//!
//! Each button renders as a colored chip ` label `; chips are packed
//! left-to-right with a 1-col gap. When the row's available width is exhausted,
//! the next button moves to a new row — every button always shows up, no
//! matter how narrow the pane gets. Per-button rects are returned so the
//! click handler can hit-test them.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Compute the row layout for `buttons`. Each entry is a label + the
/// caller-supplied associated value. `inner_pad` is the space added to either
/// side of the label inside the chip (so chip width = label.width() + 2).
pub struct PackedButton<T> {
    pub action: T,
    pub label: &'static str,
    pub rect: Rect,
}

/// Pack `buttons` into rows that fit inside `area.width`. The returned vector
/// describes every chip's on-screen rect so the caller can: (a) render each
/// row with a single Paragraph (one Line per row), or (b) hit-test clicks.
///
/// Every button is included in the result. If a single button is wider than
/// the available width, it gets its own row and is truncated visually by the
/// terminal — but the rect still tracks its full intended width so clicks land.
pub fn pack<T: Copy>(buttons: &[(&'static str, T)], area: Rect) -> Vec<PackedButton<T>> {
    let mut out: Vec<PackedButton<T>> = Vec::with_capacity(buttons.len());
    if area.width == 0 || area.height == 0 {
        return out;
    }
    let mut x = area.x;
    let mut y = area.y;
    let area_right = area.x + area.width;
    for (label, action) in buttons {
        let chip_w = chip_width(label);
        // If this chip won't fit on the current row, wrap (unless we're at
        // the start of a row already — then place it anyway so it's never
        // dropped). Reserve 1 col for the gap after, except for the last button.
        let needs_wrap = x > area.x && x + chip_w > area_right;
        if needs_wrap {
            x = area.x;
            y += 1;
        }
        let rect = Rect {
            x,
            y,
            width: chip_w,
            height: 1,
        };
        out.push(PackedButton {
            action: *action,
            label,
            rect,
        });
        // Advance with a 1-col gap.
        x = x.saturating_add(chip_w).saturating_add(1);
    }
    out
}

/// Total number of rows used by `packed` — equals the highest y minus the
/// area.y, plus 1. Returns 0 when packed is empty.
pub fn rows_used<T>(packed: &[PackedButton<T>], area: Rect) -> u16 {
    packed
        .iter()
        .map(|p| p.rect.y.saturating_sub(area.y) + 1)
        .max()
        .unwrap_or(0)
}

/// Render `packed` chips into `area`. `style_for(action)` returns the chip's
/// style. The space between chips and any trailing padding is filled with
/// `bg_style` so the row reads as a solid bar.
pub fn render<T: Copy>(
    packed: &[PackedButton<T>],
    style_for: impl Fn(T) -> Style,
    bg_style: Style,
    area: Rect,
    frame: &mut Frame,
) {
    if packed.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    // Group chips by row (y) so each row renders as a single Line.
    let mut rows: Vec<Vec<&PackedButton<T>>> = Vec::new();
    for chip in packed {
        let row_idx = (chip.rect.y - area.y) as usize;
        while rows.len() <= row_idx {
            rows.push(Vec::new());
        }
        rows[row_idx].push(chip);
    }

    for (row_idx, row_chips) in rows.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(row_chips.len() * 2);
        let mut x = area.x;
        let area_right = area.x + area.width;
        for chip in row_chips {
            // Leading gap (so chips have a blank between them).
            if x < chip.rect.x {
                let gap = (chip.rect.x - x) as usize;
                spans.push(Span::styled(" ".repeat(gap), bg_style));
                x = chip.rect.x;
            }
            let text = format!(" {} ", chip.label);
            spans.push(Span::styled(text, style_for(chip.action)));
            x = x.saturating_add(chip.rect.width);
        }
        if x < area_right {
            spans.push(Span::styled(
                " ".repeat((area_right - x) as usize),
                bg_style,
            ));
        }
        let row_area = Rect {
            x: area.x,
            y: area.y + row_idx as u16,
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Line::from(spans)), row_area);
    }
}

fn chip_width(label: &str) -> u16 {
    // Chip text is ` label ` — leading + trailing space.
    label.width().saturating_add(2) as u16
}
