use std::path::Path;

use anyhow::{Context, Result};

use super::exec;

/// A local branch with its tracking info.
#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub head: String, // full SHA
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit_at: i64, // unix seconds; 0 if unknown
    pub is_current: bool,
}

/// List local branches sorted by last-commit time descending. Current branch first.
pub async fn list(repo_root: &Path) -> Result<Vec<Branch>> {
    // Use NUL separators between fields and newline between records so subjects
    // with weird chars don't break parsing.
    let format = "%(refname:short)%00%(objectname)%00%(upstream:short)%00%(upstream:track)%00%(committerdate:unix)%00%(HEAD)";
    let out = exec::run(
        repo_root,
        [
            "for-each-ref",
            "--format",
            format,
            "refs/heads/",
        ],
    )
    .await
    .context("git for-each-ref failed")?;

    let mut branches: Vec<Branch> = out
        .split('\n')
        .filter(|l| !l.is_empty())
        .filter_map(parse_line)
        .collect();

    branches.sort_by(|a, b| {
        // current branch first, then by last commit time desc
        match (a.is_current, b.is_current) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.last_commit_at.cmp(&a.last_commit_at),
        }
    });

    Ok(branches)
}

fn parse_line(line: &str) -> Option<Branch> {
    let mut parts = line.split('\x00');
    let name = parts.next()?.to_string();
    let head = parts.next()?.to_string();
    let upstream_raw = parts.next()?.to_string();
    let track = parts.next()?.to_string();
    let date = parts.next()?;
    let head_marker = parts.next().unwrap_or("");

    if name.is_empty() {
        return None;
    }

    let (ahead, behind) = parse_track(&track);
    let upstream = if upstream_raw.is_empty() {
        None
    } else {
        Some(upstream_raw)
    };
    let last_commit_at = date.parse::<i64>().unwrap_or(0);
    let is_current = head_marker.trim() == "*";

    Some(Branch {
        name,
        head,
        upstream,
        ahead,
        behind,
        last_commit_at,
        is_current,
    })
}

/// Parse `[ahead N, behind M]` / `[ahead N]` / `[behind M]` / `[gone]` / "".
fn parse_track(s: &str) -> (u32, u32) {
    if s.is_empty() {
        return (0, 0);
    }
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    let mut ahead = 0;
    let mut behind = 0;
    for piece in inner.split(',') {
        let piece = piece.trim();
        if let Some(rest) = piece.strip_prefix("ahead ") {
            ahead = rest.parse().unwrap_or(0);
        } else if let Some(rest) = piece.strip_prefix("behind ") {
            behind = rest.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track() {
        assert_eq!(parse_track(""), (0, 0));
        assert_eq!(parse_track("[ahead 3]"), (3, 0));
        assert_eq!(parse_track("[behind 2]"), (0, 2));
        assert_eq!(parse_track("[ahead 1, behind 4]"), (1, 4));
        assert_eq!(parse_track("[gone]"), (0, 0));
    }

    #[test]
    fn parses_line() {
        let l = "main\x00abc123\x00origin/main\x00[ahead 2, behind 1]\x001700000000\x00*";
        let b = parse_line(l).unwrap();
        assert_eq!(b.name, "main");
        assert_eq!(b.head, "abc123");
        assert_eq!(b.upstream.as_deref(), Some("origin/main"));
        assert_eq!(b.ahead, 2);
        assert_eq!(b.behind, 1);
        assert_eq!(b.last_commit_at, 1_700_000_000);
        assert!(b.is_current);
    }

    #[test]
    fn parses_no_upstream() {
        let l = "feature\x00deadbeef\x00\x00\x001700000000\x00";
        let b = parse_line(l).unwrap();
        assert_eq!(b.upstream, None);
        assert_eq!(b.ahead, 0);
        assert!(!b.is_current);
    }
}
