use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};

use crate::link::LinkEndpoint;

#[derive(Debug)]
pub struct BiLinkFile {
    pub id: String,
    pub link0: LinkEndpoint,
    pub link1: LinkEndpoint,
    pub hash0: Option<String>,
    pub hash1: Option<String>,
}

impl BiLinkFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let mut id = None;
        let mut link0: Option<String> = None;
        let mut link1: Option<String> = None;
        let mut hash0 = None;
        let mut hash1 = None;
        let mut current_key: Option<&str> = None;

        // Known top-level keys — lines not starting with one of these are continuations.
        const KEYS: &[&str] = &["id", "link.0", "link.1", "hash.0", "hash.1", "resolved_at"];

        for line in text.lines() {
            if line.trim().is_empty() || line.trim().starts_with('#') {
                current_key = None;
                continue;
            }

            // Check if this line starts a new key.
            let is_new_key = KEYS.iter().any(|k| line.starts_with(k) && line[k.len()..].starts_with(':'));

            if is_new_key {
                let colon = line.find(':').unwrap();
                let key   = line[..colon].trim();
                let value = line[colon + 1..].trim().to_string();
                current_key = Some(match key {
                    "id"           => { id    = Some(value); "id" }
                    "link.0"       => { link0 = Some(value); "link.0" }
                    "link.1"       => { link1 = Some(value); "link.1" }
                    "hash.0"       => { hash0 = Some(value); "" }
                    "hash.1"       => { hash1 = Some(value); "" }
                    _              => ""
                });
            } else if let Some(key) = current_key {
                // Continuation line — append to current value (normalize whitespace).
                let continuation = line.trim();
                match key {
                    "link.0" => link0.get_or_insert_default().push_str(&format!(" {continuation}")),
                    "link.1" => link1.get_or_insert_default().push_str(&format!(" {continuation}")),
                    _ => {}
                }
            }
        }

        let parse_ep = |raw: Option<String>, field: &str| -> Result<LinkEndpoint> {
            raw.with_context(|| format!("missing '{field}' field"))?
                .parse::<LinkEndpoint>()
                .with_context(|| format!("parsing {field}"))
        };

        Ok(BiLinkFile {
            id:    id.context("missing 'id' field")?,
            link0: parse_ep(link0, "link.0")?,
            link1: parse_ep(link1, "link.1")?,
            hash0,
            hash1,
        })
    }

    /// Search for a .bilink file by id under `bilinker_dir`.
    pub fn find_by_id(bilinker_dir: &Path, id: &str) -> Result<(PathBuf, BiLinkFile)> {
        for entry in walkdir(bilinker_dir)? {
            if entry.extension().and_then(|e| e.to_str()) == Some("bilink") {
                if let Ok(bl) = BiLinkFile::load(&entry) {
                    if bl.id == id {
                        return Ok((entry, bl));
                    }
                }
            }
        }
        bail!("no .bilink file with id '{id}' found under {}", bilinker_dir.display())
    }
}

fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    if !dir.exists() {
        return Ok(result);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            result.extend(walkdir(&path)?);
        } else {
            result.push(path);
        }
    }
    Ok(result)
}
