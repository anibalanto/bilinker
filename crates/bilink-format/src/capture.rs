//! El archivo `capture/<id>.yaml`: una ubicación, y nada más.
//!
//! El id es `H(file, query, offset)` — el hash de lo único que contiene. De ahí
//! salen la inmutabilidad y la deduplicación por construcción.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::link::ByteRange;

/// Dónde está un fragmento: qué archivo, qué nodo, y qué parte del nodo.
///
/// **Es inmutable.** Cambiarle un campo le cambiaría el id, así que no se cambia:
/// se acuña otro. Ningún comando modifica un capture existente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// Path relativo a la raíz de la capa.
    pub file: String,
    /// Query tree-sitter con captura `@target`. Ausente = el archivo completo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Sub-rango relativo al nodo matcheado. Ausente = el nodo entero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<ByteRange>,
}

impl Capture {
    /// `H(file, query, offset)` — el id, que es el nombre del archivo.
    ///
    /// Se calcula sobre los campos y no sobre el texto serializado: dos formas de
    /// escribir el mismo YAML darían dos ids para la misma ubicación.
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.file.as_bytes());
        h.update([0]);
        h.update(self.query.as_deref().unwrap_or("").as_bytes());
        h.update([0]);
        h.update(self.offset.as_ref().map(|o| o.to_string()).unwrap_or_default().as_bytes());
        hex::encode(h.finalize())[..32].to_string()
    }

    /// `<layer>/.bilink/capture/`
    pub fn dir(layer: &Path) -> PathBuf {
        layer.join(".bilink").join("capture")
    }

    pub fn path_in(layer: &Path, id: &str) -> PathBuf {
        Self::dir(layer).join(format!("{id}.yaml"))
    }

    pub fn load_in(layer: &Path, id: &str) -> Result<Self> {
        let path = Self::path_in(layer, id);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("leyendo el capture {}", path.display()))?;
        serde_yaml_ng::from_str(&text)
            .with_context(|| format!("parseando {}", path.display()))
    }

    /// Escribe el capture bajo su propio id. No-op si ya existía.
    ///
    /// Devuelve `(id, path, ya_existía)`. Que exista no es una condición especial:
    /// el id sale del contenido, así que el mismo fragmento es el mismo archivo.
    pub fn write_in(&self, layer: &Path) -> Result<(String, PathBuf, bool)> {
        let id   = self.id();
        let path = Self::path_in(layer, &id);
        if path.exists() {
            return Ok((id, path, true));
        }
        std::fs::create_dir_all(Self::dir(layer))?;
        std::fs::write(&path, self.to_yaml()?)?;
        Ok((id, path, false))
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml_ng::to_string(self).context("serializando el capture")
    }

    /// Todos los captures de la capa, con su id.
    pub fn all_in(layer: &Path) -> Result<Vec<(String, Self)>> {
        let dir = Self::dir(layer);
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else { return Ok(out) };
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("yaml") { continue; }
            let Some(id) = p.file_stem().and_then(|x| x.to_str()) else { continue };
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            if let Ok(cap) = serde_yaml_ng::from_str::<Self>(&text) {
                out.push((id.to_string(), cap));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cap(file: &str, query: Option<&str>) -> Capture {
        Capture { file: file.into(), query: query.map(String::from), offset: None }
    }

    /// El id sale de la ubicación, así que la misma ubicación da el mismo id.
    #[test]
    fn the_id_is_reproducible() {
        assert_eq!(cap("a.rs", Some("(x) @target")).id(), cap("a.rs", Some("(x) @target")).id());
    }

    /// Y una ubicación distinta da un id distinto, campo por campo.
    #[test]
    fn a_different_location_is_a_different_id() {
        let base = cap("a.rs", Some("(x) @target"));
        assert_ne!(base.id(), cap("b.rs", Some("(x) @target")).id());
        assert_ne!(base.id(), cap("a.rs", Some("(y) @target")).id());
        let mut with_offset = base.clone();
        with_offset.offset = Some(ByteRange { start: 0, end: 10 });
        assert_ne!(base.id(), with_offset.id());
    }

    /// Escribir dos veces la misma ubicación no duplica: es el mismo archivo.
    #[test]
    fn writing_twice_deduplicates_by_construction() {
        let dir = tempdir().unwrap();
        let (id1, _, existed1) = cap("a.rs", Some("(x) @target")).write_in(dir.path()).unwrap();
        let (id2, _, existed2) = cap("a.rs", Some("(x) @target")).write_in(dir.path()).unwrap();
        assert_eq!(id1, id2);
        assert!(!existed1 && existed2);
        assert_eq!(Capture::all_in(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn a_capture_round_trips_through_yaml() {
        let dir = tempdir().unwrap();
        let mut c = cap("src/check.rs", Some("(function_item\n  name: (identifier) @n0) @target"));
        c.offset = Some(ByteRange { start: 42, end: 118 });
        let (id, _, _) = c.write_in(dir.path()).unwrap();
        assert_eq!(Capture::load_in(dir.path(), &id).unwrap(), c);
    }

    /// La query multilínea va en bloque, sin escapes: el `git diff` es la superficie
    /// de revisión del producto y `\n` la arruinaría.
    #[test]
    fn a_multiline_query_is_written_as_a_block() {
        let c = cap("a.rs", Some("(function_item\n  name: (identifier) @n0) @target"));
        let y = c.to_yaml().unwrap();
        assert!(!y.contains("\\n"), "la query no debería llevar escapes:\n{y}");
    }

    /// Un campo que el formato no conoce se rechaza con su nombre.
    #[test]
    fn an_unknown_field_is_rejected() {
        let err = serde_yaml_ng::from_str::<Capture>("file: a.rs\nresolved_at: 2026-01-01\n")
            .unwrap_err().to_string();
        assert!(err.contains("resolved_at"), "el error tiene que nombrar el campo: {err}");
    }
}
