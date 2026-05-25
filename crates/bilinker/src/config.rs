use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {}

impl Config {
    /// Search for `.bilinker.toml` starting from `dir` upward and return
    /// the directory that contains it (the project root).
    pub fn load_from(dir: &Path) -> Result<(PathBuf, Config)> {
        let mut current = dir.to_path_buf();
        loop {
            let candidate = current.join(".bilinker.toml");
            if candidate.exists() {
                let text = std::fs::read_to_string(&candidate)
                    .with_context(|| format!("reading {}", candidate.display()))?;
                let config: Config = toml::from_str(&text)
                    .with_context(|| format!("parsing {}", candidate.display()))?;
                return Ok((current, config));
            }
            if !current.pop() {
                anyhow::bail!(".bilinker.toml not found");
            }
        }
    }
}
