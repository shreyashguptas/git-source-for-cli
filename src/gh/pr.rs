use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
    ChangesRequested,
}

#[derive(Debug, Clone)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub head_branch: String,
    pub state: PrState,
    pub url: String,
}

#[derive(Debug, Deserialize)]
struct GhPr {
    number: u64,
    title: String,
    state: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    url: String,
    #[serde(rename = "reviewDecision", default)]
    review_decision: Option<String>,
}

/// Fetch up to 100 PRs (open + recently closed) and return them keyed by branch name.
/// `repo_root` is passed as cwd so `gh` picks the right repo.
pub async fn fetch_prs(repo_root: &Path) -> Result<HashMap<String, Pr>> {
    let out = Command::new("gh")
        .current_dir(repo_root)
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--limit",
            "100",
            "--json",
            "number,title,state,headRefName,isDraft,url,reviewDecision",
        ])
        .output()
        .await
        .context("failed to spawn `gh` (is it installed?)")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("gh pr list failed: {}", stderr.trim());
    }

    let parsed: Vec<GhPr> = serde_json::from_slice(&out.stdout).context("malformed gh JSON")?;
    let mut map = HashMap::new();
    for p in parsed {
        let state = parse_state(&p.state, p.is_draft, p.review_decision.as_deref());
        // For each branch, prefer the most recent open PR over closed/merged.
        let entry = map.entry(p.head_ref_name.clone());
        let pr = Pr {
            number: p.number,
            title: p.title,
            head_branch: p.head_ref_name,
            state,
            url: p.url,
        };
        match entry {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(pr);
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if priority(&pr.state) > priority(&e.get().state) {
                    e.insert(pr);
                }
            }
        }
    }
    Ok(map)
}

fn parse_state(state: &str, is_draft: bool, review: Option<&str>) -> PrState {
    if is_draft {
        return PrState::Draft;
    }
    if review == Some("CHANGES_REQUESTED") {
        return PrState::ChangesRequested;
    }
    match state.to_ascii_uppercase().as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    }
}

/// Higher = preferred when deduping multiple PRs for the same branch.
fn priority(state: &PrState) -> u8 {
    match state {
        PrState::Open | PrState::ChangesRequested => 4,
        PrState::Draft => 3,
        PrState::Merged => 2,
        PrState::Closed => 1,
    }
}
