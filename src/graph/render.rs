//! Render a single `Row` to colored 2-character cells.
//!
//! Each lane occupies 2 terminal columns: the glyph + a spacer (which can
//! become `─` for horizontal arms during merges/splits).

use std::collections::HashMap;

use ratatui::{
    style::{Modifier, Style},
    text::Span,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
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

/// Lane count needed to render this row (highest occupied lane index + 1).
/// The graph pane uses the max of this across all rows to pad every row's
/// graph cells to the same width — that's what keeps the hash / subject /
/// pill columns aligned regardless of how wide a particular merge fans out.
pub fn lane_count(row: &Row) -> usize {
    max_lane(row) + 1
}

/// Build a row's styled spans laid out in fixed columns:
///
///   `[graph cells (padded to max_lanes)] [hash] [subject] [pills] [· author]`
///
/// `max_lanes` pads short rows so the hash columns line up across the whole
/// graph regardless of how wide a particular merge fans out.
///
/// Ref pills (HEAD / branch / synced / origin / tags) sit *between* the
/// subject and the author so they remain visible even when the pane is
/// narrow — the subject is the elastic element and gets ellipsized first to
/// keep the branch/sync information on screen without an expand click. The
/// author is anchored at the very end of the line.
///
/// `body_width` is the usable horizontal space inside the pane (already
/// discounted for borders and any divergence marker the caller prepends).
/// Pass `usize::MAX` to disable budgeted truncation.
pub fn row_spans<'a>(
    row: &Row,
    commit: &'a Commit,
    theme: &Theme,
    is_head: bool,
    is_ahead: bool,
    max_lanes: usize,
    body_width: usize,
) -> Vec<Span<'a>> {
    let _ = is_head; // pill rendering decides HEAD per-group from the refs themselves
    let cells = build_cells(row);
    let lane_slots = max_lanes.max(cells.len());
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(lane_slots + 8);

    // 1. Graph cells, padded to `lane_slots`. The commit-lane `●` adopts the
    //    same green as the `↑` marker when this commit is ahead of the
    //    upstream — visual reinforcement that this commit is local-only and
    //    needs to be pushed. Arms and pass-throughs stay lane-coloured so the
    //    branch ribbon is still readable.
    for i in 0..lane_slots {
        if let Some(cell) = cells.get(i) {
            let lane_key = lane_key_for_cell(row, cell.lane);
            let col = color::for_lane(cell.lane, lane_key);
            let is_commit_cell = i == row.lane && cell.glyph == '●';
            let fg = if is_commit_cell && is_ahead {
                theme.added
            } else {
                col
            };
            let mut style = Style::default().fg(fg);
            if is_commit_cell && is_ahead {
                style = style.add_modifier(Modifier::BOLD);
            }
            let mut s = String::new();
            s.push(cell.glyph);
            s.push(cell.spacer);
            spans.push(Span::styled(s, style));
        } else {
            spans.push(Span::raw("  "));
        }
    }

    // 2. Separator + hash + separator.
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        commit.short_hash.clone(),
        Style::default()
            .fg(theme.fg_dim)
            .add_modifier(Modifier::DIM),
    ));
    spans.push(Span::raw(" "));

    // Pre-render pills so we know their width before deciding how much room
    // to give the subject.
    let pills: Vec<Span<'static>> = render_ref_pills(&commit.refs, theme);
    let pills_width: usize = pills.iter().map(|s| s.content.as_ref().width()).sum();
    let pills_gap = if pills.is_empty() { 0 } else { 2 };

    let author_full = format!(" · {}", commit.author);
    let author_full_width = author_full.width();

    // Width already consumed by graph + " " + hash + " ".
    let prefix_width = lane_slots * CELL_W + 1 + commit.short_hash.width() + 1;

    // Subject is the elastic element — pills must always fit, so we shave
    // the subject (and, only as a last resort, the author) until everything
    // fits inside `body_width`.
    let mut subject_budget =
        body_width.saturating_sub(prefix_width + pills_gap + pills_width + author_full_width);

    let mut author_text = author_full;
    if subject_budget < 3 {
        // Pane is so narrow we can't even show a 2-char subject + ellipsis.
        // Sacrifice the author next: pills take priority, per the spec.
        let want_subject = 3usize;
        let extra_needed = want_subject.saturating_sub(subject_budget);
        let new_author_width = author_text.width().saturating_sub(extra_needed);
        author_text = truncate_end(&author_text, new_author_width);
        subject_budget = body_width
            .saturating_sub(prefix_width + pills_gap + pills_width + author_text.width());
    }

    let subject = truncate_end(&commit.subject, subject_budget);

    // 3. Subject (may be ellipsized or empty).
    if !subject.is_empty() {
        spans.push(Span::styled(subject, Style::default().fg(theme.fg)));
    }

    // 4. Ref pills — anchored before the author so they survive narrow panes.
    if !pills.is_empty() {
        spans.push(Span::raw("  "));
        spans.extend(pills);
    }

    // 5. Author — anchored as the very last element on the line.
    if !author_text.is_empty() {
        spans.push(Span::styled(
            author_text,
            Style::default().fg(theme.fg_dim),
        ));
    }

    spans
}

/// Truncate `value` to `max_width` display columns, appending `…` if cut.
fn truncate_end(value: &str, max_width: usize) -> String {
    if value.width() <= max_width {
        return value.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    let limit = max_width - 1;
    for ch in value.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > limit {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push('…');
    out
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

    // Drop the trailing single-space separator — pills are right-aligned now,
    // so a trailing space would push them off the visible right edge.
    if let Some(last) = spans.last() {
        if last.content.as_ref() == " " {
            spans.pop();
        }
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

    // 3. Absorbed lanes — they end here, flowing into commit lane. If the
    //    same lane is ALSO spawning (a merge that consumes one branch and
    //    immediately spawns another into the same column — common at PR
    //    merges where the side branch's tail meets a fresh side branch),
    //    leave it as `│`: the lane was alive coming in and is alive going
    //    out, and the horizontal arm in the spacer already shows the merge.
    for &l in &row.absorbed {
        if l == row.lane {
            continue;
        }
        let passthrough = row.spawned.contains(&l);
        let glyph = if passthrough {
            '│'
        } else if l > row.lane {
            '╯'
        } else {
            '╰'
        };
        cells[l] = Cell {
            glyph,
            spacer: ' ',
            lane: l,
        };
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
