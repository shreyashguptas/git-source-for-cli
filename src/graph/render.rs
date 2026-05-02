//! Render a single `Row` to colored 2-character cells.
//!
//! Each lane occupies 2 terminal columns: the glyph + a spacer (which can
//! become `─` for horizontal arms during merges/splits).

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

    // Refs (branches/tags pointing here)
    for r in &commit.refs {
        let (label, style) = ref_label(r, theme, is_head);
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
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
    if lane == row.lane {
        return Some(row.lane_key.as_str());
    }
    // For non-commit lanes, derive a key from the lane state hash if any.
    row.lanes
        .get(lane)
        .and_then(|opt| opt.as_deref())
}

fn ref_label(r: &RefName, theme: &Theme, is_head: bool) -> (String, Style) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    match r {
        RefName::HeadAt(name) => (
            format!(" HEAD → {name} "),
            bold.fg(theme.branch_current).bg(theme.selection_bg(false)),
        ),
        RefName::Head => (
            " HEAD ".to_string(),
            bold.fg(theme.branch_current).bg(theme.selection_bg(false)),
        ),
        RefName::LocalBranch(name) => (
            format!(" {name} "),
            bold.fg(if is_head {
                theme.branch_current
            } else {
                theme.accent
            }),
        ),
        RefName::RemoteBranch(name) => (format!(" {name} "), Style::default().fg(theme.fg_dim)),
        RefName::Tag(name) => (
            format!(" tag:{name} "),
            Style::default().fg(theme.modified).add_modifier(Modifier::ITALIC),
        ),
        RefName::Other(name) => (format!(" {name} "), Style::default().fg(theme.fg_dim)),
    }
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
