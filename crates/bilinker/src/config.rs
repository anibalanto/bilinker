use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub workspaces: HashMap<String, Workspace>,
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub path: PathBuf,
    pub language: String,
}

impl Config {
    /// Search for `.bilinker.toml` starting from `dir` upward.
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

    pub fn workspace_root(&self, root: &Path, name: &str) -> Result<PathBuf> {
        let ws = self.workspaces.get(name)
            .with_context(|| format!("workspace '{name}' not found in .bilinker.toml"))?;
        Ok(root.join(&ws.path))
    }
}
