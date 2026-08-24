use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};

use crate::capture::CaptureFile;
use crate::link::{ByteRange, EndpointState, LinkEndpoint};

#[derive(Debug)]
pub struct BiLinkFile {
    pub uuid: String,
    pub link0: LinkEndpoint,
    pub link1: LinkEndpoint,
    pub hash0: Option<String>,
    pub hash_ast0: Option<String>,
    pub commit0: Option<String>,
    pub hash1: Option<String>,
    pub hash_ast1: Option<String>,
    pub commit1: Option<String>,
    pub range0: Option<ByteRange>,
    pub range1: Option<ByteRange>,
    pub state0: Option<EndpointState>,
    pub state1: Option<EndpointState>,
    pub resolved_at: Option<String>,
}

impl BiLinkFile {
    /// Un bilink recién creado: dos endpoints, sin nada de cache.
    pub fn new(uuid: impl Into<String>, link0: LinkEndpoint, link1: LinkEndpoint) -> Self {
        Self {
            uuid: uuid.into(),
            link0, link1,
            hash0: None, hash_ast0: None, commit0: None,
            hash1: None, hash_ast1: None, commit1: None,
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        }
    }

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
        let mut link0: Option<String> = None;
        let mut link1: Option<String> = None;
        let mut hash0 = None;
        let mut hash_ast0 = None;
        let mut commit0 = None;
        let mut hash1 = None;
        let mut hash_ast1 = None;
        let mut commit1 = None;
        let mut range0 = None;
        let mut range1 = None;
        let mut state0 = None;
        let mut state1 = None;
        let mut resolved_at = None;
        let mut current_key: Option<&'static str> = None;

        const KEYS: &[&str] = &[
            "link.0", "link.1",
            "hash.0", "hash_ast.0", "commit.0",
            "hash.1", "hash_ast.1", "commit.1",
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
                    "link.0"      => { link0      = Some(value); "link.0" }
                    "link.1"      => { link1      = Some(value); "link.1" }
                    "hash.0"      => { hash0      = Some(value); "" }
                    "hash_ast.0"  => { hash_ast0  = Some(value); "" }
                    "commit.0"    => { commit0    = Some(value); "" }
                    "hash.1"      => { hash1      = Some(value); "" }
                    "hash_ast.1"  => { hash_ast1  = Some(value); "" }
                    "commit.1"    => { commit1    = Some(value); "" }
                    "range.0"     => { range0     = Some(value); "" }
                    "range.1"     => { range1     = Some(value); "" }
                    "state.0"     => { state0     = Some(value); "" }
                    "state.1"     => { state1     = Some(value); "" }
                    "resolved_at" => { resolved_at = Some(value); "" }
                    _             => ""
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

        Ok(BiLinkFile {
            uuid:        uuid.to_string(),
            link0:       parse_ep(link0, "link.0")?,
            link1:       parse_ep(link1, "link.1")?,
            hash0, hash_ast0, commit0,
            hash1, hash_ast1, commit1,
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
            || self.hash_ast0.is_some() || self.hash_ast1.is_some()
            || self.state0.is_some() || self.state1.is_some()
            || self.range0.is_some() || self.range1.is_some()
            || self.resolved_at.is_some();

        if has_cache {
            out.push_str("\n# Cache\n");
            if let Some(h) = &self.hash0     { push_field(&mut out, "hash.0",     h); }
            if let Some(h) = &self.hash_ast0 { push_field(&mut out, "hash_ast.0", h); }
            if let Some(c) = &self.commit0   { push_field(&mut out, "commit.0",   c); }
            if let Some(r) = &self.range0    { push_field(&mut out, "range.0",    &r.to_string()); }
            if let Some(h) = &self.hash1     { push_field(&mut out, "hash.1",     h); }
            if let Some(h) = &self.hash_ast1 { push_field(&mut out, "hash_ast.1", h); }
            if let Some(c) = &self.commit1   { push_field(&mut out, "commit.1",   c); }
            if let Some(r) = &self.range1    { push_field(&mut out, "range.1",    &r.to_string()); }
            if let Some(s) = &self.state0    { push_field(&mut out, "state.0",    &s.to_string()); }
            if let Some(s) = &self.state1    { push_field(&mut out, "state.1",    &s.to_string()); }
            if let Some(t) = &self.resolved_at { push_field(&mut out, "resolved_at", t); }
        }

        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
    }

    // ─── accessors por índice de endpoint ─────────────────────────────────────

    pub fn link(&self, n: u8) -> &LinkEndpoint {
        if n == 0 { &self.link0 } else { &self.link1 }
    }

    pub fn link_mut(&mut self, n: u8) -> &mut LinkEndpoint {
        if n == 0 { &mut self.link0 } else { &mut self.link1 }
    }

    pub fn state(&self, n: u8) -> &Option<EndpointState> {
        if n == 0 { &self.state0 } else { &self.state1 }
    }

    pub fn set_state(&mut self, n: u8, s: Option<EndpointState>) {
        if n == 0 { self.state0 = s } else { self.state1 = s }
    }

    pub fn hash(&self, n: u8) -> Option<&str> {
        if n == 0 { self.hash0.as_deref() } else { self.hash1.as_deref() }
    }

    pub fn hash_ast(&self, n: u8) -> Option<&str> {
        if n == 0 { self.hash_ast0.as_deref() } else { self.hash_ast1.as_deref() }
    }

    pub fn commit(&self, n: u8) -> Option<&str> {
        if n == 0 { self.commit0.as_deref() } else { self.commit1.as_deref() }
    }

    /// Índice del endpoint estructural, si hay exactamente uno.
    pub fn structural_n(&self) -> Option<u8> {
        match (self.link0.is_structural(), self.link1.is_structural()) {
            (true, _) => Some(0),
            (_, true) => Some(1),
            _         => None,
        }
    }

    /// Resuelve el endpoint `n` a un capture.
    ///
    /// Para endpoints migrados carga el `.capture`. Para los legacy sintetiza uno
    /// en memoria desde la referencia embebida y `range.N`, de modo que el resto
    /// del código no tenga que distinguir los dos formatos. `bilinker migrate`
    /// no hace más que persistir ese capture sintetizado.
    ///
    /// `Ok(None)` si el endpoint no es estructural.
    pub fn capture_for(&self, layer: &Path, n: u8) -> Result<Option<CaptureFile>> {
        match self.link(n) {
            LinkEndpoint::Capture(uuid) => CaptureFile::load_in(layer, uuid).map(Some),
            LinkEndpoint::LegacyStructural(sref) => Ok(Some(CaptureFile {
                uuid:        format!("legacy-{}-{n}", self.uuid),
                sref:        sref.clone(),
                range:       if n == 0 { self.range0.clone() } else { self.range1.clone() },
                state:       None,
                resolved_at: self.resolved_at.clone(),
            })),
            _ => Ok(None),
        }
    }

    /// Hash of the structural endpoint's accepted content hash.
    /// Used by adjacent layer endpoints instead of hashing the full bilink file.
    pub fn structural_hash(&self) -> Option<&str> {
        self.structural_n().and_then(|n| self.hash(n))
    }

    pub fn structural_commit(&self) -> Option<&str> {
        self.structural_n().and_then(|n| self.commit(n))
    }

    pub fn find_by_id(bilinker_dir: &Path, id: &str) -> Result<(PathBuf, BiLinkFile)> {
        for entry in walkdir(bilinker_dir)? {
            if entry.extension().and_then(|e| e.to_str()) == Some("bilink") {
                if let Ok(bl) = BiLinkFile::load(&entry) {
                    if bl.uuid == id || bl.uuid.starts_with(id) {
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
        LinkEndpoint::LegacyStructural(crate::link::StructuralRef {
            file: file.into(),
            query: None,
            range: None,
        })
    }

    fn layer(path: &str) -> LinkEndpoint {
        LinkEndpoint::Layer(stratum::parse_path(path).unwrap())
    }

    #[test]
    fn roundtrip_empty_cache() {
        let dir  = tempdir().unwrap();
        let path = dir.path().join("test-uuid.bilink");

        let original = BiLinkFile::new("test-uuid", structural("file.md"), layer(".stratum/impl"));
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

        let mut original = BiLinkFile::new("abc123", structural("a.md"), structural("b.md"));
        original.hash0   = Some("aabbcc".into());
        original.commit0 = Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0".into());
        original.hash1   = Some("ddeeff".into());
        original.commit1 = Some("b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1".into());
        original.range0  = Some(ByteRange { start: 10, end: 50 });
        original.range1  = Some(ByteRange { start: 0, end: 100 });
        original.state0  = Some(EndpointState::Ok);
        original.state1  = Some(EndpointState::Altered);
        original.resolved_at = Some("2026-05-27T00:00:00Z".into());
        original.write(&path).unwrap();

        let loaded = BiLinkFile::load(&path).unwrap();
        assert_eq!(loaded.hash0.as_deref(), Some("aabbcc"));
        assert_eq!(loaded.commit0.as_deref(), Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"));
        assert_eq!(loaded.hash1.as_deref(), Some("ddeeff"));
        assert_eq!(loaded.commit1.as_deref(), Some("b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1"));
        assert_eq!(loaded.range0, Some(ByteRange { start: 10, end: 50 }));
        assert_eq!(loaded.state0, Some(EndpointState::Ok));
        assert_eq!(loaded.state1, Some(EndpointState::Altered));
        assert_eq!(loaded.resolved_at.as_deref(), Some("2026-05-27T00:00:00Z"));
    }

    #[test]
    fn uuid_comes_from_filename() {
        let text = "link.0: file.md\nlink.1: .stratum/impl\n";
        let bl = BiLinkFile::parse(text, "file-stem-uuid").unwrap();
        assert_eq!(bl.uuid, "file-stem-uuid");
    }

    #[test]
    fn find_by_id_locates_file() {
        let dir = tempdir().unwrap();
        let bl = BiLinkFile::new("my-uuid", structural("a.md"), structural("b.md"));
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
