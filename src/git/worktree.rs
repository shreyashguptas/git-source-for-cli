use std::{collections::HashMap, path::{Path, PathBuf}};

use anyhow::Result;

use super::exec;

/// Remove the worktree at `path`. Refuses if the worktree has uncommitted
/// or unpushed changes — callers should fall back to `remove_force` when
/// the user has explicitly opted into a destructive flow (e.g. branch-delete
/// with worktree confirmation).
pub async fn remove(repo_root: &Path, path: &Path) -> Result<()> {
    exec::run(
        repo_root,
        ["worktree", "remove", &path.to_string_lossy()],
    )
    .await?;
    Ok(())
}

/// Remove the worktree at `path` even if it has uncommitted changes.
/// Use only after a user-facing confirmation that discloses this risk.
pub async fn remove_force(repo_root: &Path, path: &Path) -> Result<()> {
    exec::run(
        repo_root,
        ["worktree", "remove", "--force", &path.to_string_lossy()],
    )
    .await?;
    Ok(())
}

/// Map of branch name → worktree path. A branch appears in this map when it's
/// the HEAD of any worktree (including the current one). Use this to detect
/// "this branch is checked out somewhere else, can't checkout here" cases.
///
/// Worktrees with detached HEADs are skipped (no branch name to key on).
pub async fn list(repo_root: &std::path::Path) -> Result<HashMap<String, PathBuf>> {
    // `--porcelain` gives a stable, line-oriented format we can parse.
    let out = exec::run(repo_root, ["worktree", "list", "--porcelain"]).await?;
    Ok(parse(&out))
}

fn parse(input: &str) -> HashMap<String, PathBuf> {
    // Format (one record per worktree, separated by blank lines):
    //   worktree /path/to/wt
    //   HEAD <sha>
    //   branch refs/heads/<name>          (omitted if detached)
    //   bare                              (for bare repos)
    //   locked [reason]                   (optional)
    let mut map = HashMap::new();
    let mut current_path: Option<PathBuf> = None;

    for line in input.lines() {
        if line.is_empty() {
            current_path = None;
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            current_path = Some(PathBuf::from(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("branch refs/heads/") {
            if let Some(p) = &current_path {
                map.insert(rest.trim().to_string(), p.clone());
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worktree_list_porcelain() {
        let input = "\
worktree /Users/me/repo
HEAD abc123
branch refs/heads/main

worktree /Users/me/repo-feat
HEAD def456
branch refs/heads/feature

worktree /Users/me/repo-detached
HEAD 999fff
detached

";
        let map = parse(input);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("main").map(PathBuf::as_path), Some(std::path::Path::new("/Users/me/repo")));
        assert_eq!(map.get("feature").map(PathBuf::as_path), Some(std::path::Path::new("/Users/me/repo-feat")));
        assert!(map.get("detached").is_none());
    }

    #[test]
    fn empty_input_returns_empty_map() {
        assert!(parse("").is_empty());
    }
}
