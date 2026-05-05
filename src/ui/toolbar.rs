//! Wrapping button-bar shared by the Branches and Changes panes.
//!
//! Each button renders as a colored chip ` label `; chips are packed
//! left-to-right with a horizontal gap. When the row's available width is
//! exhausted, the next button moves to a new row — every button always shows
//! up, no matter how narrow the pane gets. The toolbar is inset from the pane
//! border on every side and rows are separated by a blank row, so two-row
//! layouts read as a tidy grid rather than a wall of color.
//!
//! Per-button rects are returned so the click handler can hit-test them.

use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

// Spacing knobs. Tuned together — change one and the whole bar shifts.
const TOP_PAD: u16 = 1;
const BOTTOM_PAD: u16 = 1;
const LEFT_PAD: u16 = 1;
const RIGHT_PAD: u16 = 1;
const HORIZ_GAP: u16 = 2;
const INTER_ROW_GAP: u16 = 1;

pub struct PackedButton<T> {
    pub action: T,
    pub label: &'static str,
    pub rect: Rect,
}

/// Pack `buttons` into rows that fit inside `area.width`. Every button is
/// included; if a single button is wider than the available width it gets its
/// own row and is truncated visually by the terminal — but the rect still
/// tracks its full intended width so clicks land.
pub fn pack<T: Copy>(buttons: &[(&'static str, T)], area: Rect) -> Vec<PackedButton<T>> {
    let mut out: Vec<PackedButton<T>> = Vec::with_capacity(buttons.len());
    if area.width <= LEFT_PAD + RIGHT_PAD || area.height <= TOP_PAD + BOTTOM_PAD {
        return out;
    }
    let row_left = area.x + LEFT_PAD;
    let row_right = area.x + area.width - RIGHT_PAD;
    let mut x = row_left;
    let mut y = area.y + TOP_PAD;
    for (label, action) in buttons {
        let chip_w = chip_width(label);
        // Wrap when this chip won't fit — unless we're already at the start of
        // a row (then place it anyway so it's never dropped).
        let needs_wrap = x > row_left && x + chip_w > row_right;
        if needs_wrap {
            x = row_left;
            y = y.saturating_add(1 + INTER_ROW_GAP);
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
        x = x.saturating_add(chip_w).saturating_add(HORIZ_GAP);
    }
    out
}

/// Total number of rows used by `packed` — top pad + chip rows + inter-row
/// gaps + bottom pad. Returns 0 when nothing was packed.
pub fn rows_used<T>(packed: &[PackedButton<T>], area: Rect) -> u16 {
    let Some(max_y) = packed.iter().map(|p| p.rect.y).max() else {
        return 0;
    };
    let chip_end_offset = max_y.saturating_sub(area.y).saturating_add(1);
    chip_end_offset.saturating_add(BOTTOM_PAD)
}

/// Render `packed` chips into `area`. `style_for(action)` returns the chip's
/// style. Every cell of `area` not covered by a chip is filled with `bg_style`,
/// so the whole bar reads as a unified panel — including the top/bottom margin
/// and the gap rows between wrapped lines.
pub fn render<T: Copy>(
    packed: &[PackedButton<T>],
    style_for: impl Fn(T) -> Style,
    bg_style: Style,
    area: Rect,
    frame: &mut Frame,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Bucket chips by their row offset within `area`.
    let mut rows: Vec<Vec<&PackedButton<T>>> =
        (0..area.height).map(|_| Vec::new()).collect();
    for chip in packed {
        let offset = chip.rect.y.saturating_sub(area.y) as usize;
        if offset < rows.len() {
            rows[offset].push(chip);
        }
    }

    let area_right = area.x + area.width;
    for (row_idx, row_chips) in rows.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(row_chips.len() * 2 + 2);
        let mut x = area.x;
        for chip in row_chips {
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
