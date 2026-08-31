use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use tree_sitter::{Node, Parser, Point};

use crate::git;
use crate::grammar::{self, stable_anchor_kinds};
use crate::hash;
use bilink_format::{ByteRange, Capture};
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
            if let Some(id) = e.accepted.as_ref().and_then(|a| a.link.as_ref()).and_then(|l| l.capture_id()) {
                alive.insert(id.to_string());
            }
        }
    }

    Ok(Capture::all_in(layer)?.into_iter().filter(|(id, _)| !alive.contains(id)).collect())
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
            let (start, end, _) =
                crate::query::find_target_with_sexp(language, &old_source, q).ok()??;
            let (s, e) = (start, end);
            if s > e || e > old_source.len() { return None; }
            old_source[s..e].to_string()
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

/// Byte range absoluto del fragmento en su archivo, resolviendo la query.
pub fn absolute_range(layer: &Path, cap: &Capture) -> Result<Option<ByteRange>> {
    let path = layer.join(&cap.file);
    if !path.exists() { return Ok(None); }
    let source = std::fs::read_to_string(&path)?;

    let Some(query_str) = &cap.query else {
        return Ok(Some(ByteRange { start: 0, end: source.len() }));
    };
    let lang     = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;
    let Some((node_start, node_end, _)) =
        crate::query::find_target_with_sexp(language, &source, query_str)? else {
        return Ok(None);
    };
    Ok(Some(ByteRange { start: node_start, end: node_end }))
}

pub struct CaptureResult {
    pub capture: Capture,
    pub hash: String,
    pub commit: String,
}

pub fn capture(
    root: &Path,
    file: &str,
    start: (usize, usize), // (line, col) 1-based
    end: (usize, usize),
) -> Result<CaptureResult> {
    let commit = git::head_commit_for_file(root, file)?;
    let file_path = root.join(file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let lang = grammar::language_for_file(file);
    let language = grammar::for_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(&source, None).context("parse failed")?;

    let start_point = Point { row: start.0 - 1, column: start.1 - 1 };
    let end_point   = Point { row: end.0 - 1,   column: end.1 - 1 };

    let root_node = tree.root_node();
    let node = root_node
        .named_descendant_for_point_range(start_point, end_point)
        .context("no named node at selection")?;

    let anchors = stable_anchor_kinds(lang);
    let target = walk_up_to_anchor(node, anchors).unwrap_or(node);

    let anchor = target.parent()
        .and_then(|p| walk_up_to_anchor(p, anchors));

    // For YAML block_sequence_item: use it directly as @target (contains the whole item)
    let (target, anchor) = if target.kind() == "block_sequence_item" {
        (target, None)
    } else if let Some(a) = anchor {
        if a.kind() == "block_sequence_item" {
            (a, None)
        } else {
            (target, Some(a))
        }
    } else {
        (target, anchor)
    };

    let query = match anchor {
        None => query_for_node(target, &source, &mut 0, lang),
        Some(a) if a.id() == target.id() => query_for_node(target, &source, &mut 0, lang),
        Some(a) => {
            let path = build_path(a, target);
            query_from_path(&path, &source, &mut 0, lang)
        }
    };

    // La query tiene que identificar al nodo seleccionado y a ninguno otro. Un
    // ancla sin discriminante —un `impl` sin tipo, un comentario, un `use`—
    // matchea el primer nodo de ese tipo del archivo, y el capture apuntaría a
    // otra cosa sin fallar. Un capture mal anclado es peor que uno roto: reporta
    // OK sobre una correspondencia que no existe.
    verify_query_identifies(language.clone(), &source, &query, target, file)?;

    // **La selección elige un nodo, no un rango de bytes.**
    //
    // Un rango adentro de un nodo se corre con cualquier edición encima suya
    // dentro del mismo nodo, así que su granularidad es ilusoria: se rompe todo
    // el tiempo y hay que repuntarlo. Un ancla de nodo entero es estable, y sus
    // falsas alarmas son honestas — "esto cambió, fijate si tu spec sigue
    // valiendo". Lo que se pierde es atribución, no detección: `hash` dice que
    // cambió y `hash_ast` si fue sólo espaciado.
    //
    // Así que la selección se usa para **encontrar** el nodo y después se
    // descarta. Si hace falta más precisión, la respuesta es una query que
    // nombre algo más chico, no un recorte sobre una que nombra algo más grande.

    // El mismo recorte que aplica la resolución: el hash tiene que ser el del
    // fragmento que `check` va a comparar, no el del nodo crudo.
    let (frag_start, frag_end) =
        crate::query::trim_edges(&source, target.start_byte(), target.end_byte());
    let fragment = &source[frag_start..frag_end.min(source.len())];
    let hash = hash::sha256(fragment.as_bytes());

    Ok(CaptureResult {
        capture: Capture {
            file:   file.to_string(),
            query:  Some(query),
        },
        hash,
        commit,
    })
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

/// La query resuelve al nodo capturado, exactamente una vez.
///
/// Se verifica acá y no en `check` porque acá todavía se puede no escribir: un
/// capture que apunta al nodo equivocado se acepta en OK y no vuelve a mirarse.
fn verify_query_identifies(
    language: tree_sitter::Language,
    source:   &str,
    query_str: &str,
    target:   Node,
    file:     &str,
) -> Result<()> {
    let hits = query::find_all_targets(language, source, query_str)?;
    match hits.as_slice() {
        [m] if m.start == target.start_byte() && m.end == target.end_byte() => Ok(()),
        [] => bail!(
            "la query generada no matchea ningún nodo en {file}:\n{query_str}"
        ),
        [m] => bail!(
            "la query generada apunta a otro nodo: bytes {}~{} en vez de {}~{}. \
             El ancla `{}` no tiene con qué distinguirse en {file}:\n{query_str}",
            m.start, m.end, target.start_byte(), target.end_byte(), target.kind()
        ),
        hits => bail!(
            "la query generada matchea {} nodos. El ancla `{}` no tiene con qué \
             distinguirse en {file}:\n{query_str}\n\n\
             Seleccionar un nodo con nombre propio adentro —una función, un método— \
             da un ancla única sin inventar un criterio.",
            hits.len(), target.kind()
        ),
    }
}

fn query_for_node(node: Node, source: &str, counter: &mut usize, lang: &str) -> String {
    let name_pred = real_name_predicate(node, source, counter, lang);
    format!("({}{}) @target", node.kind(), name_pred)
}

fn query_from_path(path: &[Node], source: &str, counter: &mut usize, lang: &str) -> String {
    assert!(!path.is_empty());
    let node = path[0];
    let name_pred = real_name_predicate(node, source, counter, lang);

    if path.len() == 1 {
        return format!("({}{}) @target", node.kind(), name_pred);
    }

    let next = path[1];
    let field = field_name_for_child(node, next.id())
        .map(|f| format!("{f}: "))
        .unwrap_or_default();

    let inner = query_from_path(&path[1..], source, counter, lang);
    format!("({}{}\n  {field}{inner})", node.kind(), name_pred)
}

fn real_name_predicate(node: Node, source: &str, counter: &mut usize, lang: &str) -> String {
    // Special case: markdown section — use heading text as predicate
    if node.kind() == "section" {
        if let Some(pred) = markdown_section_predicate(node, source, counter) {
            return pred;
        }
    }
    // Special case: markdown pipe_table_row — la primera celda lo discrimina
    if node.kind() == "pipe_table_row" {
        if let Some(pred) = markdown_table_row_predicate(node, source, counter) {
            return pred;
        }
    }
    // Special case: YAML block_sequence_item — use id: or first key as predicate
    if node.kind() == "block_sequence_item" {
        if let Some(pred) = yaml_sequence_item_predicate(node, source, counter) {
            return pred;
        }
    }
    // Special case: YAML block_mapping_pair — use key as predicate
    if node.kind() == "block_mapping_pair" {
        if let Some(pred) = yaml_mapping_pair_predicate(node, source, counter) {
            return pred;
        }
    }
    // Special case: Rust impl_item — no tiene campo `name`; lo identifica el tipo
    // y, si es la implementación de un trait, el trait.
    if lang == "rust" && node.kind() == "impl_item" {
        if let Some(pred) = rust_impl_predicate(node, source, counter) {
            return pred;
        }
    }
    // El campo que lleva el nombre depende del lenguaje y del tipo de nodo; la
    // tabla vive en `grammar`. `name` es el caso mayoritario y el default.
    let field = grammar::name_field(lang, node.kind()).unwrap_or("name");
    let Some(name_child) = node.child_by_field_name(field) else {
        return String::new();
    };
    let name_type = name_child.kind();
    let name_text = &source[name_child.byte_range()];
    let cap = format!("@n{counter}");
    *counter += 1;
    format!("\n  {field}: ({name_type}) {cap} (#eq? {cap} \"{name_text}\")")
}

/// Predicado de un `impl` de Rust: el tipo implementado y, si lo hay, el trait.
///
/// Con `type:` solo, `impl Foo` y `impl Bar for Foo` producen la misma query y
/// matchean el primero de los dos que aparezca en el archivo.
fn rust_impl_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    let mut out = String::new();
    for field in ["trait", "type"] {
        let Some(child) = node.child_by_field_name(field) else { continue };
        let text = &source[child.byte_range()];
        if text.contains('"') { continue; }
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
    let text = &source[first.byte_range()];
    // Las comillas romperían la query, y una celda que las lleve no se puede
    // usar como predicado: mejor fallar acá que emitir algo que no parsea.
    if text.contains('"') || text.trim().is_empty() { return None; }
    let cap = format!("@n{counter}");
    *counter += 1;
    Some(format!("\n  (pipe_table_cell) {cap} (#eq? {cap} \"{text}\")"))
}

/// For a YAML `block_sequence_item`, find the `id:` pair inside and use its value as predicate.
fn yaml_sequence_item_predicate(node: Node, source: &str, counter: &mut usize) -> Option<String> {
    // Walk children to find block_node → block_mapping → block_mapping_pair(key=id)
    let id_value = find_yaml_id_in_sequence_item(node, source)?;
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
                .trim_matches('"').trim_matches('\'').replace('"', "\\\"");
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
    let key_text = source[key_node.byte_range()].trim().replace('"', "\\\"");
    if key_text.is_empty() { return None; }
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
                    let text = source[inline.byte_range()].trim().replace('"', "\\\"");
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
        bl.endpoint.zero.accepted = Some(bilink_format::Accepted {
            agree: Default::default(),
            link: Some(format!("capture {aprobado}").parse().unwrap()),
            hash: "deadbeef".into(),
            hash_ast: None,
        });
        bl.write(&bilink_format::BiLink::path_in(layer, "uuid1")).unwrap();

        let huerfanos: Vec<String> = orphans(layer).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(huerfanos, vec![suelto],
            "sólo el capture que nadie nombra; el aprobado sigue vivo");
    }
}
