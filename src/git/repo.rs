use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::exec;

/// A handle to a git repository on disk.
#[derive(Debug, Clone)]
pub struct Repo {
    /// Absolute path to the working tree root (the directory containing `.git`).
    pub root: PathBuf,
}

/// What HEAD currently points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadRef {
    /// HEAD points at a branch.
    Branch(String),
    /// Detached HEAD at a commit.
    Detached(String),
    /// No commits yet (unborn HEAD).
    Unborn,
}

impl Repo {
    /// Walk up from `start` to find a git repo root.
    pub fn discover(start: &Path) -> Result<Self> {
        let start = start
            .canonicalize()
            .with_context(|| format!("cannot resolve path {}", start.display()))?;

        // Use `git rev-parse --show-toplevel` to find the root — this respects
        // `GIT_DIR`, worktrees, and symlinks the same way git itself does.
        let output = std::process::Command::new("git")
            .current_dir(&start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("failed to spawn `git`")?;

        if !output.status.success() {
            anyhow::bail!("not a git repository: {}", start.display());
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if root.is_empty() {
            anyhow::bail!("git did not return a repository root");
        }
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    /// Resolve current HEAD: branch name, detached commit, or unborn.
    pub async fn head(&self) -> Result<HeadRef> {
        // Try the symbolic ref first (the common case).
        if let Some(out) =
            exec::run_optional(&self.root, ["symbolic-ref", "--quiet", "--short", "HEAD"]).await?
        {
            return Ok(HeadRef::Branch(out.trim().to_string()));
        }
        // Fall back: detached HEAD or unborn.
        if let Some(out) = exec::run_optional(&self.root, ["rev-parse", "--short", "HEAD"]).await? {
            return Ok(HeadRef::Detached(out.trim().to_string()));
        }
        Ok(HeadRef::Unborn)
    }
}
