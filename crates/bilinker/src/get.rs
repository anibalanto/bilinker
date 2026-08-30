use std::path::Path;
use anyhow::{bail, Context, Result};

use bilink_format::{BiLink, ByteRange, Capture, LinkEndpoint};

use crate::cache::Cache;
use crate::grammar;
use crate::query;
use bilink_format::link::StratumPath;

pub struct GetResult {
    pub content: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub struct DiffResult {
    pub file: String,
    pub layer_root: std::path::PathBuf,
    pub commit: String,
    pub start_line: usize,
    pub end_line: usize,
    /// None = no changes
    pub diff: Option<String>,
}

/// El fragmento que un endpoint referencia.
pub fn get(
    root: &Path,
    bilink_name: &str,
    endpoint: u8,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let path = crate::accept::find_bilink_path(root, bilink_name)?;
    let uuid = uuid_of(&path);
    let bl = BiLink::load(&path)?;
    if endpoint > 1 { bail!("el endpoint es 0 o 1"); }
    let link = &bl.endpoint.get(endpoint).link;

    match link {
        LinkEndpoint::Capture(_) => {
            let cap = crate::capture::capture_of(root, link)?
                .context("el endpoint estructural no tiene capture resoluble")?;
            resolve(root, &cap, before, after)
        }
        LinkEndpoint::Path(p) => traverse_layer(root, p.clone(), &uuid, before, after),
        // Cruzar la frontera: el fragmento vive en el clon del proveedor, que el
        // sparse-checkout ya trajo entero.
        LinkEndpoint::Repo(alias) => traverse_repo(root, alias, &uuid, before, after),
        LinkEndpoint::Abstract => bail!(
            "el endpoint {endpoint} es `abstract`: es la punta abierta y no apunta a \
             ningún fragmento de este repo"
        ),
        LinkEndpoint::Issue(id) => bail!("el endpoint {endpoint} es un issue ({id}) — se mira con worklist"),
    }
}

/// El diff entre el fragmento aceptado y el actual.
///
/// El baseline es el `commit` del endpoint —aquel en que el contenido aceptado
/// quedó establecido—, que vive en la cache. Con cache fría no está y se pide
/// correr `accept` o `check`.
pub fn get_diff(root: &Path, bilink_name: &str, endpoint: u8) -> Result<DiffResult> {
    let path = crate::accept::find_bilink_path(root, bilink_name)?;
    let uuid = uuid_of(&path);
    let bl = BiLink::load(&path)?;
    if endpoint > 1 { bail!("el endpoint es 0 o 1"); }

    let e = bl.endpoint.get(endpoint);
    let accepted = e.accepted.as_ref()
        .context("el endpoint no tiene nada aceptado — correr `bilinker accept` primero")?;

    let mut cache = Cache::load(root);
    let range = e.link.capture_id().and_then(|id| cache.capture_range(id));

    // El commit se deriva si la cache no lo tiene: una cache fría —un clon fresco,
    // otra rama— no puede dejar sin `--diff` a un endpoint que sí tiene aceptación.
    let commit = match crate::capture::capture_of(root, &e.link)? {
        Some(cap) => cache.commit_or_derive(root, &uuid, endpoint, &cap, &accepted.hash),
        None      => cache.commit(&uuid, endpoint).map(str::to_string),
    }
    .context("no se pudo ubicar en la historia el contenido aceptado: ni la cache lo \
              tiene ni aparece en los últimos commits del archivo")?;
    let _ = cache.save(root);

    match &e.link {
        LinkEndpoint::Capture(_) => {
            let cap = crate::capture::capture_of(root, &e.link)?
                .context("el endpoint estructural no tiene capture resoluble")?;
            diff_structural(root, &cap, &commit, range.as_ref(), Some(&accepted.hash))
        }
        LinkEndpoint::Path(p) => {
            let (adj_root, adj_cap, adj_commit, adj_range, adj_hash) =
                traverse_layer_for_diff(root, p.clone(), &uuid)?;
            diff_structural(&adj_root, &adj_cap,
                            adj_commit.as_deref().unwrap_or(&commit),
                            adj_range.as_ref(), adj_hash.as_deref())
        }
        // El diff cruzando la frontera es lo que **profundiza el clon**: `check` es
        // masivo y corre superficial, y traer historia se paga sólo acá, donde hay
        // un humano mirando un bilink.
        LinkEndpoint::Repo(alias) => diff_across_frontier(root, alias, &uuid, accepted),
        LinkEndpoint::Abstract => bail!(
            "el endpoint {endpoint} es `abstract`: no hay contra qué diffear"
        ),
        LinkEndpoint::Issue(id) => bail!("el endpoint {endpoint} es un issue ({id})"),
    }
}

/// El fragmento del proveedor, leído de su clon.
///
/// El clon lleva el árbol del proyecto más `.bilink/`, así que el capture remoto y
/// el archivo al que apunta vienen del mismo commit por construcción.
fn traverse_repo(
    root: &Path, alias: &str, uuid: &str,
    before: Option<(usize, usize)>, after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let clone = crate::frontier::Provider::clone_path(root, alias);
    if !clone.join(".bilink").is_dir() {
        bail!("el repo '{alias}' no está clonado. Traerlo primero: `bilinker fetch {alias}`.");
    }
    crate::frontier::verify_format_version(&clone, alias)?;

    let remote = BiLink::load(&BiLink::path_in(&clone, uuid))
        .with_context(|| format!("el bilink {uuid} no está en el repo '{alias}'"))?;
    let id = [0u8, 1u8]
        .iter()
        .find_map(|n| remote.endpoint.get(*n).link.capture_id())
        .context("el bilink remoto no tiene endpoint estructural")?;
    let cap = bilink_format::Capture::load_in(&clone, id)?;

    resolve(&clone, &cap, before, after)
}

/// Qué cambió del lado del proveedor entre lo que este repo aceptó y lo que publica.
///
/// **Ningún commit del proveedor se copia**: el de lo aceptado se descubre acá,
/// recorriendo su ref hacia atrás hasta que su `accepted` coincida con el guardado.
/// Es lo que hace que el consumidor no guarde nada más que dos hashes opacos.
fn diff_across_frontier(
    root: &Path, alias: &str, uuid: &str, accepted: &bilink_format::Accepted,
) -> Result<DiffResult> {
    let clone = crate::frontier::Provider::clone_path(root, alias);
    if !clone.join(".bilink").is_dir() {
        bail!("el repo '{alias}' no está clonado. Traerlo primero: `bilinker fetch {alias}`.");
    }
    crate::frontier::verify_format_version(&clone, alias)?;

    let provider = crate::frontier::Provider::load(root, alias)?;
    let commit = crate::frontier::deepen_until_accepted(&clone, &provider, uuid, accepted)?
        .with_context(|| format!(
            "no se encontró en la historia de '{alias}' la versión que este repo aceptó"
        ))?;

    let remote = BiLink::load(&BiLink::path_in(&clone, uuid))?;
    let id = [0u8, 1u8]
        .iter()
        .find_map(|n| remote.endpoint.get(*n).link.capture_id())
        .context("el bilink remoto no tiene endpoint estructural")?;
    let cap = bilink_format::Capture::load_in(&clone, id)?;

    diff_structural(&clone, &cap, &commit, None, None)
}

/// El uuid de un bilink es el nombre de su archivo.
fn uuid_of(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

fn diff_structural(
    root: &Path,
    cap: &Capture,
    commit: &str,
    stored_range: Option<&ByteRange>,
    hash: Option<&str>,
) -> Result<DiffResult> {
    // "after": current fragment via AST query
    let after_result = resolve(root, cap, None, None)?;
    let after_text = &after_result.content;

    // "before": el fragmento aceptado, resolviendo la query contra el contenido
    // de `commit`. Recortarlo por `stored_range` daría bytes arbitrarios: ese
    // range es la posición *actual*, que check reescribe en cada corrida.
    //
    // Si la verificación por hash falla, se cae al recorte por range: para un
    // diff informativo es mejor mostrar algo aproximado que no mostrar nada.
    let before_text = match crate::capture::accepted_text(root, cap, commit, hash) {
        Some(t) => t,
        None    => git_show_fragment(root, commit, &cap.file, stored_range)?,
    };

    let diff = if before_text.trim_end() == after_text.trim_end() {
        None
    } else {
        Some(unified_diff(&before_text, after_text, commit))
    };

    Ok(DiffResult {
        file: cap.file.clone(),
        layer_root: root.to_path_buf(),
        commit: commit.to_string(),
        start_line: after_result.start_line,
        end_line: after_result.end_line,
        diff,
    })
}

fn git_show_fragment(root: &Path, commit: &str, file: &str, range: Option<&ByteRange>) -> Result<String> {
    // `git show <commit>:<path>` resuelve el path contra la raíz del **repo**, no
    // contra el `-C`. Una capa que no sea la raíz de su repo —`subsystems/lattice`
    // dentro de accreta— necesita la traducción o el comando falla.
    let file = &crate::capture::git_path_from_repo_root(root, file);
    let output = std::process::Command::new("git")
        .args(["-C", &root.to_string_lossy(), "show", &format!("{commit}:{file}")])
        .output()
        .context("running git show")?;

    if !output.status.success() {
        bail!("git show {commit}:{file} failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let source = String::from_utf8_lossy(&output.stdout);

    let fragment = match range {
        Some(r) => {
            let start = r.start.min(source.len());
            let end   = r.end.min(source.len());
            source[start..end].to_string()
        }
        None => source.into_owned(),
    };

    Ok(fragment)
}

fn unified_diff(before: &str, after: &str, commit: &str) -> String {
    

    let dir = std::env::temp_dir();
    let before_path = dir.join("bilinker_diff_before.tmp");
    let after_path  = dir.join("bilinker_diff_after.tmp");

    let _ = std::fs::write(&before_path, before);
    let _ = std::fs::write(&after_path, after);

    let output = std::process::Command::new("diff")
        .args([
            "-u",
            "--label", &format!("aceptado ({})", &commit[..8.min(commit.len())]),
            "--label", "actual",
            &before_path.to_string_lossy(),
            &after_path.to_string_lossy(),
        ])
        .output();

    let _ = std::fs::remove_file(&before_path);
    let _ = std::fs::remove_file(&after_path);

    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => format!("--- aceptado ({})\n+++ actual\n(diff no disponible)", &commit[..8.min(commit.len())]),
    }
}

/// Find a function/method named `callee_name` in `source` using tree-sitter.
/// Returns byte range (start, end) of the matching declaration.
fn find_function_body(source: &str, lang: &str, callee_name: &str) -> Option<(usize, usize)> {
    let language = grammar::for_language(lang).ok()?;
    let escaped = callee_name.replace('"', "\\\"");
    for &anchor_kind in grammar::stable_anchor_kinds(lang) {
        let Some(field) = grammar::name_field(lang, anchor_kind) else { continue };
        let name_node_type = grammar::name_node_type(lang, anchor_kind);
        let query_str = format!(
            "({anchor_kind} {field}: ({name_node_type}) @name (#eq? @name \"{escaped}\")) @target"
        );
        if let Ok(Some((s, e))) = query::find_target(language.clone(), source, &query_str) {
            return Some((s, e));
        }
    }
    None
}

pub fn get_callee_diff(
    root: &Path,
    callee_file_abs: &str,
    callee_name: &str,
    commit: &str,
) -> Result<DiffResult> {
    let callee_path = std::path::Path::new(callee_file_abs);
    let rel_file = callee_path
        .strip_prefix(root)
        .unwrap_or(callee_path)
        .to_string_lossy()
        .to_string();

    let lang = grammar::language_for_file(&rel_file);

    let current_source = std::fs::read_to_string(callee_file_abs)
        .with_context(|| format!("reading {callee_file_abs}"))?;

    let (start_byte, end_byte) = find_function_body(&current_source, lang, callee_name)
        .ok_or_else(|| anyhow::anyhow!("function '{callee_name}' not found in {rel_file}"))?;

    let after_text = current_source[start_byte..end_byte].to_string();
    let start_line = current_source[..start_byte].chars().filter(|&c| c == '\n').count() + 1;
    let end_line   = current_source[..end_byte].chars().filter(|&c| c == '\n').count() + 1;

    let before_text = match git_show_fragment(root, commit, &rel_file, None) {
        Ok(old_source) => {
            find_function_body(&old_source, lang, callee_name)
                .map(|(s, e)| old_source[s..e].to_string())
                .unwrap_or_default()
        }
        Err(_) => String::new(),
    };

    let diff = if before_text.trim_end() == after_text.trim_end() {
        None
    } else {
        Some(unified_diff(&before_text, &after_text, commit))
    };

    Ok(DiffResult {
        file: rel_file,
        layer_root: root.to_path_buf(),
        commit: commit.to_string(),
        start_line,
        end_line,
        diff,
    })
}

fn traverse_layer_for_diff(
    root: &Path,
    layer_path: StratumPath,
    uuid: &str,
) -> Result<(std::path::PathBuf, Capture, Option<String>, Option<ByteRange>, Option<String>)> {
    let adjacent_root = {
        let p = stratum::resolve(root, root, layer_path.tokens())
            .map_err(|e| anyhow::anyhow!("resolving adjacent layer: {e}"))?;
        let (true_root, _) = crate::config::Config::load_from(&p)
            .with_context(|| format!("finding root of adjacent layer {}", p.display()))?;
        true_root
    };

    let adj_path = BiLink::path_in(&adjacent_root, uuid);
    let adjacent_bl = BiLink::load(&adj_path)
        .with_context(|| format!("no está el bilink {uuid} en la capa vecina"))?;

    let (n, cap) = structural_of(&adjacent_root, &adjacent_bl)
        .with_context(|| format!("el bilink vecino {uuid} no tiene endpoint estructural resoluble"))?;

    let cache  = Cache::load(&adjacent_root);
    let commit = cache.commit(uuid, n).map(String::from);
    let range  = adjacent_bl.endpoint.get(n).link.capture_id().and_then(|id| cache.capture_range(id));
    let hash   = adjacent_bl.endpoint.get(n).accepted.as_ref().map(|a| a.hash.clone());

    Ok((adjacent_root, cap, commit, range, hash))
}

/// El endpoint estructural de un bilink, con su capture.
fn structural_of(layer: &Path, bl: &BiLink) -> Option<(u8, Capture)> {
    for n in [0u8, 1u8] {
        if let Some(id) = bl.endpoint.get(n).link.capture_id() {
            if let Ok(cap) = Capture::load_in(layer, id) {
                return Some((n, cap));
            }
        }
    }
    None
}

fn traverse_layer(
    root: &Path,
    layer_path: StratumPath,
    uuid: &str,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let adjacent_root = {
        let p = stratum::resolve(root, root, layer_path.tokens())
            .map_err(|e| anyhow::anyhow!("resolving adjacent layer: {e}"))?;
        // Walk up to the true root of the adjacent layer (.git or .bilink)
        let (true_root, _) = crate::config::Config::load_from(&p)
            .with_context(|| format!("finding root of adjacent layer {}", p.display()))?;
        true_root
    };

    let adjacent_bl = BiLink::load(&BiLink::path_in(&adjacent_root, uuid))
        .with_context(|| format!("no está el bilink {uuid} en la capa vecina"))?;
    let (_, cap) = structural_of(&adjacent_root, &adjacent_bl)
        .with_context(|| format!("el bilink vecino {uuid} no tiene endpoint estructural resoluble"))?;

    resolve(&adjacent_root, &cap, before, after)
}

fn resolve(
    root: &Path,
    cap: &Capture,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let file_path = root.join(&cap.file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let Some(query_str) = &cap.query else {
        let total = count_lines(&source);
        return Ok(GetResult {
            content: source,
            file: cap.file.clone(),
            start_line: 1,
            end_line: total,
        });
    };

    let lang = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;

    let (node_start, node_end) = query::find_target(language, &source, query_str)?
        .with_context(|| format!("query matched nothing in {}", cap.file))?;

    let (frag_start, frag_end) = (node_start, node_end);

    let line_start = byte_to_line(&source, frag_start);
    let line_end   = byte_to_line(&source, frag_end.saturating_sub(1));

    let before_rows = before.map(|(r, _)| r).unwrap_or(0);
    let after_rows  = after.map(|(r, _)| r).unwrap_or(0);

    let ctx_start = line_start.saturating_sub(before_rows);
    let ctx_end   = (line_end + after_rows).min(count_lines(&source).saturating_sub(1));

    let content = extract_lines(&source, ctx_start, ctx_end);

    Ok(GetResult {
        content,
        file: cap.file.clone(),
        start_line: ctx_start + 1,
        end_line: ctx_end + 1,
    })
}

/// Cuenta sobre bytes y no sobre `&str`: un offset puede caer en medio de un
/// carácter multibyte —una `ó` en una spec en castellano alcanza— y cortar el
/// `&str` ahí es un panic. `\n` es ASCII, así que nunca aparece adentro de una
/// secuencia multibyte y contar bytes da lo mismo que contar caracteres.
fn byte_to_line(source: &str, byte: usize) -> usize {
    let end = byte.min(source.len());
    source.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count()
}

fn count_lines(source: &str) -> usize {
    source.lines().count()
}

fn extract_lines(source: &str, from: usize, to: usize) -> String {
    source.lines()
        .enumerate()
        .filter(|(i, _)| *i >= from && *i <= to)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El offset de un capture se mide contra el nodo y se aplica contra el
    /// archivo. Cuando las dos cosas no coinciden —el nodo se movió, la query
    /// matchea otro— el byte resultante puede caer adentro de un carácter, y en
    /// una spec en castellano eso pasa con cualquier `ó`.
    ///
    /// Es un dato posible, no una corrupción: tiene que resolver, no explotar.
    #[test]
    fn byte_to_line_survives_a_non_boundary_byte() {
        let source = "# Especificación\nsegunda\ntercera\n";
        let inside  = source.find('ó').unwrap() + 1;
        assert!(!source.is_char_boundary(inside), "el test no está partiendo nada");

        assert_eq!(byte_to_line(source, inside), 0);
        assert_eq!(byte_to_line(source, source.len()), 3);
        assert_eq!(byte_to_line(source, usize::MAX), 3);
    }
}
