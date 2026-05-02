//! Color palettes for the graph: lane lines + ref pills.
//!
//! Two separate palettes:
//!   1. `LANE_PALETTE` — saturated colors used as foreground for `│ ─ ╮ ╯` etc.
//!      They're tuned to read well on a dark terminal background.
//!   2. `PILL_PALETTE` — *light* colors used as background for branch pills.
//!      Pill text is always rendered with `PILL_FG` (a near-black ink) so the
//!      contrast is comfortably above WCAG-AA on every entry.

use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::style::Color;

/// Lane-line colors (foreground use). 8 hues that read well on dark BG.
pub const LANE_PALETTE: [Color; 8] = [
    Color::Rgb(0x4F, 0xC1, 0xFF), // sky blue (default for `main` lane)
    Color::Rgb(0x73, 0xC9, 0x91), // green
    Color::Rgb(0xE2, 0xC0, 0x8D), // amber
    Color::Rgb(0xC5, 0x86, 0xC0), // purple
    Color::Rgb(0xF4, 0x70, 0x70), // coral
    Color::Rgb(0x6E, 0xCA, 0xC9), // teal
    Color::Rgb(0xDC, 0xDC, 0xAA), // pale yellow
    Color::Rgb(0xCE, 0x91, 0x78), // tan
];

/// Background colors for branch-name pills. Light, distinct, all readable
/// with the same dark `PILL_FG` text colour.
pub const PILL_PALETTE: [Color; 8] = [
    Color::Rgb(0x9E, 0xCB, 0xFF), // light blue
    Color::Rgb(0xC8, 0xA2, 0xE2), // soft purple
    Color::Rgb(0xA8, 0xE0, 0xB6), // mint
    Color::Rgb(0xFF, 0xB0, 0xB0), // soft coral
    Color::Rgb(0xFF, 0xD8, 0x8E), // peach
    Color::Rgb(0x9E, 0xDB, 0xDB), // pale teal
    Color::Rgb(0xFF, 0xC2, 0xDC), // soft pink
    Color::Rgb(0xB6, 0xC2, 0xE6), // periwinkle
];

/// Dedicated colours so HEAD / cloud / tag are instantly recognisable —
/// they do NOT come from the per-branch hashed palette. HEAD specifically
/// uses a saturated emerald background so the current branch reads clearly
/// regardless of whether its row is selected.
pub const HEAD_PILL_BG: Color = Color::Rgb(0x22, 0xC5, 0x5E); // saturated emerald for current HEAD
pub const HEAD_PILL_FG: Color = Color::Rgb(0x05, 0x1B, 0x0E);
pub const CLOUD_CHIP_BG: Color = Color::Rgb(0x60, 0xA5, 0xFA); // sky blue cloud chip
pub const CLOUD_CHIP_FG: Color = Color::Rgb(0x05, 0x12, 0x1F);
pub const REMOTE_ONLY_BG: Color = Color::Rgb(0x6E, 0x84, 0x99); // muted slate for remote-only
pub const REMOTE_ONLY_FG: Color = Color::Rgb(0xF8, 0xFA, 0xFC);
pub const TAG_PILL_BG: Color = Color::Rgb(0xFF, 0xD7, 0x6E); // bright gold for tags
pub const TAG_PILL_FG: Color = Color::Rgb(0x1F, 0x14, 0x05);
pub const PILL_FG: Color = Color::Rgb(0x10, 0x14, 0x18); // dark ink for branch pills

/// Pick a stable lane line colour for a "lane key" (the first commit hash that
/// landed in the lane, or — if we know it — the actual branch name).
pub fn for_key(key: &str) -> Color {
    if key.is_empty() {
        return LANE_PALETTE[0];
    }
    if matches!(key, "main" | "master" | "trunk") {
        return LANE_PALETTE[0];
    }
    LANE_PALETTE[hash_index(key, LANE_PALETTE.len())]
}

/// Pill background for a given branch name (NOT a SHA). `main`/`master`/`trunk`
/// always get index 0 (blue) so the main branch is recognisable across runs.
pub fn pill_bg_for_branch(name: &str) -> Color {
    if matches!(name, "main" | "master" | "trunk") {
        return PILL_PALETTE[0];
    }
    PILL_PALETTE[hash_index(name, PILL_PALETTE.len())]
}

/// Lane line colour by lane index (used when no explicit key is known).
pub fn for_lane(lane_idx: usize, key: Option<&str>) -> Color {
    if let Some(k) = key {
        return for_key(k);
    }
    LANE_PALETTE[lane_idx % LANE_PALETTE.len()]
}

fn hash_index(s: &str, modulus: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    (hasher.finish() as usize) % modulus
}
