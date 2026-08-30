//! Los tipos de endpoint y su discriminación por prefijo.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Un extremo del bilink. **El tipo va adelante, en un prefijo.**
///
/// El prefijo no nombra el destino sino en qué lenguaje está el resto, que es lo
/// que el parser necesita saber. Y no hay fallback: un prefijo desconocido es un
/// error, no un path. Sin fallback no hace falta ninguna regla de precedencia.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEndpoint {
    /// `capture <id>` — un capture de esta misma capa, por su id de contenido.
    Capture(String),
    /// `path <stratum-path>` — una capa vecina.
    ///
    /// `path` y no `layer`: un stratum-path también cruza a sub-proyectos, que el
    /// modelo de capas distingue de las capas internas.
    Path(StratumPath),
    /// `issue <id>` — un ítem del worklist, de cualquier tipo.
    Issue(String),
    /// `repo <alias>` — un fragmento de **otro proyecto**, por un alias local.
    ///
    /// Es el endpoint `path` generalizado: misma convención de UUID compartido y
    /// mismo `.bilink/` implícito, pero la dirección se resuelve por un
    /// `.bilink/.{alias}.toml` en vez de por un path relativo. El valor es un
    /// nombre local y **nunca una URL**: toda la identidad del proveedor vive en
    /// ese `.toml`, y no repartida en N bilinks.
    Repo(String),
    /// `abstract` — la punta abierta de un bilink que otro proyecto consume.
    ///
    /// **Es el único endpoint sin valor**, y por eso no necesita ninguna regla de
    /// desempate: ninguna otra forma se le parece. Tampoco lleva `accepted` — no
    /// hay nada que bendecir del lado abierto, y con el bloque entero ausente eso
    /// es una ausencia y no una lista de campos vacíos.
    Abstract,
}

/// Un path Stratum, con su texto original.
///
/// Se guarda el texto porque es lo que va al archivo: reformatearlo cambiaría el
/// archivo sin que nadie lo haya editado.
#[derive(Debug, Clone, PartialEq)]
pub struct StratumPath {
    raw: String,
    tokens: stratum::StratumPath,
}

impl StratumPath {
    pub fn tokens(&self) -> &stratum::StratumPath { &self.tokens }
    pub fn as_str(&self) -> &str { &self.raw }
}

impl FromStr for StratumPath {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let tokens = stratum::parse_path(s)
            .map_err(|e| anyhow::anyhow!("path Stratum inválido '{s}': {e}"))?;
        Ok(Self { raw: s.to_string(), tokens })
    }
}

impl fmt::Display for StratumPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.raw) }
}

/// Los prefijos que el parser reconoce, y qué endpoint construye cada uno.
///
/// **Es la única lista.** El parser la recorre y el esquema JSON la publica, así que
/// agregar un tipo de endpoint obliga a tocarla, eso cambia el esquema, y el guard
/// de versión lo detecta. Sin esto un tipo nuevo sería aditivo y silencioso — que es
/// exactamente el modo de falla que importa entre proyectos: un parser viejo leería
/// `abstract` como un path de capa, sin fallar.
pub const ENDPOINT_PREFIXES: &[&str] = &["capture", "path", "issue", "repo", "abstract"];

/// Los prefijos que **no llevan valor**. Hoy es uno solo.
pub const VALUELESS_PREFIXES: &[&str] = &["abstract"];

impl LinkEndpoint {
    /// Apunta a un fragmento de archivo mediante un capture de esta capa.
    pub fn is_structural(&self) -> bool { matches!(self, Self::Capture(_)) }

    /// Cruza a otro proyecto. El alias se resuelve por `.bilink/.{alias}.toml`.
    pub fn repo_alias(&self) -> Option<&str> {
        match self { Self::Repo(a) => Some(a), _ => None }
    }

    /// La punta abierta. Su estado es `OPEN`, constante, y `accept .` no la toca.
    pub fn is_abstract(&self) -> bool { matches!(self, Self::Abstract) }

    /// El id del capture referenciado, si el endpoint es estructural.
    pub fn capture_id(&self) -> Option<&str> {
        match self { Self::Capture(id) => Some(id), _ => None }
    }

    /// El prefijo con que este endpoint se escribe.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Capture(_) => "capture",
            Self::Path(_)    => "path",
            Self::Issue(_)   => "issue",
            Self::Repo(_)    => "repo",
            Self::Abstract   => "abstract",
        }
    }
}

impl FromStr for LinkEndpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        let (prefix, rest) = trimmed.split_once(' ')
            .map(|(p, r)| (p, r.trim()))
            .unwrap_or((trimmed, ""));

        if rest.is_empty() && !VALUELESS_PREFIXES.contains(&prefix) {
            bail!("endpoint sin valor: '{trimmed}'. Se esperaba `<prefijo> <valor>`, \
                   con el prefijo entre {}", ENDPOINT_PREFIXES.join(", "));
        }
        if !rest.is_empty() && VALUELESS_PREFIXES.contains(&prefix) {
            bail!("`{prefix}` no lleva valor, y trae '{rest}'");
        }

        match prefix {
            "capture"  => Ok(LinkEndpoint::Capture(rest.to_string())),
            "issue"    => Ok(LinkEndpoint::Issue(rest.to_string())),
            "path"     => Ok(LinkEndpoint::Path(rest.parse()?)),
            "repo"     => Ok(LinkEndpoint::Repo(rest.to_string())),
            "abstract" => Ok(LinkEndpoint::Abstract),
            other => bail!(
                "prefijo de endpoint desconocido: '{other}'. Los reconocidos son {}. \
                 Un prefijo desconocido es un error, no un path: no hay fallback.",
                ENDPOINT_PREFIXES.join(", ")
            ),
        }
    }
}

impl fmt::Display for LinkEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capture(id) => write!(f, "capture {id}"),
            Self::Path(p)     => write!(f, "path {p}"),
            Self::Issue(id)   => write!(f, "issue {id}"),
            Self::Repo(alias) => write!(f, "repo {alias}"),
            // Sin valor y sin espacio al final: el archivo dice `link: abstract`.
            Self::Abstract    => f.write_str("abstract"),
        }
    }
}

/// Rango de bytes `start~end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteRange { pub start: usize, pub end: usize }

impl FromStr for ByteRange {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let (start, end) = s.split_once('~').context("el rango es `start~end`")?;
        Ok(Self {
            start: start.trim().parse().context("offset inicial inválido")?,
            end:   end.trim().parse().context("offset final inválido")?,
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
// `link` y `offset` son strings en el archivo —`capture 67ba…`, `3226~5109`— y no
// objetos. Serializarlos como tal mantiene el modelo y el disco diciendo lo mismo,
// que es lo que hace que el esquema publicado sirva para validar archivos ajenos.

macro_rules! string_repr {
    ($t:ty, $name:literal, $body:expr) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
            }
        }
        impl JsonSchema for $t {
            fn schema_name() -> Cow<'static, str> { $name.into() }
            fn json_schema(_: &mut SchemaGenerator) -> Schema { $body }
        }
    };
}

string_repr!(LinkEndpoint, "LinkEndpoint", json_schema!({
    "type": "string",
    "description": "Un extremo del bilink, con el tipo en el prefijo.",
    "prefixes": ENDPOINT_PREFIXES,
}));

string_repr!(ByteRange, "ByteRange", json_schema!({
    "type": "string",
    "pattern": r"^\d+~\d+$",
    "description": "Rango de bytes `start~end`, relativo al nodo matcheado.",
}));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_prefix_parses() {
        for prefix in ENDPOINT_PREFIXES {
            let raw = if VALUELESS_PREFIXES.contains(prefix) {
                prefix.to_string()
            } else {
                format!("{prefix} abc")
            };
            let ep: LinkEndpoint = raw.parse().unwrap();
            assert_eq!(ep.to_string(), raw, "`{prefix}` no round-trippea");
            assert_eq!(ep.prefix(), *prefix);
        }
    }

    /// `abstract` es el único sin valor, y por eso no necesita desempate.
    #[test]
    fn abstract_takes_no_value() {
        assert_eq!("abstract".parse::<LinkEndpoint>().unwrap(), LinkEndpoint::Abstract);
        assert!("abstract hsi".parse::<LinkEndpoint>().is_err(),
                "`abstract` no lleva valor");
    }

    /// El endpoint repo guarda un alias local, nunca una URL.
    #[test]
    fn repo_holds_a_local_alias() {
        let ep: LinkEndpoint = "repo hsi".parse().unwrap();
        assert_eq!(ep.repo_alias(), Some("hsi"));
        assert!(!ep.is_structural(), "no apunta a un capture de esta capa");
    }

    /// Un prefijo desconocido falla. **No hay fallback a path.**
    #[test]
    fn an_unknown_prefix_is_an_error() {
        for raw in ["bilink 7f3d", "subsystems/bilinker>impl", "docs/spec.md"] {
            assert!(raw.parse::<LinkEndpoint>().is_err(),
                "'{raw}' debería fallar: no hay fallback a path");
        }
    }

    /// El endpoint layer del formato 1 —un path pelado— ya no parsea.
    #[test]
    fn a_bare_stratum_path_is_no_longer_a_path_endpoint() {
        assert!(">impl".parse::<LinkEndpoint>().is_err());
        assert_eq!("path >impl".parse::<LinkEndpoint>().unwrap().prefix(), "path");
    }

    #[test]
    fn a_range_round_trips() {
        let r: ByteRange = "3226~5109".parse().unwrap();
        assert_eq!(r, ByteRange { start: 3226, end: 5109 });
        assert_eq!(r.to_string(), "3226~5109");
    }
}
