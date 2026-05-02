//! Branch/lane → palette index. Stable across refreshes (hash-based) so the
//! same branch always paints in the same color.

use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::style::Color;

/// Eight distinguishable hues that read well on a dark background.
/// Order matters: index 0 is reserved for `main`/`master`.
pub const PALETTE: [Color; 8] = [
    Color::Rgb(0x4F, 0xC1, 0xFF), // sky blue (main)
    Color::Rgb(0x73, 0xC9, 0x91), // green
    Color::Rgb(0xE2, 0xC0, 0x8D), // amber
    Color::Rgb(0xC5, 0x86, 0xC0), // purple
    Color::Rgb(0xF4, 0x70, 0x70), // coral
    Color::Rgb(0x6E, 0xCA, 0xC9), // teal
    Color::Rgb(0xDC, 0xDC, 0xAA), // pale yellow
    Color::Rgb(0xCE, 0x91, 0x78), // tan
];

/// Pick a stable color for a "lane key" (we use the first commit hash that
/// landed in the lane, which roughly corresponds to a branch tip).
pub fn for_key(key: &str) -> Color {
    if key.is_empty() {
        return PALETTE[0];
    }
    // Special-case the main branches so they're always the same.
    if matches!(key, "main" | "master" | "trunk") {
        return PALETTE[0];
    }
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let h = hasher.finish() as usize;
    PALETTE[h % PALETTE.len()]
}

/// Color for a lane index, with the branch-key override applied if present.
pub fn for_lane(lane_idx: usize, key: Option<&str>) -> Color {
    if let Some(k) = key {
        return for_key(k);
    }
    PALETTE[lane_idx % PALETTE.len()]
}
