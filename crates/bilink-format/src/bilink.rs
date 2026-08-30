//! El archivo `<uuid>.yaml`: una declaración y dos decisiones.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::link::LinkEndpoint;

/// Un bilink: exactamente dos endpoints, distinguibles por su índice.
///
/// **La aridad es fija y la garantiza el tipo.** Tres endpoints se rechaza; que
/// falte el `1`, también. Deja de ser algo que hay que verificar y pasa a ser algo
/// que no se puede escribir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BiLink {
    /// Clasifica la relación. Inerte: no afecta ningún hash ni ningún estado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub endpoint: Endpoints,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Endpoints {
    #[serde(rename = "0")] pub zero: Endpoint,
    #[serde(rename = "1")] pub one:  Endpoint,
}

/// Un extremo: a qué apunta, y qué se aprobó de él.
///
/// > `apply` escribe `link`. `accept` escribe `accepted`. `check` no escribe nada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub link: LinkEndpoint,
    /// **Su ausencia es `PENDING`, literalmente.** La invariante de aceptación no
    /// se enuncia: es este `Option`, y el bloque no se puede escribir a medias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Accepted>,
    /// Etiqueta del rol de este extremo. Inerte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Lo que alguien aprobó: una ubicación y un contenido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Accepted {
    /// La ubicación aprobada. Ausente en un endpoint `issue`, que no tiene capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkEndpoint>,
    pub hash: String,
    /// Sólo donde hay gramática tree-sitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_ast: Option<String>,
}

impl Endpoints {
    pub fn get(&self, n: u8) -> &Endpoint {
        match n { 0 => &self.zero, 1 => &self.one, _ => panic!("endpoint {n}: la aridad es 0 o 1") }
    }
    pub fn get_mut(&mut self, n: u8) -> &mut Endpoint {
        match n { 0 => &mut self.zero, 1 => &mut self.one, _ => panic!("endpoint {n}: la aridad es 0 o 1") }
    }
}

impl BiLink {
    pub fn new(link0: LinkEndpoint, link1: LinkEndpoint) -> Self {
        Self {
            kind: None,
            endpoint: Endpoints {
                zero: Endpoint { link: link0, accepted: None, name: None },
                one:  Endpoint { link: link1, accepted: None, name: None },
            },
        }
    }

    pub fn dir(layer: &Path) -> PathBuf { layer.join(".bilink") }

    pub fn path_in(layer: &Path, uuid: &str) -> PathBuf {
        Self::dir(layer).join(format!("{uuid}.yaml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        serde_yaml_ng::from_str(&text).with_context(|| format!("parseando {}", path.display()))
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(path, self.to_yaml()?)?;
        Ok(())
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml_ng::to_string(self).context("serializando el bilink")
    }

    /// El `accepted` del endpoint estructural, si hay uno.
    ///
    /// Es lo que un endpoint `path` copia de su vecino: los dos valores, no el hash
    /// del archivo. Copiar el archivo entero haría que cualquier reordenamiento o
    /// comentario del vecino disparara `CHAIN_DIRTY`.
    pub fn structural_accepted(&self) -> Option<&Accepted> {
        for n in [0u8, 1u8] {
            let e = self.endpoint.get(n);
            if e.link.is_structural() {
                return e.accepted.as_ref();
            }
        }
        None
    }
}

/// Los archivos de bilink de una carpeta `.bilink/`, sin recursión.
///
/// Sin recursión a propósito: `capture/`, `cache/` e `index/` son subcarpetas de la
/// misma raíz y ninguna contiene bilinks.
pub fn bilink_files(bilink_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(bilink_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()) == Some("yaml"))
        .filter(|p| !p.file_name().and_then(|n| n.to_str())
                      .map(|n| n.starts_with('.')).unwrap_or(false))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ep(raw: &str) -> LinkEndpoint { raw.parse().unwrap() }

    #[test]
    fn a_bilink_round_trips() {
        let dir = tempdir().unwrap();
        let mut bl = BiLink::new(ep("capture abc123"), ep("path >impl"));
        bl.endpoint.zero.accepted = Some(Accepted {
            link: Some(ep("capture abc123")),
            hash: "c00e0760".into(),
            hash_ast: Some("1b9e44a2".into()),
        });
        let p = BiLink::path_in(dir.path(), "7f3d8e9a");
        bl.write(&p).unwrap();
        assert_eq!(BiLink::load(&p).unwrap(), bl);
    }

    /// La aridad no se verifica: no se puede escribir otra cosa.
    #[test]
    fn the_arity_is_the_type() {
        let three = "endpoint:\n  0: {link: capture a}\n  1: {link: capture b}\n  2: {link: capture c}\n";
        let err = serde_yaml_ng::from_str::<BiLink>(three).unwrap_err().to_string();
        assert!(err.contains('2'), "tres endpoints se rechaza: {err}");

        let missing = "endpoint:\n  0: {link: capture a}\n";
        let err = serde_yaml_ng::from_str::<BiLink>(missing).unwrap_err().to_string();
        assert!(err.contains('1'), "que falte el 1 se rechaza: {err}");
    }

    /// `accepted` está completo o ausente. No hay medio bloque.
    #[test]
    fn accepted_is_all_or_nothing() {
        let no_hash = "endpoint:\n  0: {link: capture a, accepted: {link: capture a}}\n  1: {link: capture b}\n";
        let err = serde_yaml_ng::from_str::<BiLink>(no_hash).unwrap_err().to_string();
        assert!(err.contains("hash"), "accepted sin hash se rechaza: {err}");

        let loose = "endpoint:\n  0: {link: capture a, hash: deadbeef}\n  1: {link: capture b}\n";
        assert!(serde_yaml_ng::from_str::<BiLink>(loose).is_err(),
            "un hash suelto fuera del bloque se rechaza");
    }

    /// Su ausencia *es* PENDING: no hay campo que lo diga.
    #[test]
    fn a_fresh_bilink_has_no_accepted() {
        let bl = BiLink::new(ep("capture a"), ep("path >impl"));
        assert!(bl.endpoint.zero.accepted.is_none());
        let y = bl.to_yaml().unwrap();
        assert!(!y.contains("accepted"), "sin aceptar, el bloque no se escribe:\n{y}");
    }

    /// Las claves `0` y `1` matchean por nombre, no por posición.
    #[test]
    fn the_endpoint_keys_match_by_name() {
        let reversed = "endpoint:\n  1: {link: path >impl}\n  0: {link: capture a}\n";
        let bl: BiLink = serde_yaml_ng::from_str(reversed).unwrap();
        assert_eq!(bl.endpoint.zero.link, ep("capture a"));
        assert_eq!(bl.endpoint.one.link, ep("path >impl"));
    }

    /// Un campo desconocido se rechaza con su nombre, nunca se descarta.
    ///
    /// Descartarlo en silencio es cómo un binario viejo vaciaría las aceptaciones.
    #[test]
    fn an_unknown_field_is_rejected_by_name() {
        let raw = "endpoint:\n  0: {link: capture a}\n  1: {link: capture b}\nresolved_at: 2026-01-01\n";
        let err = serde_yaml_ng::from_str::<BiLink>(raw).unwrap_err().to_string();
        assert!(err.contains("resolved_at"), "{err}");
    }
}
