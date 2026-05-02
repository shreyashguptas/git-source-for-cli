use std::path::Path;

use anyhow::{Context, Result};

use super::exec;

/// Working-tree + index status, parsed from `git status --porcelain=v2 --branch`.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    /// For renames/copies, the original path.
    pub from: Option<String>,
    pub kind: ChangeKind,
    /// Index status (staged change).
    pub staged: Option<XY>,
    /// Working-tree status (unstaged change).
    pub unstaged: Option<XY>,
    pub conflicted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Ignored,
    Conflicted,
    TypeChanged,
    Unknown,
}

/// One char from porcelain v2: M/A/D/R/C/T/. (unmodified) — preserved verbatim
/// so the UI can show subtle differences (e.g. "added" vs "modified-staged").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XY(pub char);

pub async fn fetch(repo_root: &Path) -> Result<Status> {
    let out = exec::run(
        repo_root,
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ],
    )
    .await
    .context("git status failed")?;
    Ok(parse(&out))
}

/// Parse porcelain v2, NUL-terminated.
///
/// Format reference: https://git-scm.com/docs/git-status#_porcelain_format_version_2
fn parse(input: &str) -> Status {
    let mut status = Status::default();
    // Split on NUL — but rename/copy entries embed two paths separated by NUL,
    // so we need stateful parsing rather than a naive split.
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // find next NUL
        let end = bytes[i..]
            .iter()
            .position(|b| *b == 0)
            .map(|p| i + p)
            .unwrap_or(bytes.len());
        let line = std::str::from_utf8(&bytes[i..end]).unwrap_or("");
        i = end + 1;

        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("# ") {
            parse_header(rest, &mut status);
            continue;
        }

        match line.chars().next() {
            Some('1') => {
                if let Some(fc) = parse_ordinary(line) {
                    status.files.push(fc);
                }
            }
            Some('2') => {
                // Renames/copies have an extra "<sep><orig_path>" NUL-terminated piece.
                let orig_end = bytes[i..]
                    .iter()
                    .position(|b| *b == 0)
                    .map(|p| i + p)
                    .unwrap_or(bytes.len());
                let orig = std::str::from_utf8(&bytes[i..orig_end]).unwrap_or("");
                i = orig_end + 1;
                if let Some(fc) = parse_renamed(line, orig) {
                    status.files.push(fc);
                }
            }
            Some('u') => {
                if let Some(fc) = parse_unmerged(line) {
                    status.files.push(fc);
                }
            }
            Some('?') => {
                if let Some(path) = line.strip_prefix("? ") {
                    status.files.push(FileChange {
                        path: path.to_string(),
                        from: None,
                        kind: ChangeKind::Untracked,
                        staged: None,
                        unstaged: Some(XY('?')),
                        conflicted: false,
                    });
                }
            }
            Some('!') => {
                if let Some(path) = line.strip_prefix("! ") {
                    status.files.push(FileChange {
                        path: path.to_string(),
                        from: None,
                        kind: ChangeKind::Ignored,
                        staged: None,
                        unstaged: Some(XY('!')),
                        conflicted: false,
                    });
                }
            }
            _ => {}
        }
    }
    status
}

fn parse_header(line: &str, status: &mut Status) {
    // Headers we care about:
    //   branch.head <name>      (or "(detached)")
    //   branch.upstream <name>
    //   branch.ab +N -M
    let mut parts = line.splitn(2, ' ');
    let key = parts.next().unwrap_or("");
    let val = parts.next().unwrap_or("").trim();
    match key {
        "branch.head" => {
            if val != "(detached)" && !val.is_empty() {
                status.branch = Some(val.to_string());
            }
        }
        "branch.upstream" => {
            if !val.is_empty() {
                status.upstream = Some(val.to_string());
            }
        }
        "branch.ab" => {
            // "+N -M"
            for piece in val.split_whitespace() {
                if let Some(n) = piece.strip_prefix('+') {
                    status.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = piece.strip_prefix('-') {
                    status.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

/// Ordinary: "1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>"
fn parse_ordinary(line: &str) -> Option<FileChange> {
    let mut parts = line.splitn(9, ' ');
    parts.next()?; // "1"
    let xy = parts.next()?;
    // skip sub, mH, mI, mW, hH, hI
    for _ in 0..6 {
        parts.next()?;
    }
    let path = parts.next()?.to_string();

    let (x, y) = xy_chars(xy)?;
    Some(FileChange {
        path,
        from: None,
        kind: kind_from_xy(x, y),
        staged: if x != '.' { Some(XY(x)) } else { None },
        unstaged: if y != '.' { Some(XY(y)) } else { None },
        conflicted: false,
    })
}

/// Renamed/copied: "2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>" + orig path on next NUL field
fn parse_renamed(line: &str, orig: &str) -> Option<FileChange> {
    let mut parts = line.splitn(10, ' ');
    parts.next()?; // "2"
    let xy = parts.next()?;
    for _ in 0..6 {
        parts.next()?;
    }
    parts.next()?; // X<score>, e.g. R100
    let path = parts.next()?.to_string();

    let (x, y) = xy_chars(xy)?;
    let kind = if xy.contains('R') {
        ChangeKind::Renamed
    } else if xy.contains('C') {
        ChangeKind::Copied
    } else {
        ChangeKind::Modified
    };

    Some(FileChange {
        path,
        from: Some(orig.to_string()),
        kind,
        staged: if x != '.' { Some(XY(x)) } else { None },
        unstaged: if y != '.' { Some(XY(y)) } else { None },
        conflicted: false,
    })
}

/// Unmerged: "u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>"
fn parse_unmerged(line: &str) -> Option<FileChange> {
    let mut parts = line.splitn(11, ' ');
    parts.next()?; // "u"
    let _xy = parts.next()?;
    for _ in 0..8 {
        parts.next()?;
    }
    let path = parts.next()?.to_string();
    Some(FileChange {
        path,
        from: None,
        kind: ChangeKind::Conflicted,
        staged: Some(XY('U')),
        unstaged: Some(XY('U')),
        conflicted: true,
    })
}

fn xy_chars(xy: &str) -> Option<(char, char)> {
    let mut chars = xy.chars();
    Some((chars.next()?, chars.next()?))
}

fn kind_from_xy(x: char, y: char) -> ChangeKind {
    let primary = if x != '.' { x } else { y };
    match primary {
        'M' => ChangeKind::Modified,
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        'U' => ChangeKind::Conflicted,
        _ => ChangeKind::Unknown,
    }
}

impl ChangeKind {
    /// Single-char prefix matching VS Code / git's display: M, A, D, R, U, ?, !
    pub fn glyph(self) -> char {
        match self {
            ChangeKind::Modified => 'M',
            ChangeKind::Added => 'A',
            ChangeKind::Deleted => 'D',
            ChangeKind::Renamed => 'R',
            ChangeKind::Copied => 'C',
            ChangeKind::TypeChanged => 'T',
            ChangeKind::Untracked => '?',
            ChangeKind::Ignored => '!',
            ChangeKind::Conflicted => 'U',
            ChangeKind::Unknown => ' ',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_headers_and_files() {
        let input = "# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
            1 .M N... 100644 100644 100644 abc abc src/main.rs\0\
            ? README.md\0";
        let s = parse(input);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.ahead, 2);
        assert_eq!(s.behind, 1);
        assert_eq!(s.files.len(), 2);
        assert_eq!(s.files[0].path, "src/main.rs");
        assert_eq!(s.files[0].kind, ChangeKind::Modified);
        assert!(s.files[0].staged.is_none());
        assert_eq!(s.files[1].kind, ChangeKind::Untracked);
    }

    #[test]
    fn parses_renamed() {
        let input = "2 R. N... 100644 100644 100644 abc abc R100 new_name.rs\0old_name.rs\0";
        let s = parse(input);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].path, "new_name.rs");
        assert_eq!(s.files[0].from.as_deref(), Some("old_name.rs"));
        assert_eq!(s.files[0].kind, ChangeKind::Renamed);
    }

    #[test]
    fn parses_detached_head() {
        let input = "# branch.head (detached)\0";
        let s = parse(input);
        assert!(s.branch.is_none());
    }
}
