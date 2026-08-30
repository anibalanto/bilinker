//! El archivo `.capture`: dónde está un fragmento, y cómo volver a encontrarlo.
//!
//! Sólo el formato. El algoritmo que *produce* un capture —el walk-up por el AST,
//! la construcción de la query tree-sitter— vive en `bilinker::capture`, porque
//! depende de tree-sitter y de las gramáticas, y el formato no depende de nada.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::link::{ByteRange, StructuralRef};

/// Estado de resolución de un capture: ¿dónde está el fragmento?
///
/// Distinto de `EndpointState`, que responde si lo que hay coincide con lo
/// aceptado. Un capture no sabe nada de hashes aceptados.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum CaptureState {
    /// La query matchea; `range` actualizado.
    #[serde(rename = "RESOLVED")]
    Resolved,
    /// El archivo cambió de path (git rename ≥ 50%).
    #[serde(rename = "MOVED")]
    Moved,
    /// Anchor renombrado; nodo del mismo tipo con nombre distinto.
    #[serde(rename = "REANCHORED")]
    Reanchored,
    /// La query no matchea y no se localiza el anchor.
    #[serde(rename = "UNANCHORED")]
    Unanchored,
    /// El archivo no existe; eliminación rastreable en git.
    #[serde(rename = "DELETED")]
    Deleted,
    /// El archivo no se puede leer o parsear.
    #[serde(rename = "BROKEN")]
    Broken,
}

impl CaptureState {
    /// Un capture que no está en este conjunto no permite evaluar a sus referentes.
    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved)
    }
}

impl fmt::Display for CaptureState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolved   => write!(f, "RESOLVED"),
            Self::Moved      => write!(f, "MOVED"),
            Self::Reanchored => write!(f, "REANCHORED"),
            Self::Unanchored => write!(f, "UNANCHORED"),
            Self::Deleted    => write!(f, "DELETED"),
            Self::Broken     => write!(f, "BROKEN"),
        }
    }
}

impl FromStr for CaptureState {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "RESOLVED"   => Ok(Self::Resolved),
            "MOVED"      => Ok(Self::Moved),
            "REANCHORED" => Ok(Self::Reanchored),
            "UNANCHORED" => Ok(Self::Unanchored),
            "DELETED"    => Ok(Self::Deleted),
            "BROKEN"     => Ok(Self::Broken),
            other        => anyhow::bail!("estado de capture desconocido: '{other}'"),
        }
    }
}

/// Un capture: dónde está un fragmento, y cómo volver a encontrarlo.
///
/// No contiene hashes ni commits — eso es aceptación, y vive en el bilink.
/// Varios bilinks pueden referenciar el mismo capture.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CaptureFile {
    pub uuid: String,
    /// `file`, `query` y `offset` (el `range` de `StructuralRef` es el offset
    /// relativo al nodo matcheado, no el absoluto en el archivo).
    pub sref: StructuralRef,
    /// Cache: byte range absoluto de la última resolución.
    pub range: Option<ByteRange>,
    pub state: Option<CaptureState>,
    pub resolved_at: Option<String>,
}

impl CaptureFile {
    /// `<layer>/.bilink/capture/`
    pub fn dir(layer: &Path) -> PathBuf {
        layer.join(".bilink").join("capture")
    }

    pub fn path_in(layer: &Path, uuid: &str) -> PathBuf {
        Self::dir(layer).join(format!("{uuid}.capture"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let uuid = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        Self::parse(&text, &uuid).with_context(|| format!("parsing {}", path.display()))
    }

    /// Carga el capture `uuid` de la capa `layer`.
    pub fn load_in(layer: &Path, uuid: &str) -> Result<Self> {
        Self::load(&Self::path_in(layer, uuid))
    }

    pub fn parse(text: &str, uuid: &str) -> Result<Self> {
        const KEYS: &[&str] = &["file", "query", "offset", "range", "state", "resolved_at"];

        let mut file: Option<String> = None;
        let mut query: Option<String> = None;
        let mut offset: Option<String> = None;
        let mut range: Option<String> = None;
        let mut state: Option<String> = None;
        let mut resolved_at: Option<String> = None;
        let mut current_key: Option<&'static str> = None;

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
                    "file"        => { file        = Some(value); "" }
                    "query"       => { query       = Some(value); "query" }
                    "offset"      => { offset      = Some(value); "" }
                    "range"       => { range       = Some(value); "" }
                    "state"       => { state       = Some(value); "" }
                    "resolved_at" => { resolved_at = Some(value); "" }
                    _             => ""
                });
            } else if current_key == Some("query") {
                // Las queries tree-sitter son multilínea.
                query.get_or_insert_default().push_str(&format!("\n  {}", line.trim()));
            }
        }

        Ok(CaptureFile {
            uuid: uuid.to_string(),
            sref: StructuralRef {
                file:  file.context("missing 'file' field")?,
                query,
                range: offset.as_deref().map(str::parse).transpose()
                           .context("parsing offset")?,
            },
            range: range.as_deref().map(str::parse).transpose().context("parsing range")?,
            state: state.as_deref().map(str::parse).transpose().context("parsing state")?,
            resolved_at,
        })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        out.push_str(&format!("file:   {}\n", self.sref.file));
        if let Some(q) = &self.sref.query {
            out.push_str(&format!("query:  {q}\n"));
        }
        if let Some(o) = &self.sref.range {
            out.push_str(&format!("offset: {o}\n"));
        }

        if self.range.is_some() || self.state.is_some() || self.resolved_at.is_some() {
            out.push_str("\n# Cache\n");
            if let Some(r) = &self.range       { out.push_str(&format!("range:       {r}\n")); }
            if let Some(s) = &self.state       { out.push_str(&format!("state:       {s}\n")); }
            if let Some(t) = &self.resolved_at { out.push_str(&format!("resolved_at: {t}\n")); }
        }

        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
    }

    pub fn write_in(&self, layer: &Path) -> Result<PathBuf> {
        let path = Self::path_in(layer, &self.uuid);
        self.write(&path)?;
        Ok(path)
    }

    /// Todos los captures de una capa.
    pub fn all_in(layer: &Path) -> Result<Vec<CaptureFile>> {
        let dir = Self::dir(layer);
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(out) };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("capture") { continue; }
            if let Ok(c) = Self::load(&path) { out.push(c); }
        }
        Ok(out)
    }
}

/// Resuelve un endpoint a su referencia estructural, sin necesidad del bilink.
///
/// Carga el capture si el endpoint está migrado; usa la referencia embebida si
/// todavía es legacy. `Ok(None)` si el endpoint no es estructural.
pub fn sref_of(layer: &Path, endpoint: &crate::link::LinkEndpoint) -> Result<Option<StructuralRef>> {
    use crate::link::LinkEndpoint;
    match endpoint {
        LinkEndpoint::Capture(uuid) => Ok(Some(CaptureFile::load_in(layer, uuid)?.sref)),
        LinkEndpoint::LegacyStructural(sref) => Ok(Some(sref.clone())),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod capture_file_tests {
    use super::*;
    use tempfile::tempdir;

    fn sref(file: &str, query: Option<&str>) -> StructuralRef {
        StructuralRef { file: file.into(), query: query.map(String::from), range: None }
    }

    fn write_cap(layer: &Path, uuid: &str, s: StructuralRef) {
        CaptureFile { uuid: uuid.into(), sref: s, range: None, state: None, resolved_at: None }
            .write_in(layer).unwrap();
    }

    #[test]
    fn roundtrip_preserves_query_and_offset() {
        let dir  = tempdir().unwrap();
        let path = dir.path().join("x.capture");
        CaptureFile {
            uuid: "x".into(),
            sref: StructuralRef {
                file:  "a.rs".into(),
                query: Some("(function_item\n  name: (identifier) @n0) @target".into()),
                range: Some(ByteRange { start: 4, end: 20 }),
            },
            range: Some(ByteRange { start: 100, end: 200 }),
            state: Some(CaptureState::Resolved),
            resolved_at: Some("2026-08-24T00:00:00Z".into()),
        }.write(&path).unwrap();

        let back = CaptureFile::load(&path).unwrap();
        assert_eq!(back.sref.range, Some(ByteRange { start: 4, end: 20 }));
        assert_eq!(back.range, Some(ByteRange { start: 100, end: 200 }));
        assert_eq!(back.state, Some(CaptureState::Resolved));
        assert!(back.sref.query.as_deref().unwrap().contains("name: (identifier)"));
    }
}

