use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "gsc",
    version,
    about = "Git source control TUI — VS Code's source control panel in your terminal",
    long_about = None,
)]
pub struct Cli {
    /// Path to the git repository (defaults to current directory).
    #[arg(short, long, value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Verbose logging (writes to stderr; redirect to a file to inspect).
    #[arg(short, long)]
    pub verbose: bool,
}

impl Cli {
    pub fn repo_path(&self) -> Result<PathBuf> {
        match &self.path {
            Some(p) => Ok(p.clone()),
            None => Ok(std::env::current_dir()?),
        }
    }
}
