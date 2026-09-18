use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use tree_sitter::{Node, Parser, Point};

use crate::git;
use crate::grammar::{self, stable_anchor_kinds};
use crate::hash;
use std::collections::BTreeMap;

use bilink_format::{ByteRange, Capture, DeclaredDimension, Ranges};
use crate::query;

/// La ubicación aprobada de un endpoint, si el endpoint es estructural.
///
/// Devuelve `None` para `path` e `issue`, que no tienen capture.
pub fn capture_of(layer: &Path, link: &bilink_format::LinkEndpoint) -> Result<Option<Capture>> {
    match link.capture_id() {
        Some(id) => Ok(Some(Capture::load_in(layer, id)?)),
        None => Ok(None),
    }
}

/// Busca un capture de la capa con la misma referencia exacta.
///
/// La igualdad es `(file, query, offset)`: referencias idénticas describen la
/// misma ubicación, así que comparten capture. Es el mismo criterio que usa la
/// migración — si no, cada cadena nueva volvería a duplicar lo que aquélla unificó.
// `find_equivalent` ya no existe. El id de un capture es el hash de su ubicación,
// así que dos referencias iguales son el mismo archivo: no hay nada que buscar.

/// Captura una selección y escribe su capture. Devuelve `(id, path, ya_existía)`.
pub fn capture_to_file(
    layer: &Path,
    file:  &str,
    start: (usize, usize),
    end:   (usize, usize),
) -> Result<(String, PathBuf, bool)> {
    capture(layer, file, start, end)?.capture.write_in(layer)
}

/// El archivo entero como capture: sin query, sin offset.
///
/// **No exige que el archivo exista.** Un capture es una ubicación, y sin query no
/// hay nada que parsear para calcular su id. Exigirlo rompería el caso de declarar
/// una cadena hacia una capa que todavía no se creó —el estado `TODO`—, que es una
/// intención declarada y no un error.
pub fn capture_file_whole(layer: &Path, file: &str) -> Result<(String, PathBuf, bool)> {
    Capture { file: file.to_string(), query: None }.write_in(layer)
}

/// Los captures que no alcanza ningún bilink.
///
/// **Mark & sweep sobre dos clases de raíz**, no una: un capture está vivo si lo
/// referencia un `link` —la ubicación vigente— **o** un `accepted.link` —la que
/// alguien aprobó. Barrer sólo por la primera borraría el capture que dice dónde
/// estaba lo aceptado, y con él la capacidad de decidir si una ubicación cambió.
pub fn orphans(layer: &Path) -> Result<Vec<(String, Capture)>> {
    use std::collections::HashSet;
    let mut alive: HashSet<String> = HashSet::new();

    for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = bilink_format::BiLink::load(&path) else { continue };
        for n in [0u8, 1u8] {
            let e = bl.endpoint.get(n);
            if let Some(id) = e.link.capture_id() {
                alive.insert(id.to_string());
            }
            // **El vecindario declarado también referencia.** Sin esta línea el
            // primer `prune` sobre una capa con cierre de firma se lleva los vecinos,
            // y lo que queda es un `accepted` apuntando a captures que no existen: un
            // `UNRESOLVED` masivo producido por una limpieza.
            for id in declared_neighbours(e) {
                alive.insert(id);
            }
            // **Todas las entradas, no la primera.** Con `accepted` como lista una
            // decisión desplazada sigue referenciando sus captures hasta que alguien
            // resuelva la divergencia — borrarlos dejaría la entrada apuntando al
            // vacío y con eso se perdería el lado del desacuerdo que no ganó.
            for a in &e.accepted {
                if let Some(id) = a.link.as_ref().and_then(|l| l.capture_id()) {
                    alive.insert(id.to_string());
                }
                for id in accepted_neighbours(a) {
                    alive.insert(id);
                }
            }
        }
    }

    Ok(Capture::all_in(layer)?.into_iter().filter(|(id, _)| !alive.contains(id)).collect())
}

/// Los captures que el vecindario **declarado** de un endpoint nombra.
///
/// Un nivel `unknown` no nombra ninguno — no tiene ids que nombrar — y por eso no
/// mantiene vivo a nadie.
fn declared_neighbours(e: &bilink_format::Endpoint) -> Vec<String> {
    e.n.iter()
        .flat_map(|n| n.0.values())
        .flat_map(|lvl| lvl.link.known_ids().into_iter().flatten().cloned())
        .collect()
}

/// Los captures que el vecindario **aceptado** de una entrada nombra.
///
/// Una renuncia no nombra ninguno, y por eso el `match` no tiene rama para ella: es
/// el mismo motivo por el que `n` es un campo con tres estados y no dos.
fn accepted_neighbours(a: &bilink_format::Accepted) -> Vec<String> {
    let Some(bilink_format::N::Levels(levels)) = a.n.as_ref() else { return Vec::new() };
    levels.values().flat_map(|nb| nb.link.known_ids().into_iter().flatten().cloned()).collect()
}

pub(crate) fn git_path_from_repo_root(layer: &Path, file: &str) -> String {
    let top = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());

    match top {
        Some(t) => {
            let root = Path::new(t.trim());
            // Los dos absolutos: `layer` puede venir relativo, y ahí el
            // `strip_prefix` falla en silencio y devuelve el path relativo a la
            // capa — que git resuelve contra la raíz del repo y no encuentra.
            let abs  = layer.canonicalize().unwrap_or_else(|_| layer.to_path_buf());
            let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            match abs.strip_prefix(&root) {
                Ok(rel) if !rel.as_os_str().is_empty() =>
                    format!("{}/{file}", rel.display()),
                _ => file.to_string(),
            }
        }
        None => file.to_string(),
    }
}

/// El texto del fragmento tal como quedó aceptado en `commit`.
///
/// **No recorta el contenido viejo por el `range` guardado.** `check` reescribe
/// `range` en cada corrida, así que apunta a dónde está el fragmento *ahora*;
/// recortar contenido de otro commit con una posición actual da bytes
/// arbitrarios. En su lugar resuelve la query contra el contenido de ese commit.
///
/// Con `expected_hash` presente, verifica que el resultado hashee a ese valor y
/// devuelve `None` si no coincide. Es preferible no devolver nada que devolver
/// el fragmento equivocado: quien llama toma decisiones a partir de este texto.
pub fn accepted_text(
    layer:         &Path,
    cap:           &Capture,
    commit:        &str,
    expected_hash: Option<&str>,
) -> Option<String> {
    let old_source = source_at(layer, cap, commit)?;

    let text = match &cap.query {
        None => old_source.clone(),
        Some(q) => {
            let lang     = grammar::language_for_file(&cap.file);
            let language = grammar::for_language(lang).ok()?;
            let fragment = crate::query::find_fragment(language, &old_source, q).ok()??;
            if fragment.ranges.end() > old_source.len() { return None; }
            fragment.ranges.text(&old_source)
        }
    };

    match expected_hash {
        Some(h) if hash::sha256(text.as_bytes()) != h => None,
        _ => Some(text),
    }
}

/// El archivo del capture tal como estaba en `commit`.
pub(crate) fn source_at(layer: &Path, cap: &Capture, commit: &str) -> Option<String> {
    // `git show <commit>:<path>` resuelve el path contra la **raíz del repo**, no
    // contra el `-C`. Cuando la capa no es la raíz —una capa de specs dentro de
    // un repo mayor— pasar el path relativo a la capa hace fallar el comando.
    let repo_rel = git_path_from_repo_root(layer, &cap.file);
    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "show", &format!("{commit}:{repo_rel}")])
        .output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8(out.stdout).ok()
}

/// El commit donde el fragmento tenía el contenido aceptado, derivado de git.
///
/// `commit` es un derivado y vive en la cache, que no está en git: un clon fresco
/// no lo tiene. Sin él, `accepted.hash` es un hash que no se puede resolver a
/// texto, y sin el texto aceptado `check` no puede distinguir EXPANDED de
/// DISPLACED de ALTERED. Que se re-derive es lo que hace que sacarlo del formato
/// no le cueste nada a nadie.
///
/// **Un walk hacia atrás, no `git log -L`.** Aquél encuentra cuándo esas líneas
/// quedaron como están *ahora*; lo que se busca es dónde el fragmento tenía el
/// contenido *aceptado*, que en un endpoint con drift es otro commit y
/// probablemente otras líneas.
///
/// Acotado por dos lados: sólo se pregunta por endpoints ya no-OK, y el walk tiene
/// techo, porque un hash de algo que nunca existió en esta rama recorrería la
/// historia entera para contestar que no. Al llegar al techo devuelve `None`, y
/// quien preguntó degrada en vez de fallar.
///
/// **Se camina la ref, no la rama.** Es lo que vuelve cierto que la ref protege
/// también a la derivación, y no sólo al `commit` guardado. Un rebase a secas no
/// hace falta que lo cubra nadie —preserva el contenido, así que el fragmento
/// aceptado aparece igual en el commit reescrito— pero un squash o un
/// `filter-branch` sí: ahí el contenido intermedio deja de existir en la historia de
/// la rama, y el único lugar donde sigue estando es la ref, que absorbió ese commit
/// como segundo padre y no se rebasea nunca.
///
/// Sin ref —un repo que todavía no cortó— se camina `HEAD`, que es lo único que hay.
pub fn derive_commit(layer: &Path, cap: &Capture, accepted_hash: &str) -> Option<String> {
    const TECHO: usize = 500;

    let repo_rel = git_path_from_repo_root(layer, &cap.file);
    let start = history_root(layer);
    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "log", "--format=%H",
               &format!("-{TECHO}"), &start, "--", &repo_rel])
        .output().ok()?;
    if !out.status.success() { return None; }

    String::from_utf8(out.stdout).ok()?
        .lines()
        .find(|c| accepted_text(layer, cap, c, Some(accepted_hash)).is_some())
        .map(str::to_string)
}

/// Desde dónde se camina la historia de un archivo: `refs/bilink/<branch>` si la
/// rama tiene ref, `HEAD` si no.
///
/// La ref alcanza todo commit del proyecto alguna vez absorbido, así que su historia
/// es un superconjunto de la de la rama — incluye lo que un squash borró de ella. Y
/// como la ref lleva el árbol del proyecto adentro, los paths son los mismos.
fn history_root(layer: &Path) -> String {
    crate::bilink_ref::Repo::open(layer)
        .ok()
        .and_then(|repo| {
            let branch = repo.branch()?;
            repo.ref_tip(&branch).map(|_| crate::bilink_ref::Repo::ref_name(&branch))
        })
        .unwrap_or_else(|| "HEAD".to_string())
}

/// Los rangos absolutos del fragmento en su archivo, resolviendo la query.
pub fn absolute_range(layer: &Path, cap: &Capture) -> Result<Option<Ranges>> {
    let path = layer.join(&cap.file);
    if !path.exists() { return Ok(None); }
    let source = std::fs::read_to_string(&path)?;

    let Some(query_str) = &cap.query else {
        return Ok(Some(Ranges::one(0, source.len())));
    };
    let lang     = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;
    let Some(fragment) = crate::query::find_fragment(language, &source, query_str)? else {
        return Ok(None);
    };
    Ok(Some(fragment.ranges))
}

pub struct CaptureResult {
    pub capture: Capture,
    pub hash: String,
    pub commit: String,
    /// Lo que se vigila, en orden de archivo: el nodo entero, o las partes de sus
    /// dimensiones cuando un generador las declaró.
    ///
    /// Es lo que la vista previa marca: sin esto, quien crea un endpoint no tiene
    /// con qué ver si agarró lo que quería, y un capture es opaco después de escrito.
    pub ranges: Ranges,
    /// Las dimensiones que declaró el generador, por nombre. Vacío sin `--as`.
    pub dimensions: BTreeMap<String, DeclaredDimension>,
    /// Lo que resolvió cada una, en el orden de los nombres.
    pub parts: Vec<(String, Ranges)>,
}

pub fn capture(
    root: &Path,
    file: &str,
    start: (usize, usize), // (line, col) 1-based
    end: (usize, usize),
) -> Result<CaptureResult> {
    capture_as(root, file, &[(start, end)], None)
}

/// El capture del nodo señalado, y las dimensiones que un [`CaptureGenerator`]
/// declare sobre él.
///
/// **Las posiciones se descartan.** Sirven para *encontrar* el nodo —cada una
/// resuelve al ancla estable más cercana— y lo que se guarda es la query. Todas
/// tienen que caer en el mismo: dos nodos son dos contratos, y un capture es una
/// ubicación.
pub fn capture_as(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
    generator: Option<&dyn CaptureGenerator>,
) -> Result<CaptureResult> {
    let commit = git::head_commit_for_file(root, file)?;
    let Computed { capture, hash, ranges, dimensions, parts } = compute_as(root, file, sel, generator)?;
    Ok(CaptureResult { capture, hash, commit, ranges, dimensions, parts })
}

/// El capture, su hash y lo que se vigila, **sin preguntarle nada a git**.
///
/// Es lo que necesita el [vecindario](crate::neighbours), que acuña un capture por
/// vecino sólo para tener su id y su hash: pedir git ahí ataría el cálculo del id a
/// que el archivo esté versionado, que no tiene nada que ver.
pub fn compute(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
    generator: Option<&dyn CaptureGenerator>,
) -> Result<(Capture, String, Ranges)> {
    let c = compute_as(root, file, sel, generator)?;
    Ok((c.capture, c.hash, c.ranges))
}

/// Lo que sale de señalar un nodo: su capture, y lo que el generador declaró.
pub struct Computed {
    pub capture: Capture,
    /// El hash de lo que se vigila: el nodo, o sus dimensiones concatenadas.
    pub hash: String,
    /// Lo que se vigila, en orden de archivo.
    pub ranges: Ranges,
    pub dimensions: BTreeMap<String, DeclaredDimension>,
    /// Lo que resolvió cada dimensión, en el orden de los nombres.
    pub parts: Vec<(String, Ranges)>,
}

/// El capture del nodo señalado y, con generador, sus dimensiones verificadas.
///
/// **El capture es siempre el del núcleo.** Con `--as` o sin él, el mismo nodo da
/// la misma query y por lo tanto el mismo archivo: el generador no toca la
/// ubicación, declara qué se vigila de ella.
pub fn compute_as(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
    generator: Option<&dyn CaptureGenerator>,
) -> Result<Computed> {
    if sel.is_empty() {
        bail!("un capture con posiciones necesita al menos una");
    }
    let file_path = root.join(file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let lang = grammar::language_for_file(file);
    let language = grammar::for_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(&source, None).context("parse failed")?;

    let root_node = tree.root_node();
    let anchors   = stable_anchor_kinds(lang);

    // Cada posición a su nodo, sin repetir: dos posiciones adentro de la misma
    // función son la misma función, no dos partes.
    let mut pointed: Vec<Node> = Vec::new();
    let mut standalone = false;
    for (start, end) in sel {
        let (node, alone) = target_node_at(root_node, *start, *end, anchors)?;
        standalone |= alone;
        if !pointed.iter().any(|t| t.id() == node.id()) {
            pointed.push(node);
        }
    }
    pointed.sort_by_key(|n| (n.start_byte(), n.end_byte()));

    // **Dos nodos son dos contratos.** La query identifica un nodo y no compone el
    // fragmento: juntar dos haría que la parte que falte arrastre al ancla que está
    // intacta.
    if let [a, b, ..] = pointed.as_slice() {
        bail!(
            "las posiciones caen en nodos distintos: el `{}` de la línea {} y el `{}` \
             de la línea {}. Un capture es un nodo.\n       \
             Señalar uno solo, o vigilar sus partes con `--as`.",
            a.kind(), a.start_position().row + 1, b.kind(), b.start_position().row + 1,
        );
    }
    let node = pointed[0];

    let ctx = GenCtx { source: &source, lang, anchors, standalone };
    let query = pattern_for(&ctx, node);

    // La query tiene que identificar al nodo señalado y a ninguno otro. Un ancla sin
    // discriminante —un `impl` sin tipo, un comentario, un `use`— matchea el primer
    // nodo de ese tipo del archivo, y el capture apuntaría a otra cosa sin fallar.
    // Un capture mal anclado es peor que uno roto: reporta OK sobre una
    // correspondencia que no existe.
    let range = verify_query_identifies(language.clone(), &source, &query, node, file)?;

    // **La selección elige nodos, no rangos de bytes.** Un rango adentro de un nodo
    // se corre con cualquier edición encima suya dentro del mismo nodo, así que su
    // granularidad es ilusoria. Si hace falta más precisión, la respuesta es una
    // query que nombre algo más chico, o una dimensión que nombre la parte.
    let capture = Capture { file: file.to_string(), query: Some(query) };

    let Some(g) = generator else {
        let hash = hash::sha256(range.text(&source).as_bytes());
        return Ok(Computed { capture, hash, ranges: range, dimensions: BTreeMap::new(), parts: Vec::new() });
    };

    let mut dimensions = BTreeMap::new();
    let mut parts = Vec::new();
    let anchor = (range.start(), range.end());
    for d in g.dimensions(&ctx, node)? {
        let resolved = verify_dimension(language.clone(), &source, &d, anchor, file)?;
        dimensions.insert(d.name.clone(), DeclaredDimension { query: d.query });
        parts.push((d.name, resolved));
    }
    parts.sort_by(|a, b| a.0.cmp(&b.0));

    let mut all: Vec<ByteRange> = parts.iter().flat_map(|(_, r)| r.parts().to_vec()).collect();
    all.sort_by_key(|r| (r.start, r.end));
    all.dedup();
    let ranges = Ranges::new(all).unwrap_or(range);
    let hash = hash::sha256(ranges.text(&source).as_bytes());
    Ok(Computed { capture, hash, ranges, dimensions, parts })
}

/// Una dimensión resuelve contra el nodo del capture, y a las partes que el
/// generador señaló.
///
/// Se verifica acá y no en `check` por lo mismo que la query del capture: acá todavía
/// se puede no escribir, y una dimensión que vigila otra cosa reporta OK sobre una
/// parte que nadie aprobó.
fn verify_dimension(
    language: tree_sitter::Language,
    source:   &str,
    d:        &GeneratedDimension,
    anchor:   (usize, usize),
    file:     &str,
) -> Result<Ranges> {
    let mut esperado: Vec<(usize, usize)> = d.targets.iter()
        .map(|t| query::trim_edges(source, t.start_byte(), t.end_byte()))
        .collect();
    esperado.sort_unstable();
    let Some(f) = query::dimension(language, source, &d.query, anchor)? else {
        bail!("la dimensión `{}` no resuelve en {file}:\n{}", d.name, d.query);
    };
    let got: Vec<(usize, usize)> = f.ranges.parts().iter().map(|r| (r.start, r.end)).collect();
    if got != esperado {
        bail!(
            "la dimensión `{}` resuelve a {} y el generador señaló {} en {file}:\n{}",
            d.name, fmt_ranges(&got), fmt_ranges(&esperado), d.query
        );
    }
    Ok(f.ranges)
}

/// Lo que un generador necesita saber del archivo, y nada más.
pub struct GenCtx<'a> {
    pub source:  &'a str,
    pub lang:    &'a str,
    /// Los tipos de nodo que se consideran anclas estables en este lenguaje.
    pub anchors: &'a [&'a str],
    /// El nodo señalado tiene que ser la raíz del patrón — un item de secuencia
    /// YAML, que se identifica solo y cuyo ancestro es la secuencia entera.
    pub standalone: bool,
}

/// Una dimensión que un generador declara: su nombre, su query, y los nodos que
/// espera que resuelva.
///
/// Los nodos viajan con la query para poder **verificar** que resuelve a lo que el
/// generador quiso. Sin eso, un generador con un error declara una dimensión que
/// vigila otra cosa y no falla.
pub struct GeneratedDimension<'t> {
    pub name:    String,
    pub query:   String,
    pub targets: Vec<Node<'t>>,
}

/// Quién declara qué se vigila de un nodo.
///
/// **Un generador declara y desaparece.** El capture es el del núcleo, y las
/// dimensiones quedan escritas en el endpoint con su query: no dependen de que el
/// generador exista, y un `as` que nombra uno ausente es un dato que no se pudo usar.
///
/// **Sin carga dinámica.** Plugins `.so` es complejidad que todavía no pidió nadie;
/// el trait deja la puerta abierta y el registro es un `Vec` que arma el binario.
pub trait CaptureGenerator {
    fn name(&self) -> &'static str;

    /// Qué hace, en una línea. Es lo que lista `--as` sin valor.
    fn describe(&self) -> &'static str;

    /// ¿Este generador tiene algo que decir sobre este nodo?
    ///
    /// **Sólo para sugerir, nunca para elegir.** Un generador que acierta cuando no
    /// querías ya te escribió otra cosa, y lo que un endpoint vigila no se ve sin ir
    /// a buscarlo.
    fn applies(&self, file: &str, source: &str, node: Node) -> bool;

    /// Las dimensiones que se vigilan del nodo, cada una con su query y sus nodos.
    fn dimensions<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Vec<GeneratedDimension<'t>>>;

    /// Cómo se llama este endpoint, en el vocabulario de este generador.
    ///
    /// **Se compone de lo vigilado, no se guarda.** Un alias guardado es un valor
    /// derivado con vida propia: el día que cambia la ruta sigue diciendo lo viejo, y
    /// lo diría en silencio porque los campos semánticos son inertes. Un rótulo falso
    /// sobre una referencia verificada es peor que no tener rótulo.
    ///
    /// **Y es de cada generador, no del formato.** Un endpoint se nombra por su verbo
    /// y su ruta; una firma, por su método. Un generador que no sepa nombrar devuelve
    /// `None` y el bilink se muestra por UUID.
    ///
    /// `parts` es lo que resolvió cada dimensión declarada, por nombre.
    fn alias(&self, _source: &str, _parts: &[(String, Ranges)]) -> Option<String> { None }
}

/// Los generadores que este binario conoce.
pub fn generators() -> Vec<Box<dyn CaptureGenerator>> {
    vec![
        Box::new(crate::generators::Interface),
        Box::new(crate::generators::SpringController),
    ]
}

/// El generador con ese nombre, o un error que lista los que hay.
pub fn generator_named(name: &str) -> Result<Box<dyn CaptureGenerator>> {
    generators().into_iter().find(|g| g.name() == name).ok_or_else(|| anyhow::anyhow!(
        "no hay un modo `{name}`.\n       Los que hay: {}",
        generators().iter().map(|g| g.name()).collect::<Vec<_>>().join(", ")
    ))
}

/// Los generadores que tendrían algo que decir sobre este nodo.
pub fn suggestions_for(file: &str, source: &str, node: Node) -> Vec<&'static str> {
    generators().iter().filter(|g| g.applies(file, source, node)).map(|g| g.name()).collect()
}

/// Los generadores que tendrían algo que decir sobre la posición señalada.
///
/// Es lo que la vista previa **sugiere** cuando no se pidió ninguno. Sugerir y no
/// elegir: un generador que acierta cuando no querías ya te escribió otra cosa, y un
/// capture es opaco después de escrito.
pub fn suggest_for(layer: &Path, file: &str, pos: (usize, usize)) -> Result<Vec<&'static str>> {
    let source = std::fs::read_to_string(layer.join(file))?;
    let lang     = grammar::language_for_file(file);
    let language = grammar::for_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(&source, None).context("parse failed")?;
    let (node, _) = target_node_at(tree.root_node(), pos, pos, stable_anchor_kinds(lang))?;
    Ok(suggestions_for(file, &source, node))
}

/// El patrón que identifica a `node`, anclado en el ancla estable que lo contiene.
///
/// Es el mismo con `--as` o sin él: la query nombra la ubicación, y lo que se vigila
/// de ella lo declaran las dimensiones.
pub fn pattern_for(ctx: &GenCtx<'_>, node: Node) -> String {
    let pattern_root = if ctx.standalone {
        node
    } else {
        node.parent().and_then(|n| walk_up_to_anchor(n, ctx.anchors)).unwrap_or(node)
    };

    let mut kids: std::collections::HashMap<usize, Vec<Node>> = std::collections::HashMap::new();
    let path = build_path(pattern_root, node);
    for pair in path.windows(2) {
        kids.entry(pair[0].id()).or_default().push(pair[1]);
    }
    emit_pattern(pattern_root, &kids, node.id(), ctx.source, &mut 0, ctx.lang)
}

/// El nodo que una posición señala: el ancla estable más cercana que la contiene.
///
/// El `bool` dice que ese nodo tiene que ser la raíz del patrón y no colgar de un
/// ancla de arriba. Pasa con un item de secuencia YAML, que se identifica solo por
/// su `id:` y cuyo ancestro es la secuencia entera.
fn target_node_at<'a>(
    root:    Node<'a>,
    start:   (usize, usize),
    end:     (usize, usize),
    anchors: &[&str],
) -> Result<(Node<'a>, bool)> {
    let start_point = Point { row: start.0 - 1, column: start.1 - 1 };
    let end_point   = Point { row: end.0 - 1,   column: end.1 - 1 };

    let node = root
        .named_descendant_for_point_range(start_point, end_point)
        .context("no named node at selection")?;

    let target = walk_up_to_anchor(node, anchors).unwrap_or(node);
    if target.kind() == "block_sequence_item" {
        return Ok((target, true));
    }
    match target.parent().and_then(|p| walk_up_to_anchor(p, anchors)) {
        Some(a) if a.kind() == "block_sequence_item" => Ok((a, true)),
        _ => Ok((target, false)),
    }
}

/// El patrón de un nodo: su predicado de nombre, los hijos que llevan al target,
/// y su `@target` si lo es.
///
/// **Los hijos salen en orden de archivo**, y el predicado de nombre con ellos. No
/// es cosmético: tree-sitter exige que los hijos de un patrón vayan en el orden de
/// la gramática, y en Java las anotaciones van antes del nombre.
fn emit_pattern(
    node:    Node,
    kids:    &std::collections::HashMap<usize, Vec<Node>>,
    target:  usize,
    source:  &str,
    counter: &mut usize,
    lang:    &str,
) -> String {
    let (pred, pred_node) = real_name_predicate(node, source, counter, lang);
    let pred_pos = pred_node.map(|n| n.start_byte()).unwrap_or(node.start_byte());

    let mut parts: Vec<(usize, String)> = Vec::new();
    if !pred.is_empty() { parts.push((pred_pos, pred)); }

    for kid in kids.get(&node.id()).cloned().unwrap_or_default() {
        let field = field_name_for_child(node, kid.id())
            .map(|f| format!("{f}: "))
            .unwrap_or_default();
        let inner = emit_pattern(kid, kids, target, source, counter, lang);
        parts.push((kid.start_byte(), format!("\n  {field}{inner}")));
    }

    // **Una sobrecarga no se distingue por el nombre**: lo que la distingue son los
    // tipos de sus parámetros, que van después del nombre en la gramática.
    if let Some(params) = overload_parameters(node, source, counter, lang) {
        let at = node.child_by_field_name("parameters").map(|n| n.start_byte()).unwrap_or(pred_pos);
        parts.push((at, format!("\n  {params}")));
    }
    parts.sort_by_key(|(pos, _)| *pos);

    let body: String = parts.into_iter().map(|(_, s)| s).collect();
    let pattern = format!("({}{})", node.kind(), body);
    if node.id() == target { format!("{pattern} @target") } else { pattern }
}

/// Los tipos de los parámetros de un método de Java sobrecargado, como predicados.
///
/// **En orden y sin huecos**, con `.` entre hijos: `(Short)` no tiene que matchear
/// `(Short, Integer)`. **Con `#match?` y no `#eq?`**, para que el último `#eq?` de la
/// query siga siendo el nombre del método. `None` si el método no se repite en su
/// clase: ahí el nombre alcanza.
fn overload_parameters(node: Node, source: &str, counter: &mut usize, lang: &str) -> Option<String> {
    if lang != "java" || node.kind() != "method_declaration" || !is_overloaded(node, source) {
        return None;
    }
    let params = node.child_by_field_name("parameters")?;
    let mut c = params.walk();
    let mut hijos = Vec::new();
    for p in params.named_children(&mut c) {
        match p.child_by_field_name("type") {
            Some(t) if p.kind() == "formal_parameter" => {
                let n = format!("@n{counter}");
                *counter += 1;
                hijos.push(format!(
                    "(formal_parameter type: (_) {n} (#match? {n} \"^{}$\"))",
                    query::escape_query_string(&regex_literal(&source[t.byte_range()]))));
            }
            _ => hijos.push(format!("({})", p.kind())),
        }
    }
    if hijos.is_empty() {
        return Some("parameters: (formal_parameters)".to_string());
    }
    Some(format!("parameters: (formal_parameters\n    .\n    {}\n    .)",
                 hijos.join("\n    .\n    ")))
}

/// Si otro método del mismo cuerpo se llama igual: una sobrecarga, que el nombre
/// solo no distingue.
fn is_overloaded(method: Node, source: &str) -> bool {
    let (Some(body), Some(name)) = (method.parent(), method.child_by_field_name("name")) else { return false };
    let nombre = &source[name.byte_range()];
    let mut c = body.walk();
    let repetido = body.children(&mut c)
        .filter(|n| n.kind() == "method_declaration" && n.id() != method.id())
        .any(|n| n.child_by_field_name("name").is_some_and(|x| &source[x.byte_range()] == nombre));
    repetido
}

/// Un texto como regex que lo matchea literal.
fn regex_literal(texto: &str) -> String {
    let mut out = String::with_capacity(texto.len());
    for ch in texto.chars() {
        if "\\.^$|?*+()[]{}".contains(ch) { out.push('\\'); }
        out.push(ch);
    }
    out
}

fn build_path<'a>(ancestor: Node<'a>, descendant: Node<'a>) -> Vec<Node<'a>> {
    if ancestor.id() == descendant.id() {
        return vec![ancestor];
    }
    for i in 0..ancestor.child_count() {
        let child = ancestor.child(i).unwrap();
        if node_contains(child, descendant.id()) {
            let mut path = vec![ancestor];
            path.extend(build_path(child, descendant));
            return path;
        }
    }
    vec![ancestor]
}

fn node_contains(node: Node, target_id: usize) -> bool {
    if node.id() == target_id { return true; }
    for i in 0..node.child_count() {
        if node_contains(node.child(i).unwrap(), target_id) {
            return true;
        }
    }
    false
}

/// La query resuelve al nodo señalado, exactamente una vez.
///
/// Se verifica acá y no en `check` porque acá todavía se puede no escribir: un
/// capture que apunta al nodo equivocado se acepta en OK y no vuelve a mirarse.
///
/// Devuelve el rango resuelto —ya recortado— para no volver a correr la query: es el
/// mismo que `check` va a comparar, y el que la vista previa marca.
fn verify_query_identifies(
    language:  tree_sitter::Language,
    source:    &str,
    query_str: &str,
    target:    Node,
    file:      &str,
) -> Result<Ranges> {
    let esperado = vec![query::trim_edges(source, target.start_byte(), target.end_byte())];
    let hits = query::find_all_fragments(language, source, query_str)?;
    let kind = target.kind();

    match hits.as_slice() {
        [] => bail!("la query generada no matchea ningún nodo en {file}:\n{query_str}"),
        [f] => {
            let got: Vec<(usize, usize)> = f.ranges.parts().iter()
                .map(|r| (r.start, r.end))
                .collect();
            if got == esperado {
                return Ok(f.ranges.clone());
            }
            bail!(
                "la query generada apunta a otro nodo: {} en vez de {}. \
                 El ancla `{kind}` no tiene con qué distinguirse en {file}:\n{query_str}",
                fmt_ranges(&got), fmt_ranges(&esperado)
            )
        }
        hits => bail!(
            "la query generada matchea {} veces. El ancla `{kind}` no tiene con qué \
             distinguirse en {file}:\n{query_str}\n\n\
             Seleccionar un nodo con nombre propio adentro —una función, un método— \
             da un ancla única sin inventar un criterio.",
            hits.len()
        ),
    }
}

fn fmt_ranges(rs: &[(usize, usize)]) -> String {
    rs.iter().map(|(a, b)| format!("{a}~{b}")).collect::<Vec<_>>().join(",")
}

/// El predicado que identifica al nodo, y **el nodo del AST que ese predicado
/// captura** cuando lo hay.
///
/// El nodo hace falta por dos cosas: para ordenar el predicado entre las partes
/// —en Java las anotaciones van antes del nombre— y porque con `--as interface` el
/// nombre es a la vez el ancla y una parte capturada, y hay que escribir las dos
/// cosas sobre el mismo nodo en vez de emitirlo dos veces.
///
/// Los casos especiales devuelven `None`: su predicado cae sobre un heading, una
/// celda o una clave, que nunca son parte de una firma.
fn real_name_predicate<'a>(
    node: Node<'a>, source: &str, counter: &mut usize, lang: &str,
) -> (String, Option<Node<'a>>) {
    // Special case: markdown section — use heading text as predicate
    if node.kind() == "section" {
        if let Some(pred) = markdown_section_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // Special case: markdown pipe_table_row — la primera celda lo discrimina
    if node.kind() == "pipe_table_row" {
        if let Some(pred) = markdown_table_row_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // Special case: Gherkin — característica, regla y escenario, por su título
    if lang == "gherkin" {
        if let Some(pred) = gherkin_title_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // Special case: YAML block_sequence_item — use id: or first key as predicate
    if node.kind() == "block_sequence_item" {
        if let Some(pred) = yaml_sequence_item_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // Special case: YAML block_mapping_pair — use key as predicate
    if node.kind() == "block_mapping_pair" {
        if let Some(pred) = yaml_mapping_pair_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // Special case: Rust impl_item — no tiene campo `name`; lo identifica el tipo
    // y, si es la implementación de un trait, el trait.
    if lang == "rust" && node.kind() == "impl_item" {
        if let Some(pred) = rust_impl_predicate(node, source, counter) {
            return (pred, None);
        }
    }
    // El campo que lleva el nombre depende del lenguaje y del tipo de nodo; la
    // tabla vive en `grammar`. `name` es el caso mayoritario y el default.
    let field = grammar::name_field(lang, node.kind()).unwrap_or("name");
    let Some(name_child) = node.child_by_field_name(field) else {
        return (String::new(), None);
    };
    let name_type = name_child.kind();
    let name_text = query::escape_query_string(&source[name_child.byte_range()]);
    let cap = format!("@n{counter}");
    *counter += 1;
    (format!("\n  {field}: ({name_type}) {cap} (#eq? {cap} \"{name_text}\")"), Some(name_child))
}

/// Predicado de un `impl` de Rust: el tipo implementado y, si lo hay, el trait.
///
/// Con `type:` solo, `impl Foo` y `impl Bar for Foo` producen la misma query y
/// matchean el primero de los dos que aparezca en el archivo.
fn rust_impl_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    let mut out = String::new();
    for field in ["trait", "type"] {
        let Some(child) = node.child_by_field_name(field) else { continue };
        let text = query::escape_query_string(&source[child.byte_range()]);
        let cap = format!("@n{counter}");
        *counter += 1;
        out.push_str(&format!("\n  {field}: ({}) {cap} (#eq? {cap} \"{text}\")", child.kind()));
    }
    (!out.is_empty()).then_some(out)
}

/// Una fila de tabla markdown, identificada por el texto de su primera celda.
///
/// Es el análogo del `id:` de un item de secuencia YAML: la fila no tiene nombre
/// propio, pero en una tabla de spec la primera columna **es** el discriminante —
/// el estado, el campo, el comando del que habla la fila.
///
/// Sin esto una fila de tabla no se puede capturar, y hay que caer a un rango de
/// bytes dentro de la sección: un ancla que se corre con cualquier fila que se
/// agregue más arriba. Ver [`concepts/capture.md`](../../../concepts/capture.md).
fn markdown_table_row_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    let mut c = node.walk();
    let first = node.children(&mut c).find(|n| n.kind() == "pipe_table_cell")?;
    if source[first.byte_range()].trim().is_empty() { return None; }
    let text = query::escape_query_string(&source[first.byte_range()]);
    let cap = format!("@n{counter}");
    *counter += 1;
    Some(format!("\n  (pipe_table_cell) {cap} (#eq? {cap} \"{text}\")"))
}

/// Una característica, una regla o un escenario de Gherkin, identificados por su título.
///
/// El título es el `context` de la línea que empieza con la palabra clave, y esa
/// línea cuelga de un nodo intermedio distinto en cada caso: el encabezado en la
/// característica y la regla, el `scenario` en el escenario. El camino entero entra
/// en el predicado, porque el `context` de un paso tiene otro kind pero el de una
/// línea de `Ejemplos` no.
fn gherkin_title_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    let (wrapper, lines): (&str, &[&str]) = match node.kind() {
        "feature"             => ("feature_header", &["feature_line"]),
        "rule"                => ("rule_header", &["rule_line"]),
        "scenario_definition" => ("scenario", &["scenario_line", "scenario_outline_line"]),
        _ => return None,
    };
    let mut c = node.walk();
    let wrap = node.named_children(&mut c).find(|n| n.kind() == wrapper)?;
    let mut c = wrap.walk();
    let line = wrap.named_children(&mut c).find(|n| lines.contains(&n.kind()))?;
    let mut c = line.walk();
    let title = line.named_children(&mut c).find(|n| n.kind() == "context")?;
    let text = query::escape_query_string(&source[title.byte_range()]);
    let cap = format!("@n{counter}");
    *counter += 1;
    Some(format!("\n  ({wrapper} ({} (context) {cap} (#eq? {cap} \"{text}\")))", line.kind()))
}

/// For a YAML `block_sequence_item`, find the `id:` pair inside and use its value as predicate.
fn yaml_sequence_item_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    // Walk children to find block_node → block_mapping → block_mapping_pair(key=id)
    let id_value = query::escape_query_string(&find_yaml_id_in_sequence_item(node, source)?);
    let cap = format!("@n{counter}");
    *counter += 1;
    Some(format!(
        " (block_node (block_mapping (block_mapping_pair\n  key: (flow_node) @_ (#eq? @_ \"id\")\n  value: (flow_node) {cap} (#eq? {cap} \"{id_value}\"))))"
    ))
}

fn find_yaml_id_in_sequence_item<'a>(node: Node<'a>, source: &str) -> Option<String> {
    if node.kind() == "block_mapping_pair" {
        let key = node.child_by_field_name("key")?;
        if source[key.byte_range()].trim() == "id" {
            let val = node.child_by_field_name("value")?;
            let v = source[val.byte_range()].trim()
                .trim_matches('"').trim_matches('\'').to_string();
            if !v.is_empty() { return Some(v); }
        }
        return None;
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(id) = find_yaml_id_in_sequence_item(child, source) {
                return Some(id);
            }
        }
    }
    None
}

/// For a YAML `block_mapping_pair`, use the key text as predicate.
fn yaml_mapping_pair_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    let key_node = node.child_by_field_name("key")?;
    if source[key_node.byte_range()].trim().is_empty() { return None; }
    let key_text = query::escape_query_string(source[key_node.byte_range()].trim());
    let key_type = key_node.kind();
    let cap = format!("@n{counter}");
    *counter += 1;
    Some(format!("\n  key: ({key_type}) {cap} (#eq? {cap} \"{key_text}\")"))
}

/// For a markdown `section` node, find the heading text to use as predicate.
/// Produces: `(section (atx_heading (inline) @n0 (#eq? @n0 "Heading text"))) @target`
fn markdown_section_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind().contains("heading") {
            // Find inline content inside the heading
            for j in 0..child.child_count() {
                let inline = child.child(j)?;
                if inline.kind() == "inline" || inline.kind().contains("inline") {
                    let text = query::escape_query_string(source[inline.byte_range()].trim());
                    let cap = format!("@n{counter}");
                    *counter += 1;
                    return Some(format!(
                        "\n  ({} (inline) {cap} (#eq? {cap} \"{text}\"))",
                        child.kind()
                    ));
                }
            }
        }
    }
    None
}

fn field_name_for_child<'a>(parent: Node<'a>, child_id: usize) -> Option<&'a str> {
    for i in 0..parent.child_count() as u32 {
        if let Some(c) = parent.child(i as usize) {
            if c.id() == child_id {
                return parent.field_name_for_child(i);
            }
        }
    }
    None
}

fn walk_up_to_anchor<'a>(node: Node<'a>, anchors: &[&str]) -> Option<Node<'a>> {
    let mut current = node;
    loop {
        if anchors.contains(&current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

// ─── recapture ────────────────────────────────────────────────────────────────

pub struct Recaptured {
    pub old_uuid: Option<String>,
    pub new_uuid: String,
    /// El capture nuevo ya existía y se reusó.
    pub reused: bool,
    /// El capture anterior quedó sin referentes.
    pub orphaned: bool,
}

/// Repunta el endpoint `n` de un bilink a un fragmento nuevo.
///
/// Existe porque `UNANCHORED` y `REANCHORED`-sin-fix son estados esperables —una
/// sección renombrada, un test reescrito— y la única alternativa era editar
/// `link.N` a mano. Un reemplazo de texto sobre el campo que define a qué apunta
/// un vínculo no valida nada: ni que el capture exista, ni que esté en la misma
/// capa, ni que el endpoint sea estructural.
///
/// No acepta: dejar el endpoint en su estado real y que un humano confirme el
/// contenido es la misma separación que entre `apply` y `accept`.
pub fn recapture(
    layer:  &Path,
    bilink: &Path,
    n:      u8,
    file:   &str,
    pos:    Option<((usize, usize), (usize, usize))>,
    generator: Option<&dyn CaptureGenerator>,
) -> Result<Recaptured> {
    use bilink_format::BiLink;
    let mut bl = BiLink::load(bilink)?;

    let e = bl.endpoint.get(n);
    let Some(old_id) = e.link.capture_id().map(String::from) else {
        bail!("el endpoint {n} no es estructural (es {}) — no tiene capture que repuntar", e.link);
    };

    let mut dimensions = None;
    let (new_id, _, reused) = match (pos, generator) {
        (Some(sel), Some(g)) => {
            let c = compute_as(layer, file, &[sel], Some(g))?;
            dimensions = Some(c.dimensions);
            c.capture.write_in(layer)?
        }
        (None, Some(g))      => bail!("`--as {}` necesita una posición: genera la query de lo que se señaló", g.name()),
        (Some((start, end)), None) => capture_to_file(layer, file, start, end)?,
        (None, None)         => capture_file_whole(layer, file)?,
    };
    // **Sobre el mismo nodo, `--as` no repunta: declara.** El capture es el del
    // núcleo con generador o sin él, así que es cómo un endpoint pasa a vigilar
    // partes, o una parte que el nodo no tenía al capturarlo.
    let same = old_id == new_id;
    let as_new = generator.map(|g| g.name().to_string());
    if same && (as_new.is_none()
        || (e.r#as == as_new && dimensions.as_ref() == Some(&e.dimensions)))
    {
        bail!("el endpoint {n} ya apunta a ese capture — nada que repuntar ni que declarar");
    }

    let endpoint = bl.endpoint.get_mut(n);
    endpoint.link = format!("capture {new_id}").parse()?;
    if as_new.is_some() {
        endpoint.r#as = as_new;
    }
    if let Some(d) = dimensions {
        endpoint.dimensions = d;
    }
    bl.write(bilink)?;

    // El estado cacheado describía el endpoint de antes: dejarlo mentiría. Con otro
    // capture la ubicación cambió; con el mismo, cambió qué se vigila, y nadie lo
    // aprobó.
    let uuid = bilink.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let mut cache = crate::cache::Cache::load(layer);
    let state = if same { crate::state::EndpointState::Altered } else { crate::state::EndpointState::Relocated };
    cache.set_endpoint_state(&uuid, n, state);
    cache.save(layer)?;

    // ¿El anterior quedó huérfano? Se informa, no se borra: puede tener otros
    // referentes, y borrar por si acaso es peor que dejar basura inocua.
    let orphaned = orphans(layer)?.iter().any(|(id, _)| *id == old_id);

    Ok(Recaptured { old_uuid: Some(old_id), new_uuid: new_id, reused, orphaned })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_cap(layer: &Path, file: &str) -> String {
        Capture { file: file.into(), query: None }
            .write_in(layer).unwrap().0
    }

    /// `prune` conserva lo que alcanza un `link` **o** un `accepted.link`.
    ///
    /// La segunda raíz es la que el formato anterior no tenía: barrer sólo por la
    /// primera borraría el capture que dice dónde estaba lo aceptado.
    #[test]
    fn orphans_walks_both_kinds_of_root() {
        let dir   = tempdir().unwrap();
        let layer = dir.path();
        let vigente  = write_cap(layer, "a.rs");
        let aprobado = write_cap(layer, "b.rs");
        let suelto   = write_cap(layer, "c.rs");

        // Un endpoint que `apply` repuntó: su link apunta a uno y su accepted a otro.
        let mut bl = bilink_format::BiLink::new(
            format!("capture {vigente}").parse().unwrap(),
            "issue 3a".parse().unwrap());
        bl.endpoint.zero.accepted = vec![bilink_format::Accepted {
            agree: Default::default(),
            link: Some(format!("capture {aprobado}").parse().unwrap()),
            hash: "deadbeef".into(),
            hash_ast: None,
            n: None,
            dimensions: Default::default(),
        }];
        bl.write(&bilink_format::BiLink::path_in(layer, "uuid1")).unwrap();

        let huerfanos: Vec<String> = orphans(layer).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(huerfanos, vec![suelto],
            "sólo el capture que nadie nombra; el aprobado sigue vivo");
    }
}

#[cfg(test)]
mod prune_neighbourhood_tests {
    use super::*;
    use bilink_format::{Accepted, BiLink, CaptureSet, DeclaredN, LinkEndpoint, N, Neighbourhood};
    use tempfile::tempdir;

    fn cap(layer: &Path, file: &str) -> String {
        let c = Capture { file: file.into(), query: None };
        let id = c.id();
        c.write_in(layer).unwrap();
        id
    }

    /// **Un capture nombrado sólo por el vecindario no es huérfano.**
    ///
    /// Sin esto el primer `prune` sobre una capa con cierre de firma se lleva los
    /// vecinos, y el `accepted` queda apuntando a captures que no existen.
    #[test]
    fn a_capture_named_only_by_the_neighbourhood_survives() {
        let d = tempdir().unwrap();
        let layer = d.path();
        let frag = cap(layer, "Svc.rs");
        let vecino = cap(layer, "Dto.rs");
        let suelto = cap(layer, "Nadie.rs");

        let mut bl = BiLink::new(format!("capture {frag}").parse().unwrap(), LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).n = Some(DeclaredN::of_level_1(CaptureSet::new(vec![vecino.clone()])));
        bl.write(&BiLink::path_in(layer, "11111111-1111-4111-8111-111111111111")).unwrap();

        let huerfanos: Vec<String> = orphans(layer).unwrap().into_iter().map(|(id, _)| id).collect();
        assert!(huerfanos.contains(&suelto), "el que nadie nombra sí es huérfano: {huerfanos:?}");
        assert!(!huerfanos.contains(&vecino), "el vecino declarado no: {huerfanos:?}");
        assert!(!huerfanos.contains(&frag), "ni el fragmento: {huerfanos:?}");
    }

    /// **Y tampoco el de una decisión desplazada.**
    ///
    /// Con `accepted` como lista, la entrada que no ganó sigue referenciando sus
    /// captures hasta que alguien resuelva. Borrarlos perdería el lado del desacuerdo
    /// que no ganó, que es justo lo que la lista existe para no perder.
    #[test]
    fn the_captures_of_a_displaced_decision_survive() {
        let d = tempdir().unwrap();
        let layer = d.path();
        let frag = cap(layer, "Svc.rs");
        let gano = cap(layer, "Dto.rs");
        let perdio = cap(layer, "DtoViejo.rs");

        let entrada = |v: &str, h: &str| Accepted {
            agree: Default::default(),
            link: Some(format!("capture {frag}").parse().unwrap()),
            hash: h.into(),
            hash_ast: None,
            n: Some(N::of_level_1(Neighbourhood {
                link: CaptureSet::new(vec![v.to_string()]).into(),
                hash: h.into(),
                hash_ast: None,
            })),
            dimensions: Default::default(),
        };

        let mut bl = BiLink::new(format!("capture {frag}").parse().unwrap(), LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).accepted = vec![entrada(&gano, "h1"), entrada(&perdio, "h2")];
        bl.write(&BiLink::path_in(layer, "22222222-2222-4222-8222-222222222222")).unwrap();

        let huerfanos: Vec<String> = orphans(layer).unwrap().into_iter().map(|(id, _)| id).collect();
        assert!(huerfanos.is_empty(), "las dos entradas referencian: {huerfanos:?}");
    }

    /// Una renuncia no nombra ningún capture, y no tiene por qué.
    #[test]
    fn a_decline_names_nothing() {
        let a = Accepted {
            agree: Default::default(), link: None,
            hash: "h".into(), hash_ast: None, n: Some(N::declined()),
            dimensions: Default::default(),
        };
        assert!(accepted_neighbours(&a).is_empty());
    }
}

#[cfg(test)]
mod gherkin_tests {
    use super::*;
    use tempfile::tempdir;

    const FEATURE: &str = "\
# language: es
@modulo:tableros
Característica: Tableros

  Los tableros estadísticos de ui3.

  Regla: Jurisdicción

    @TAB-J-01 @permiso:LEER_JURISDICCION
    Escenario: ver las inscripciones de la jurisdicción
      Dado un usuario con permiso LEER_JURISDICCION en una jurisdicción
      Cuando entra a Tableros y elige \"Inscripciones\"
      Entonces ve la cantidad de alumnos inscriptos

    @TAB-J-02
    Esquema del escenario: ver el tablero de <tablero>
      Dado un usuario con permiso LEER_JURISDICCION en una jurisdicción
      Cuando entra a Tableros y elige \"<tablero>\"
      Entonces ve el tablero

      Ejemplos:
        | tablero        |
        | Calificaciones |

  Regla: Unidad de servicio

    @TAB-U-01
    Escenario: ver las inscripciones de la jurisdicción
      Dado un usuario con permiso LEER_UNIDAD_SERVICIO
      Entonces ve la cantidad de alumnos inscriptos
";

    /// El texto que el capture agarra al señalar `line:col`.
    fn captured(source: &str, line: usize, col: usize) -> Result<(String, String)> {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("tableros.feature"), source).unwrap();
        let (cap, _, ranges) = compute(dir.path(), "tableros.feature", &[((line, col), (line, col))], None)?;
        Ok((cap.query.unwrap(), ranges.text(source)))
    }

    /// Señalar un paso captura el escenario entero: sus etiquetas, su título y sus pasos.
    #[test]
    fn a_step_captures_its_whole_scenario_with_tags() {
        let (query, text) = captured(FEATURE, 12, 9).unwrap();
        assert!(text.starts_with("@TAB-J-01 @permiso:LEER_JURISDICCION"), "sin las etiquetas:\n{text}");
        assert!(text.contains("Entonces ve la cantidad de alumnos inscriptos"), "sin los pasos:\n{text}");
        assert!(!text.contains("TAB-J-02"), "se llevó el escenario siguiente:\n{text}");
        assert!(query.contains("\"ver las inscripciones de la jurisdicción\""), "el título no es el ancla:\n{query}");
        assert!(query.contains("\"Jurisdicción\""), "no lo ata a su regla:\n{query}");
    }

    #[test]
    fn a_scenario_outline_is_anchored_by_its_title() {
        let (query, text) = captured(FEATURE, 17, 7).unwrap();
        assert!(text.starts_with("@TAB-J-02"), "{text}");
        assert!(text.contains("| Calificaciones |"), "sin los ejemplos:\n{text}");
        assert!(query.contains("\"ver el tablero de <tablero>\""), "{query}");
    }

    #[test]
    fn a_rule_is_anchored_by_its_title() {
        let (query, text) = captured(FEATURE, 25, 5).unwrap();
        assert!(text.starts_with("Regla: Unidad de servicio"), "{text}");
        assert!(text.contains("@TAB-U-01"), "{text}");
        assert!(!text.contains("TAB-J-01"), "{text}");
        assert!(query.contains("\"Unidad de servicio\""), "{query}");
    }

    #[test]
    fn a_feature_is_anchored_by_its_title() {
        let (query, text) = captured(FEATURE, 3, 3).unwrap();
        assert!(text.contains("Característica: Tableros"), "{text}");
        assert!(query.contains("\"Tableros\""), "{query}");
    }

    /// Dos escenarios de la misma regla con el mismo título no son ancla.
    #[test]
    fn two_scenarios_with_the_same_title_are_refused() {
        let twice = FEATURE.replace("    @TAB-J-02\n",
            "    Escenario: ver las inscripciones de la jurisdicción\n      Entonces ve otra cosa\n\n    @TAB-J-02\n");
        let err = captured(&twice, 12, 9).unwrap_err().to_string();
        assert!(err.contains("matchea 2 veces") || err.contains("otros nodos"), "{err}");
    }

    /// Con el mismo título en reglas distintas, la regla los distingue.
    #[test]
    fn the_rule_tells_apart_scenarios_with_the_same_title() {
        let (_, text) = captured(FEATURE, 29, 9).unwrap();
        assert!(text.starts_with("@TAB-U-01"), "{text}");
    }
}

#[cfg(test)]
mod one_node_tests {
    use super::*;
    use tempfile::tempdir;

    const DASHBOARD: &str = "\
public class Dashboard {
    public AlertaDto alertas(Short idJur) {
        return svc.alertas(idJur);
    }

    public List<AccionDto> alertas(Integer idUs, List<Long> anios) {
        return svc.acciones(idUs, anios);
    }

    public int otro(int c) {
        return c;
    }
}
";

    fn layer(src: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Dashboard.java"), src).unwrap();
        d
    }

    /// **La sobrecarga ancla en el núcleo, no en un generador.** Sin `--as`, el
    /// método se distingue de su hermano por los tipos de sus parámetros, y la query
    /// sigue teniendo un solo `@target`.
    #[test]
    fn an_overloaded_method_anchors_on_its_parameter_types() {
        let d = layer(DASHBOARD);
        let (cap, _, ranges) = compute(d.path(), "Dashboard.java", &[((6, 5), (6, 5))], None).unwrap();
        let q = cap.query.unwrap();
        assert!(q.contains(r#""^Integer$""#) && q.contains(r#""^List<Long>$""#), "{q}");
        assert_eq!(q.matches("@target").count(), 1, "un solo nodo:\n{q}");
        let last_eq = q.lines().filter(|l| l.contains("#eq?")).last().unwrap_or("");
        assert!(last_eq.contains(r#""alertas""#), "el último #eq? es el nombre:\n{q}");
        assert!(ranges.text(DASHBOARD).contains("svc.acciones"), "es el método entero");
    }

    /// Un método que no se repite ancla sólo en su nombre, como siempre.
    #[test]
    fn a_method_that_is_not_overloaded_anchors_on_its_name() {
        let d = layer(DASHBOARD);
        let (cap, _, _) = compute(d.path(), "Dashboard.java", &[((10, 5), (10, 5))], None).unwrap();
        assert!(!cap.query.unwrap().contains("#match?"));
    }

    /// Dos posiciones en nodos distintos son dos contratos, y un capture es uno.
    #[test]
    fn positions_in_distinct_nodes_are_refused() {
        let d = layer(DASHBOARD);
        let err = compute(d.path(), "Dashboard.java", &[((2, 5), (2, 5)), ((10, 5), (10, 5))], None)
            .unwrap_err().to_string();
        assert!(err.contains("nodos distintos"), "{err}");
        assert!(err.contains("--as"), "el error dice qué hacer:\n{err}");
    }

    /// Dos posiciones en el mismo nodo son ese nodo, una vez.
    #[test]
    fn positions_in_the_same_node_are_that_node() {
        let d = layer(DASHBOARD);
        let dos = compute(d.path(), "Dashboard.java", &[((10, 5), (10, 5)), ((11, 9), (11, 9))], None).unwrap();
        let una = compute(d.path(), "Dashboard.java", &[((10, 5), (10, 5))], None).unwrap();
        assert_eq!(dos.0.id(), una.0.id());
    }
}
