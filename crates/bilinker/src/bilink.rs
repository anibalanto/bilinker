use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};

use crate::link::{ByteRange, EndpointState, LinkEndpoint};

#[derive(Debug)]
pub struct BiLinkFile {
    pub uuid: String,
    pub link0: LinkEndpoint,
    pub link1: LinkEndpoint,
    pub hash0: Option<String>,
    pub hash1: Option<String>,
    pub range0: Option<ByteRange>,
    pub range1: Option<ByteRange>,
    pub state0: Option<EndpointState>,
    pub state1: Option<EndpointState>,
    pub resolved_at: Option<String>,
}

impl BiLinkFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let uuid = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        Self::parse(&text, &uuid).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str, uuid: &str) -> Result<Self> {
        let mut id_field: Option<String> = None;
        let mut link0: Option<String> = None;
        let mut link1: Option<String> = None;
        let mut hash0 = None;
        let mut hash1 = None;
        let mut range0 = None;
        let mut range1 = None;
        let mut state0 = None;
        let mut state1 = None;
        let mut resolved_at = None;
        let mut current_key: Option<&'static str> = None;

        const KEYS: &[&str] = &[
            "id", "link.0", "link.1",
            "hash.0", "hash.1",
            "range.0", "range.1",
            "state.0", "state.1",
            "resolved_at",
        ];

        for line in text.lines() {
            if line.trim().is_empty() || line.trim().starts_with('#') {
                current_key = None;
                continue;
            }

            let is_new_key = KEYS.iter().any(|k| {
                line.starts_with(k) && line[k.len()..].starts_with(':')
            });

            if is_new_key {
                let colon = line.find(':').unwrap();
                let key   = line[..colon].trim();
                let value = line[colon + 1..].trim().to_string();
                current_key = Some(match key {
                    "id"         => { id_field   = Some(value); "id" }
                    "link.0"     => { link0       = Some(value); "link.0" }
                    "link.1"     => { link1       = Some(value); "link.1" }
                    "hash.0"     => { hash0       = Some(value); "" }
                    "hash.1"     => { hash1       = Some(value); "" }
                    "range.0"    => { range0      = Some(value); "" }
                    "range.1"    => { range1      = Some(value); "" }
                    "state.0"    => { state0      = Some(value); "" }
                    "state.1"    => { state1      = Some(value); "" }
                    "resolved_at"=> { resolved_at = Some(value); "" }
                    _            => ""
                });
            } else if let Some(key) = current_key {
                let cont = line.trim();
                match key {
                    "link.0" => link0.get_or_insert_default().push_str(&format!(" {cont}")),
                    "link.1" => link1.get_or_insert_default().push_str(&format!(" {cont}")),
                    _ => {}
                }
            }
        }

        let parse_ep = |raw: Option<String>, field: &str| -> Result<LinkEndpoint> {
            raw.with_context(|| format!("missing '{field}' field"))?
                .parse::<LinkEndpoint>()
                .with_context(|| format!("parsing {field}"))
        };

        let final_uuid = id_field.unwrap_or_else(|| uuid.to_string());

        Ok(BiLinkFile {
            uuid:        final_uuid,
            link0:       parse_ep(link0, "link.0")?,
            link1:       parse_ep(link1, "link.1")?,
            hash0,
            hash1,
            range0:      range0.as_deref().map(str::parse).transpose()
                             .context("parsing range.0")?,
            range1:      range1.as_deref().map(str::parse).transpose()
                             .context("parsing range.1")?,
            state0:      state0.as_deref().map(str::parse).transpose()
                             .context("parsing state.0")?,
            state1:      state1.as_deref().map(str::parse).transpose()
                             .context("parsing state.1")?,
            resolved_at,
        })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::new();

        push_field(&mut out, "link.0", &self.link0.to_string());
        push_field(&mut out, "link.1", &self.link1.to_string());

        let has_cache = self.hash0.is_some() || self.hash1.is_some()
            || self.state0.is_some() || self.state1.is_some();

        if has_cache {
            out.push_str("\n# --- cache generada por bilinker, no editar a mano ---\n");
            if let Some(h) = &self.hash0  { push_field(&mut out, "hash.0",  h); }
            if let Some(r) = &self.range0 { push_field(&mut out, "range.0", &r.to_string()); }
            if let Some(h) = &self.hash1  { push_field(&mut out, "hash.1",  h); }
            if let Some(r) = &self.range1 { push_field(&mut out, "range.1", &r.to_string()); }
            if let Some(s) = &self.state0 { push_field(&mut out, "state.0", &s.to_string()); }
            if let Some(s) = &self.state1 { push_field(&mut out, "state.1", &s.to_string()); }
            if let Some(t) = &self.resolved_at { push_field(&mut out, "resolved_at", t); }
        }

        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
    }

    pub fn find_by_id(bilinker_dir: &Path, id: &str) -> Result<(PathBuf, BiLinkFile)> {
        for entry in walkdir(bilinker_dir)? {
            if entry.extension().and_then(|e| e.to_str()) == Some("bilink") {
                if let Ok(bl) = BiLinkFile::load(&entry) {
                    if bl.uuid == id {
                        return Ok((entry, bl));
                    }
                }
            }
        }
        bail!("no .bilink file with id '{id}' found under {}", bilinker_dir.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{ByteRange, EndpointState};
    use tempfile::tempdir;

    fn structural(file: &str) -> LinkEndpoint {
        LinkEndpoint::Structural(crate::link::StructuralRef {
            file: file.into(),
            query: None,
            range: None,
        })
    }

    fn layer(path: &str) -> LinkEndpoint {
        LinkEndpoint::Layer(estrato::parse_path(path).unwrap())
    }

    #[test]
    fn roundtrip_empty_cache() {
        let dir  = tempdir().unwrap();
        let path = dir.path().join("test-uuid.bilink");

        let original = BiLinkFile {
            uuid:        "test-uuid".into(),
            link0:       structural("file.md"),
            link1:       layer(".estrato/impl"),
            hash0: None, hash1: None,
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        original.write(&path).unwrap();

        let loaded = BiLinkFile::load(&path).unwrap();
        assert_eq!(loaded.uuid, "test-uuid");
        assert!(loaded.hash0.is_none());
        assert!(loaded.state0.is_none());
    }

    #[test]
    fn roundtrip_full_cache() {
        let dir  = tempdir().unwrap();
        let path = dir.path().join("abc123.bilink");

        let original = BiLinkFile {
            uuid:        "abc123".into(),
            link0:       structural("a.md"),
            link1:       structural("b.md"),
            hash0:       Some("aabbcc".into()),
            hash1:       Some("ddeeff".into()),
            range0:      Some(ByteRange { start: 10, end: 50 }),
            range1:      Some(ByteRange { start: 0, end: 100 }),
            state0:      Some(EndpointState::Ok),
            state1:      Some(EndpointState::Altered),
            resolved_at: Some("2026-05-25T00:00:00Z".into()),
        };
        original.write(&path).unwrap();

        let loaded = BiLinkFile::load(&path).unwrap();
        assert_eq!(loaded.hash0.as_deref(), Some("aabbcc"));
        assert_eq!(loaded.hash1.as_deref(), Some("ddeeff"));
        assert_eq!(loaded.range0, Some(ByteRange { start: 10, end: 50 }));
        assert_eq!(loaded.state0, Some(EndpointState::Ok));
        assert_eq!(loaded.state1, Some(EndpointState::Altered));
        assert_eq!(loaded.resolved_at.as_deref(), Some("2026-05-25T00:00:00Z"));
    }

    #[test]
    fn parse_legacy_id_field() {
        let text = "id: my-old-id\nlink.0: file.md\nlink.1: .estrato/impl\n";
        let bl = BiLinkFile::parse(text, "ignored-uuid").unwrap();
        assert_eq!(bl.uuid, "my-old-id");
    }

    #[test]
    fn parse_uuid_from_filename_when_no_id_field() {
        let text = "link.0: file.md\nlink.1: .estrato/impl\n";
        let bl = BiLinkFile::parse(text, "file-stem-uuid").unwrap();
        assert_eq!(bl.uuid, "file-stem-uuid");
    }

    #[test]
    fn find_by_id_locates_file() {
        let dir = tempdir().unwrap();
        let bl = BiLinkFile {
            uuid: "my-uuid".into(),
            link0: structural("a.md"),
            link1: structural("b.md"),
            hash0: None, hash1: None,
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = dir.path().join("my-uuid.bilink");
        bl.write(&path).unwrap();

        let (found_path, found_bl) = BiLinkFile::find_by_id(dir.path(), "my-uuid").unwrap();
        assert_eq!(found_path, path);
        assert_eq!(found_bl.uuid, "my-uuid");
    }
}

fn push_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

pub fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
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
