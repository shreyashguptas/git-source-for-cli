use std::{collections::HashSet, path::Path};

use anyhow::{Context, Result};

use super::exec;

/// One commit from `git log --all`. Refs are the names that point exactly at
/// this commit (local branches, remote branches, tags, HEAD).
#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub time: i64,
    pub refs: Vec<RefName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefName {
    Head,                       // bare "HEAD" — only on detached
    LocalBranch(String),        // "main"
    RemoteBranch(String),       // "origin/main"
    Tag(String),                // "v1.0"
    Other(String),              // anything we couldn't classify
    HeadAt(String),             // "HEAD -> main" — name is "main"
}

/// Fetch the commit DAG from `git log --all` in date order.
/// Caps at `limit` commits to keep memory and rendering bounded.
pub async fn fetch(repo_root: &Path, limit: usize) -> Result<Vec<Commit>> {
    fetch_with_revs(repo_root, ["--all".to_string()], limit).await
}

/// Fetch history for a specific local branch without checking it out.
/// Include its upstream when present so ahead/behind commits can both appear.
pub async fn fetch_branch(
    repo_root: &Path,
    branch: &str,
    upstream: Option<&str>,
    limit: usize,
) -> Result<Vec<Commit>> {
    let mut revs = vec![branch.to_string()];
    let has_upstream = upstream.filter(|s| !s.is_empty()).is_some();
    if let Some(upstream) = upstream.filter(|s| !s.is_empty()) {
        revs.push(upstream.to_string());
    }
    match fetch_with_revs(repo_root, revs, limit).await {
        Ok(commits) => Ok(commits),
        Err(e) if has_upstream => fetch_with_revs(repo_root, [branch.to_string()], limit)
            .await
            .map_err(|_| e),
        Err(e) => Err(e),
    }
}

async fn fetch_with_revs<I>(repo_root: &Path, revs: I, limit: usize) -> Result<Vec<Commit>>
where
    I: IntoIterator<Item = String>,
{
    // %H hash, %h short, %P parents (space-separated), %s subject, %an author,
    // %at author-time-unix, %D refs decoration (no parens, comma-separated).
    let format = "%H%x00%h%x00%P%x00%s%x00%an%x00%at%x00%D";
    let pretty = format!("--pretty=format:{format}");
    let limit_arg = format!("-{limit}");
    let mut args = vec![
        "log".to_string(),
        "--date-order".to_string(),
        pretty,
        limit_arg,
    ];
    args.extend(revs);
    let out = exec::run(repo_root, args)
        .await
        .context("git log failed")?;

    Ok(out
        .split('\n')
        .filter(|l| !l.is_empty())
        .filter_map(parse_line)
        .collect())
}

/// Recent commit subjects from HEAD, newest first. Used as a style anchor
/// for AI-generated commit messages so the model matches the repo's voice
/// (conventional commits, prefix style, tense, etc.) without per-repo config.
/// Empty on any error or in an unborn repo — callers treat this as best-effort.
pub async fn recent_subjects(repo_root: &Path, limit: usize) -> Vec<String> {
    let limit_arg = format!("-{limit}");
    match exec::run_optional(
        repo_root,
        ["log", &limit_arg, "--pretty=format:%s", "HEAD"],
    )
    .await
    {
        Ok(Some(out)) => out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// SHAs that are reachable from HEAD but NOT from HEAD's upstream — i.e.
/// commits the current branch has that origin doesn't.
///
/// Returns an empty set if there's no upstream, no HEAD, or any other error
/// (we don't want to spam the user with toast warnings on every refresh).
pub async fn ahead_of_upstream(repo_root: &Path) -> HashSet<String> {
    rev_list_set(repo_root, "@{upstream}..HEAD").await
}

pub async fn ahead_of_branch(repo_root: &Path, branch: &str) -> HashSet<String> {
    rev_list_set(repo_root, &format!("{branch}@{{upstream}}..{branch}")).await
}

async fn rev_list_set(repo_root: &Path, range: &str) -> HashSet<String> {
    match exec::run_optional(repo_root, ["rev-list", range, "--pretty=format:%H"]).await {
        Ok(Some(out)) => out
            .lines()
            // `--pretty=format:%H` emits both the format line AND a "commit X" line;
            // we only want the bare SHAs.
            .filter(|l| !l.starts_with("commit "))
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        _ => HashSet::new(),
    }
}

/// SHAs reachable from HEAD's upstream but NOT from HEAD — commits origin
/// has that the local branch doesn't (i.e. behind).
pub async fn behind_upstream(repo_root: &Path) -> HashSet<String> {
    rev_list_set(repo_root, "HEAD..@{upstream}").await
}

pub async fn behind_branch(repo_root: &Path, branch: &str) -> HashSet<String> {
    rev_list_set(repo_root, &format!("{branch}..{branch}@{{upstream}}")).await
}

fn parse_line(line: &str) -> Option<Commit> {
    let mut parts = line.split('\x00');
    let hash = parts.next()?.to_string();
    let short_hash = parts.next()?.to_string();
    let parents_raw = parts.next()?.to_string();
    let subject = parts.next()?.to_string();
    let author = parts.next()?.to_string();
    let time = parts.next()?.parse::<i64>().unwrap_or(0);
    let refs_raw = parts.next().unwrap_or("").to_string();

    if hash.is_empty() {
        return None;
    }

    let parents = parents_raw
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let refs = parse_refs(&refs_raw);

    Some(Commit {
        hash,
        short_hash,
        parents,
        subject,
        author,
        time,
        refs,
    })
}

fn parse_refs(s: &str) -> Vec<RefName> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(classify_ref)
        .collect()
}

fn classify_ref(s: &str) -> RefName {
    if let Some(rest) = s.strip_prefix("HEAD -> ") {
        return RefName::HeadAt(rest.to_string());
    }
    if s == "HEAD" {
        return RefName::Head;
    }
    if let Some(rest) = s.strip_prefix("tag: ") {
        return RefName::Tag(rest.to_string());
    }
    if s.contains('/') {
        return RefName::RemoteBranch(s.to_string());
    }
    if !s.contains(' ') {
        return RefName::LocalBranch(s.to_string());
    }
    RefName::Other(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_commit() {
        let l = "abc123def\x00abc123d\x00parent1 parent2\x00fix: thing\x00Alice\x001700000000\x00HEAD -> main, origin/main, tag: v1.0";
        let c = parse_line(l).unwrap();
        assert_eq!(c.hash, "abc123def");
        assert_eq!(c.short_hash, "abc123d");
        assert_eq!(c.parents, vec!["parent1", "parent2"]);
        assert_eq!(c.subject, "fix: thing");
        assert_eq!(c.author, "Alice");
        assert_eq!(c.time, 1_700_000_000);
        assert_eq!(
            c.refs,
            vec![
                RefName::HeadAt("main".into()),
                RefName::RemoteBranch("origin/main".into()),
                RefName::Tag("v1.0".into()),
            ]
        );
    }

    #[test]
    fn parses_root_commit_no_refs() {
        let l = "abc\x00abc\x00\x00init\x00Bob\x001700000000\x00";
        let c = parse_line(l).unwrap();
        assert!(c.parents.is_empty());
        assert!(c.refs.is_empty());
    }
}
