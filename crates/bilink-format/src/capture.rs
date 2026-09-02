//! El archivo `capture/<id>.yaml`: una ubicación, y nada más.
//!
//! El id es el hash de lo único que contiene. De ahí salen la inmutabilidad y la
//! deduplicación por construcción.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};


/// Dónde está un fragmento: qué archivo y qué nodo.
///
/// **Es inmutable.** Cambiarle un campo le cambiaría el id, así que no se cambia:
/// se acuña otro. Ningún comando modifica un capture existente.
///
/// **No hay sub-rango.** Un rango de bytes adentro de un nodo se corre con
/// cualquier edición encima suya dentro del mismo nodo: su granularidad es
/// ilusoria, se rompe todo el tiempo y hay que repuntarlo. Si hace falta más
/// precisión, la respuesta es una query — que nombre algo más chico, o que nombre
/// varios nodos y deje el resto afuera.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// Path relativo a la raíz de la capa.
    pub file: String,
    /// Query tree-sitter con una o más capturas `@target`. Ausente = el archivo
    /// completo.
    ///
    /// Con varias, el fragmento es la concatenación de los `@target` en orden de
    /// archivo, unida por [`crate::FRAGMENT_SEPARATOR`]. Sigue siendo un string y
    /// no una lista: un patrón único ancla los fragmentos entre sí, y tree-sitter
    /// lo matchea entero o no lo matchea — con una lista habría resolución parcial,
    /// que es un fragmento a medias sin estado que lo nombre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

impl Capture {
    /// El id, que es el nombre del archivo: **cada campo seguido de un `\0`**.
    ///
    /// El terminador va después de cada campo y no entre campos, y eso importa:
    /// así el id no cambia cuando un campo desaparece del formato. Es lo que
    /// permitió sacar el `offset` sin re-acuñar los 316 captures que existían —
    /// ninguno lo tenía, y su contribución al hash era la cadena vacía.
    ///
    /// Se calcula sobre los campos y no sobre el texto serializado: dos formas de
    /// escribir el mismo YAML darían dos ids para la misma ubicación.
    pub fn id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.file.as_bytes());
        h.update([0]);
        h.update(self.query.as_deref().unwrap_or("").as_bytes());
        h.update([0]);
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
        // **Crear el primer capture es crear la capa**, y una capa sin `version` es
        // indistinguible de una anterior a que el campo existiera.
        crate::version::ensure_layer(layer)?;
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
        Capture { file: file.into(), query: query.map(String::from) }
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
        assert_ne!(base.id(), cap("a.rs", None).id());
    }

    /// El terminador va **después** de cada campo, no entre campos.
    ///
    /// Es lo que hace que sacar un campo del formato no cambie ningún id: el que
    /// desaparece contribuía la cadena vacía, y su `\0` sigue estando. Sin esto,
    /// quitar el `offset` habría obligado a re-acuñar los 316 captures que había.
    #[test]
    fn the_id_terminates_each_field_instead_of_joining_them() {
        use sha2::{Digest, Sha256};
        let c = cap("a.rs", Some("(x) @target"));
        let mut h = Sha256::new();
        for campo in ["a.rs", "(x) @target"] {
            h.update(campo.as_bytes());
            h.update([0]);
        }
        assert_eq!(c.id(), hex::encode(h.finalize())[..32].to_string());
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
        let c = cap("src/check.rs", Some("(function_item\n  name: (identifier) @n0) @target"));
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
