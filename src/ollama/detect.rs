//! Probe a local Ollama instance for availability + installed models.
//!
//! Mirrors the shape of `gh::detect` so the UI can show a unified status badge
//! and so the rest of the app can do `if app.ollama is Ready { ... }` without
//! caring about the underlying transport.

use super::client;

/// Result of probing the local Ollama instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Availability {
    /// We haven't probed yet (used at startup before the first detect lands).
    #[default]
    Unknown,
    /// Couldn't reach Ollama at the configured base URL.
    NotRunning,
    /// Ollama is running but no models are installed (`ollama pull <model>` first).
    NoModels,
    /// Ollama is running and has at least one model installed.
    Ready { models: Vec<String> },
}

/// Probe `<base_url>/api/tags`. Cheap localhost call, ~few ms when running.
pub async fn detect(base_url: &str) -> Availability {
    match client::list_models(base_url).await {
        Ok(models) if !models.is_empty() => Availability::Ready { models },
        Ok(_) => Availability::NoModels,
        Err(_) => Availability::NotRunning,
    }
}
