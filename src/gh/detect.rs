use tokio::process::Command;

/// Result of probing the local environment for `gh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// `gh` is installed and authenticated for at least one host.
    Ready,
    /// `gh` is installed but not authenticated.
    NotAuthed,
    /// `gh` is not installed (or not on `$PATH`).
    NotInstalled,
}

/// Probe `gh auth status`. Cheap (~50ms) — call once on startup.
pub async fn detect() -> Availability {
    let out = match Command::new("gh").arg("--version").output().await {
        Ok(o) => o,
        Err(_) => return Availability::NotInstalled,
    };
    if !out.status.success() {
        return Availability::NotInstalled;
    }
    // `gh auth status` exits 0 if authed, non-zero otherwise.
    match Command::new("gh").args(["auth", "status"]).output().await {
        Ok(o) if o.status.success() => Availability::Ready,
        _ => Availability::NotAuthed,
    }
}
