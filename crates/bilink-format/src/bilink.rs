//! El archivo `<uuid>.yaml`: una declaración y dos decisiones.

use std::collections::BTreeSet;
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

/// Exactamente dos endpoints, `0` y `1`.
///
/// La serialización es manual por una sola razón: `#[serde(rename = "0")]` sobre un
/// campo produce una **clave string**, y el YAML sale con `'0':` entre comillas. Las
/// claves son enteras y así se escriben.
///
/// La aridad sigue siendo del tipo: la struct tiene dos campos y no puede tener otra
/// cosa. Lo que la deserialización agrega es el mensaje de error cuando el archivo
/// dice algo distinto.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    pub zero: Endpoint,
    pub one:  Endpoint,
}

impl Serialize for Endpoints {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(2))?;
        m.serialize_entry(&0u8, &self.zero)?;
        m.serialize_entry(&1u8, &self.one)?;
        m.end()
    }
}

impl<'de> Deserialize<'de> for Endpoints {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use std::collections::BTreeMap;

        // Las claves se aceptan enteras o entrecomilladas. Escribimos enteras; leer
        // las dos formas cuesta nada y evita que un archivo escrito a mano —o por
        // una versión anterior del serializador— deje de parsear por un detalle de
        // presentación que YAML no distingue al leer.
        let raw: BTreeMap<String, Endpoint> = BTreeMap::deserialize(d)?;

        let mut zero = None;
        let mut one  = None;
        for (k, v) in raw {
            match k.as_str() {
                "0" => zero = Some(v),
                "1" => one  = Some(v),
                other => return Err(serde::de::Error::custom(format!(
                    "endpoint '{other}': la aridad es fija en dos, `0` y `1`"))),
            }
        }
        Ok(Endpoints {
            zero: zero.ok_or_else(|| serde::de::Error::custom("falta el endpoint `0`"))?,
            one:  one.ok_or_else(|| serde::de::Error::custom("falta el endpoint `1`"))?,
        })
    }
}

impl JsonSchema for Endpoints {
    fn schema_name() -> std::borrow::Cow<'static, str> { "Endpoints".into() }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let ep = g.subschema_for::<Endpoint>();
        schemars::json_schema!({
            "type": "object",
            "description": "Los dos endpoints del bilink. La aridad es fija.",
            "properties": { "0": ep, "1": ep },
            "required": ["0", "1"],
            "additionalProperties": false,
        })
    }
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

/// Lo que alguien aprobó: una ubicación y un contenido, y quiénes lo aprobaron.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Accepted {
    /// Quiénes aprobaron **exactamente estos valores**.
    ///
    /// Va primero y en bloque, un nombre por línea, porque **`git blame` sólo puede
    /// atribuir una línea a un commit**: en flow, N endosos colapsan en un lugar y
    /// blame devuelve el del último. Un nombre por línea deja cada uno atribuible
    /// —autor, fecha y firma— y por eso el campo no guarda el commit de nadie.
    ///
    /// Un `BTreeSet` y no un `Vec` porque es un set: el orden alfabético es
    /// canónico —el cronológico dependería del orden de un merge, que no es un
    /// hecho sobre nada— y la unicidad no es algo que haya que recordar mantener.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub agree: BTreeSet<String>,
    /// La ubicación aprobada. Ausente en un endpoint `issue`, que no tiene capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkEndpoint>,
    pub hash: String,
    /// Sólo donde hay gramática tree-sitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_ast: Option<String>,
}

impl Accepted {
    /// Si dos `accepted` aprueban lo mismo, **sin mirar quiénes**.
    ///
    /// Es la comparación que importa en todos lados menos en la serialización:
    /// `agree` no participa de ningún estado ni de ningún hash, y dos personas que
    /// aprobaron el mismo contenido aprobaron el mismo contenido.
    pub fn same_values(&self, other: &Self) -> bool {
        self.link == other.link
            && self.hash == other.hash
            && self.hash_ast == other.hash_ast
    }
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
            agree: ["ana".to_string(), "pablo".to_string()].into(),
            link: Some(ep("capture abc123")),
            hash: "c00e0760".into(),
            hash_ast: Some("1b9e44a2".into()),
        });
        let p = BiLink::path_in(dir.path(), "7f3d8e9a");
        bl.write(&p).unwrap();
        assert_eq!(BiLink::load(&p).unwrap(), bl);
    }

    /// Las claves van **sin comillas**: son enteras, no strings.
    ///
    /// `#[serde(rename = "0")]` las habría escrito como `'0':`, que YAML lee igual
    /// pero se ve como un detalle de implementación filtrado al archivo.
    #[test]
    fn the_endpoint_keys_are_written_unquoted() {
        let y = BiLink::new(ep("capture a"), ep("path >impl")).to_yaml().unwrap();
        assert!(y.contains("\n  0:\n"), "el endpoint 0 va sin comillas:\n{y}");
        assert!(y.contains("\n  1:\n"), "el endpoint 1 va sin comillas:\n{y}");
        assert!(!y.contains("'0'") && !y.contains("\"0\""), "no debería haber comillas:\n{y}");
    }

    /// Y se leen de las dos formas: al leer, YAML no distingue.
    #[test]
    fn quoted_keys_still_parse() {
        let quoted = "endpoint:\n  '0': {link: capture a}\n  '1': {link: capture b}\n";
        let bl: BiLink = serde_yaml_ng::from_str(quoted).unwrap();
        assert_eq!(bl.endpoint.zero.link, ep("capture a"));
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
