//! El archivo `<uuid>.yaml`: una declaración y dos decisiones.

use std::collections::{BTreeMap, BTreeSet};
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
    /// El vecindario **declarado**: a qué captures apunta, por nivel.
    ///
    /// Es al `link` del fragmento lo que `accepted.n` es a `accepted.link`. Lo
    /// mantiene `apply`, que para eso recibe el proveedor: un vecino que se mudó es
    /// un `MOVED` que git resuelve, pero el conjunto también gana y pierde miembros
    /// cuando la firma cambia, y qué tipo entró sólo lo sabe un language server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<DeclaredN>,
    /// Las decisiones sobre este endpoint. **Una lista, y más de una es un estado.**
    ///
    /// La lista vacía es `PENDING`. Con exactamente una, valen las comparaciones de
    /// siempre. Con dos o más el estado es `CONSENSUS_DIVERGED` y no se evalúa
    /// ningún otro eje: no hay un valor contra el cual compararlos.
    ///
    /// **No es una estructura para sostener dos verdades**: es una forma de no perder
    /// ninguna mientras alguien resuelve. Antes la segunda aceptación pisaba a la
    /// primera en silencio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted: Vec<Accepted>,
    /// Etiqueta del rol de este extremo. Inerte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Con qué generador se capturó este extremo: el nombre que tomó `--as.N`.
    /// Inerte.
    ///
    /// **Es la receta, no el valor.** Lo que el generador sabe componer —la ruta de
    /// un endpoint, su verbo— sale del fragmento cada vez que se lee, que es lo que
    /// no puede mentir. Guardar el valor lo dejaría envejecer en silencio, porque
    /// este campo no entra en ningún hash; guardar con qué componerlo no envejece.
    ///
    /// **Se guarda y no se deduce al leer.** Un generador sabe decir si tiene algo
    /// que decir sobre un nodo, y eso sirve para sugerir y nunca para elegir: si al
    /// escribir se rechaza adivinar cuál es, deducirlo al leer es adivinar lo mismo
    /// sin nadie mirando la sugerencia.
    ///
    /// Que nombre un generador que no está instalado **no es un error**: es un dato
    /// que no se pudo usar. El capture resuelve igual y `check` contesta igual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#as: Option<String>,
}


/// Los captures de un vecindario: `capture <id> <id> …`
///
/// **Lleva el prefijo y va en una línea**, con la misma forma que un
/// [`LinkEndpoint`](crate::LinkEndpoint) de captura. Un vecino sólo puede ser un
/// capture, así que el prefijo no discrimina nada — lo que hace es que quien lee el
/// archivo no tenga que aprender dos formas para la misma cosa.
///
/// **Ordenado por id**, que es la clave del fold. El id es `sha256(file \0 query \0)`:
/// no lleva contenido, así que un reformateo no lo mueve, y cambia exactamente cuando
/// un vecino entra, sale, se muda o se renombra.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptureSet(Vec<String>);

impl CaptureSet {
    /// Los ids, ya ordenados.
    pub fn new(mut ids: Vec<String>) -> Self {
        ids.sort();
        ids.dedup();
        Self(ids)
    }
    pub fn ids(&self) -> &[String] { &self.0 }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn len(&self) -> usize { self.0.len() }
}

impl std::fmt::Display for CaptureSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "capture")?;
        for id in &self.0 { write!(f, " {id}")?; }
        Ok(())
    }
}

impl std::str::FromStr for CaptureSet {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let resto = s.strip_prefix("capture").ok_or_else(|| anyhow::anyhow!(
            "un vecindario se escribe `capture <id>…`, y esto empieza con {:?}",
            s.split_whitespace().next().unwrap_or("")))?;
        // **Un prefijo sin ids es un error, no un conjunto vacío.** Vacío se escribe
        // omitiendo el campo: `capture` solo no dice nada.
        let ids: Vec<String> = resto.split_whitespace().map(str::to_string).collect();
        if ids.is_empty() { anyhow::bail!("`capture` sin ningún id: un vecindario vacío se omite"); }
        Ok(Self::new(ids))
    }
}

impl Serialize for CaptureSet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CaptureSet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for CaptureSet {
    fn schema_name() -> std::borrow::Cow<'static, str> { "CaptureSet".into() }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Los captures de un vecindario: `capture <id> <id> …`",
            "pattern": "^capture( [0-9a-f]{32})+$",
        })
    }
}

/// El literal con que se escribe una ubicación que no se sabe.
pub const LEVEL_LINK_UNKNOWN: &str = "unknown";

/// La ubicación de **un nivel** del vecindario, con sus tres formas.
///
/// | | |
/// |---|---|
/// | *(ausente)* | se miró y no hay vecinos — una firma cuyos tipos son todos de otra capa |
/// | `capture <id> <id> …` | éstos son los vecinos, ordenados por id |
/// | `unknown` | el contrato está y de qué vecinos salió no se sabe |
///
/// **`unknown` va en este slot y no en un campo hermano.** Un `unknown: true` al lado
/// de `link` deja escribible `link: capture <id>` con la contradicción encima — la
/// misma familia de combinación inválida por la que [`N`] es un campo con tres estados
/// y no tres campos sueltos. Y no va en [`N`]: ahí sería un estado del vecindario
/// entero, y escribirlo tiraría los hashes, que son la parte que sí se tiene.
///
/// Ver `concepts/bilink.md` § "El `link` de un nivel del vecindario".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelLink {
    /// `capture <id> …` — éstos son los vecinos. El conjunto vacío es la ausencia del
    /// campo, y se omite al serializar.
    Captures(CaptureSet),
    /// `unknown` — el nivel está adquirido y su ubicación no se sabe.
    ///
    /// **Es incomparable**: no hay ids de ninguno de los dos lados, así que dos
    /// `unknown` no coinciden — no poder comparar no es que coincida.
    Unknown,
}

impl Default for LevelLink {
    /// La ausencia, que es el conjunto vacío. **Nunca `unknown`**: eso es un dato que
    /// alguien escribió, no lo que se asume cuando no hay nada escrito.
    fn default() -> Self { Self::Captures(CaptureSet::default()) }
}

impl LevelLink {
    /// El conjunto, **sólo cuando la ubicación se sabe**. `None` es `unknown`.
    pub fn captures(&self) -> Option<&CaptureSet> {
        match self { Self::Captures(c) => Some(c), Self::Unknown => None }
    }

    /// Los ids, **sólo cuando la ubicación se sabe**. `None` es `unknown`, y no es un
    /// conjunto vacío: quien pregunta tiene que decidir qué hace con eso, que es lo
    /// que impide leer *"no sé dónde"* como *"no hay vecinos"*.
    pub fn known_ids(&self) -> Option<&[String]> { self.captures().map(CaptureSet::ids) }

    pub fn is_unknown(&self) -> bool { matches!(self, Self::Unknown) }

    /// La ausencia del campo: el conjunto vacío. **`unknown` no es ausencia.**
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Captures(c) if c.is_empty())
    }
}

impl From<CaptureSet> for LevelLink {
    fn from(c: CaptureSet) -> Self { Self::Captures(c) }
}

impl std::fmt::Display for LevelLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Captures(c) => c.fmt(f),
            Self::Unknown     => f.write_str(LEVEL_LINK_UNKNOWN),
        }
    }
}

impl std::str::FromStr for LevelLink {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        if s.trim() == LEVEL_LINK_UNKNOWN { return Ok(Self::Unknown); }
        s.parse::<CaptureSet>().map(Self::Captures).with_context(|| format!(
            "la ubicación de un nivel se escribe `capture <id>…` o `{LEVEL_LINK_UNKNOWN}`"))
    }
}

impl Serialize for LevelLink {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LevelLink {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for LevelLink {
    fn schema_name() -> std::borrow::Cow<'static, str> { "LevelLink".into() }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "La ubicación de un nivel del vecindario: los captures de \
                            sus vecinos, o `unknown` cuando el contrato está y su \
                            ubicación no se sabe.",
            "pattern": "^(capture( [0-9a-f]{32})+|unknown)$",
        })
    }
}

/// El vecindario **declarado**: qué captures lo componen, por nivel.
///
/// **No lleva hashes ni `declined`**, y las dos ausencias son la misma razón: un hash
/// es una decisión y renunciar también. Acá va sólo lo que `apply` mantiene — a qué
/// apunta el vecindario, igual que `link` para el fragmento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct DeclaredN(pub BTreeMap<u8, DeclaredLevel>);

impl DeclaredN {
    pub fn of_level_1(link: impl Into<LevelLink>) -> Self {
        Self(BTreeMap::from([(1u8, DeclaredLevel { link: link.into() })]))
    }
    pub fn level(&self, k: u8) -> Option<&DeclaredLevel> { self.0.get(&k) }
}

/// Un nivel del vecindario declarado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredLevel {
    pub link: LevelLink,
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
    /// El **vecindario de nivel 1**: los tipos que la firma menciona, un salto.
    ///
    /// Es un valor que bilinker guarda y compara **sin poder calcularlo por su
    /// cuenta** — resolver un tipo hasta su declaración es trabajo de language
    /// server. Se compara, no se resuelve: el mismo patrón que un `accepted.link`
    /// de endpoint layer, que lleva una copia opaca de un id ajeno.
    ///
    /// **Un campo con tres estados**, y los niveles adentro. Ver `N`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<N>,
}

/// El vecindario, con sus tres estados — y el tercero es que este campo no esté.
///
/// Escrito como campos sueltos —`hash_n1`, `hash_ast_n1` y una marca aparte—
/// quedaban representables combinaciones que no significan nada: un fold de ASTs sin
/// el fold de textos que lo acompaña, y una renuncia conviviendo con el valor al que
/// se renunció. **Que ningún código las produzca no es lo mismo que que no se puedan
/// escribir**, y el YAML lo escribe cualquiera a mano.
///
/// **El nivel 0 —el fragmento— no entra.** Se hashea igual que un vecino y la
/// escalera queda tentadora, pero es obligatorio, no se puede renunciar, y sale de
/// tree-sitter y no de un language server. Adentro del mapa, `n: {}` sería una
/// aceptación sin contenido aprobado. Ver `concepts/accept.md` § "El nivel 0 no
/// entra".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum N {
    /// `n: declined` — alguien aceptó sin vecindario, a propósito.
    ///
    /// **Va en el contenedor y no adentro de un nivel**, porque la renuncia es una
    /// sola y es del 1 para arriba: el nivel 2 son los campos de los tipos que el 1
    /// resuelve, así que renunciar al 1 deja al 2 sin base. Escrita adentro del
    /// nivel 1 decía *"el nivel 1 fue renunciado"* cuando quiere decir *"el
    /// vecindario fue renunciado"* — y obligaba a preguntarse qué pasa con el 2.
    Declined(DeclinedMark),
    /// `n: {1: {…}, 2: {…}}` — se resolvió, por nivel.
    Levels(BTreeMap<u8, Neighbourhood>),
}

/// El literal `declined`, y nada más.
///
/// Un enum de un solo valor y no un `bool`: `declined: true` obligaría a decidir qué
/// significa `false`, y no significa nada — la ausencia ya dice lo otro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeclinedMark {
    Declined,
}

/// Los dos folds de **un nivel**, que van juntos o no van.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Neighbourhood {
    /// Los captures de los vecinos que este fold cubre.
    ///
    /// **Sin esto el vecindario era el único lugar del formato donde una ubicación
    /// externa se hasheaba cruda**, y de esa anomalía salían tres cosas: que el hash
    /// cubriera el *nombre* del tipo y no su forma, que un vecino que cambia de
    /// archivo mueva el fold sin que nadie sepa por qué, y que no se pudiera
    /// preguntar qué tipos son.
    ///
    /// **Vacío se omite, y no hay ambigüedad**: que el nivel exista ya dice que el
    /// vecindario se adquirió, así que la ausencia del campo sólo puede significar
    /// *"se miró y no hay vecinos"* — el caso legítimo de una firma cuyos tipos
    /// resuelven todos fuera de la capa, como `Result<T>` con el `Result` de anyhow.
    ///
    /// Una firma de **puros primitivos** no llega hasta acá: no aporta ninguna
    /// posición, así que el nivel no se adquiere y no hay campo del que hablar.
    ///
    /// Es una consecuencia de la forma string: `capture <id> <id>` no tiene cómo
    /// escribir una lista vacía —`capture` solo es degenerado y se rechaza— así que
    /// el vacío se dice callándose. Con una secuencia YAML se habría podido escribir
    /// `[]`, que dice lo mismo con una palabra más.
    #[serde(default, skip_serializing_if = "LevelLink::is_absent")]
    pub link: LevelLink,
    /// SHA-256 plegado de los vecinos, en orden de id de capture.
    pub hash: String,
    /// Ídem sobre sus s-expressions, y **todo-o-nada**.
    ///
    /// Presente sólo si **todos** los vecinos del nivel tienen gramática. Si a
    /// alguno le falta, un cambio real en ése movería `hash` y no éste, y eso se
    /// leería como "sólo formateo" cuando no lo fue — un falso RESTYLED es peor que
    /// ningún estado.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_ast: Option<String>,
}

impl N {
    /// La renuncia, sin tener que nombrar el marcador.
    pub fn declined() -> Self { Self::Declined(DeclinedMark::Declined) }

    /// El vecindario de un nivel, si se adquirió.
    pub fn level(&self, k: u8) -> Option<&Neighbourhood> {
        match self { Self::Levels(m) => m.get(&k), Self::Declined(_) => None }
    }

    /// Un solo nivel, que es el único que hoy se resuelve.
    pub fn of_level_1(nb: Neighbourhood) -> Self {
        Self::Levels(BTreeMap::from([(1u8, nb)]))
    }

    /// Si hay algún nivel adquirido.
    pub fn is_acquired(&self) -> bool { matches!(self, Self::Levels(_)) }
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
            && self.n == other.n
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
                zero: Endpoint { link: link0, n: None, accepted: Vec::new(), name: None, r#as: None },
                one:  Endpoint { link: link1, n: None, accepted: Vec::new(), name: None, r#as: None },
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

    /// El `accepted` del endpoint estructural, si hay **uno solo**.
    ///
    /// Es lo que un endpoint `path` copia de su vecino: los dos valores, no el hash
    /// del archivo. Copiar el archivo entero haría que cualquier reordenamiento o
    /// comentario del vecino disparara `CHAIN_DIRTY`.
    ///
    /// **Con el vecino divergido devuelve `None`**, y eso es provisorio: qué
    /// corresponde copiar cuando el proveedor no tiene una sola respuesta está
    /// abierto en `concepts/bilink.md` § "si cruza la frontera". No copiar es la
    /// salida conservadora — deja al consumidor sin poder aceptar, que es visible y
    /// no miente. Para poder decidirlo después, la divergencia se distingue de la
    /// ausencia con [`structural_diverged`](Self::structural_diverged).
    pub fn structural_accepted(&self) -> Option<&Accepted> {
        for n in [0u8, 1u8] {
            let e = self.endpoint.get(n);
            if e.link.is_structural() {
                return match e.accepted.as_slice() { [one] => Some(one), _ => None };
            }
        }
        None
    }

    /// Si el endpoint estructural tiene **más de una** decisión.
    ///
    /// Existe para que *"el vecino no aceptó"* y *"el vecino no se puso de acuerdo"*
    /// no le lleguen iguales al consumidor. Son dos cosas distintas y la segunda no
    /// se arregla esperando.
    pub fn structural_diverged(&self) -> bool {
        for n in [0u8, 1u8] {
            let e = self.endpoint.get(n);
            if e.link.is_structural() { return e.accepted.len() > 1 }
        }
        false
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
        bl.endpoint.zero.accepted = vec![Accepted {
            agree: ["ana".to_string(), "pablo".to_string()].into(),
            link: Some(ep("capture abc123")),
            hash: "c00e0760".into(),
            hash_ast: Some("1b9e44a2".into()),
            n: None,
        }];
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
        let no_hash = "endpoint:\n  0: {link: capture a, accepted: [{link: capture a}]}\n  1: {link: capture b}\n";
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
        assert!(bl.endpoint.zero.accepted.is_empty());
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

/// Los tres estados de `n`, en el archivo.
///
/// Es un enum **untagged**: la forma del valor lo discrimina, un string o un mapa.
/// Vale probarlo por separado porque untagged falla en silencio si las dos variantes
/// se pisan, y acá no se pisan por construcción.
#[cfg(test)]
mod n_shape_tests {
    use super::*;

    fn round_trip(n: Option<N>) -> (String, Option<N>) {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Holder {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            n: Option<N>,
        }
        let y = serde_yaml_ng::to_string(&Holder { n }).unwrap();
        let back: Holder = serde_yaml_ng::from_str(&y).unwrap();
        (y, back.n)
    }

    /// Adquirido: un mapa con los dos folds.
    #[test]
    fn an_acquired_neighbourhood_is_keyed_by_level() {
        let n = Some(N::of_level_1(Neighbourhood { link: Default::default(), hash: "96c765b9".into(), hash_ast: Some("88e834c4".into()),
        }));
        let (y, back) = round_trip(n.clone());
        assert_eq!(y, "n:\n  1:\n    hash: 96c765b9\n    hash_ast: 88e834c4\n");
        assert_eq!(back, n);
    }

    /// Y `hash_ast` es opcional **adentro**: todo-o-nada sobre los vecinos, no sobre
    /// el campo entero.
    #[test]
    fn the_ast_fold_is_optional_inside_a_level() {
        let n = Some(N::of_level_1(Neighbourhood { link: Default::default(), hash: "96c765b9".into(), hash_ast: None }));
        let (y, back) = round_trip(n.clone());
        assert_eq!(y, "n:\n  1:\n    hash: 96c765b9\n");
        assert_eq!(back, n);
    }

    /// **La puerta que el mapa deja abierta**: un nivel 2 es una clave más, y no una
    /// forma nueva. No se resuelve todavía; lo que se prueba es que la forma lo
    /// admite sin cambiar nada.
    #[test]
    fn a_second_level_is_one_more_key() {
        let n = Some(N::Levels(BTreeMap::from([
            (1u8, Neighbourhood { link: Default::default(), hash: "96c765b9".into(), hash_ast: Some("88e834c4".into()) }),
            (2u8, Neighbourhood { link: Default::default(), hash: "4b1e0d77".into(), hash_ast: None }),
        ])));
        let (y, back) = round_trip(n.clone());
        assert_eq!(y, "n:\n  1:\n    hash: 96c765b9\n    hash_ast: 88e834c4\n  2:\n    hash: 4b1e0d77\n");
        assert_eq!(back, n);
    }

    /// La renuncia: un string, y nada más.
    #[test]
    fn a_decline_is_the_bare_word() {
        let (y, back) = round_trip(Some(N::declined()));
        assert_eq!(y, "n: declined\n");
        assert_eq!(back, Some(N::declined()));
    }

    /// Ausente es el tercer estado, y no se serializa.
    #[test]
    fn absent_is_the_third_state() {
        let (y, back) = round_trip(None);
        assert_eq!(y, "{}\n");
        assert_eq!(back, None);
    }

    /// **Lo que el plegado vuelve imposible.** Los tres estados que la forma plana
    /// dejaba escribir ya no tienen dónde ir: un fold de ASTs sin su fold de textos
    /// no parsea, porque `hash` es obligatorio adentro del mapa.
    #[test]
    fn an_ast_fold_without_its_text_fold_cannot_be_written() {
        let r: Result<N, _> = serde_yaml_ng::from_str("1:\n  hash_ast: 88e834c4\n");
        assert!(r.is_err(), "un hash_ast suelto no es un nivel");
    }

    /// Y una renuncia no puede convivir con el valor al que se renunció: son
    /// variantes del mismo campo, no dos campos.
    #[test]
    fn a_decline_cannot_carry_a_hash() {
        let r: Result<N, _> = serde_yaml_ng::from_str("declined:\n  hash: 96c765b9\n");
        assert!(r.is_err(), "o se renunció, o hay niveles");
    }

    /// **El caso que la `003` no pudo escribir**: el contrato conservado y su
    /// ubicación desconocida. Es un nivel adquirido, no una renuncia.
    #[test]
    fn a_level_can_hold_its_contract_and_not_its_location() {
        let n = Some(N::of_level_1(Neighbourhood {
            link: LevelLink::Unknown,
            hash: "d3fba7c7".into(),
            hash_ast: Some("319cfc6f".into()),
        }));
        let (y, back) = round_trip(n.clone());
        assert_eq!(y, "n:\n  1:\n    link: unknown\n    hash: d3fba7c7\n    hash_ast: 319cfc6f\n");
        assert_eq!(back, n);
        assert!(n.as_ref().unwrap().is_acquired(), "adquirido, no renunciado");
    }
}

/// Las tres formas del `link` de un nivel, y las dos que no se pueden confundir.
#[cfg(test)]
mod level_link_tests {
    use super::*;

    /// La ausencia es el conjunto vacío, y `unknown` **no** es la ausencia: si lo
    /// fuera, restituir hashes sin el valor diría lo mismo que escribirlo.
    #[test]
    fn unknown_is_not_the_absence() {
        assert!(LevelLink::default().is_absent());
        assert!(!LevelLink::Unknown.is_absent());
        assert!(LevelLink::Unknown.is_unknown());
    }

    /// **No hay cómo leer `unknown` como un conjunto vacío**, que es el error que el
    /// tipo existe para impedir: quien pregunta por los ids recibe `None`.
    #[test]
    fn unknown_hands_out_no_ids() {
        assert_eq!(LevelLink::Unknown.known_ids(), None);
        assert_eq!(LevelLink::default().known_ids(), Some(&[][..]));
        assert!(LevelLink::Unknown.captures().is_none());
    }

    #[test]
    fn every_form_round_trips_through_its_string() {
        let ids = CaptureSet::new(vec!["a".repeat(32), "b".repeat(32)]);
        for raw in ["unknown", &format!("{ids}")] {
            let l: LevelLink = raw.parse().unwrap();
            assert_eq!(l.to_string(), raw, "'{raw}' no round-trippea");
        }
    }

    /// Un valor que no es ninguna de las dos formas **falla**, y el error nombra las
    /// dos. Es lo que hace que un parser de 4.0.0 no lea `unknown` como otra cosa: el
    /// slot no tiene fallback.
    #[test]
    fn anything_else_is_an_error() {
        for raw in ["", "capture", "pending", "new-data-pending", "unknown abc"] {
            let err = raw.parse::<LevelLink>().unwrap_err().to_string();
            assert!(err.contains("unknown") || err.contains("capture"),
                    "'{raw}' falla sin decir qué se esperaba: {err}");
        }
    }
}
