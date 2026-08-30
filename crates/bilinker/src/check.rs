//! `bilinker check` — verifica, y **no escribe ni un byte en git**.
//!
//! Opera en dos pasos y sobre **dos dimensiones**. Primero resuelve el capture
//! —dónde está el fragmento—, después compara contra `accepted` —dónde se aprobó
//! que estuviera, y qué se aprobó que dijera. Todo lo que produce va a la
//! [cache](crate::cache).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use bilink_format::bilink::bilink_files;
use bilink_format::{BiLink, ByteRange, Capture, LinkEndpoint};

use crate::cache::Cache;
use crate::state::{CaptureState, EndpointState};
use crate::{grammar, hash, query};

pub struct CheckResult {
    pub uuid: String,
    pub state0: EndpointState,
    pub state1: EndpointState,
}

impl CheckResult {
    /// Los dos endpoints en OK. **Decide qué se imprime.**
    pub fn all_ok(&self) -> bool { self.state0.is_ok() && self.state1.is_ok() }

    /// Nada que exija una decisión humana. **Decide el código de salida.**
    pub fn is_clean(&self) -> bool { self.state0.is_clean() && self.state1.is_clean() }
}

/// Verifica una capa y deja el resultado en la cache.
pub fn check(root: &Path, path: &Path) -> Result<Vec<CheckResult>> {
    let layer = if path.join(".bilink").is_dir() { path.to_path_buf() } else { root.to_path_buf() };
    // **La cache se invalida sola al cambiar de rama.** Sin esto una capa devuelve
    // estados de la rama anterior en silencio: `git checkout` no toca `.bilink/`, y
    // los estados cacheados describen bilinks que ya no están en el árbol.
    let ref_commit = Cache::ref_commit_of(&layer);
    let mut cache = Cache::load_for(&layer, ref_commit.as_deref());
    cache.ref_commit = ref_commit;
    let mut out = Vec::new();

    // Un mismo capture se resuelve **una sola vez**, aunque lo referencien varios
    // endpoints. La comparación contra `accepted` sí corre por endpoint, porque
    // cada uno tiene el suyo.
    let mut resolved: HashMap<String, (CaptureState, Option<ByteRange>)> = HashMap::new();

    for path in bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        let mut states = [EndpointState::Pending; 2];
        for n in [0u8, 1u8] {
            states[n as usize] = check_endpoint(&layer, &bl, uuid, n, &mut resolved, &mut cache)?;
            cache.set_endpoint_state(uuid, n, states[n as usize]);
        }
        out.push(CheckResult { uuid: uuid.to_string(), state0: states[0], state1: states[1] });
    }

    for (id, (state, range)) in &resolved {
        cache.set_capture(id, *state, range.as_ref());
    }
    cache.save(&layer)?;
    out.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    Ok(out)
}

fn check_endpoint(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    resolved: &mut HashMap<String, (CaptureState, Option<ByteRange>)>,
    cache: &mut Cache,
) -> Result<EndpointState> {
    let e = bl.endpoint.get(n);
    match &e.link {
        LinkEndpoint::Path(p)   => check_path(layer, p, uuid, e.accepted.as_ref()),
        LinkEndpoint::Issue(id) => check_issue(layer, id, e.accepted.as_ref()),
        LinkEndpoint::Capture(cap_id) => {
            let cap = match Capture::load_in(layer, cap_id) {
                Ok(c) => c,
                // El capture que el link nombra no está: no hay ubicación que evaluar.
                Err(_) => return Ok(EndpointState::Unresolved),
            };

            // La resolución se cachea por capture, pero la aceptación es por
            // endpoint: dos endpoints sobre el mismo capture pueden haber aprobado
            // contenidos distintos, y el que resuelva primero es el que aporta el
            // texto para puntuar un reanclaje. Es una aproximación consciente —
            // resolver una vez por capture es lo que la spec pide— y sólo afecta a
            // qué candidato gana en un caso ya ambiguo.
            let (state, range) = match resolved.get(cap_id) {
                Some(v) => v.clone(),
                None => {
                    let v = resolve_capture(layer, &cap, e.accepted.as_ref(), cache.commit(uuid, n))?;
                    resolved.insert(cap_id.clone(), v.clone());
                    v
                }
            };
            if !state.is_resolved() {
                return Ok(EndpointState::Unresolved);
            }

            let Some(accepted) = &e.accepted else { return Ok(EndpointState::Pending) };

            // ── dimensión de ubicación ────────────────────────────────────────
            //
            // Dos ids: no abre ningún archivo. Por eso se decide **siempre**, incluso
            // donde la otra dimensión degrada por no poder recuperar el texto aceptado.
            if accepted.link.as_ref() != Some(&e.link) {
                return Ok(EndpointState::Relocated);
            }

            // ── dimensión de contenido ────────────────────────────────────────
            //
            // El commit se deriva si la cache no lo tiene. Sin él, `accepted.hash`
            // es un hash que no se puede resolver a texto, y sin el texto aceptado
            // EXPANDED, DISPLACED y REANCHORED degradan todos a ALTERED — o sea,
            // un clon fresco perdería las tres distinciones.
            let cached_commit = cache.commit(uuid, n).map(str::to_string);
            let cached_state  = cache.endpoint_state(uuid, n);
            let mut derived: Option<Option<String>> = None;
            let state = {
                let mut derive = || -> Option<String> {
                    derived
                        .get_or_insert_with(|| match &cached_commit {
                            Some(c) => Some(c.clone()),
                            None => crate::capture::derive_commit(layer, &cap, &accepted.hash),
                        })
                        .clone()
                };
                let mut src = CommitSource {
                    cached: cached_commit.as_deref(),
                    derive: &mut derive,
                };
                compare_content(layer, &cap, accepted, range.as_ref(), &mut src, cached_state)?
            };
            // Lo derivado se guarda: el walk cuesta un `git show` por commit y el
            // mismo endpoint se consulta más de una vez en una corrida.
            if let Some(Some(c)) = &derived {
                if cached_commit.is_none() { cache.set_commit(uuid, n, c); }
            }
            Ok(state)
        }
    }
}

// ─── dimensión 1: ¿dónde está? ────────────────────────────────────────────────

/// Resuelve un capture contra el árbol actual.
///
/// Recibe `accepted` porque **REANCHORED lo necesita**: para decidir si un nodo con
/// otro nombre es el mismo fragmento hay que compararlo contra el texto aceptado, y
/// ese texto se recupera de git con `(hash, commit)`. Sin eso el anchor renombrado
/// se reporta como UNANCHORED —"no está"— en vez de "está, con otro nombre".
///
/// Es la única cosa de la aceptación que la dimensión de ubicación mira, y sólo para
/// puntuar: el estado que devuelve sigue siendo sobre dónde está el fragmento.
pub(crate) fn resolve_capture(
    layer: &Path,
    cap: &Capture,
    accepted: Option<&bilink_format::Accepted>,
    commit: Option<&str>,
) -> Result<(CaptureState, Option<ByteRange>)> {
    let path = layer.join(&cap.file);

    if !path.exists() {
        if git_renamed_to(layer, &cap.file).is_some() {
            return Ok((CaptureState::Moved, None));
        }
        if git_knows_file(layer, &cap.file) {
            return Ok((CaptureState::Deleted, None));
        }
        return Ok((CaptureState::Broken, None));
    }

    let Ok(source) = std::fs::read_to_string(&path) else {
        return Ok((CaptureState::Broken, None));
    };

    // Sin query, el capture es el archivo entero.
    let Some(query_str) = &cap.query else {
        return Ok((CaptureState::Resolved, Some(ByteRange { start: 0, end: source.len() })));
    };

    let lang     = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;

    let Some((node_start, node_end, _)) =
        query::find_target_with_sexp(language.clone(), &source, query_str)?
    else {
        // La query no matchea: ¿el anchor se renombró, o el fragmento desapareció?
        let hash = accepted.map(|a| a.hash.as_str());
        if find_renamed_anchor(layer, language, &source, query_str, cap, hash, commit)?.is_some() {
            return Ok((CaptureState::Reanchored, None));
        }
        if git_fragment_vanished(layer, &cap.file, hash) {
            return Ok((CaptureState::Deleted, None));
        }
        return Ok((CaptureState::Unanchored, None));
    };

    Ok((CaptureState::Resolved, Some(ByteRange { start: node_start, end: node_end })))
}

// ─── dimensión 2: ¿coincide con lo aceptado? ──────────────────────────────────

/// De dónde sale el commit del contenido aceptado.
///
/// Son dos cosas distintas y por eso van separadas: el que la cache **ya tenía**
/// habilita el fast path y nada más, y el derivado cuesta un walk por la historia
/// del archivo. Derivarlo antes de saber si hace falta lo cobraría también sobre
/// los endpoints OK, que son los que nunca lo necesitan —el hash decide antes—, y
/// el costo dejaría de estar acotado por lo que está roto.
pub(crate) struct CommitSource<'a> {
    pub cached: Option<&'a str>,
    /// Se llama a lo sumo una vez, y sólo después de que el hash dijo que hay drift.
    pub derive: &'a mut dyn FnMut() -> Option<String>,
}

pub(crate) fn compare_content(
    layer: &Path,
    cap: &Capture,
    accepted: &bilink_format::Accepted,
    range: Option<&ByteRange>,
    commit: &mut CommitSource<'_>,
    cached: Option<EndpointState>,
) -> Result<EndpointState> {
    // Fast path: el archivo no cambió desde el commit del contenido aceptado.
    //
    // **Sólo vale para conservar un OK.** La cache se escribe leyendo el árbol de
    // trabajo, no el commit, así que un estado no-OK pudo calcularse sobre una
    // edición que después se revirtió: el diff sale vacío y el estado viejo
    // describiría un contenido que ya no está.
    if let (Some(c), Some(EndpointState::Ok)) = (commit.cached, cached) {
        if !git_file_changed(layer, &cap.file, c) {
            return Ok(EndpointState::Ok);
        }
    }

    let source = std::fs::read_to_string(layer.join(&cap.file))?;
    let Some(r) = range else { return Ok(EndpointState::Unresolved) };
    let fragment = &source[r.start..r.end.min(source.len())];

    if hash::sha256(fragment.as_bytes()) == accepted.hash {
        return Ok(EndpointState::Ok);
    }

    // El texto aceptado, recuperado de git y verificado contra `accepted.hash`. Con
    // él, la frontera entre EXPANDED y DISPLACED es un test de subcadena y no un
    // umbral:
    //
    //   fragmento ⊃ aceptado          → creció alrededor        → EXPANDED
    //   fragmento ⊅ aceptado, nodo sí → se corrió, sigue igual  → DISPLACED
    let text = (commit.derive)()
        .and_then(|c| crate::capture::accepted_text(layer, cap, &c, Some(&accepted.hash)));

    if let Some(t) = text.as_deref() {
        if !t.is_empty() && fragment.len() > t.len() && fragment.contains(t) {
            return Ok(EndpointState::Expanded);
        }
    }

    // Sólo formato: el texto difiere y el AST no.
    //
    // La pregunta la decide la gramática, no el archivo: donde el AST no
    // discrimina contenido —prosa— el sexp de una sección es el mismo con
    // cualquier texto adentro, y compararlo diría RESTYLED de una reescritura
    // entera. Se consulta la gramática antes que `accepted`, así que un
    // `hash_ast` guardado por una versión anterior queda inerte en vez de mentir.
    let lang = grammar::language_for_file(&cap.file);
    if grammar::ast_discriminates_content(lang) {
        if let (Some(expected_ast), Some(q)) = (&accepted.hash_ast, &cap.query) {
            let language = grammar::for_language(lang)?;
            if let Some((_, _, sexp)) = query::find_target_with_sexp(language, &source, q)? {
                if hash::sha256(sexp.as_bytes()) == *expected_ast {
                    return Ok(EndpointState::Restyled);
                }
            }
        }
    }

    Ok(EndpointState::Altered)
}

// ─── endpoints que no son estructurales ───────────────────────────────────────

/// Un endpoint `path` copia los **dos** valores aceptados de su vecino.
fn check_path(
    layer: &Path,
    p: &bilink_format::link::StratumPath,
    uuid: &str,
    accepted: Option<&bilink_format::Accepted>,
) -> Result<EndpointState> {
    // Sin `accepted` y sin capa, es una intención declarada: TODO, no un error.
    let absent = if accepted.is_none() { EndpointState::Todo } else { EndpointState::Broken };

    let Ok(target) = stratum::resolve(layer, layer, p.tokens()) else { return Ok(absent) };
    let adj_path = layer.join(&target).join(".bilink").join(format!("{uuid}.yaml"));
    if !adj_path.exists() { return Ok(absent); }

    let Ok(adj) = BiLink::load(&adj_path) else { return Ok(EndpointState::Broken) };
    let Some(adj_accepted) = adj.structural_accepted() else {
        // El vecino existe y nunca se aceptó: no hay contra qué comparar.
        return Ok(EndpointState::Pending);
    };
    let Some(mine) = accepted else { return Ok(EndpointState::Pending) };

    // Los dos valores, no uno: la ubicación aprobada del vecino y su contenido.
    let same = mine.hash == adj_accepted.hash && mine.link == adj_accepted.link;
    Ok(if same { EndpointState::Ok } else { EndpointState::ChainDirty })
}

/// Un endpoint `issue` se hashea como el contenido del archivo del ítem.
fn check_issue(layer: &Path, id: &str, accepted: Option<&bilink_format::Accepted>) -> Result<EndpointState> {
    let (item, _) = crate::issue::resolve_issue_path(layer, id)?;
    let Some(item) = item else {
        return Ok(if accepted.is_none() { EndpointState::Todo } else { EndpointState::Broken });
    };
    let Some(accepted) = accepted else { return Ok(EndpointState::Pending) };
    let Ok(text) = std::fs::read_to_string(&item) else { return Ok(EndpointState::Broken) };

    Ok(if hash::sha256(text.as_bytes()) == accepted.hash {
        EndpointState::Ok
    } else {
        EndpointState::Altered
    })
}

// ─── git ──────────────────────────────────────────────────────────────────────

/// Nueva ruta del archivo si git detecta un rename (≥ 50% de similitud).
///
/// Sin pathspec: filtrar por el path viejo puede impedir que git detecte el
/// rename, porque el destino queda fuera del filtro.
pub(crate) fn git_renamed_to(layer_root: &Path, file: &str) -> Option<String> {
    for args in [
        &["diff", "-M", "--name-status", "HEAD"][..],
        &["diff", "-M", "--name-status", "--cached"][..],
    ] {
        let out = std::process::Command::new("git")
            .args(["-C", &layer_root.to_string_lossy()])
            .args(args)
            .output()
            .ok()?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if !line.starts_with('R') { continue; }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() == 3 && parts[1] == file && layer_root.join(parts[2]).exists() {
                return Some(parts[2].to_string());
            }
        }
    }
    None
}

/// ¿Git tiene historial de este archivo?
///
/// Distingue "el archivo se borró" de "esta referencia nunca apuntó a nada".
fn git_knows_file(layer_root: &Path, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &layer_root.to_string_lossy(), "log", "--oneline", "-1", "--", file])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Umbral de similitud para dar por reanclado un fragmento.
///
/// Es el mismo 50% que usa `git diff -M` para renames de archivos: la analogía
/// es exacta —encontrar a dónde se fue algo que cambió de nombre— y usar el
/// mismo número evita dos criterios distintos para la misma pregunta.
const REANCHOR_THRESHOLD: f64 = 0.5;

/// Margen mínimo sobre el segundo candidato.
///
/// Sin esto, un archivo con varias funciones de forma parecida produciría un
/// REANCHORED arbitrario. Ante un empate es preferible UNANCHORED: que un humano
/// mire es mejor que reanclar al nodo equivocado.
const REANCHOR_MARGIN: f64 = 0.15;

/// Busca a dónde se fue un fragmento cuyo anchor cambió de nombre.
///
/// No compara hashes: `hash.N` es exacto, y renombrar un anchor casi siempre
/// cambia el fragmento —el nombre suele estar *dentro* de lo capturado—, así que
/// una comparación exacta no dispararía nunca. En su lugar recupera el texto
/// aceptado desde git (`commit.N` + el range guardado, igual que `get --diff`) y
/// puntúa cada candidato por similitud.
pub(crate) fn find_renamed_anchor(
    root:      &Path,
    language:  tree_sitter::Language,
    source:    &str,
    query_str: &str,
    cap:       &Capture,
    hash:      Option<&str>,
    commit:    Option<&str>,
) -> Result<Option<(String, f64)>> {
    let Some(old_text) = commit.and_then(|c| crate::capture::accepted_text(root, cap, c, hash)) else {
        return Ok(None);
    };

    let relaxed = query::relax_name_predicates(query_str);
    let Ok(matches) = query::find_all_targets(language, source, &relaxed) else {
        return Ok(None); // la query relajada puede no ser válida; no es un error
    };

    let mut scored: Vec<(String, f64)> = Vec::new();
    for m in matches {
        let Some(name) = m.name.clone() else { continue };
        if m.start > m.end || m.end > source.len() { continue; }
        scored.push((name, hash::similarity(&old_text, &source[m.start..m.end])));
    }

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some((name, best)) = scored.first().cloned() else { return Ok(None) };
    if best < REANCHOR_THRESHOLD { return Ok(None); }

    let second = scored.get(1).map(|(_, s)| *s).unwrap_or(0.0);
    if best - second < REANCHOR_MARGIN { return Ok(None); }

    Ok(Some((name, best)))
}

/// ¿El fragmento aceptado existió alguna vez en el historial de este archivo?
///
/// `git log -S` busca commits que agreguen o quiten esa cadena. Si aparece,
/// hubo un commit que se llevó el fragmento — eso es DELETED, rastreable. Si no
/// aparece nunca, la referencia nunca ancló a algo que git haya visto.
fn git_fragment_vanished(layer_root: &Path, file: &str, hash: Option<&str>) -> bool {
    let Some(hash) = hash else { return false };
    std::process::Command::new("git")
        .args(["-C", &layer_root.to_string_lossy(), "log", "--oneline", "-1",
               "-S", hash, "--", file])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// ¿El archivo cambió desde `commit`?
///
/// Ante la duda, `true`: si git no puede resolver la comparación —commit
/// inexistente, repo sin historial— no se puede concluir que el archivo no
/// cambió, y asumirlo saltea la verificación y reporta un estado obsoleto.
fn git_file_changed(layer_root: &Path, file: &str, commit: &str) -> bool {
    std::process::Command::new("git")
        // `<commit>` sin `..HEAD`: compara contra el árbol de trabajo, no contra
        // HEAD. Con `..HEAD` los cambios sin commitear quedaban invisibles y el
        // fast-path devolvía el estado cacheado.
        .args(["-C", &layer_root.to_string_lossy(), "diff", "--name-only",
               commit, "--", file])
        .output()
        .map(|o| !o.status.success() || !o.stdout.is_empty())
        .unwrap_or(true)
}

/// Finds all bilinks referencing `file_path` across all layers under `root`.
/// Returns `(bilink_path, endpoint_index, absolute_range)`.
/// Uses `.bilink/.index` per layer when valid; falls back to O(N) scan.
/// Los endpoints que referencian un archivo, con el rango que la cache tiene.
///
/// El rango es un derivado: con la cache fría no está, y este comando cae a vacío
/// en vez de resolver. Quien lo necesite corre `check` primero.
pub fn find_by_file(root: &Path, file_path: &Path) -> Result<Vec<(PathBuf, u8, ByteRange)>> {
    let mut results = Vec::new();
    for layer_root in crate::index::layer_roots(root) {
        let Ok(rel) = file_path.strip_prefix(&layer_root) else { continue };
        let Some(rel_str) = rel.to_str() else { continue };

        let cache = Cache::load(&layer_root);
        let bilink_dir = layer_root.join(".bilink");

        for (uuid, n) in crate::index::lookup(&layer_root, rel_str)? {
            let bilink_path = bilink_dir.join(format!("{uuid}.yaml"));
            let Ok(bl) = BiLink::load(&bilink_path) else { continue };
            let Some(id) = bl.endpoint.get(n).link.capture_id() else { continue };
            if let Some(r) = cache.capture_range(id) {
                results.push((bilink_path, n, r));
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bilink_format::Accepted;
    use tempfile::tempdir;

    const QUERY: &str = r#"(section (atx_heading (inline) @n0 (#eq? @n0 "Spec"))) @target"#;

    fn sexp_hash(source: &str, query: &str) -> String {
        let language = grammar::for_language("markdown").unwrap();
        let (_, _, sexp) = query::find_target_with_sexp(language, source, query)
            .unwrap()
            .expect("la query debería resolver");
        hash::sha256(sexp.as_bytes())
    }

    /// Sobre prosa, un `hash_ast` guardado no se consulta aunque coincida.
    ///
    /// Una versión anterior lo escribía también para markdown. La gramática se
    /// consulta antes que `accepted`, así que el residuo queda inerte: acá el
    /// hash es el del contenido actual —coincide exacto— y aun así el estado es
    /// ALTERED, porque en prosa la pregunta no se hace.
    #[test]
    fn a_stored_ast_hash_over_prose_is_never_consulted() {
        let d = tempdir().unwrap();
        let file = "spec.md";
        let before = "# Spec\n\nLo que decía antes.\n";
        let after  = "# Spec\n\nOtra cosa completamente distinta.\n";
        std::fs::write(d.path().join(file), after).unwrap();

        let cap = Capture { file: file.into(), query: Some(QUERY.into()) };
        let accepted = Accepted {
            link: None,
            hash: hash::sha256(before.as_bytes()),
            hash_ast: Some(sexp_hash(after, QUERY)),   // coincidiría, si se mirara
        };
        let range = ByteRange { start: 0, end: after.len() };

        let mut derive = || None;
        let mut src = CommitSource { cached: None, derive: &mut derive };
        let state = compare_content(d.path(), &cap, &accepted, Some(&range), &mut src, None).unwrap();
        assert_eq!(state, EndpointState::Altered,
                   "en prosa el AST no discrimina contenido: no hay RESTYLED que dar");
    }
}
