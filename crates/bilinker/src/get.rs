use std::path::Path;
use anyhow::{bail, Context, Result};

use bilink_format::{BiLink, Capture, LinkEndpoint, Ranges, FRAGMENT_SEPARATOR};

use crate::cache::Cache;
use crate::state::CaptureState;
use crate::grammar;
use crate::query;
use bilink_format::link::StratumPath;

pub struct GetResult {
    /// El texto del fragmento: las partes unidas por el separador del `hash`.
    ///
    /// **Es lo que `--raw` imprime, y ya no es el default.** Lo que se lee en una
    /// terminal es [`view`]; esto sirve para comparar, y comparar lo hacen `check` y
    /// `--diff`. Ver `commands/get.md` § "`--raw` es el texto, y no es el default".
    pub content: String,
    /// El fragmento sobre sus líneas, con números y huecos. Vacío si no hay archivo
    /// que mostrar.
    pub view: String,
    pub file: String,
    /// Los tramos de líneas que se muestran, 1-based e inclusivos: uno por parte
    /// del fragmento. Son una lista y no un par porque un fragmento de varios
    /// `@target` no ocupa un tramo contiguo, y un par tendría que mentir eligiendo
    /// el que las abarca a todas.
    pub lines: Vec<(usize, usize)>,
}

impl GetResult {
    /// Los tramos como los imprime la salida: `12–14, 30–33`.
    pub fn line_span(&self) -> String {
        self.lines.iter()
            .map(|(a, b)| format!("{a}–{b}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
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
    let accepted = e.accepted.first()
        .context("el endpoint no tiene nada aceptado — correr `bilinker accept` primero")?;

    // **Cruzar la frontera se despacha antes de derivar nada.** El commit del
    // contenido aceptado de un endpoint repo vive en la historia del *proveedor*, no
    // en ésta: pedírselo a este repo es preguntarle por algo que nunca tuvo, y el
    // prólogo de abajo fallaría con "no aparece en los últimos commits del archivo"
    // — un mensaje cierto sobre la pregunta equivocada.
    match &e.link {
        LinkEndpoint::Repo(alias) => {
            return diff_across_frontier(root, alias, &uuid, accepted);
        }
        LinkEndpoint::Abstract => bail!(
            "el endpoint {endpoint} es `abstract`: es la punta abierta, y no hay \
             contra qué diffear de este lado"
        ),
        _ => {}
    }

    let mut cache = Cache::load(root);
    let range = e.link.capture_id().and_then(|id| cache.capture_ranges(id));

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
        LinkEndpoint::Issue(id) => bail!("el endpoint {endpoint} es un issue ({id})"),
        // Los dos de la frontera ya se despacharon arriba, antes de derivar.
        LinkEndpoint::Repo(_) | LinkEndpoint::Abstract => unreachable!(),
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
    stored_range: Option<&Ranges>,
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
        start_line: after_result.lines.first().map_or(1, |l| l.0),
        end_line: after_result.lines.last().map_or(1, |l| l.1),
        diff,
    })
}

fn git_show_fragment(root: &Path, commit: &str, file: &str, range: Option<&Ranges>) -> Result<String> {
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
        Some(r) => r.text(&source),
        None    => source.into_owned(),
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
        if let Ok(Some(f)) = query::find_fragment(language.clone(), source, &query_str) {
            return Some((f.ranges.start(), f.ranges.end()));
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
) -> Result<(std::path::PathBuf, Capture, Option<String>, Option<Ranges>, Option<String>)> {
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
    let range  = adjacent_bl.endpoint.get(n).link.capture_id().and_then(|id| cache.capture_ranges(id));
    let hash   = adjacent_bl.endpoint.get(n).accepted.first().map(|a| a.hash.clone());

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

/// La referencia que un endpoint que no resuelve **igual puede mostrar**.
///
/// El estado ya dice que el fragmento no está; lo que falta saber es **cuál era**,
/// para decidir a dónde repuntarlo. Y `UNRESOLVED` es el estado que obliga a
/// intervenir a mano: no tiene fix automático y no se resuelve aceptando.
///
/// Sin esto, la salida obliga a abrir el `.yaml` del capture y leerlo — que es
/// exactamente lo que el formato evita pedirle a nadie.
#[derive(Debug, Clone)]
pub struct Unresolved {
    pub file: String,
    pub capture: String,
    pub query: Option<String>,
    /// El nombre que la query busca, si lo tiene. Es lo que hay que ir a mirar.
    pub anchor: Option<String>,
    pub state: CaptureState,
    /// Qué falló, en una línea.
    pub reason: String,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "# {}", self.file)?;
        writeln!(f, "# capture {}…  ({})", &self.capture[..8.min(self.capture.len())], self.state)?;
        if let Some(q) = &self.query {
            writeln!(f, "query: {q}")?;
        }
        Ok(())
    }
}

fn resolve(
    root: &Path,
    cap: &Capture,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let file_path = root.join(&cap.file);
    // **El archivo no está tampoco es "no se pudo leer".** El estado sabe si se
    // movió, si el anchor se renombró, o si no está en ninguna parte, y de ahí sale
    // qué comando corresponde: se pregunta lo mismo que cuando la query no matchea.
    let source = std::fs::read_to_string(&file_path).map_err(|_| fail(root, cap))?;

    let Some(query_str) = &cap.query else {
        let total = count_lines(&source);
        let ranges = Ranges::one(0, source.len());
        let view = crate::preview::fragment_view(&source, &ranges, 0, 0);
        return Ok(GetResult {
            content: source,
            view,
            file: cap.file.clone(),
            lines: vec![(1, total)],
        });
    };

    let lang = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;

    let fragment = query::find_fragment(language, &source, query_str)?
        .ok_or_else(|| fail(root, cap))?;

    let before_rows = before.map(|(r, _)| r).unwrap_or(0);
    let after_rows  = after.map(|(r, _)| r).unwrap_or(0);
    let last_line   = count_lines(&source).saturating_sub(1);

    // Una parte por `@target`, cada una con su contexto y su tramo de líneas. Se
    // muestran unidas por el mismo separador que las une en el `hash`: lo que se
    // lee es el fragmento, no un recorte del archivo entre la primera y la última.
    let mut blocks = Vec::new();
    let mut lines  = Vec::new();
    for part in fragment.ranges.parts() {
        let line_start = byte_to_line(&source, part.start);
        let line_end   = byte_to_line(&source, part.end.saturating_sub(1));

        let ctx_start = line_start.saturating_sub(before_rows);
        let ctx_end   = (line_end + after_rows).min(last_line);

        blocks.push(extract_lines(&source, ctx_start, ctx_end));
        lines.push((ctx_start + 1, ctx_end + 1));
    }

    Ok(GetResult {
        content: blocks.join(FRAGMENT_SEPARATOR),
        view: crate::preview::fragment_view(&source, &fragment.ranges, before_rows, after_rows),
        file: cap.file.clone(),
        lines,
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

// ─── la referencia de un endpoint que no resuelve ─────────────────────────────

/// Arma la vista de un capture que no resolvió, **re-derivando su estado**.
///
/// El estado dice qué pasó de verdad —el anchor se renombró, el archivo se movió,
/// no está en ninguna parte— y de ahí sale qué comando corresponde. Sin él, decir
/// *"la query no matcheó"* es cierto y no sirve: es la observación, no la causa.
pub fn unresolved_for(root: &Path, cap: &Capture) -> Unresolved {
    let state = crate::check::resolve_capture(root, cap, None, None)
        .map(|(s, _)| s)
        .unwrap_or(CaptureState::Unanchored);

    let anchor = cap.query.as_deref().and_then(query::anchor_name);
    let reason = match state {
        CaptureState::Moved => format!(
            "el archivo se movió y el fragmento ya no está en `{}`.\n  \
             Repuntar con `bilinker apply`.",
            cap.file
        ),
        CaptureState::Reanchored => format!(
            "el anchor `{}` se renombró.\n  Repuntar con `bilinker apply`.",
            anchor.as_deref().unwrap_or("?")
        ),
        // **La causa que el mensaje de `apply` escondía.** `git diff -M` sólo ve lo
        // que git conoce, así que un archivo nuevo sin `git add` es invisible y el
        // rename no se detecta. Se busca el anchor entre los archivos sin trackear:
        // si aparece, es un hecho y se lo nombra, no una sugerencia genérica.
        _ => match (&anchor, anchor.as_deref().and_then(|a| untracked_with(root, a))) {
            (Some(a), Some(f)) => format!(
                "el anchor `{a}` está en `{f}`, que no está trackeado — por eso \
                 git no reporta el rename.\n  `git add {f}` y volver a correr."
            ),
            (Some(a), None) => format!(
                "el anchor `{a}` no está en el archivo.\n  \
                 Repuntar con `bilinker recapture <uuid>.<N> <archivo> <línea>:<col>`, \
                 o `bilinker remove <uuid>` si el fragmento ya no existe."
            ),
            (None, _) => format!(
                "el fragmento no se encontró en `{}`.\n  \
                 Repuntar con `bilinker recapture <uuid>.<N> <archivo> <línea>:<col>`.",
                cap.file
            ),
        },
    };

    Unresolved {
        file: cap.file.clone(),
        capture: cap.id(),
        query: cap.query.clone(),
        anchor,
        state,
        reason,
    }
}

/// El error de un endpoint que no resuelve: **la referencia primero, la causa
/// después.** Sigue siendo un error —no hay fragmento que devolver— y lo que cambia
/// es que la salida ya no obliga a abrir el `.yaml` del capture para leer la query.
fn fail(root: &Path, cap: &Capture) -> anyhow::Error {
    let u = unresolved_for(root, cap);
    anyhow::anyhow!("{u}\n{}", u.reason)
}

/// El primer archivo sin trackear que contiene ese nombre.
///
/// **Sin `.bilink/`.** Un capture guarda su query, y la query lleva el nombre del
/// anchor adentro: sin este filtro, el primer candidato que aparece es el archivo
/// del capture que se está tratando de resolver.
fn untracked_with(root: &Path, anchor: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C", &root.to_string_lossy(), "ls-files", "--others", "--exclude-standard"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|f| !f.split('/').any(|c| c == ".bilink"))
        .find(|f| {
            std::fs::read_to_string(root.join(f))
                .map(|t| t.contains(anchor))
                .unwrap_or(false)
        })
        .map(str::to_string)
}
