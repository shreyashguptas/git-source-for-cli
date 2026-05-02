use std::path::Path;

use anyhow::{Context, Result};

use super::exec;

/// Diff for a working-tree file. If `staged` is true, shows the index diff
/// (vs. HEAD); otherwise shows the unstaged diff (working tree vs. index).
pub async fn file(repo_root: &Path, path: &str, staged: bool) -> Result<String> {
    let mut args: Vec<&str> = vec!["diff", "--no-color"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    args.push(path);
    exec::run(repo_root, args).await.context("git diff failed")
}

/// Combined diff for an untracked file: shows the whole file as additions.
pub async fn untracked(repo_root: &Path, path: &str) -> Result<String> {
    // `git diff --no-index /dev/null <path>` returns non-zero when there's a diff,
    // so we use run_optional and treat None as "no diff" (empty file).
    let none = "/dev/null";
    Ok(exec::run_optional(
        repo_root,
        ["diff", "--no-color", "--no-index", "--", none, path],
    )
    .await?
    .unwrap_or_default())
}

/// Show a commit: full message + diff vs first parent (or empty parent for root).
pub async fn show(repo_root: &Path, sha: &str) -> Result<String> {
    exec::run(
        repo_root,
        ["show", "--no-color", "--stat", "-p", "--format=fuller", sha],
    )
    .await
    .context("git show failed")
}

/// Classify one diff line for coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `diff --git a/x b/x`
    Header,
    /// `index abc..def 100644`, `--- a/x`, `+++ b/x`, `new file mode ...`
    Meta,
    /// `@@ -1,3 +1,4 @@ context`
    Hunk,
    /// `+added`
    Add,
    /// `-removed`
    Del,
    /// ` context`
    Context,
    /// stat lines like ` src/main.rs | 12 +++---`
    Stat,
    /// commit message + author headers (in `git show` output)
    CommitMeta,
    /// blank lines, or anything we couldn't classify
    Other,
}

/// Classify one line of git diff/show output.
pub fn classify(line: &str) -> DiffLineKind {
    if let Some(first) = line.chars().next() {
        match first {
            '+' if line.starts_with("+++") => return DiffLineKind::Meta,
            '+' => return DiffLineKind::Add,
            '-' if line.starts_with("---") => return DiffLineKind::Meta,
            '-' => return DiffLineKind::Del,
            '@' if line.starts_with("@@") => return DiffLineKind::Hunk,
            ' ' => {
                // Could be context OR a stat line ("  src/main.rs | 12 +++---")
                if line.contains(" | ") && (line.contains('+') || line.contains('-')) {
                    return DiffLineKind::Stat;
                }
                return DiffLineKind::Context;
            }
            _ => {}
        }
    }
    if line.starts_with("diff --git") {
        return DiffLineKind::Header;
    }
    if line.starts_with("commit ")
        || line.starts_with("Author")
        || line.starts_with("AuthorDate")
        || line.starts_with("Commit")
        || line.starts_with("CommitDate")
        || line.starts_with("Merge:")
    {
        return DiffLineKind::CommitMeta;
    }
    if line.starts_with("index ")
        || line.starts_with("new file")
        || line.starts_with("deleted file")
        || line.starts_with("similarity")
        || line.starts_with("rename ")
        || line.starts_with("Binary files")
    {
        return DiffLineKind::Meta;
    }
    DiffLineKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_lines() {
        assert_eq!(classify("+added"), DiffLineKind::Add);
        assert_eq!(classify("-removed"), DiffLineKind::Del);
        assert_eq!(classify("+++ b/x"), DiffLineKind::Meta);
        assert_eq!(classify("--- a/x"), DiffLineKind::Meta);
        assert_eq!(classify("@@ -1,3 +1,4 @@"), DiffLineKind::Hunk);
        assert_eq!(classify(" context"), DiffLineKind::Context);
        assert_eq!(classify("diff --git a/x b/x"), DiffLineKind::Header);
        assert_eq!(classify("index abc..def 100644"), DiffLineKind::Meta);
        assert_eq!(classify("commit abc123"), DiffLineKind::CommitMeta);
        assert_eq!(classify(" src/main.rs | 4 ++--"), DiffLineKind::Stat);
    }
}
