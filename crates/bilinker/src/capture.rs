use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use tree_sitter::{Node, Parser, Point};

use crate::git;
use crate::grammar::{self, stable_anchor_kinds};
use crate::hash;
use bilink_format::{Capture, Ranges};
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
    // `git show <commit>:<path>` resuelve el path contra la **raíz del repo**, no
    // contra el `-C`. Cuando la capa no es la raíz —una capa de specs dentro de
    // un repo mayor— pasar el path relativo a la capa hace fallar el comando.
    let repo_rel = git_path_from_repo_root(layer, &cap.file);
    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "show", &format!("{commit}:{repo_rel}")])
        .output().ok()?;
    if !out.status.success() { return None; }
    let old_source = String::from_utf8(out.stdout).ok()?;

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
    /// Los rangos que la query resuelve, en orden de archivo — uno por `@target`.
    ///
    /// Es lo que se hashea, y es lo que la vista previa marca: sin esto, quien crea
    /// un capture no tiene con qué ver si la query agarró lo que quería, y un
    /// capture es opaco después de escrito.
    pub ranges: Ranges,
}

pub fn capture(
    root: &Path,
    file: &str,
    start: (usize, usize), // (line, col) 1-based
    end: (usize, usize),
) -> Result<CaptureResult> {
    capture_many(root, file, &[(start, end)])
}

/// Un capture de N partes: una query con un `@target` por posición señalada.
///
/// **Las posiciones se descartan.** Sirven para *encontrar* los nodos —cada una
/// resuelve al ancla estable más cercana, igual que una sola— y lo que se guarda es
/// la query. El orden en que se pasan tampoco se guarda: el fragmento va en orden de
/// archivo.
///
/// **La query se ancla una sola vez**, en el ancla estable que contiene a todas las
/// partes. Eso es lo que las ancla *entre sí*: `@RequestMapping` **de la clase que
/// contiene** al método, y no "el primer `@RequestMapping` del archivo". Una lista
/// de queries independientes perdería justamente eso.
pub fn capture_many(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
) -> Result<CaptureResult> {
    capture_as(root, file, sel, None)
}

/// Como [`capture_many`], dejando que un [`CaptureGenerator`] escriba la query.
///
/// **Un generador toma una posición.** Genera *la* query de *eso* que se señaló, y
/// dos cosas señaladas son dos contratos y no uno con dos mitades. Sin generador,
/// las posiciones son las que sean.
pub fn capture_as(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
    generator: Option<&dyn CaptureGenerator>,
) -> Result<CaptureResult> {
    let commit = git::head_commit_for_file(root, file)?;
    let (capture, hash, ranges) = compute(root, file, sel, generator)?;
    Ok(CaptureResult { capture, hash, commit, ranges })
}

/// El capture y su hash, **sin preguntarle nada a git**.
///
/// `capture_as` es esto más el commit del archivo. Van separados porque hay un
/// llamador que no necesita el commit y **no debería necesitar un repo**: el
/// [vecindario](crate::neighbours) acuña un capture por vecino sólo para tener su id
/// y su hash, y pedir git ahí ataría el cálculo del id a que el archivo esté
/// versionado — que no tiene nada que ver.
pub fn compute(
    root: &Path,
    file: &str,
    sel:  &[((usize, usize), (usize, usize))],
    generator: Option<&dyn CaptureGenerator>,
) -> Result<(Capture, String, bilink_format::Ranges)> {
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

    if let Some(g) = generator {
        if sel.len() != 1 {
            bail!(
                "`--as {}` toma una posición y se pasaron {}: un generador escribe \
                 la query de lo que se señaló, y dos cosas señaladas son dos \
                 contratos.",
                g.name(), sel.len()
            );
        }
    }

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

    let ctx = GenCtx { source: &source, lang, anchors, standalone };
    let Generated { query, mut targets } = match generator {
        Some(g) => g.query(&ctx, pointed[0])?,
        None    => generate_default(&ctx, &pointed),
    };
    targets.sort_by_key(|n| (n.start_byte(), n.end_byte()));
    targets.dedup_by_key(|n| n.id());

    // La query tiene que identificar a los nodos seleccionados y a ninguno otro. Un
    // ancla sin discriminante —un `impl` sin tipo, un comentario, un `use`—
    // matchea el primer nodo de ese tipo del archivo, y el capture apuntaría a
    // otra cosa sin fallar. Un capture mal anclado es peor que uno roto: reporta
    // OK sobre una correspondencia que no existe.
    if let Some((outer, inner)) = nested_pair(&targets) {
        bail!(
            "una parte contiene a la otra: el `{}` de la línea {} está adentro del \
             `{}` de la línea {}, y el fragmento las contaría dos veces.\n       \
             Cada posición resuelve al ancla estable más cercana; señalar algo \
             adentro del ancla externa no la hace más chica.",
            inner.kind(), inner.start_position().row + 1,
            outer.kind(), outer.start_position().row + 1,
        );
    }

    let ranges = verify_query_identifies(language.clone(), &source, &query, &targets, file)?;

    // **La selección elige nodos, no rangos de bytes.**
    //
    // Un rango adentro de un nodo se corre con cualquier edición encima suya
    // dentro del mismo nodo, así que su granularidad es ilusoria: se rompe todo
    // el tiempo y hay que repuntarlo. Un ancla de nodo entero es estable, y sus
    // falsas alarmas son honestas — "esto cambió, fijate si tu spec sigue
    // valiendo". Lo que se pierde es atribución, no detección: `hash` dice que
    // cambió y `hash_ast` si fue sólo espaciado.
    //
    // Así que la selección se usa para **encontrar** los nodos y después se
    // descarta. Si hace falta más precisión, la respuesta es una query — que nombre
    // algo más chico, o que nombre varios nodos y deje el resto afuera—, no un
    // recorte sobre una que nombra algo más grande.

    // El recorte lo aplicó la resolución, parte por parte: el hash tiene que ser el
    // del fragmento que `check` va a comparar, no el de los nodos crudos.
    let hash = hash::sha256(ranges.text(&source).as_bytes());

    Ok((Capture { file: file.to_string(), query: Some(query) }, hash, ranges))
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

/// La query que un generador escribió, y los nodos que espera que capture.
///
/// Los nodos viajan con la query para poder **verificar** que resuelve a lo que el
/// generador quiso. Sin eso, un generador con un error escribe un capture que
/// apunta a otra cosa y no falla — la falla que este proyecto llama peor que un
/// capture roto.
pub struct Generated<'t> {
    pub query:   String,
    pub targets: Vec<Node<'t>>,
}

/// Quién escribe la query de un capture.
///
/// **Un generador genera y desaparece.** El capture que queda es una query normal:
/// no dice quién lo generó, no depende de que el generador exista, y se podría
/// haber escrito señalando las posiciones a mano. Eso lo fuerza el formato y no la
/// disciplina — el id es `sha256(file \0 query \0)`, así que no hay dónde dejar el
/// rastro aunque uno quisiera.
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
    /// querías ya te escribió otra cosa, y un capture es opaco después.
    fn applies(&self, file: &str, source: &str, node: Node) -> bool;

    /// La query, y los nodos que espera que capture.
    fn query<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Generated<'t>>;

    /// Cómo se llama este fragmento, en el vocabulario de este generador.
    ///
    /// **Se compone del fragmento, no se guarda.** Un alias guardado es un valor
    /// derivado con vida propia: el día que cambia la ruta sigue diciendo lo viejo, y
    /// lo diría en silencio porque los campos semánticos son inertes. Un rótulo falso
    /// sobre una referencia verificada es peor que no tener rótulo.
    ///
    /// **Y es de cada generador, no del formato.** Un endpoint se nombra por su verbo
    /// y su ruta; una firma, por su método. Un generador que no sepa nombrar devuelve
    /// `None` y el bilink se muestra por UUID, que es lo que se muestra hoy.
    ///
    /// La `query` entra porque a veces el nombre vive ahí y no en el fragmento: donde
    /// la anotación no lleva literal, el nombre del método es el ancla y **no** es
    /// contenido capturado.
    fn alias(&self, _source: &str, _ranges: &Ranges, _query: &str) -> Option<String> { None }
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

/// Sin `--as`: los nodos señalados, enteros.
fn generate_default<'t>(ctx: &GenCtx<'_>, pointed: &[Node<'t>]) -> Generated<'t> {
    Generated {
        query:   pattern_for(ctx, pointed, pointed),
        targets: pointed.to_vec(),
    }
}

/// El patrón que captura `targets`, anclado en lo que contiene a `pointed`.
///
/// **El ancla la deciden las posiciones señaladas, no las partes.** Con
/// `--as interface` las partes son hijos del método, y anclar en su ancestro común
/// daría el método a secas — sin la clase que lo distingue de otro método homónimo
/// en el mismo archivo. Lo que hay que anclar es lo que se señaló.
///
/// **La query se ancla una sola vez**, y eso es lo que ancla las partes *entre sí*:
/// `@RequestMapping` **de la clase que contiene** al método, y no "el primer
/// `@RequestMapping` del archivo". Una lista de queries independientes perdería
/// justamente eso.
///
/// Con una posición sola y una parte sola es el ancla de siempre —la que la
/// envuelve—, y por eso una query de un `@target` sale idéntica a la que salía.
pub fn pattern_for(ctx: &GenCtx<'_>, pointed: &[Node], targets: &[Node]) -> String {
    let lca = pointed.iter().copied().reduce(lowest_common_ancestor)
        .expect("al menos una posición");
    let pattern_root = if ctx.standalone && pointed.len() == 1 {
        lca
    } else {
        let from = if pointed.len() == 1 { lca.parent() } else { Some(lca) };
        from.and_then(|n| walk_up_to_anchor(n, ctx.anchors)).unwrap_or(lca)
    };

    // El camino de cada parte, fundido en un árbol: los tramos compartidos se
    // escriben una vez, que es lo que hace que el patrón sea uno solo.
    let mut kids: std::collections::HashMap<usize, Vec<Node>> = std::collections::HashMap::new();
    for t in targets {
        let path = build_path(pattern_root, *t);
        for pair in path.windows(2) {
            let entry = kids.entry(pair[0].id()).or_default();
            if !entry.iter().any(|n| n.id() == pair[1].id()) {
                entry.push(pair[1]);
            }
        }
    }
    let target_ids: std::collections::HashSet<usize> = targets.iter().map(|n| n.id()).collect();
    emit_pattern(pattern_root, &kids, &target_ids, ctx.source, &mut 0, ctx.lang)
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

/// El primer par de partes donde una contiene a la otra, si lo hay.
///
/// Los nodos vienen ordenados por posición, así que alcanza con comparar cada uno
/// con los siguientes mientras arranquen antes de que él termine.
fn nested_pair<'a>(targets: &[Node<'a>]) -> Option<(Node<'a>, Node<'a>)> {
    for (i, outer) in targets.iter().enumerate() {
        for inner in &targets[i + 1..] {
            if inner.start_byte() >= outer.end_byte() { break; }
            if inner.end_byte() <= outer.end_byte() { return Some((*outer, *inner)); }
        }
    }
    None
}

/// El ancestro común más profundo de dos nodos.
fn lowest_common_ancestor<'a>(a: Node<'a>, b: Node<'a>) -> Node<'a> {
    let mut chain = std::collections::HashSet::new();
    let mut cur = Some(a);
    while let Some(n) = cur {
        chain.insert(n.id());
        cur = n.parent();
    }
    let mut cur = Some(b);
    while let Some(n) = cur {
        if chain.contains(&n.id()) { return n; }
        cur = n.parent();
    }
    b
}

/// El patrón de un nodo: su predicado de nombre, los hijos que llevan a una parte,
/// y su propio `@target` si lo es.
///
/// **Las partes salen en orden de archivo**, y el predicado de nombre con ellas. No
/// es cosmético: tree-sitter exige que los hijos de un patrón vayan en el orden de
/// la gramática, y en Java las anotaciones van antes del nombre. Emitir el nombre
/// primero por costumbre produce un *impossible pattern* en cuanto una parte cae en
/// los modificadores.
fn emit_pattern(
    node:    Node,
    kids:    &std::collections::HashMap<usize, Vec<Node>>,
    targets: &std::collections::HashSet<usize>,
    source:  &str,
    counter: &mut usize,
    lang:    &str,
) -> String {
    let (mut pred, pred_node) = real_name_predicate(node, source, counter, lang);
    let pred_pos = pred_node.map(|n| n.start_byte()).unwrap_or(node.start_byte());

    let mut children: Vec<Node> = kids.get(&node.id()).cloned().unwrap_or_default();

    // **El nombre puede ser a la vez el ancla y una parte.** Pasa con
    // `--as interface`: la firma incluye el nombre, y el nombre es lo que la query
    // usa para encontrar el nodo. Es un solo nodo del AST, así que lleva las dos
    // capturas juntas en vez de emitirse dos veces.
    if let Some(pn) = pred_node {
        if targets.contains(&pn.id()) {
            pred = mark_target_in_predicate(&pred);
            children.retain(|k| k.id() != pn.id());
        }
    }

    let mut parts: Vec<(usize, String)> = Vec::new();
    if !pred.is_empty() { parts.push((pred_pos, pred)); }

    for kid in children {
        let field = field_name_for_child(node, kid.id())
            .map(|f| format!("{f}: "))
            .unwrap_or_default();
        let inner = emit_pattern(kid, kids, targets, source, counter, lang);
        parts.push((kid.start_byte(), format!("\n  {field}{inner}")));
    }
    parts.sort_by_key(|(pos, _)| *pos);

    let body: String = parts.into_iter().map(|(_, s)| s).collect();
    let pattern = format!("({}{})", node.kind(), body);
    if targets.contains(&node.id()) { format!("{pattern} @target") } else { pattern }
}

/// Agrega `@target` a la captura del predicado, antes del `#eq?`.
///
/// Cirugía sobre el string y no una estructura, porque el predicado tiene una sola
/// forma y la escribe una sola función: `… {cap} (#eq? {cap} "…")`.
fn mark_target_in_predicate(pred: &str) -> String {
    match pred.find(" (#eq?") {
        Some(i) => format!("{} @target{}", &pred[..i], &pred[i..]),
        None    => pred.to_string(),
    }
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

/// La query resuelve a los nodos capturados, exactamente una vez.
///
/// Se verifica acá y no en `check` porque acá todavía se puede no escribir: un
/// capture que apunta al nodo equivocado se acepta en OK y no vuelve a mirarse.
///
/// Devuelve los rangos resueltos —ya recortados— para no volver a correr la query:
/// son los mismos que `check` va a comparar, y los que la vista previa marca.
fn verify_query_identifies(
    language:  tree_sitter::Language,
    source:    &str,
    query_str: &str,
    targets:   &[Node],
    file:      &str,
) -> Result<Ranges> {
    let esperado: Vec<(usize, usize)> = targets.iter()
        .map(|t| query::trim_edges(source, t.start_byte(), t.end_byte()))
        .collect();

    let hits = query::find_all_fragments(language, source, query_str)?;
    let n = targets.len();
    let kinds = || targets.iter().map(|t| t.kind()).collect::<Vec<_>>().join(", ");

    match hits.as_slice() {
        [] => bail!("la query generada no matchea ningún nodo en {file}:\n{query_str}"),
        [f] => {
            let got: Vec<(usize, usize)> = f.ranges.parts().iter()
                .map(|r| (r.start, r.end))
                .collect();
            if got == esperado {
                return Ok(f.ranges.clone());
            }
            if got.len() != n {
                bail!(
                    "la query generada captura {} parte(s) y se señalaron {n} en {file}. \
                     Las anclas `{}` no caen todas bajo un mismo patrón:\n{query_str}",
                    got.len(), kinds()
                );
            }
            bail!(
                "la query generada apunta a otros nodos: {} en vez de {}. \
                 El ancla `{}` no tiene con qué distinguirse en {file}:\n{query_str}",
                fmt_ranges(&got), fmt_ranges(&esperado), kinds()
            )
        }
        hits => bail!(
            "la query generada matchea {} veces. El ancla `{}` no tiene con qué \
             distinguirse en {file}:\n{query_str}\n\n\
             Seleccionar un nodo con nombre propio adentro —una función, un método— \
             da un ancla única sin inventar un criterio.",
            hits.len(), kinds()
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
) -> Result<Recaptured> {
    use bilink_format::BiLink;
    let mut bl = BiLink::load(bilink)?;

    let e = bl.endpoint.get(n);
    let Some(old_id) = e.link.capture_id().map(String::from) else {
        bail!("el endpoint {n} no es estructural (es {}) — no tiene capture que repuntar", e.link);
    };

    let (new_id, _, reused) = match pos {
        Some((start, end)) => capture_to_file(layer, file, start, end)?,
        None               => capture_file_whole(layer, file)?,
    };
    if old_id == new_id {
        bail!("el endpoint {n} ya apunta a ese capture — nada que repuntar");
    }

    bl.endpoint.get_mut(n).link = format!("capture {new_id}").parse()?;
    bl.write(bilink)?;

    // El estado cacheado describía el capture viejo: dejarlo mentiría.
    let uuid = bilink.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
    let mut cache = crate::cache::Cache::load(layer);
    cache.set_endpoint_state(&uuid, n, crate::state::EndpointState::Relocated);
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
        };
        assert!(accepted_neighbours(&a).is_empty());
    }
}
