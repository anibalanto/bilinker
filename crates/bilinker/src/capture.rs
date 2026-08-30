use std::path::{Path, PathBuf};
use anyhow::{bail, Context, Result};
use tree_sitter::{Node, Parser, Point};

use crate::git;
use crate::grammar::{self, stable_anchor_kinds};
use crate::hash;
use crate::link::{ByteRange, StructuralRef};
use crate::query;

/// El formato del `.capture` vive en `bilink-format`; acá está el algoritmo que lo
/// produce. Se re-exporta para que el resto del crate siga diciendo
/// `capture::CaptureFile` sin saber de dónde sale el tipo.
pub use bilink_format::capture::{sref_of, CaptureFile, CaptureState};

/// Busca un capture de la capa con la misma referencia exacta.
///
/// La igualdad es `(file, query, offset)`: referencias idénticas describen la
/// misma ubicación, así que comparten capture. Es el mismo criterio que usa la
/// migración — si no, cada cadena nueva volvería a duplicar lo que aquélla unificó.
pub fn find_equivalent(layer: &Path, sref: &StructuralRef) -> Option<String> {
    CaptureFile::all_in(layer).ok()?.into_iter()
        .find(|c| c.sref == *sref)
        .map(|c| c.uuid)
}

/// Captura un fragmento y persiste el `.capture` en la capa.
///
/// Devuelve el UUID del capture creado, listo para referenciar desde un `link.N`.
pub fn capture_to_file(
    layer: &Path,
    file:  &str,
    start: (usize, usize),
    end:   (usize, usize),
    now:   &str,
) -> Result<(String, PathBuf, bool)> {
    let result = capture(layer, file, start, end)?;

    if let Some(uuid) = find_equivalent(layer, &result.endpoint) {
        return Ok((uuid.clone(), CaptureFile::path_in(layer, &uuid), true));
    }

    let uuid = uuid::Uuid::new_v4().to_string();
    let mut cap = CaptureFile {
        uuid:        uuid.clone(),
        sref:        result.endpoint,
        range:       None,
        state:       Some(CaptureState::Resolved),
        resolved_at: Some(now.to_string()),
    };
    cap.range = absolute_range(layer, &cap.sref)?;

    let path = cap.write_in(layer)?;
    Ok((uuid, path, false))
}

/// Captura un archivo completo, sin query.
pub fn capture_file_whole(layer: &Path, file: &str, now: &str) -> Result<(String, PathBuf, bool)> {
    let sref = StructuralRef { file: file.to_string(), query: None, range: None };
    if let Some(uuid) = find_equivalent(layer, &sref) {
        return Ok((uuid.clone(), CaptureFile::path_in(layer, &uuid), true));
    }

    let uuid = uuid::Uuid::new_v4().to_string();
    let mut cap = CaptureFile {
        uuid:        uuid.clone(),
        sref,
        range:       None,
        state:       Some(CaptureState::Resolved),
        resolved_at: Some(now.to_string()),
    };
    cap.range = absolute_range(layer, &cap.sref)?;
    let path = cap.write_in(layer)?;
    Ok((uuid, path, false))
}

/// Captures de la capa que ningún `.bilink` referencia.
///
/// Un capture huérfano no rompe nada: se resuelve en cada `check` sin que nadie
/// lea el resultado. Limpiarlo es higiene, no reparación.
pub fn orphans(layer: &Path) -> Result<Vec<CaptureFile>> {
    use std::collections::HashSet;
    let mut referenced: HashSet<String> = HashSet::new();

    for path in crate::bilink::walkdir(&layer.join(".bilink"))? {
        if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        let Ok(bl) = crate::bilink::BiLinkFile::load(&path) else { continue };
        for n in [0u8, 1u8] {
            if let Some(uuid) = bl.link(n).capture_uuid() {
                referenced.insert(uuid.to_string());
            }
        }
    }

    Ok(CaptureFile::all_in(layer)?.into_iter()
        .filter(|c| !referenced.contains(&c.uuid))
        .collect())
}

/// Path de un archivo de la capa, relativo a la raíz del repo git.
///
/// Los paths de un capture son relativos a su capa, pero `git show <commit>:<p>`
/// los resuelve contra la raíz del repo. Cuando coinciden da igual; cuando la
/// capa está anidada dentro de un repo mayor, no.
fn git_path_from_repo_root(layer: &Path, file: &str) -> String {
    let top = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());

    match top {
        Some(t) => {
            let root = Path::new(t.trim());
            match layer.strip_prefix(root) {
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
    sref:          &StructuralRef,
    commit:        &str,
    expected_hash: Option<&str>,
) -> Option<String> {
    // `git show <commit>:<path>` resuelve el path contra la **raíz del repo**, no
    // contra el `-C`. Cuando la capa no es la raíz —una capa de specs dentro de
    // un repo mayor— pasar el path relativo a la capa hace fallar el comando.
    let repo_rel = git_path_from_repo_root(layer, &sref.file);
    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "show", &format!("{commit}:{repo_rel}")])
        .output().ok()?;
    if !out.status.success() { return None; }
    let old_source = String::from_utf8(out.stdout).ok()?;

    let text = match &sref.query {
        None => old_source.clone(),
        Some(q) => {
            let lang     = grammar::language_for_file(&sref.file);
            let language = grammar::for_language(lang).ok()?;
            let (start, end, _) =
                crate::query::find_target_with_sexp(language, &old_source, q).ok()??;
            let (s, e) = match &sref.range {
                Some(r) => (start + r.start, (start + r.end).min(old_source.len())),
                None    => (start, end),
            };
            if s > e || e > old_source.len() { return None; }
            old_source[s..e].to_string()
        }
    };

    match expected_hash {
        Some(h) if hash::sha256(text.as_bytes()) != h => None,
        _ => Some(text),
    }
}

/// Byte range absoluto del fragmento en su archivo, resolviendo la query.
pub fn absolute_range(layer: &Path, sref: &StructuralRef) -> Result<Option<ByteRange>> {
    let path = layer.join(&sref.file);
    if !path.exists() { return Ok(None); }
    let source = std::fs::read_to_string(&path)?;

    let Some(query_str) = &sref.query else {
        return Ok(Some(ByteRange { start: 0, end: source.len() }));
    };
    let lang     = grammar::language_for_file(&sref.file);
    let language = grammar::for_language(lang)?;
    let Some((node_start, node_end, _)) =
        crate::query::find_target_with_sexp(language, &source, query_str)? else {
        return Ok(None);
    };
    Ok(Some(match &sref.range {
        Some(off) => ByteRange {
            start: node_start + off.start,
            end:   (node_start + off.end).min(source.len()),
        },
        None => ByteRange { start: node_start, end: node_end },
    }))
}

pub struct CaptureResult {
    pub endpoint: StructuralRef,
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

    let start_byte = byte_for_point(&source, start_point);
    let end_byte   = byte_for_point(&source, end_point);
    // Single-point selection → capture the whole anchor; use relative offsets only for real ranges.
    let range = if start_byte == end_byte {
        None
    } else if start_byte != target.start_byte() || end_byte != target.end_byte() {
        let rel_start = start_byte.saturating_sub(target.start_byte());
        let rel_end   = end_byte.saturating_sub(target.start_byte());
        Some(crate::link::ByteRange { start: rel_start, end: rel_end })
    } else {
        None
    };

    let (frag_start, frag_end) = match &range {
        Some(r) => (target.start_byte() + r.start, target.start_byte() + r.end),
        None    => (target.start_byte(), target.end_byte()),
    };
    let fragment = &source[frag_start..frag_end.min(source.len())];
    let hash = hash::sha256(fragment.as_bytes());

    Ok(CaptureResult {
        endpoint: StructuralRef {
            file: file.to_string(),
            query: Some(query),
            range,
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

fn byte_for_point(source: &str, point: Point) -> usize {
    let mut line = 0;
    for (i, c) in source.char_indices() {
        if line == point.row {
            return i + point.column.min(source.len() - i);
        }
        if c == '\n' {
            line += 1;
        }
    }
    source.len()
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
    now:    &str,
) -> Result<Recaptured> {
    let mut bl = crate::bilink::BiLinkFile::load(bilink)?;

    // Un endpoint layer o task no tiene capture que repuntar.
    if !bl.link(n).is_structural() {
        anyhow::bail!(
            "link.{n} no es un endpoint estructural (es {}) — no tiene capture que repuntar",
            bl.link(n)
        );
    }
    let old_uuid = bl.link(n).capture_uuid().map(String::from);

    let (new_uuid, _, reused) = match pos {
        Some((start, end)) => capture_to_file(layer, file, start, end, now)?,
        None               => capture_file_whole(layer, file, now)?,
    };

    if old_uuid.as_deref() == Some(new_uuid.as_str()) {
        anyhow::bail!("link.{n} ya apunta a ese capture — nada que repuntar");
    }

    *bl.link_mut(n) = crate::link::LinkEndpoint::Capture(new_uuid.clone());
    // El estado anterior describía el capture viejo: dejarlo mentiría hasta el
    // próximo `check`.
    bl.set_state(n, None);
    bl.resolved_at = Some(now.to_string());
    bl.write(bilink)?;

    // ¿El capture anterior quedó huérfano? Se informa, no se borra: puede tener
    // otros referentes, y borrar por si acaso es peor que dejar basura inocua.
    let orphaned = match &old_uuid {
        None => false,
        Some(u) => orphans(layer)?.iter().any(|c| c.uuid == *u),
    };

    Ok(Recaptured { old_uuid, new_uuid, reused, orphaned })
}

#[cfg(test)]
mod capture_lookup_tests {
    use super::*;
    use bilink_format::link::StructuralRef;
    use tempfile::tempdir;

    fn sref(file: &str, query: Option<&str>) -> StructuralRef {
        StructuralRef { file: file.into(), query: query.map(String::from), range: None }
    }

    fn write_cap(layer: &Path, uuid: &str, s: StructuralRef) {
        CaptureFile { uuid: uuid.into(), sref: s, range: None, state: None, resolved_at: None }
            .write_in(layer).unwrap();
    }

    #[test]
    fn find_equivalent_matches_identical_reference() {
        let dir = tempdir().unwrap();
        write_cap(dir.path(), "cap-a", sref("a.rs", Some("(function_item) @target")));
        let found = find_equivalent(dir.path(), &sref("a.rs", Some("(function_item) @target")));
        assert_eq!(found.as_deref(), Some("cap-a"));
    }

    #[test]
    fn find_equivalent_ignores_different_reference() {
        let dir = tempdir().unwrap();
        write_cap(dir.path(), "cap-a", sref("a.rs", Some("(function_item) @target")));
        assert!(find_equivalent(dir.path(), &sref("a.rs", Some("(struct_item) @target"))).is_none());
        assert!(find_equivalent(dir.path(), &sref("b.rs", Some("(function_item) @target"))).is_none());
    }

    #[test]
    fn orphans_excludes_referenced_captures() {
        let dir   = tempdir().unwrap();
        let layer = dir.path();
        write_cap(layer, "cap-usado",    sref("a.rs", None));
        write_cap(layer, "cap-huerfano", sref("b.rs", None));

        crate::bilink::BiLinkFile::new("uuid1",
            crate::link::LinkEndpoint::Capture("cap-usado".into()),
            crate::link::LinkEndpoint::Issue("3a".into()))
            .write(&layer.join(".bilink/uuid1.bilink")).unwrap();

        let o = orphans(layer).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].uuid, "cap-huerfano");
    }
}
