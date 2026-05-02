//! Persistent user configuration loaded from `~/.config/gsc/config.json`.
//!
//! The file is created on first save (e.g. when the user picks a model) and is
//! optional — when absent, `Config::default()` is used. We use JSON instead of
//! TOML to avoid pulling in another parser; serde_json is already a dep.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Selected model. None = not yet picked; UI auto-picks the first available.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            model: None,
            system_prompt: default_system_prompt(),
        }
    }
}

fn default_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_system_prompt() -> String {
    "You are a senior software engineer writing one git commit message subject \
line for a staged diff. Output ONLY the subject — no body, no quotes, no \
explanation. Use imperative voice (\"add\", \"fix\", \"refactor\"), keep it under \
72 characters, and be specific about what changed and why."
        .to_string()
}

/// `~/.config/gsc/config.json`. Returns `None` if `$HOME` is unset.
pub fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("gsc")
            .join("config.json"),
    )
}

/// Read the config, returning `Config::default()` on any error (missing file,
/// corrupt JSON, etc.) so the app always boots.
pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(data) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

/// Persist the config to disk. Creates the parent directory if missing.
pub fn save(config: &Config) -> Result<()> {
    let path = config_path().context("HOME is not set — cannot locate config")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("creating config directory")?;
    }
    let json = serde_json::to_string_pretty(config).context("serializing config")?;
    std::fs::write(&path, json).context("writing config file")?;
    Ok(())
}
