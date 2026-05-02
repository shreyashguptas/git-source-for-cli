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
    "You are a senior software engineer writing a git commit message for a \
staged diff. Output ONLY the commit message — no preamble, no explanation, no \
code fences, no surrounding quotes.\n\
\n\
SUBJECT (line 1): `<type>(<scope>): <description>`. Pick the type from: feat, \
fix, refactor, perf, docs, test, chore, build, ci, style. Include a scope \
when one or two words naturally identify the touched area (component, module, \
feature); omit it otherwise — never invent one. Write in imperative voice \
(\"add\", \"fix\", \"rename\", \"extract\") and be specific: name the concrete \
thing that changed AND the user-visible effect or motivation when it fits. \
Avoid filler verbs like \"update\", \"change\", \"improve\", \"tweak\" unless \
nothing more specific applies. Aim for 60–72 characters; do not pad. Match \
the tense, casing, and prefix style of the recent commit subjects you are \
shown — they are the source of truth for this repo's voice. No trailing \
period.\n\
\n\
BLANK LINE (line 2): exactly one empty line separating subject from body.\n\
\n\
BODY (lines 3+): 2–5 short sentences or hyphen bullets that a future reader \
scanning git log six months from now would actually want. Focus on \
motivation, behavior change, and trade-offs — not a file-by-file walkthrough \
of the diff and not a restatement of the subject. Mention any non-obvious \
caveats, follow-ups, or context that wouldn't be visible from the code alone. \
Wrap body lines at roughly 72 characters. Skip the body entirely only when \
the subject genuinely says everything (e.g. `docs: fix typo in README`)."
        .to_string()
}

/// The first-shipped default prompt. Kept verbatim so we can detect users who
/// are still on it and migrate them forward when we improve the default.
/// Append (never edit) entries here as the default evolves.
const LEGACY_DEFAULT_PROMPTS: &[&str] = &[
    "You are a senior software engineer writing one git commit message subject \
line for a staged diff. Output ONLY the subject — no body, no quotes, no \
explanation. Use imperative voice (\"add\", \"fix\", \"refactor\"), keep it under \
72 characters, and be specific about what changed and why.",
    // Subject-only prompt shipped after the first one — migrated forward when
    // we started asking for a body too.
    "You are a senior software engineer writing one git commit message subject \
line for a staged diff. Output ONLY the subject — a single line, no body, no \
quotes, no explanation, no trailing period.\n\
\n\
Format: `<type>(<scope>): <description>` — e.g. `feat(ui): ...`, `fix(graph): \
...`, `refactor(git): ...`. Pick the type from: feat, fix, refactor, perf, \
docs, test, chore, build, ci, style. Include a scope when one or two words \
naturally identify the touched area (component, module, feature); omit it \
otherwise — never invent one.\n\
\n\
Write the description in imperative voice (\"add\", \"fix\", \"rename\", \
\"extract\") and make it genuinely descriptive: name the concrete thing that \
changed AND the user-visible effect or motivation when it fits. Prefer \
\"add settings modal with model picker and base URL field\" over \"add \
settings\"; prefer \"fix crash when staging a deleted submodule\" over \
\"fix bug\". Avoid filler verbs like \"update\", \"change\", \"improve\", \
\"tweak\" unless nothing more specific applies.\n\
\n\
Aim for 60–100 characters. Going slightly over is fine when the extra words \
add real information; do not pad. Match the tense, casing, and prefix style \
of the recent commit subjects you are shown — they are the source of truth \
for this repo's voice.",
];

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
    let mut cfg: Config = serde_json::from_str(&data).unwrap_or_default();
    // Forward-migrate the system prompt: if the user is still on a legacy
    // shipped default (i.e. they never customized it), replace it with the
    // current default so they get prompt improvements automatically.
    if LEGACY_DEFAULT_PROMPTS
        .iter()
        .any(|p| cfg.ollama.system_prompt == *p)
    {
        cfg.ollama.system_prompt = default_system_prompt();
    }
    cfg
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
