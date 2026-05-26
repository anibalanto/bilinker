use std::path::{Path, PathBuf};
use anyhow::Result;

#[derive(Debug, Default)]
pub struct Config {}

impl Config {
    /// Find the project root by walking up from `dir` looking for `.bilink/`,
    /// then falling back to the git root (`.git/`).
    pub fn load_from(dir: &Path) -> Result<(PathBuf, Config)> {
        let mut current = dir.to_path_buf();
        loop {
            if current.join(".bilink").is_dir() || current.join(".git").exists() {
                return Ok((current, Config {}));
            }
            if !current.pop() {
                return Ok((dir.to_path_buf(), Config {}));
            }
        }
    }
}
