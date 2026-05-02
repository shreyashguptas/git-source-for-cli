use std::{ffi::OsStr, path::Path};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

/// Run `git` with the given args inside `cwd`, returning stdout as a String.
/// On non-zero exit, returns an error including stderr.
pub async fn run<I, S>(cwd: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .await
        .context("failed to spawn `git` — is it installed and on $PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "git exited with status {}: {}",
            output.status.code().unwrap_or(-1),
            if stderr.is_empty() { "(no stderr)" } else { &stderr }
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Same as `run` but returns `Ok(None)` instead of an error when git fails —
/// useful for "is this repo X?" probes where failure is meaningful.
pub async fn run_optional<I, S>(cwd: &Path, args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .await
        .context("failed to spawn `git`")?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else {
        Ok(None)
    }
}
