use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use anyhow::{bail, Context};
use stratum::StratumPath;

/// Estado de aceptación de un endpoint de bilink: ¿lo que hay coincide con `hash.N`?
///
/// Los estados de *resolución* —¿dónde está el fragmento?— viven en el capture
/// (ver `capture::CaptureState`). `Unresolved` es el puente: el capture no
/// resolvió, así que este endpoint no puede evaluarse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum EndpointState {
    #[serde(rename = "PENDING")]
    Pending,
    #[serde(rename = "OK")]
    Ok,
    #[serde(rename = "TODO")]
    Todo,
    #[serde(rename = "DISPLACED")]
    Displaced,
    #[serde(rename = "EXPANDED")]
    Expanded,
    #[serde(rename = "ALTERED")]
    Altered,
    #[serde(rename = "RESTYLED")]
    Restyled,
    #[serde(rename = "UNRESOLVED")]
    Unresolved,
    #[serde(rename = "CHAIN_DIRTY")]
    ChainDirty,
    // — solo para endpoints layer y bilink —
    #[serde(rename = "BROKEN")]
    Broken,
    // — legacy: producidos por el formato anterior, se migran —
    #[serde(rename = "MOVED")]
    Moved,
    #[serde(rename = "REANCHORED")]
    Reanchored,
    #[serde(rename = "UNANCHORED")]
    Unanchored,
    #[serde(rename = "DELETED")]
    Deleted,
}

impl fmt::Display for EndpointState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending     => write!(f, "PENDING"),
            Self::Ok          => write!(f, "OK"),
            Self::Todo        => write!(f, "TODO"),
            Self::Moved       => write!(f, "MOVED"),
            Self::Displaced   => write!(f, "DISPLACED"),
            Self::Reanchored  => write!(f, "REANCHORED"),
            Self::Expanded    => write!(f, "EXPANDED"),
            Self::Unanchored  => write!(f, "UNANCHORED"),
            Self::Altered     => write!(f, "ALTERED"),
            Self::Restyled    => write!(f, "RESTYLED"),
            Self::Deleted     => write!(f, "DELETED"),
            Self::Broken      => write!(f, "BROKEN"),
            Self::Unresolved  => write!(f, "UNRESOLVED"),
            Self::ChainDirty  => write!(f, "CHAIN_DIRTY"),
        }
    }
}

impl FromStr for EndpointState {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "PENDING"      => Ok(Self::Pending),
            "OK"           => Ok(Self::Ok),
            "TODO"         => Ok(Self::Todo),
            "MOVED"        => Ok(Self::Moved),
            "DISPLACED"    => Ok(Self::Displaced),
            "REANCHORED"   => Ok(Self::Reanchored),
            "EXPANDED"     => Ok(Self::Expanded),
            "UNANCHORED"   => Ok(Self::Unanchored),
            "ALTERED"      => Ok(Self::Altered),
            "RESTYLED"     => Ok(Self::Restyled),
            "DELETED"      => Ok(Self::Deleted),
            "BROKEN"       => Ok(Self::Broken),
            "UNRESOLVED"   => Ok(Self::Unresolved),
            "CHAIN_DIRTY"  => Ok(Self::ChainDirty),
            other          => bail!("estado desconocido: '{other}'"),
        }
    }
}

/// Returns the state as a string, or "NONE" if no state has been recorded yet.
pub fn state_str(state: &Option<EndpointState>) -> String {
    state.as_ref().map_or_else(|| "NONE".to_string(), |s| s.to_string())
}

/// A parsed bilink endpoint.
///
/// Un endpoint estructural no describe el fragmento: referencia un capture de la
/// misma capa, que es quien guarda `file`, `query` y `offset`.
///
/// Disambiguation: `capture <uuid>` y `issue <id>` se reconocen por prefijo. El
/// resto se interpreta como path Stratum, salvo que tenga `::` o extensión de
/// archivo — en cuyo caso es formato anterior al split y se parsea como
/// `LegacyStructural` para que `bilinker migrate` pueda convertirlo.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEndpoint {
    /// `capture <uuid>` — referencia a un `.capture` de esta misma capa.
    Capture(String),
    Layer(StratumPath),
    /// `issue <id>` — un ítem del worklist, en `<project-root>/.stratum/worklist/<id>.<tipo>.md`.
    ///
    /// Se llama issue y no task porque apunta a cualquier tipo de ítem —épica, user
    /// story o task—, y `task` es además el nombre del tipo hoja del worklist.
    Issue(String),
    /// Formato anterior al split capture/bilink. Solo lo produce el parser al leer
    /// archivos sin migrar; `bilinker migrate` lo convierte en `Capture`.
    LegacyStructural(StructuralRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StructuralRef {
    pub file: String,
    pub query: Option<String>,
    pub range: Option<ByteRange>,
}

/// Un bilink conecta exactamente dos endpoints. La aridad es fija: la
/// multiplicidad la aporta el capture, que puede tener N bilinks asociados.
#[derive(Debug, Clone)]
pub struct BiLink {
    pub id: String,
    pub link0: LinkEndpoint,
    pub link1: LinkEndpoint,
    pub hash0: Option<String>,
    pub hash1: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl LinkEndpoint {
    /// Apunta a un fragmento de archivo: `Capture`, o el legacy sin migrar.
    pub fn is_structural(&self) -> bool {
        matches!(self, Self::Capture(_) | Self::LegacyStructural(_))
    }

    /// UUID del capture referenciado, si el endpoint es `Capture`.
    pub fn capture_uuid(&self) -> Option<&str> {
        match self {
            Self::Capture(uuid) => Some(uuid),
            _ => None,
        }
    }

    /// Referencia estructural embebida, solo en archivos sin migrar.
    pub fn legacy_sref(&self) -> Option<&StructuralRef> {
        match self {
            Self::LegacyStructural(r) => Some(r),
            _ => None,
        }
    }
}

/// Los prefijos que el parser reconoce, y qué endpoint construye cada uno.
///
/// **Es la única lista.** El parser la recorre y el esquema JSON la publica, así que
/// agregar un tipo de endpoint —`repo <alias>`, `abstract`— obliga a tocarla, eso
/// cambia el esquema, y el guard de versión lo detecta. Sin esto un tipo nuevo sería
/// **aditivo y silencioso**: un parser viejo leería `abstract` como un path de capa
/// sin fallar, que es el caso que ADR-0006 pone como motivo del guard.
pub const ENDPOINT_PREFIXES: &[(&str, fn(String) -> LinkEndpoint)] = &[
    ("capture", LinkEndpoint::Capture),
    ("issue",   LinkEndpoint::Issue),
];

impl FromStr for LinkEndpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();

        for (prefix, build) in ENDPOINT_PREFIXES {
            let Some(rest) = trimmed.strip_prefix(*prefix).and_then(|r| r.strip_prefix(' ')) else {
                continue;
            };
            let rest = rest.trim();
            if !rest.is_empty() {
                return Ok(build(rest.to_string()));
            }
        }

        if trimmed.contains("::") {
            return Ok(LinkEndpoint::LegacyStructural(trimmed.parse()?));
        }
        // No `::`: check if the last path component has a file extension.
        let last = std::path::Path::new(trimmed)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let looks_like_file = last.contains('.') && last != "." && last != "..";
        if looks_like_file {
            return Ok(LinkEndpoint::LegacyStructural(StructuralRef {
                file:  trimmed.to_string(),
                query: None,
                range: None,
            }));
        }
        let tokens = stratum::parse_path(trimmed)
            .map_err(|e| anyhow::anyhow!("invalid stratum path '{s}': {e}"))?;
        Ok(LinkEndpoint::Layer(tokens))
    }
}

impl FromStr for StructuralRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(3, "::").map(str::trim).collect();
        match parts.as_slice() {
            [file] => Ok(Self {
                file: file.to_string(),
                query: None,
                range: None,
            }),
            [file, query] => Ok(Self {
                file: file.to_string(),
                query: Some(query.to_string()),
                range: None,
            }),
            [file, query, range] => Ok(Self {
                file: file.to_string(),
                query: Some(query.to_string()),
                range: Some(range.parse().context("invalid start~end range")?),
            }),
            _ => bail!("expected `file [:: query [:: start~end]]`, got: {s}"),
        }
    }
}

impl fmt::Display for LinkEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkEndpoint::Capture(uuid) => write!(f, "capture {uuid}"),
            LinkEndpoint::Layer(tokens) => {
                write!(f, "{}", stratum::format_path(tokens))
            }
            LinkEndpoint::Issue(id) => write!(f, "issue {id}"),
            LinkEndpoint::LegacyStructural(r) => write!(f, "{r}"),
        }
    }
}

impl fmt::Display for StructuralRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(q) = &self.query {
            write!(f, " :: {q}")?;
            if let Some(r) = &self.range {
                write!(f, " :: {r}")?;
            }
        }
        Ok(())
    }
}

impl FromStr for ByteRange {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (start, end) = s.split_once('~').context("range must be `start~end`")?;
        Ok(Self {
            start: start.trim().parse().context("invalid start offset")?,
            end: end.trim().parse().context("invalid end offset")?,
        })
    }
}

impl fmt::Display for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}~{}", self.start, self.end)
    }
}

// ─── serialización de los tipos que en disco son un string ────────────────────
//
// `link` y `offset` son strings en el archivo —`capture <uuid>`, `3226~5109`— y
// no objetos. Serializarlos como tal mantiene el modelo y el disco diciendo lo
// mismo, que es lo que hace que el esquema publicado sirva para validar archivos
// ajenos (ADR-0006, decisión 3) y no sólo para describir el tipo de Rust.

/// Serializa como su forma de disco y parsea con `FromStr`.
macro_rules! string_repr {
    ($t:ty, $name:literal, $desc:literal) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(d)?;
                raw.parse().map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $t {
            fn schema_name() -> Cow<'static, str> { $name.into() }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "description": $desc })
            }
        }
    };
}

impl Serialize for LinkEndpoint {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LinkEndpoint {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for LinkEndpoint {
    fn schema_name() -> Cow<'static, str> { "LinkEndpoint".into() }

    /// Publica los prefijos reconocidos, no sólo "es un string".
    ///
    /// Es lo que hace que agregar un tipo de endpoint cambie el esquema. Con un
    /// `{"type": "string"}` a secas, un tipo nuevo pasaría el guard sin que el hash
    /// se moviera — que es justo el agujero que el guard existe para tapar.
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let prefixes: Vec<&str> = ENDPOINT_PREFIXES.iter().map(|(p, _)| *p).collect();
        json_schema!({
            "type": "string",
            "description": "Un extremo del bilink. Con prefijo cuando el tipo es \
                            explícito; si no, un path Stratum hacia la capa vecina.",
            "prefixes": prefixes,
        })
    }
}
string_repr!(
    ByteRange, "ByteRange",
    "Rango de bytes `start~end`. Relativo al nodo cuando es `offset`; absoluto en el archivo cuando es `range`."
);

#[cfg(test)]
mod repr_tests {
    use super::*;

    /// Un endpoint viaja como el string que está en el archivo, no como un objeto.
    #[test]
    fn an_endpoint_serializes_to_its_on_disk_form() {
        let ep = LinkEndpoint::Issue("3a".into());
        assert_eq!(serde_json::to_string(&ep).unwrap(), "\"issue 3a\"");
        let back: LinkEndpoint = serde_json::from_str("\"issue 3a\"").unwrap();
        assert_eq!(back, ep);
    }

    #[test]
    fn a_range_serializes_to_start_tilde_end() {
        let r = ByteRange { start: 3226, end: 5109 };
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"3226~5109\"");
        let back: ByteRange = serde_json::from_str("\"3226~5109\"").unwrap();
        assert_eq!(back, r);
    }

    /// Todo prefijo de la tabla lo reconoce el parser, y el parser no reconoce otros.
    ///
    /// Ata las dos puntas: la tabla es lo que el esquema publica, así que si pudiera
    /// desincronizarse del parser el esquema mentiría sin que nada fallara.
    #[test]
    fn every_declared_prefix_is_parsed() {
        for (prefix, _) in ENDPOINT_PREFIXES {
            let ep: LinkEndpoint = format!("{prefix} abc").parse().unwrap();
            assert_eq!(ep.to_string(), format!("{prefix} abc"),
                "el prefijo `{prefix}` está en la tabla pero no round-trippea");
        }
    }

    /// Un prefijo que no está en la tabla no produce un endpoint con prefijo.
    #[test]
    fn an_undeclared_prefix_is_not_prefixed() {
        for word in ["repo", "abstract", "bilink", "task"] {
            let ep: LinkEndpoint = format!("{word} abc").parse().unwrap();
            assert!(!matches!(ep, LinkEndpoint::Capture(_) | LinkEndpoint::Issue(_)),
                "`{word}` no está declarado y no debería parsear como endpoint con prefijo");
        }
    }

    /// Los estados van con el nombre que tienen en el archivo, no con el de Rust.
    #[test]
    fn a_state_serializes_with_its_on_disk_name() {
        assert_eq!(serde_json::to_string(&EndpointState::ChainDirty).unwrap(), "\"CHAIN_DIRTY\"");
        assert_eq!(serde_json::to_string(&EndpointState::Ok).unwrap(), "\"OK\"");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_structural_without_range() {
        let ep: LinkEndpoint = "Persona.java :: (class_declaration name:#eq?Persona)".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::LegacyStructural(_)));
    }

    #[test]
    fn parse_structural_with_range() {
        let ep: LinkEndpoint = "docs/architecture.md :: (paragraph) @target :: 42~87".parse().unwrap();
        if let LinkEndpoint::LegacyStructural(r) = ep {
            assert_eq!(r.range, Some(ByteRange { start: 42, end: 87 }));
        } else {
            panic!("expected LegacyStructural");
        }
    }

    #[test]
    fn parse_layer_simple_path() {
        let ep: LinkEndpoint = "persona-voting-impl".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Layer(_)));
    }

    #[test]
    fn parse_layer_stratum_down() {
        let ep: LinkEndpoint = ">tech-decisions>impl".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Layer(_)));
    }

    #[test]
    fn roundtrip_structural() {
        let s = "docs/architecture.md :: (paragraph) @target :: 42~87";
        let ep: LinkEndpoint = s.parse().unwrap();
        assert_eq!(ep.to_string(), s);
    }

    #[test]
    fn parse_whole_file_endpoint() {
        let ep: LinkEndpoint = "docs/architecture.md".parse().unwrap();
        if let LinkEndpoint::LegacyStructural(r) = ep {
            assert_eq!(r.file, "docs/architecture.md");
            assert!(r.query.is_none());
            assert!(r.range.is_none());
        } else {
            panic!("expected LegacyStructural");
        }
    }

    #[test]
    fn roundtrip_whole_file() {
        let s = "docs/architecture.md";
        let ep: LinkEndpoint = s.parse().unwrap();
        assert_eq!(ep.to_string(), s);
    }

    #[test]
    fn parse_capture_endpoint() {
        let ep: LinkEndpoint = "capture 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a".parse().unwrap();
        assert_eq!(ep, LinkEndpoint::Capture("7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a".into()));
        assert_eq!(ep.to_string(), "capture 7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a");
    }

    #[test]
    fn capture_prefix_without_id_is_not_a_capture() {
        // "capture" solo, sin id, no es un endpoint capture — cae a path Stratum
        let ep: LinkEndpoint = "capture".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Layer(_)));
    }

    #[test]
    fn parse_issue_endpoint() {
        let ep: LinkEndpoint = "issue 3a".parse().unwrap();
        assert_eq!(ep, LinkEndpoint::Issue("3a".into()));
        assert_eq!(ep.to_string(), "issue 3a");
    }

    #[test]
    fn parse_issue_endpoint_longer_id() {
        let ep: LinkEndpoint = "issue 1f".parse().unwrap();
        assert_eq!(ep, LinkEndpoint::Issue("1f".into()));
    }

    /// `task` fue el nombre anterior del prefijo y no se reconoce más.
    ///
    /// Cae a path Stratum como cualquier valor sin prefijo conocido: no hay
    /// compatibilidad hacia atrás porque nunca se escribió un `task <id>` en disco.
    #[test]
    fn the_old_task_prefix_is_not_an_issue() {
        let ep: LinkEndpoint = "task 3a".parse().unwrap();
        assert!(!matches!(ep, LinkEndpoint::Issue(_)));
    }

    #[test]
    fn todo_state_roundtrip() {
        let s = "TODO";
        let state: EndpointState = s.parse().unwrap();
        assert_eq!(state, EndpointState::Todo);
        assert_eq!(state.to_string(), "TODO");
    }
}
