//! Watch the repo's `.git/` directory for external mutations and post a `Tick`
//! whenever something interesting changes (HEAD, index, refs).
//!
//! Uses [`notify-debouncer-mini`] to coalesce rapid bursts (a `git commit`
//! touches several files in quick succession).

use std::{path::Path, time::Duration};

use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use tokio::sync::mpsc;

use crate::event::AppEvent;

/// Spawn a blocking-thread watcher. Posts an `AppEvent::Tick` on any
/// `.git/HEAD`, `.git/index`, `.git/refs/**`, or `.git/packed-refs` change.
pub fn spawn(repo_root: &Path, tx: mpsc::Sender<AppEvent>) -> Result<()> {
    let git_dir = repo_root.join(".git");
    if !git_dir.exists() {
        // Worktree or bare repo — skip silently rather than failing startup.
        return Ok(());
    }

    let (notify_tx, notify_rx) = std::sync::mpsc::channel();

    // Debouncer must outlive the thread; we leak it intentionally because the
    // process lifetime IS the watcher lifetime.
    let mut debouncer = new_debouncer(Duration::from_millis(120), move |res| {
        let _ = notify_tx.send(res);
    })?;

    // Watch HEAD + refs + index. Recursive on .git/refs to catch new branches.
    debouncer
        .watcher()
        .watch(&git_dir.join("HEAD"), RecursiveMode::NonRecursive)
        .ok();
    debouncer
        .watcher()
        .watch(&git_dir.join("index"), RecursiveMode::NonRecursive)
        .ok();
    debouncer
        .watcher()
        .watch(&git_dir.join("refs"), RecursiveMode::Recursive)
        .ok();
    debouncer
        .watcher()
        .watch(&git_dir.join("packed-refs"), RecursiveMode::NonRecursive)
        .ok();
    // .keep_alive
    Box::leak(Box::new(debouncer));

    std::thread::spawn(move || {
        for res in notify_rx {
            match res {
                Ok(_events) => {
                    // Coalesce: just post a Tick — App's tick handler reloads everything.
                    let _ = tx.blocking_send(AppEvent::Tick);
                }
                Err(_) => break,
            }
        }
    });

    Ok(())
}
