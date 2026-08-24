use std::collections::HashSet;
use std::path::{Path, PathBuf};
use anyhow::Result;
use chrono::Utc;

use crate::bilink::{walkdir, BiLinkFile};
use crate::chain::resolve_layer_link;
use crate::grammar;
use crate::hash;
use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
use crate::query;
use crate::task::resolve_task_path;

#[derive(Debug)]
pub struct CheckResult {
    pub uuid: String,
    pub state0: EndpointState,
    pub state1: EndpointState,
    pub updated: bool,
}

impl CheckResult {
    pub fn is_clean(&self) -> bool {
        use EndpointState::*;
        matches!(self.state0, Ok | Moved | Displaced | Reanchored | Expanded | Todo | Restyled)
            && matches!(self.state1, Ok | Moved | Displaced | Reanchored | Expanded | Todo | Restyled)
    }
}

pub fn check(root: &Path, path: &Path) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();

    if path.is_file() {
        results.push(check_file(root, path)?);
        return Ok(results);
    }

    let bilink_dir = if path.ends_with(".bilink") { path.to_path_buf() }
                     else { path.join(".bilink") };

    for entry in walkdir(&bilink_dir)? {
        if entry.extension().and_then(|e| e.to_str()) == Some("bilink")
            && !entry.ancestors().any(|a| a.ends_with(".pending"))
        {
            results.push(check_file(root, &entry)?);
        }
    }
    Ok(results)
}

fn check_file(root: &Path, bilink_path: &Path) -> Result<CheckResult> {
    let mut bl = BiLinkFile::load(bilink_path)?;

    let layer_root = bilink_path
        .parent().and_then(|p| p.parent())
        .unwrap_or(root);

    let uuid = bl.uuid.clone();

    // El `range` de partida sale del capture, no del bilink.
    let cap0 = bl.capture_for(layer_root, 0).ok().flatten();
    let cap1 = bl.capture_for(layer_root, 1).ok().flatten();

    let (state0, range0) =
        check_endpoint(root, layer_root, &bl.link0, &uuid, bl.hash0.as_deref(), bl.hash_ast0.as_deref(),
                       cap0.as_ref().and_then(|c| c.range.as_ref()), bl.commit0.as_deref(), bl.state0.as_ref())?;

    let (state1, range1) =
        check_endpoint(root, layer_root, &bl.link1, &uuid, bl.hash1.as_deref(), bl.hash_ast1.as_deref(),
                       cap1.as_ref().and_then(|c| c.range.as_ref()), bl.commit1.as_deref(), bl.state1.as_ref())?;

    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // El resultado de la resolución se escribe en el capture; el estado de
    // aceptación, en el bilink. `range.N` ya no vive en el bilink.
    // La resolución va al capture; la aceptación, al bilink. Si el capture no
    // resolvió, el endpoint no puede evaluarse y queda UNRESOLVED: el detalle de
    // por qué lo lleva el capture.
    let mut cap_updated = false;
    let mut states      = [state0.clone(), state1.clone()];

    for (n, cap, range) in [(0u8, cap0, &range0), (1u8, cap1, &range1)] {
        let Some(mut cap) = cap else { continue };
        let cap_state = capture_state_for(&states[n as usize]);

        if !cap_state.is_resolved() {
            states[n as usize] = EndpointState::Unresolved;
        }

        // Solo los endpoints ya migrados tienen archivo que escribir; los legacy
        // llevan su ubicación embebida en el bilink hasta que corra `migrate`.
        if bl.link(n).capture_uuid().is_none() { continue; }

        if cap.range.as_ref() != range.as_ref() || cap.state.as_ref() != Some(&cap_state) {
            cap.range       = range.clone();
            cap.state       = Some(cap_state);
            cap.resolved_at = Some(now.clone());
            cap.write_in(layer_root)?;
            cap_updated = true;
        }
    }

    let [state0, state1] = states;
    let updated = cap_updated
        || bl.state0.as_ref() != Some(&state0)
        || bl.state1.as_ref() != Some(&state1);

    bl.range0      = None;
    bl.range1      = None;
    bl.state0      = Some(state0.clone());
    bl.state1      = Some(state1.clone());
    bl.resolved_at = Some(now);

    bl.write(bilink_path)?;


    Ok(CheckResult { uuid, state0, state1, updated })
}

/// Estado de resolución del capture, derivado del estado del endpoint.
fn capture_state_for(state: &EndpointState) -> crate::capture::CaptureState {
    use crate::capture::CaptureState as C;
    match state {
        EndpointState::Unanchored => C::Unanchored,
        EndpointState::Deleted    => C::Deleted,
        EndpointState::Broken     => C::Broken,
        EndpointState::Moved      => C::Moved,
        EndpointState::Reanchored => C::Reanchored,
        _                         => C::Resolved,
    }
}

fn check_endpoint(
    root: &Path,
    layer_root: &Path,
    endpoint: &LinkEndpoint,
    uuid: &str,
    hash: Option<&str>,
    hash_ast: Option<&str>,
    stored_range: Option<&ByteRange>,
    commit: Option<&str>,
    cached_state: Option<&EndpointState>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    match endpoint {
        LinkEndpoint::Capture(_) | LinkEndpoint::LegacyStructural(_) => {
            let sref = crate::capture::sref_of(layer_root, endpoint)?
                .expect("endpoint estructural sin referencia resoluble");
            check_structural(layer_root, &sref, hash, hash_ast, stored_range, commit, cached_state)
        }
        LinkEndpoint::Layer(tokens)    => check_layer(layer_root, tokens, uuid, hash),
        LinkEndpoint::Task(id)         => check_task(layer_root, id, hash),
    }
}

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
/// Distingue "el archivo se borró" de "esta referencia nunca apuntó a nada":
/// lo primero es rastreable y accionable, lo segundo es un bilink roto.
fn git_knows_file(layer_root: &Path, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &layer_root.to_string_lossy(), "log", "--oneline", "-1", "--", file])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Busca un nodo del mismo tipo que el anchor pero con otro nombre, cuyo
/// contenido hashee a `hash`.
///
/// Relaja los predicados de nombre de la query y recorre los candidatos. Si uno
/// coincide, el anchor se renombró y la query se puede reparar.
pub(crate) fn find_renamed_anchor(
    language: tree_sitter::Language,
    source:   &str,
    query_str: &str,
    hash:      &str,
    sref:      &StructuralRef,
) -> Result<Option<String>> {
    let relaxed = query::relax_name_predicates(query_str);
    let Ok(matches) = query::find_all_targets(language, source, &relaxed) else {
        return Ok(None); // la query relajada puede no ser válida; no es un error
    };

    for m in matches {
        let (start, end) = match &sref.range {
            Some(r) => (m.start + r.start, (m.start + r.end).min(source.len())),
            None    => (m.start, m.end),
        };
        if start > source.len() || end > source.len() || start > end { continue; }
        if hash::sha256(source[start..end].as_bytes()) == hash {
            return Ok(m.name);
        }
    }
    Ok(None)
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

pub(crate) fn check_structural(
    root: &Path,
    sref: &StructuralRef,
    hash: Option<&str>,
    hash_ast: Option<&str>,
    stored_range: Option<&ByteRange>,
    commit: Option<&str>,
    cached_state: Option<&EndpointState>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let file_path = root.join(&sref.file);

    // 1. El archivo no está donde lo dejamos: ¿se movió, o se borró?
    if !file_path.exists() {
        if git_renamed_to(root, &sref.file).is_some() {
            return Ok((EndpointState::Moved, None));
        }
        if git_knows_file(root, &sref.file) {
            return Ok((EndpointState::Deleted, None));
        }
        return Ok((EndpointState::Broken, None));
    }

    // Fast path: if file unchanged since accepted commit, reuse cached state.
    if let (Some(commit), Some(state)) = (commit, cached_state) {
        if !git_file_changed(root, &sref.file, commit) {
            return Ok((state.clone(), stored_range.cloned()));
        }
    }

    let source = std::fs::read_to_string(&file_path)?;

    let Some(query_str) = &sref.query else {
        let new_hash = hash::sha256(source.as_bytes());
        let range    = ByteRange { start: 0, end: source.len() };
        let state = if hash.is_none() {
            EndpointState::Pending
        } else if hash == Some(new_hash.as_str()) {
            EndpointState::Ok
        } else {
            EndpointState::Altered
        };
        return Ok((state, Some(range)));
    };

    let lang     = grammar::language_for_file(&sref.file);
    let language = grammar::for_language(lang)?;
    let node_range = query::find_target_with_sexp(language.clone(), &source, query_str)?;

    // 2. La query no matchea: ¿el anchor se renombró, o desapareció?
    let Some((node_start, node_end, sexp)) = node_range else {
        if let Some(hash) = hash {
            if find_renamed_anchor(language, &source, query_str, hash, sref)?.is_some() {
                return Ok((EndpointState::Reanchored, None));
            }
        }
        // El anchor no está y no se renombró. Si git tiene historial del archivo,
        // el fragmento se fue en algún commit rastreable; si no, la referencia
        // simplemente ya no ancla.
        if git_fragment_vanished(root, &sref.file, hash) {
            return Ok((EndpointState::Deleted, None));
        }
        return Ok((EndpointState::Unanchored, None));
    };

    let (frag_start, frag_end) = match &sref.range {
        Some(r) => (node_start + r.start, (node_start + r.end).min(source.len())),
        None    => (node_start, node_end),
    };
    let fragment      = &source[frag_start..frag_end];
    let new_hash      = hash::sha256(fragment.as_bytes());
    let new_hash_ast  = hash::sha256(sexp.as_bytes());
    let new_range     = ByteRange { start: frag_start, end: frag_end };

    if hash.is_none() {
        return Ok((EndpointState::Pending, Some(new_range)));
    }

    if hash == Some(new_hash.as_str()) {
        return Ok((EndpointState::Ok, Some(new_range)));
    }

    // Text changed — check if AST is identical (formatting-only change)
    if hash_ast.is_some() && hash_ast == Some(new_hash_ast.as_str()) {
        return Ok((EndpointState::Restyled, Some(new_range)));
    }

    if let (Some(stored_hash), Some(sr)) = (hash, stored_range) {
        let frag_len = sr.end - sr.start;
        if let Some(displaced) = find_in_node(&source, node_start, node_end, stored_hash, frag_len) {
            return Ok((EndpointState::Displaced, Some(displaced)));
        }
    }

    Ok((EndpointState::Altered, Some(new_range)))
}

fn check_layer(
    layer_root: &Path,
    tokens: &stratum::StratumPath,
    uuid: &str,
    stored_hash: Option<&str>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let absent = if stored_hash.is_none() { EndpointState::Todo } else { EndpointState::Broken };

    let target_layer = match stratum::resolve(layer_root, layer_root, tokens) {
        Ok(p)  => p,
        Err(_) => return Ok((absent, None)),
    };

    let target_bilink = resolve_layer_link(
        &layer_root.join(".bilink").join(format!("{uuid}.bilink")),
        layer_root,
        &target_layer,
        uuid,
    );

    if !target_bilink.exists() {
        return Ok((absent, None));
    }

    // Hash = structural endpoint's accepted hash in the adjacent bilink.
    // This avoids circular dependency: accepting a layer endpoint never modifies
    // the adjacent bilink file, so the hash never cascades back.
    let adj_bl = crate::bilink::BiLinkFile::load(&target_bilink)?;
    let Some(adj_struct_hash) = adj_bl.structural_hash() else {
        return Ok((EndpointState::Pending, None));
    };

    let state = if stored_hash.is_none() {
        EndpointState::Pending
    } else if stored_hash == Some(adj_struct_hash) {
        EndpointState::Ok
    } else {
        EndpointState::ChainDirty
    };

    Ok((state, None))
}

fn check_task(
    layer_root: &Path,
    task_id: &str,
    stored_hash: Option<&str>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let (task_path, _) = resolve_task_path(layer_root, task_id);
    let task_dir = match task_path.parent() {
        Some(d) => d.to_path_buf(),
        None => return Ok((EndpointState::Broken, None)),
    };
    let filename = match task_path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f.to_string(),
        None => return Ok((EndpointState::Broken, None)),
    };
    let sref = StructuralRef { file: filename, query: None, range: None };
    check_structural(&task_dir, &sref, stored_hash, None, None, None, None)
}

fn find_in_node(
    source: &str,
    node_start: usize,
    node_end: usize,
    target_hash: &str,
    frag_len: usize,
) -> Option<ByteRange> {
    if frag_len == 0 || frag_len > node_end.saturating_sub(node_start) {
        return None;
    }
    let node = &source[node_start..node_end];
    let mut start = 0;
    while start + frag_len <= node.len() {
        if source.is_char_boundary(node_start + start) {
            let end = start + frag_len;
            if end <= node.len() && source.is_char_boundary(node_start + end) {
                if hash::sha256(node[start..end].as_bytes()) == target_hash {
                    return Some(ByteRange {
                        start: node_start + start,
                        end: node_start + end,
                    });
                }
            }
        }
        start += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilink::BiLinkFile;
    use crate::hash;
    use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
    use tempfile::tempdir;

    fn whole_file_endpoint(file: &str) -> LinkEndpoint {
        LinkEndpoint::LegacyStructural(StructuralRef {
            file: file.into(),
            query: None,
            range: None,
        })
    }

    fn layer_endpoint(path: &str) -> LinkEndpoint {
        LinkEndpoint::Layer(stratum::parse_path(path).unwrap())
    }

    fn make_bilink(dir: &Path, uuid: &str, link0: LinkEndpoint, link1: LinkEndpoint) -> std::path::PathBuf {
        let bl = BiLinkFile::new(uuid, link0, link1);
        let path = dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        path
    }

    #[test]
    fn check_whole_file_first_time_is_pending() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "hello world").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("a.md"),
            whole_file_endpoint("a.md"),
        );

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Pending);
        assert_eq!(result.state1, EndpointState::Pending);
    }

    #[test]
    fn check_whole_file_ok_when_hash_matches() {
        let dir = tempdir().unwrap();
        let content = b"stable content";
        std::fs::write(dir.path().join("a.md"), content).unwrap();
        let stored_hash = hash::sha256(content);

        let bilink_dir = dir.path().join(".bilink");
        let mut bl = BiLinkFile::new("uuid1", whole_file_endpoint("a.md"), whole_file_endpoint("a.md"));
        bl.hash0 = Some(stored_hash.clone());
        bl.commit0 = Some("abc1234".into());
        bl.hash1 = Some(stored_hash);
        bl.commit1 = Some("abc1234".into());
        bl.range0 = Some(ByteRange { start: 0, end: content.len() });
        bl.range1 = Some(ByteRange { start: 0, end: content.len() });
        bl.state0 = Some(EndpointState::Ok);
        bl.state1 = Some(EndpointState::Ok);
        bl.resolved_at = Some("2026-01-01T00:00:00Z".into());
        let path = bilink_dir.join("uuid1.bilink");
        bl.write(&path).unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Ok);
    }

    #[test]
    fn check_whole_file_altered_when_hash_differs() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "new content").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let mut bl = BiLinkFile::new("uuid1", whole_file_endpoint("a.md"), whole_file_endpoint("a.md"));
        bl.hash0   = Some("old-hash-that-wont-match".into());
        bl.commit0 = Some("abc1234".into());
        bl.hash1   = Some("old-hash-that-wont-match".into());
        bl.commit1 = Some("abc1234".into());
        let path = bilink_dir.join("uuid1.bilink");
        bl.write(&path).unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Altered);
    }

    #[test]
    fn check_unresolved_when_file_missing() {
        let dir = tempdir().unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("missing.md"),
            whole_file_endpoint("missing.md"),
        );

        // El archivo no está: eso es un problema de ubicación, no del vínculo.
        // El bilink no puede evaluarse; el detalle lo lleva el capture.
        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Unresolved);
    }

    /// Prepara un repo git con un archivo commiteado.
    fn git_repo_with(dir: &Path, file: &str, content: &str) {
        std::fs::create_dir_all(dir.join(file).parent().unwrap()).ok();
        std::fs::write(dir.join(file), content).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "init"],
        ] {
            std::process::Command::new("git")
                .args(["-C", &dir.to_string_lossy()]).args(&args)
                .output().unwrap();
        }
    }

    /// Crea un capture y el bilink que lo referencia, con hash aceptado.
    fn capture_bilink(
        layer: &Path, uuid: &str, file: &str, query: Option<&str>, hash: &str,
    ) -> std::path::PathBuf {
        let cap = crate::capture::CaptureFile {
            uuid: format!("cap-{uuid}"),
            sref: StructuralRef {
                file: file.into(),
                query: query.map(String::from),
                range: None,
            },
            range: None, state: None, resolved_at: None,
        };
        cap.write_in(layer).unwrap();

        let mut bl = BiLinkFile::new(uuid,
            LinkEndpoint::Capture(cap.uuid.clone()),
            layer_endpoint(".stratum/impl"));
        bl.hash0   = Some(hash.into());
        bl.commit0 = Some("0000000".into());
        let path = layer.join(".bilink").join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        path
    }

    #[test]
    fn check_detects_moved_after_git_rename() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let content = "# Doc\n";
        git_repo_with(root, "a.md", content);

        let path = capture_bilink(root, "uuid1", "a.md", None, &hash::sha256(content.as_bytes()));

        // Rename rastreable por git: el contenido no cambia.
        std::process::Command::new("git")
            .args(["-C", &root.to_string_lossy(), "mv", "a.md", "b.md"])
            .output().unwrap();

        let result = check_file(root, &path).unwrap();
        assert_eq!(result.state0, EndpointState::Unresolved,
                   "el bilink no puede evaluarse mientras la ubicación no resuelva");

        let cap = crate::capture::CaptureFile::load_in(root, "cap-uuid1").unwrap();
        assert_eq!(cap.state, Some(crate::capture::CaptureState::Moved),
                   "el capture es quien sabe que el archivo se movió");
    }

    #[test]
    fn check_detects_reanchored_when_anchor_is_renamed() {
        let dir  = tempdir().unwrap();
        let root = dir.path();

        // El fragmento capturado es el *cuerpo*, no la firma: por eso renombrar
        // la función deja el hash intacto y el anchor es lo único que cambia.
        let before = "fn foo() {\n    let x = 1;\n}\n";
        let brace  = before.find('{').unwrap();
        let fn_end = before.trim_end().len();
        let body   = &before[brace..fn_end];

        git_repo_with(root, "a.rs", before);

        let query = r#"(function_item name: (identifier) @n0 (#eq? @n0 "foo")) @target"#;
        let cap = crate::capture::CaptureFile {
            uuid: "cap-uuid1".into(),
            sref: StructuralRef {
                file:  "a.rs".into(),
                query: Some(query.into()),
                range: Some(ByteRange { start: brace, end: fn_end }),
            },
            range: None, state: None, resolved_at: None,
        };
        cap.write_in(root).unwrap();

        let mut bl = BiLinkFile::new("uuid1",
            LinkEndpoint::Capture("cap-uuid1".into()),
            layer_endpoint(".stratum/impl"));
        bl.hash0   = Some(hash::sha256(body.as_bytes()));
        bl.commit0 = Some("0000000".into());
        let path = root.join(".bilink/uuid1.bilink");
        bl.write(&path).unwrap();

        // Mismo largo, así que el cuerpo queda en los mismos offsets.
        std::fs::write(root.join("a.rs"), "fn bar() {\n    let x = 1;\n}\n").unwrap();

        check_file(root, &path).unwrap();
        let cap = crate::capture::CaptureFile::load_in(root, "cap-uuid1").unwrap();
        assert_eq!(cap.state, Some(crate::capture::CaptureState::Reanchored),
                   "el anchor se renombró pero el fragmento sigue ahí");
    }

    #[test]
    fn check_detects_deleted_when_file_removed_from_git() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let content = "# Doc\n";
        git_repo_with(root, "a.md", content);

        let path = capture_bilink(root, "uuid1", "a.md", None, &hash::sha256(content.as_bytes()));
        std::fs::remove_file(root.join("a.md")).unwrap();

        check_file(root, &path).unwrap();
        let cap = crate::capture::CaptureFile::load_in(root, "cap-uuid1").unwrap();
        assert_eq!(cap.state, Some(crate::capture::CaptureState::Deleted),
                   "git tiene historial del archivo: la eliminación es rastreable");
    }

    #[test]
    fn check_layer_first_time_is_pending() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        std::fs::write(adj_dir.join(format!("{uuid}.bilink")), "link.0: a.md\nlink.1: b.md\n").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, uuid,
            whole_file_endpoint("a.md"),
            layer_endpoint(".stratum/impl"),
        );
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Pending);
    }

    #[test]
    fn check_layer_ok_when_hash_matches() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        // Adjacent bilink has an accepted structural endpoint (link.1 = b.md, hash.1 set)
        let adj_struct_hash = "deadbeefdeadbeef".to_string();
        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        let mut adj_bl = BiLinkFile::new(uuid, layer_endpoint("../.."), whole_file_endpoint("b.md"));
        adj_bl.hash1   = Some(adj_struct_hash.clone());
        adj_bl.commit1 = Some("abc1234".into());
        adj_bl.write(&adj_dir.join(format!("{uuid}.bilink"))).unwrap();

        // Spec bilink stores adj structural hash as its layer endpoint hash
        let bilink_dir = dir.path().join(".bilink");
        let mut bl = BiLinkFile::new(uuid, whole_file_endpoint("a.md"), layer_endpoint(".stratum/impl"));
        bl.hash1   = Some(adj_struct_hash);
        bl.commit1 = Some("abc1234".into());
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Ok);
    }

    #[test]
    fn check_layer_chain_dirty_when_hash_differs() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        // Adjacent bilink has structural hash "current-hash"
        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        let mut adj_bl = BiLinkFile::new(uuid, layer_endpoint("../.."), whole_file_endpoint("b.md"));
        adj_bl.hash1   = Some("current-hash".into());
        adj_bl.commit1 = Some("abc1234".into());
        adj_bl.write(&adj_dir.join(format!("{uuid}.bilink"))).unwrap();

        // Spec bilink stores a different (stale) hash
        let bilink_dir = dir.path().join(".bilink");
        let mut bl = BiLinkFile::new(uuid, whole_file_endpoint("a.md"), layer_endpoint(".stratum/impl"));
        bl.hash1   = Some("stale-hash-000".into());
        bl.commit1 = Some("abc1234".into());
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::ChainDirty);
    }

    #[test]
    fn check_layer_todo_when_adjacent_missing_and_no_hash() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let bilink_dir = dir.path().join(".bilink");
        std::fs::write(dir.path().join("a.md"), "content").unwrap();
        let path = make_bilink(&bilink_dir, uuid,
            whole_file_endpoint("a.md"),
            layer_endpoint(".stratum/impl"),
        );

        // No hash stored, target layer doesn't exist → TODO (intentional absence)
        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Todo);
    }

    #[test]
    fn check_layer_broken_when_adjacent_missing_but_had_hash() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let bilink_dir = dir.path().join(".bilink");
        std::fs::write(dir.path().join("a.md"), "content").unwrap();
        let mut bl = BiLinkFile::new(uuid, whole_file_endpoint("a.md"), layer_endpoint(".stratum/impl"));
        bl.hash1   = Some("previously-accepted-hash".into());
        bl.commit1 = Some("abc1234".into());
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();

        // Hash present but target gone → BROKEN (regression)
        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Broken);
    }

    #[test]
    fn check_writes_state_and_timestamp() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), "# Title\nContent here.").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("doc.md"),
            whole_file_endpoint("doc.md"),
        );

        check_file(dir.path(), &path).unwrap();

        let updated = BiLinkFile::load(&path).unwrap();
        assert!(updated.state0.is_some(),      "state.0 should be written");
        assert!(updated.resolved_at.is_some(), "resolved_at should be written");
        assert!(updated.hash0.is_none(),        "check must not modify hash.0");
    }

    #[test]
    fn check_dir_processes_all_bilinks() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "content a").unwrap();
        std::fs::write(dir.path().join("b.md"), "content b").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("a.md"),
            whole_file_endpoint("a.md"),
        );
        make_bilink(&bilink_dir, "uuid2",
            whole_file_endpoint("b.md"),
            whole_file_endpoint("b.md"),
        );

        let results = check(dir.path(), dir.path()).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.state0 == EndpointState::Pending));
    }
}

/// Finds all bilinks referencing `file_path` across all layers under `root`.
/// Returns `(bilink_path, endpoint_index, absolute_range)`.
/// Uses `.bilink/.index` per layer when valid; falls back to O(N) scan.
pub fn find_by_file(root: &Path, file_path: &Path) -> Result<Vec<(PathBuf, u8, ByteRange)>> {
    let mut results = Vec::new();

    for layer_root in crate::index::layer_roots(root) {
        let Ok(rel) = file_path.strip_prefix(&layer_root) else { continue };
        let Some(rel_str) = rel.to_str() else { continue };

        let bilink_dir = layer_root.join(".bilink");
        for (uuid, n) in crate::index::lookup(&layer_root, rel_str)? {
            let bilink_path = bilink_dir.join(format!("{uuid}.bilink"));
            let Ok(bl) = BiLinkFile::load(&bilink_path) else { continue };
            // El range vive en el capture; `capture_for` lo sintetiza para legacy.
            let Ok(Some(cap)) = bl.capture_for(&layer_root, n) else { continue };
            if let Some(r) = cap.range {
                results.push((bilink_path, n, r));
            }
        }
    }

    Ok(results)
}
