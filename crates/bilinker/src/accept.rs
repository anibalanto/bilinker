//! `bilinker accept` — el único que escribe una decisión.
//!
//! Aceptar es decir *"revisé esto y lo apruebo"*, y hay **dos cosas que aprobar**:
//! dónde está el fragmento y qué dice. Se pueden aprobar juntas o por separado.

use std::path::Path;

use anyhow::{bail, Context, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{Accepted, BiLink, Capture, LinkEndpoint};

use crate::cache::Cache;
use crate::state::EndpointState;
use crate::{grammar, hash, query};

/// Qué dimensiones aprueba esta aceptación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct What {
    pub place:   bool,
    pub content: bool,
}

impl Default for What {
    /// Por defecto se aprueban las dos.
    fn default() -> Self { Self { place: true, content: true } }
}

impl What {
    pub fn place_only()   -> Self { Self { place: true,  content: false } }
    pub fn content_only() -> Self { Self { place: false, content: true } }
}

pub struct AcceptResult {
    pub uuid: String,
    pub n: u8,
    pub hash: String,
    pub commit: Option<String>,
}

/// Acepta un endpoint.
pub fn accept(layer: &Path, uuid: &str, n: u8, what: What) -> Result<AcceptResult> {
    let path = find_bilink_path(layer, uuid)?;
    let uuid = path.file_stem().and_then(|s| s.to_str())
        .context("el nombre del bilink no es un uuid")?.to_string();

    let mut bl = BiLink::load(&path)?;
    let mut cache = Cache::load(layer);

    let (accepted, commit) = compute(layer, &bl, &uuid, n, what, &cache)?;
    if let Some(c) = &commit {
        cache.set_commit(&uuid, n, c);
    }

    let e = bl.endpoint.get_mut(n);
    let hash = accepted.hash.clone();
    e.accepted = Some(accepted);
    bl.write(&path)?;

    // El estado cacheado describe la comparación anterior y ya no vale.
    cache.set_endpoint_state(&uuid, n, EndpointState::Ok);
    cache.save(layer)?;

    Ok(AcceptResult { uuid, n, hash, commit })
}

/// Calcula el bloque `accepted` para un endpoint.
fn compute(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    what: What,
    cache: &Cache,
) -> Result<(Accepted, Option<String>)> {
    let e = bl.endpoint.get(n);
    let previous = e.accepted.as_ref();

    match &e.link {
        LinkEndpoint::Capture(id) => {
            let cap = Capture::load_in(layer, id)?;
            let (state, range) = crate::check::resolve_capture(layer, &cap, previous, cache.commit(uuid, n))?;
            if !state.is_resolved() {
                bail!("el capture no resuelve ({state}): no se puede aprobar contenido \
                       que no se pudo localizar");
            }
            let range = range.context("el capture resolvió sin rango")?;

            // El fragmento tiene que estar commiteado.
            //
            // No es una recomendación: `commit` es el commit en que el fragmento
            // quedó con el contenido aprobado, y ese commit **no existe** si el
            // fragmento no está commiteado. Sin él no hay `git show`, y sin eso
            // `check` no puede recuperar el texto aceptado.
            if working_tree_dirty(layer, &cap.file) {
                bail!("{} tiene cambios sin commitear.\n       \
                       Aceptar fija un contenido, y ese contenido tiene que existir \
                       en la historia.", cap.file);
            }

            let source = std::fs::read_to_string(layer.join(&cap.file))?;
            let fragment = &source[range.start..range.end.min(source.len())];
            let content_hash = hash::sha256(fragment.as_bytes());
            let ast_hash = ast_hash_of(layer, &cap, &source)?;

            let accepted = Accepted {
                // Aprobar la ubicación es escribir el link vigente en `accepted`.
                link: if what.place {
                    Some(e.link.clone())
                } else {
                    previous.and_then(|a| a.link.clone())
                },
                hash: if what.content {
                    content_hash
                } else {
                    previous.map(|a| a.hash.clone())
                        .context("no hay contenido previo que conservar: aceptar con --place \
                                  exige que el endpoint ya tuviera algo aprobado")?
                },
                // Nunca se conserva un `hash_ast` que la gramática no puede
                // producir: uno guardado por una versión anterior sobreviviría a
                // cada `accept --place` y seguiría estando ahí para mentir.
                hash_ast: if what.content {
                    ast_hash
                } else if grammar::ast_discriminates_content(grammar::language_for_file(&cap.file)) {
                    previous.and_then(|a| a.hash_ast.clone())
                } else {
                    None
                },
            };

            // `commit` es el commit **del contenido**, no el HEAD de quien acepta.
            // Con el HEAD, el mismo acto daba distinto según quién y cuándo lo
            // hiciera, y el valor no describía nada del fragmento.
            let commit = what.content
                .then(|| content_commit(layer, &cap, &range))
                .flatten()
                .or_else(|| cache.commit(uuid, n).map(String::from));

            Ok((accepted, commit))
        }

        // Un endpoint `path` copia los **dos** valores del endpoint estructural de
        // su vecino: qué ubicación y qué contenido se aprobaron ahí.
        LinkEndpoint::Path(p) => {
            let target = stratum::resolve(layer, layer, p.tokens())
                .map_err(|e| anyhow::anyhow!("resolviendo el endpoint path: {e:?}"))?;
            let adj_path = layer.join(&target).join(".bilink").join(format!("{uuid}.yaml"));
            let adj = BiLink::load(&adj_path)
                .with_context(|| format!("leyendo el bilink vecino {}", adj_path.display()))?;
            let adj_accepted = adj.structural_accepted()
                .context("el vecino todavía no tiene ningún endpoint estructural aceptado; \
                          aceptarlo primero")?;
            Ok((adj_accepted.clone(), None))
        }

        // Un `issue` no lleva `accepted.link`: la ubicación de un ítem es su id.
        LinkEndpoint::Issue(id) => {
            let (item, root) = crate::issue::resolve_issue_path(layer, id)?;
            let item = item.with_context(|| format!("no hay ítem de worklist con id '{id}'"))?;
            let text = std::fs::read_to_string(&item)
                .with_context(|| format!("leyendo {}", item.display()))?;
            let rel = item.strip_prefix(&root).unwrap_or(&item).display().to_string();
            Ok((
                Accepted { link: None, hash: hash::sha256(text.as_bytes()), hash_ast: None },
                crate::git::try_head_commit_for_file(&root, &rel),
            ))
        }
    }
}

/// Acepta todo lo que necesita atención en la capa.
///
/// Existe para el caso en que ya se revisó todo, no para el caso en que no se
/// revisó nada: cada estado no-OK es un puntero al fragmento que hay que mirar.
pub fn accept_all(layer: &Path) -> Result<Vec<AcceptResult>> {
    let cache = Cache::load(layer);
    let mut out = Vec::new();

    for path in bilink_files(&layer.join(".bilink")) {
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(bl) = BiLink::load(&path) else { continue };

        for n in [0u8, 1u8] {
            let needs = match cache.endpoint_state(uuid, n) {
                Some(s) => !s.is_ok(),
                None    => bl.endpoint.get(n).accepted.is_none(),
            };
            if !needs { continue; }
            match accept(layer, uuid, n, What::default()) {
                Ok(r)  => out.push(r),
                Err(e) => eprintln!("warn  {}.{n}: {e}", &uuid[..8.min(uuid.len())]),
            }
        }
    }
    Ok(out)
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// ¿El archivo tiene cambios sin commitear?
fn working_tree_dirty(layer: &Path, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "status", "--porcelain", "--", file])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// El commit en que el fragmento quedó con este contenido.
///
/// `git log -L` recorre la historia de un rango de líneas: su primer commit es
/// aquel en que las líneas quedaron como están. Es nativo y offline.
fn content_commit(layer: &Path, cap: &Capture, range: &bilink_format::ByteRange) -> Option<String> {
    let source = std::fs::read_to_string(layer.join(&cap.file)).ok()?;
    let line_of = |byte: usize| source[..byte.min(source.len())].lines().count().max(1);
    let (a, b) = (line_of(range.start), line_of(range.end));

    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "log", "-L",
               &format!("{a},{b}:{}", cap.file), "--format=%H", "-s", "-n", "1"])
        .output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout).lines().next().map(str::to_string)
}

/// El hash de la s-expression, **sólo donde el AST discrimina el contenido**.
fn ast_hash_of(layer: &Path, cap: &Capture, source: &str) -> Result<Option<String>> {
    let _ = layer;
    let Some(q) = &cap.query else { return Ok(None) };
    let lang = grammar::language_for_file(&cap.file);
    if !grammar::ast_discriminates_content(lang) { return Ok(None); }
    let Ok(language) = grammar::for_language(lang) else { return Ok(None) };
    Ok(query::find_target_with_sexp(language, source, q)?
        .map(|(_, _, sexp)| hash::sha256(sexp.as_bytes())))
}

/// El bilink cuyo uuid empieza con el prefijo dado.
pub fn find_bilink_path(layer: &Path, prefix: &str) -> Result<std::path::PathBuf> {
    let hits: Vec<_> = bilink_files(&layer.join(".bilink")).into_iter()
        .filter(|p| p.file_stem().and_then(|s| s.to_str())
                     .map(|s| s.starts_with(prefix)).unwrap_or(false))
        .collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().expect("uno")),
        0 => bail!("no hay bilink que empiece con '{prefix}'"),
        n => bail!("'{prefix}' es ambiguo: {n} bilinks coinciden"),
    }
}
