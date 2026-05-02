use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream, KeyEvent, MouseEvent};
use futures_util::{FutureExt, StreamExt};
use tokio::{sync::mpsc, time::interval};

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::{
    gh::{Availability, Pr},
    git::{Branch, Commit, HeadRef, Status},
    ollama::Availability as OllamaAvailability,
    ui::panes::details::DetailsContent,
};

/// All the things `App` reacts to. Single point of change for new event sources.
#[derive(Debug)]
pub enum AppEvent {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Quit,

    HeadLoaded(HeadRef),
    BranchesLoaded(Vec<Branch>),
    CommitsLoaded(Vec<Commit>),
    StatusLoaded(Status),
    /// SHAs of commits that are local-only vs HEAD's upstream (`@{upstream}..HEAD`)
    /// and the inverse (`HEAD..@{upstream}`). Empty when there's no upstream.
    DivergenceLoaded { ahead: HashSet<String>, behind: HashSet<String> },
    /// Map of branch name → worktree path (for branches checked out elsewhere).
    WorktreesLoaded(HashMap<String, PathBuf>),
    GhAvailability(Availability),
    PrsLoaded(HashMap<String, Pr>),
    /// Async-loaded body for the details overlay.
    OverlayLoaded(DetailsContent),
    /// Async-loaded body for the inline preview pane. The first field is the
    /// target identity (sha or file:path) so stale fetches can be discarded.
    PreviewLoaded(String, DetailsContent),
    /// A background load failed — message is the user-facing one-liner.
    LoadFailed(String),

    /// Probe of the local Ollama instance — null when unreachable, otherwise
    /// includes the list of installed models.
    OllamaAvailability(OllamaAvailability),
    /// One streamed token from a commit-message generation.
    OllamaToken(String),
    /// Streaming finished cleanly. The full assembled message is included so
    /// the input handler can do final cleanup (trim, take first line, etc.).
    OllamaDone(String),
    /// Streaming failed — message is the user-facing one-liner.
    OllamaError(String),
}

/// Spawn the crossterm input + tick + signal tasks.
/// Returns the sender (so `App` can post its own messages from background tasks)
/// and the receiver (for the main event loop).
pub fn spawn_event_loop(buf: usize, tick_rate: Duration) -> (mpsc::Sender<AppEvent>, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel(buf);

    // crossterm input task
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut stream = EventStream::new();
            while let Some(maybe_event) = stream.next().fuse().await {
                let Ok(event) = maybe_event else { continue };
                let app_event = match event {
                    CtEvent::Key(k) => AppEvent::Key(k),
                    CtEvent::Mouse(m) => AppEvent::Mouse(m),
                    CtEvent::Resize(w, h) => AppEvent::Resize(w, h),
                    _ => continue,
                };
                if tx.send(app_event).await.is_err() {
                    break;
                }
            }
        });
    }

    // tick task
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut ticker = interval(tick_rate);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // discard immediate first tick
            loop {
                ticker.tick().await;
                if tx.send(AppEvent::Tick).await.is_err() {
                    break;
                }
            }
        });
    }

    // Ctrl-C handler — defensive; key event usually arrives first.
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = tx.send(AppEvent::Quit).await;
            }
        });
    }

    (tx, rx)
}
