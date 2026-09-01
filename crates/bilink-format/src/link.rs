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

/// El separador entre los fragmentos de un capture con más de un `@target`.
///
/// Los rangos no son contiguos, así que hay que decidir qué va entre uno y el
/// siguiente — y esa decisión **entra en el `hash`**. Pegarlos sin nada produce un
/// texto que no existe en ningún archivo; meter el texto intermedio deja entrar el
/// cuerpo del método por la ventana cuando dos capturas lo tienen en el medio.
///
/// **Y no se vuelve a tocar.** Cambiarlo movería de una vez el hash de todos los
/// captures multi-fragmento, y todos pasarían a ALTERED sin que nadie tocara el
/// código. Ver `concepts/capture.md` § "El separador es `\n`".
pub const FRAGMENT_SEPARATOR: &str = "\n";

/// Los rangos de un fragmento: uno por captura `@target`, en orden de archivo.
///
/// El fragmento es su concatenación unida por [`FRAGMENT_SEPARATOR`], y es eso lo
/// que se hashea. Con un solo rango la concatenación es el rango, así que un
/// capture de un solo `@target` hashea exactamente lo que hasheaba antes de que
/// hubiera varios.
///
/// Nunca está vacío: un fragmento sin rangos no es un fragmento, es que la query no
/// resolvió — y eso se dice con `None`, no con una lista de cero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranges(Vec<ByteRange>);

impl Ranges {
    /// El caso de siempre: un solo nodo.
    pub fn one(start: usize, end: usize) -> Self {
        Self(vec![ByteRange { start, end }])
    }

    /// Los rangos ya ordenados por posición en el archivo. Devuelve `None` si la
    /// lista viene vacía, que es lo que hace imposible representar un fragmento sin
    /// partes.
    pub fn new(mut parts: Vec<ByteRange>) -> Option<Self> {
        if parts.is_empty() { return None; }
        parts.sort_by_key(|r| (r.start, r.end));
        Some(Self(parts))
    }

    pub fn parts(&self) -> &[ByteRange] { &self.0 }

    /// Dónde empieza la primera parte.
    pub fn start(&self) -> usize { self.0.first().expect("nunca vacío").start }

    /// Dónde termina la última. **No es el final del fragmento** cuando hay varias
    /// partes: entre medio hay texto que el fragmento no cubre.
    pub fn end(&self) -> usize { self.0.last().expect("nunca vacío").end }

    /// El fragmento: cada parte, unidas por [`FRAGMENT_SEPARATOR`].
    pub fn text(&self, source: &str) -> String {
        self.0.iter()
            .map(|r| &source[r.start.min(source.len())..r.end.min(source.len())])
            .collect::<Vec<_>>()
            .join(FRAGMENT_SEPARATOR)
    }
}

impl FromStr for Ranges {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let parts = s.split(',')
            .map(|p| p.trim().parse::<ByteRange>())
            .collect::<Result<Vec<_>>>()?;
        Self::new(parts).context("un fragmento tiene al menos un rango")
    }
}

impl fmt::Display for Ranges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, r) in self.0.iter().enumerate() {
            if i > 0 { f.write_str(",")?; }
            write!(f, "{r}")?;
        }
        Ok(())
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

string_repr!(Ranges, "Ranges", json_schema!({
    "type": "string",
    "pattern": r"^\d+~\d+(,\d+~\d+)*$",
    "description": "Los rangos del fragmento, `start~end` separados por coma, en orden de archivo.",
}));

#[cfg(test)]
mod ranges_tests {
    use super::*;

    /// El orden es el del archivo, no el de la query: cómo se escribió el patrón
    /// es un detalle del patrón, y el fragmento —que es lo que se hashea— no puede
    /// depender de él.
    #[test]
    fn parts_come_out_in_file_order() {
        let r = Ranges::new(vec![
            ByteRange { start: 30, end: 40 },
            ByteRange { start: 10, end: 20 },
        ]).unwrap();
        assert_eq!(r.to_string(), "10~20,30~40");
    }

    /// Un fragmento sin partes no se puede representar: eso no es un fragmento
    /// vacío, es que la query no resolvió, y eso se dice con `None`.
    #[test]
    fn a_fragment_without_parts_does_not_exist() {
        assert!(Ranges::new(vec![]).is_none());
        assert!("".parse::<Ranges>().is_err());
    }

    /// El texto es la concatenación unida por el separador, y con una sola parte
    /// es la parte: por eso un capture de un `@target` hashea lo mismo que antes.
    #[test]
    fn the_text_is_the_parts_joined_by_the_separator() {
        let src = "0123456789abcdefghij";
        let r = Ranges::new(vec![
            ByteRange { start: 0,  end: 3 },
            ByteRange { start: 10, end: 13 },
        ]).unwrap();
        assert_eq!(r.text(src), format!("012{FRAGMENT_SEPARATOR}abc"));
        assert_eq!(Ranges::one(0, 3).text(src), "012");
    }

    #[test]
    fn round_trips_through_its_string_form() {
        let r: Ranges = "10~20,30~40".parse().unwrap();
        assert_eq!(r.parts().len(), 2);
        assert_eq!(r.start(), 10);
        assert_eq!(r.end(), 40);
        assert_eq!(r.to_string(), "10~20,30~40");
        assert_eq!("3226~5109".parse::<Ranges>().unwrap(), Ranges::one(3226, 5109));
    }
}

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
