use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream, KeyEvent};
use futures_util::{FutureExt, StreamExt};
use tokio::{sync::mpsc, time::interval};

use std::collections::HashMap;

use crate::{
    gh::{Availability, Pr},
    git::{Branch, Commit, HeadRef, Status},
    ui::panes::details::DetailsContent,
};

/// All the things `App` reacts to. Single point of change for new event sources.
#[derive(Debug)]
pub enum AppEvent {
    Tick,
    Key(KeyEvent),
    Resize(u16, u16),
    Quit,

    HeadLoaded(HeadRef),
    BranchesLoaded(Vec<Branch>),
    CommitsLoaded(Vec<Commit>),
    StatusLoaded(Status),
    GhAvailability(Availability),
    PrsLoaded(HashMap<String, Pr>),
    /// Async-loaded body for the details overlay.
    OverlayLoaded(DetailsContent),
    /// A background load failed — message is the user-facing one-liner.
    LoadFailed(String),
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
