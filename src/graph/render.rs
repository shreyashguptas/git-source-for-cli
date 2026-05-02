//! Render a single `Row` to colored 2-character cells.
//!
//! Each lane occupies 2 terminal columns: the glyph + a spacer (which can
//! become `─` for horizontal arms during merges/splits).

use std::collections::HashMap;

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};

use crate::{
    git::{Commit, RefName},
    graph::{color, Row},
    ui::theme::Theme,
};

const CELL_W: usize = 2;

/// Glyph + spacer chars for one lane cell.
#[derive(Debug, Clone, Copy)]
struct Cell {
    glyph: char,
    spacer: char,
    /// Color this cell should use (lane color, or commit lane color for arms).
    lane: usize,
}

impl Cell {
    const EMPTY: Cell = Cell {
        glyph: ' ',
        spacer: ' ',
        lane: 0,
    };
}

/// Build a row's cells from a `Row` and produce styled spans.
/// `prefix_width` is the number of cells (i.e. `CELL_W * lane_count`) to render —
/// callers can use this to pad short rows so the message column lines up.
pub fn row_spans<'a>(
    row: &Row,
    commit: &'a Commit,
    theme: &Theme,
    is_head: bool,
) -> Vec<Span<'a>> {
    let cells = build_cells(row);
    let mut spans = Vec::with_capacity(cells.len() * 2 + 4);

    for cell in &cells {
        let lane_key = lane_key_for_cell(row, cell.lane);
        let col = color::for_lane(cell.lane, lane_key);
        let glyph_style = Style::default().fg(col);
        let mut s = String::new();
        s.push(cell.glyph);
        s.push(cell.spacer);
        spans.push(Span::styled(s, glyph_style));
    }

    // Trailing space before the message text.
    spans.push(Span::raw(" "));

    // Short hash
    spans.push(Span::styled(
        commit.short_hash.clone(),
        Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::DIM),
    ));
    spans.push(Span::raw(" "));

    // Refs (branches/tags pointing here) — VS Code-style pills.
    let _ = is_head; // pill rendering decides this per-group from the refs themselves.
    for span in render_ref_pills(&commit.refs, theme) {
        spans.push(span);
    }

    // Subject + author
    spans.push(Span::styled(
        commit.subject.as_str(),
        Style::default().fg(theme.fg),
    ));
    spans.push(Span::styled(
        format!(" · {}", commit.author),
        Style::default().fg(theme.fg_dim),
    ));

    spans
}

fn lane_key_for_cell(row: &Row, lane: usize) -> Option<&str> {
    // Use the per-lane seed snapshot so a lane keeps the same colour across
    // every row it's active for. Falls back to the lane's expected SHA only
    // when we never seeded the lane (rare edge case).
    if let Some(k) = row.lane_keys.get(lane) {
        if !k.is_empty() {
            return Some(k.as_str());
        }
    }
    row.lanes.get(lane).and_then(|opt| opt.as_deref())
}

/// One logical branch's presence at a commit — consolidates HEAD/local/remote
/// refs that share the same branch name.
#[derive(Debug, Default)]
struct RefGroup {
    name: String,
    has_local: bool,
    has_remote: bool,
    is_head: bool,
}

/// Group a commit's refs by logical branch name. `origin/main` and `main`
/// collapse into one group with `has_remote=true, has_local=true`.
/// Returns (branch groups sorted with HEAD first, tag names sorted, bare-detached-HEAD flag).
fn group_refs(refs: &[RefName]) -> (Vec<RefGroup>, Vec<String>, bool) {
    let mut groups: HashMap<String, RefGroup> = HashMap::new();
    let mut tags: Vec<String> = Vec::new();
    let mut bare_head = false;

    for r in refs {
        match r {
            RefName::HeadAt(name) => {
                let g = groups.entry(name.clone()).or_insert_with(|| RefGroup {
                    name: name.clone(),
                    ..RefGroup::default()
                });
                g.has_local = true;
                g.is_head = true;
            }
            RefName::Head => bare_head = true,
            RefName::LocalBranch(name) => {
                let g = groups.entry(name.clone()).or_insert_with(|| RefGroup {
                    name: name.clone(),
                    ..RefGroup::default()
                });
                g.has_local = true;
            }
            RefName::RemoteBranch(full) => {
                // "origin/main" → "main"; "fork/feature/foo" → "feature/foo"
                let logical = full
                    .split_once('/')
                    .map(|(_remote, rest)| rest.to_string())
                    .unwrap_or_else(|| full.clone());
                let g = groups.entry(logical.clone()).or_insert_with(|| RefGroup {
                    name: logical,
                    ..RefGroup::default()
                });
                g.has_remote = true;
            }
            RefName::Tag(name) => tags.push(name.clone()),
            RefName::Other(_) => {}
        }
    }

    let mut groups: Vec<RefGroup> = groups.into_values().collect();
    // HEAD branch first, then alphabetical.
    groups.sort_by(|a, b| {
        b.is_head
            .cmp(&a.is_head)
            .then_with(|| a.name.cmp(&b.name))
    });
    tags.sort();
    (groups, tags, bare_head)
}

/// Build VS Code-style pills with colour-coded backgrounds. Every pill uses
/// dark text on a bright background so contrast is consistently high.
///
/// The previous design used a `☁` glyph for "this branch is on origin", but
/// in many monospace fonts that character renders thin and washed-out. We
/// switched to text-sized labels so the chips read at the same visual weight
/// as the rest of the UI.
///
/// - HEAD          → bright green pill `◉ <branch>`
/// - local branch  → light per-branch coloured pill `⎇ <name>`
/// - synced        → adjacent sky-blue chip with the word `synced`
/// - remote-only   → muted slate pill `↓ origin/<name>` (down-arrow = "you'd pull this")
/// - tags          → gold pill `▸ <name>`
/// - detached HEAD → standalone bright green pill `◉ HEAD`
fn render_ref_pills(refs: &[RefName], _theme: &Theme) -> Vec<Span<'static>> {
    let (groups, tags, bare_head) = group_refs(refs);
    let mut spans = Vec::new();

    // Detached HEAD that isn't co-located with any branch label → standalone pill.
    if bare_head && !groups.iter().any(|g| g.is_head) {
        spans.push(Span::styled(
            " ◉ HEAD ".to_string(),
            Style::default()
                .bg(color::HEAD_PILL_BG)
                .fg(color::HEAD_PILL_FG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    for g in groups {
        if g.has_local {
            let icon = if g.is_head { "◉" } else { "⎇" };
            let (bg, fg) = if g.is_head {
                (color::HEAD_PILL_BG, color::HEAD_PILL_FG)
            } else {
                (color::pill_bg_for_branch(&g.name), color::PILL_FG)
            };
            spans.push(Span::styled(
                format!(" {icon} {} ", g.name),
                Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
            ));
        } else if g.has_remote {
            // Remote-only branch — chunky text label, ↓ glyph implies "would pull".
            spans.push(Span::styled(
                format!(" ↓ origin/{} ", g.name),
                Style::default()
                    .bg(color::REMOTE_ONLY_BG)
                    .fg(color::REMOTE_ONLY_FG)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // If both local AND remote, append a `synced` chip immediately after
        // the local pill (no gap). Using the word so it carries text-weight in
        // every font instead of relying on the thin `☁` glyph.
        if g.has_local && g.has_remote {
            spans.push(Span::styled(
                " synced ".to_string(),
                Style::default()
                    .bg(color::CLOUD_CHIP_BG)
                    .fg(color::CLOUD_CHIP_FG)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        spans.push(Span::raw(" "));
    }

    for t in tags {
        spans.push(Span::styled(
            format!(" ▸ {t} "),
            Style::default()
                .bg(color::TAG_PILL_BG)
                .fg(color::TAG_PILL_FG)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    spans
}

/// Build the 2-char cells for a single row.
fn build_cells(row: &Row) -> Vec<Cell> {
    let max_lane = max_lane(row);
    let mut cells = vec![Cell::EMPTY; max_lane + 1];

    // 1. Pass-through `│` for every active lane in the snapshot, except the
    //    commit lane itself (which we set last) and lanes ending here.
    for (idx, slot) in row.lanes.iter().enumerate() {
        if idx >= cells.len() {
            break;
        }
        if slot.is_some() {
            cells[idx] = Cell {
                glyph: '│',
                spacer: ' ',
                lane: idx,
            };
        }
    }

    // 2. Spawned lanes (extra parents). Direction depends on lane vs commit.
    for &l in &row.spawned {
        if l == row.lane {
            continue;
        }
        if l > row.lane {
            cells[l] = Cell {
                glyph: '╮',
                spacer: ' ',
                lane: l,
            };
        } else {
            cells[l] = Cell {
                glyph: '╭',
                spacer: ' ',
                lane: l,
            };
        }
    }

    // 3. Absorbed lanes — they end here, flowing into commit lane.
    for &l in &row.absorbed {
        if l == row.lane {
            continue;
        }
        if l > row.lane {
            cells[l] = Cell {
                glyph: '╯',
                spacer: ' ',
                lane: l,
            };
        } else {
            cells[l] = Cell {
                glyph: '╰',
                spacer: ' ',
                lane: l,
            };
        }
    }

    // 4. Commit cell.
    let has_right_arm = row
        .absorbed
        .iter()
        .chain(row.spawned.iter())
        .any(|&l| l > row.lane);
    cells[row.lane] = Cell {
        glyph: '●',
        spacer: if has_right_arm { '─' } else { ' ' },
        lane: row.lane,
    };

    // 5. Fill horizontal arms between commit lane and each side-lane.
    fill_arms(&mut cells, row);

    cells
}

fn max_lane(row: &Row) -> usize {
    let mut m = row.lane;
    for &l in row.absorbed.iter().chain(row.spawned.iter()) {
        if l > m {
            m = l;
        }
    }
    if !row.lanes.is_empty() {
        // last index of last `Some`
        if let Some((idx, _)) = row
            .lanes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, slot)| slot.is_some())
        {
            if idx > m {
                m = idx;
            }
        }
    }
    m
}

fn fill_arms(cells: &mut [Cell], row: &Row) {
    let arm_lanes: Vec<usize> = row
        .absorbed
        .iter()
        .chain(row.spawned.iter())
        .copied()
        .filter(|&l| l != row.lane)
        .collect();

    for l in arm_lanes {
        if l > row.lane {
            // Fill (commit_lane, l) exclusive on both ends with `──`.
            for col in (row.lane + 1)..l {
                if cells[col].glyph == ' ' {
                    cells[col] = Cell {
                        glyph: '─',
                        spacer: '─',
                        lane: row.lane,
                    };
                } else {
                    // Crossing a passing-through lane — use a junction.
                    cells[col].spacer = '─';
                }
            }
            // The cell at l: spacer doesn't need ─ (the corner already turns).
            cells[row.lane].spacer = '─';
        } else if l < row.lane {
            for col in (l + 1)..row.lane {
                if cells[col].glyph == ' ' {
                    cells[col] = Cell {
                        glyph: '─',
                        spacer: '─',
                        lane: row.lane,
                    };
                } else {
                    cells[col].spacer = '─';
                }
            }
            // The arm enters the commit cell from the left — but we render
            // left-to-right, so the visible result is fine without modification.
        }
    }
}

/// Total terminal width consumed by the graph cells for a given row.
#[allow(dead_code)]
pub fn graph_width(row: &Row) -> usize {
    (max_lane(row) + 1) * CELL_W
}
