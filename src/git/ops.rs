//! Write operations: stage, unstage, commit, push, pull, fetch, branch, checkout.
//!
//! Each function returns the user-facing summary (last line of git's output, or
//! a synthesized one) so the UI can show a toast on success/failure.

use std::path::Path;

use anyhow::Result;

use super::exec;

pub async fn stage(repo_root: &Path, path: &str) -> Result<String> {
    exec::run(repo_root, ["add", "--", path]).await?;
    Ok(format!("staged {path}"))
}

pub async fn unstage(repo_root: &Path, path: &str) -> Result<String> {
    // `git restore --staged` is the modern command; fall back to `reset HEAD --` if
    // it errors (e.g. for unborn HEAD on a brand-new repo).
    if exec::run(repo_root, ["restore", "--staged", "--", path])
        .await
        .is_err()
    {
        exec::run(repo_root, ["reset", "HEAD", "--", path]).await?;
    }
    Ok(format!("unstaged {path}"))
}

pub async fn stage_all(repo_root: &Path) -> Result<String> {
    exec::run(repo_root, ["add", "-A"]).await?;
    Ok("staged all changes".to_string())
}

pub async fn unstage_all(repo_root: &Path) -> Result<String> {
    let _ = exec::run(repo_root, ["reset", "HEAD"]).await;
    Ok("unstaged all changes".to_string())
}

/// Commit with the given message. Honors user's git config (signing, hooks).
pub async fn commit(repo_root: &Path, message: &str) -> Result<String> {
    let out = exec::run(repo_root, ["commit", "-m", message]).await?;
    Ok(out
        .lines()
        .next()
        .unwrap_or("commit created")
        .to_string())
}

pub async fn push(repo_root: &Path) -> Result<String> {
    let out = exec::run(repo_root, ["push"]).await?;
    Ok(summary_or(out, "pushed"))
}

/// Push with `-u origin <branch>` — used when the branch has no upstream yet.
pub async fn push_set_upstream(repo_root: &Path, branch: &str) -> Result<String> {
    let out = exec::run(repo_root, ["push", "-u", "origin", branch]).await?;
    Ok(summary_or(out, &format!("pushed and set upstream to origin/{branch}")))
}

pub async fn pull(repo_root: &Path) -> Result<String> {
    // --ff-only: never merge; the user can drop to terminal for non-ff cases.
    let out = exec::run(repo_root, ["pull", "--ff-only"]).await?;
    Ok(summary_or(out, "pulled"))
}

pub async fn fetch_all(repo_root: &Path) -> Result<String> {
    let out = exec::run(repo_root, ["fetch", "--all", "--prune"]).await?;
    Ok(summary_or(out, "fetched all remotes"))
}

pub async fn checkout(repo_root: &Path, branch: &str) -> Result<String> {
    exec::run(repo_root, ["checkout", branch]).await?;
    Ok(format!("checked out {branch}"))
}

pub async fn create_and_checkout(repo_root: &Path, branch: &str) -> Result<String> {
    exec::run(repo_root, ["checkout", "-b", branch]).await?;
    Ok(format!("created and checked out {branch}"))
}

pub async fn create_branch_at(repo_root: &Path, branch: &str, sha: &str) -> Result<String> {
    exec::run(repo_root, ["branch", branch, sha]).await?;
    Ok(format!("created {branch} at {sha}"))
}

/// Safe delete: refuses if branch is unmerged.
pub async fn delete_branch(repo_root: &Path, branch: &str) -> Result<String> {
    exec::run(repo_root, ["branch", "-d", branch]).await?;
    Ok(format!("deleted branch {branch}"))
}

/// Force delete. Use with confirmation.
pub async fn force_delete_branch(repo_root: &Path, branch: &str) -> Result<String> {
    exec::run(repo_root, ["branch", "-D", branch]).await?;
    Ok(format!("force-deleted branch {branch}"))
}

/// Discard unstaged changes to a file (and remove if untracked).
/// Move HEAD back one commit, keeping the change staged in the working
/// tree (`git reset --soft HEAD^`). The undone changes reappear in the
/// Changes pane as staged, so nothing is lost — the operation is reversible
/// via the reflog. Caller is responsible for checking that HEAD is on a
/// branch, has a parent, and isn't already pushed.
pub async fn uncommit_soft(repo_root: &Path) -> Result<String> {
    exec::run(repo_root, ["reset", "--soft", "HEAD^"]).await?;
    Ok("uncommitted — changes are back in the staging area".to_string())
}

/// Summary of the current HEAD commit. Returns `None` when the repo has no
/// commits yet (unborn HEAD). `has_parent` is false on the root commit, so
/// callers can refuse to uncommit it.
pub struct HeadSummary {
    pub short: String,
    pub subject: String,
    pub has_parent: bool,
}

pub async fn head_summary(repo_root: &Path) -> Result<Option<HeadSummary>> {
    // %h short hash, %s subject. NUL-separated so subject lines with `:` etc.
    // don't confuse parsing.
    let out = match exec::run_optional(
        repo_root,
        ["log", "-1", "--pretty=format:%h%x00%s", "HEAD"],
    )
    .await?
    {
        Some(s) => s,
        None => return Ok(None),
    };
    let line = out.lines().next().unwrap_or("");
    let mut parts = line.splitn(2, '\x00');
    let short = parts.next().unwrap_or("").to_string();
    let subject = parts.next().unwrap_or("").to_string();
    if short.is_empty() {
        return Ok(None);
    }
    // Parent check is independent — a successful HEAD log doesn't tell us
    // whether HEAD has a parent. `rev-parse --verify HEAD^` exits non-zero
    // on the root commit.
    let has_parent = exec::run_optional(repo_root, ["rev-parse", "--verify", "HEAD^"])
        .await
        .ok()
        .flatten()
        .is_some();
    Ok(Some(HeadSummary {
        short,
        subject,
        has_parent,
    }))
}

pub async fn discard_file(repo_root: &Path, path: &str) -> Result<String> {
    // restore tracked changes; if untracked, remove the file from disk.
    if exec::run(repo_root, ["restore", "--", path]).await.is_err() {
        // Untracked file: rm it.
        let full = repo_root.join(path);
        tokio::fs::remove_file(&full).await.ok();
    }
    Ok(format!("discarded {path}"))
}

/// Last non-empty line from git's output, or a fallback summary.
fn summary_or(out: String, fallback: &str) -> String {
    out.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}
