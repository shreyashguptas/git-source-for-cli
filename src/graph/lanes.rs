//! Lane allocation for git DAG rendering.
//!
//! For each commit (in date-descending order), this computes:
//!   - the lane (column) where its `●` glyph lives
//!   - which other lanes are "absorbed" here (incoming merges)
//!   - which new lanes spawn for additional parents
//!   - the full lane state at this row (for drawing pass-through `│` lines)
//!
//! The algorithm is straightforward: maintain a `Vec<Option<Hash>>` of lane
//! "expectations" — each slot holds the SHA the lane is waiting to draw next.
//! When we hit that commit, the lane "lands" on it.

use std::collections::HashMap;

use crate::git::Commit;

/// One rendered row in the graph (1:1 with a commit).
#[derive(Debug, Clone)]
pub struct Row {
    pub commit_idx: usize,
    /// The column where `●` is drawn.
    pub lane: usize,
    /// Other lanes that were waiting on this commit and are now absorbed
    /// (rendered as `╯`/`╰` joining into `lane`).
    pub absorbed: Vec<usize>,
    /// Lanes that were just allocated for this commit's extra parents
    /// (rendered as `╮`/`╭` flowing from `lane`).
    pub spawned: Vec<usize>,
    /// Full lane state at THIS row's commit (after lane allocation, before
    /// the next commit). Slots that are `Some` will draw `│` if they're not
    /// the commit/absorbed/spawned lane themselves.
    pub lanes: Vec<Option<String>>,
    /// Stable color key for `lane` (first commit hash that ever landed there).
    pub lane_key: String,
}

/// Compute the per-row layout for a slice of commits in date order (newest first).
pub fn layout(commits: &[Commit]) -> Vec<Row> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut lane_keys: Vec<String> = Vec::new();
    let mut hash_to_lane: HashMap<String, usize> = HashMap::new();
    let mut rows = Vec::with_capacity(commits.len());

    for (i, c) in commits.iter().enumerate() {
        // Step 1: find ALL lanes expecting this commit.
        let mut occupied: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(idx, h)| {
                if h.as_deref() == Some(c.hash.as_str()) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();
        occupied.sort_unstable();

        // Step 2: pick the commit's lane.
        let commit_lane = match occupied.first() {
            Some(&l) => l,
            None => {
                // Brand-new lane (e.g. branch tip with no children in the log)
                let l = first_free_lane(&lanes);
                ensure_lane(&mut lanes, &mut lane_keys, l, &c.hash);
                l
            }
        };

        // Color key: first SHA that landed in this lane keeps it.
        if lane_keys.get(commit_lane).map(String::is_empty).unwrap_or(true) {
            ensure_lane_key(&mut lane_keys, commit_lane, &c.hash);
        }
        let lane_key = lane_keys[commit_lane].clone();

        // Step 3: capture absorbed lanes (everything other than commit_lane).
        let absorbed: Vec<usize> = occupied
            .iter()
            .copied()
            .filter(|&l| l != commit_lane)
            .collect();

        // Step 4: clear all occupied lanes; we'll re-fill the commit_lane below.
        for &l in &occupied {
            lanes[l] = None;
        }
        // Don't clear the lane_key — it's a stable color seed.

        // Step 5: assign first parent (if any) to commit_lane.
        let mut spawned = Vec::new();
        if let Some(p1) = c.parents.first() {
            ensure_lane(&mut lanes, &mut lane_keys, commit_lane, &lane_key);
            lanes[commit_lane] = Some(p1.clone());
            hash_to_lane.entry(p1.clone()).or_insert(commit_lane);
        } else {
            // Root commit — lane terminates after this row.
            // (We leave lanes[commit_lane] = None so it's freed.)
        }

        // Step 6: assign extra parents.
        for parent in c.parents.iter().skip(1) {
            // Reuse a lane that's already expecting this parent (collapse).
            if let Some(&existing) = hash_to_lane.get(parent) {
                if lanes
                    .get(existing)
                    .map(|h| h.as_deref() == Some(parent.as_str()))
                    .unwrap_or(false)
                {
                    spawned.push(existing);
                    continue;
                }
            }
            // Otherwise, prefer reusing one of the just-freed absorbed lanes.
            let reusable = absorbed
                .iter()
                .copied()
                .find(|&l| lanes.get(l).map(Option::is_none).unwrap_or(false));
            let l = match reusable {
                Some(l) => l,
                None => first_free_lane(&lanes),
            };
            ensure_lane(&mut lanes, &mut lane_keys, l, parent);
            lanes[l] = Some(parent.clone());
            hash_to_lane.insert(parent.clone(), l);
            spawned.push(l);
        }

        // Step 7: snapshot lane state for this row.
        let lane_snapshot = lanes.clone();

        // Step 8: trim trailing free lanes so the visualization isn't padded
        // with empty space.
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
            lane_keys.pop();
        }

        rows.push(Row {
            commit_idx: i,
            lane: commit_lane,
            absorbed,
            spawned,
            lanes: lane_snapshot,
            lane_key,
        });
    }
    rows
}

fn first_free_lane(lanes: &[Option<String>]) -> usize {
    lanes
        .iter()
        .position(Option::is_none)
        .unwrap_or(lanes.len())
}

fn ensure_lane(
    lanes: &mut Vec<Option<String>>,
    lane_keys: &mut Vec<String>,
    idx: usize,
    key_seed: &str,
) {
    while lanes.len() <= idx {
        lanes.push(None);
        lane_keys.push(String::new());
    }
    if lane_keys[idx].is_empty() {
        lane_keys[idx] = key_seed.to_string();
    }
}

fn ensure_lane_key(lane_keys: &mut [String], idx: usize, key_seed: &str) {
    if let Some(k) = lane_keys.get_mut(idx) {
        if k.is_empty() {
            *k = key_seed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            short_hash: hash.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            subject: String::new(),
            author: String::new(),
            time: 0,
            refs: Vec::new(),
        }
    }

    #[test]
    fn linear_history_uses_one_lane() {
        let cs = vec![
            commit("c", &["b"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ];
        let rows = layout(&cs);
        assert_eq!(rows.len(), 3);
        for r in &rows {
            assert_eq!(r.lane, 0);
            assert!(r.absorbed.is_empty());
            assert!(r.spawned.is_empty());
        }
    }

    #[test]
    fn merge_commit_marks_absorbed_lane() {
        // c is a merge of b and x. Topologically:
        //   * c (parents: b, x)
        //   * b (parent: a)
        //   * x (parent: a)
        //   * a
        let cs = vec![
            commit("c", &["b", "x"]),
            commit("b", &["a"]),
            commit("x", &["a"]),
            commit("a", &[]),
        ];
        let rows = layout(&cs);
        // c should land on lane 0, with x spawning to lane 1.
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[0].spawned, vec![1]);
        assert!(rows[0].absorbed.is_empty());
        // b is on lane 0 (continuation of c's first parent)
        assert_eq!(rows[1].lane, 0);
        // x is on lane 1
        assert_eq!(rows[2].lane, 1);
        // a absorbs lane 1 into lane 0 (or vice versa) — test that the
        // remaining commit is on a single lane and absorbs the other.
        assert_eq!(rows[3].absorbed.len() + rows[3].spawned.len(), 1);
    }

    #[test]
    fn branch_tip_with_no_children_gets_new_lane() {
        // Two independent histories in `git log --all`:
        //   * b (parent: a)
        //   * x (parent: a)   <- a separate branch tip
        //   * a
        let cs = vec![
            commit("b", &["a"]),
            commit("x", &["a"]),
            commit("a", &[]),
        ];
        let rows = layout(&cs);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1, "second branch tip should get its own lane");
        // `a` is now expected by both lanes 0 and 1.
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[2].absorbed, vec![1]);
    }
}
