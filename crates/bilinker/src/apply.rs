//! `bilinker apply` — corrige **dónde apunta** un endpoint, y nada más.
//!
//! Acuña el capture de la ubicación nueva y repunta el `link`. **No escribe
//! `accepted`**, así que el endpoint no queda `Ok`: queda en `Relocated` hasta que
//! alguien apruebe la ubicación nueva.
//!
//! > `apply` propone, `accept` dispone.
//!
//! No hay fork ni copy-on-write. Un capture es inmutable y su id es el hash de su
//! ubicación, así que corregir una ubicación **siempre** produce un capture
//! distinto: se acuña y se repunta un solo `link`. Los demás referentes no se
//! enteran, sin que haya que decidir nada.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{BiLink, ByteRange, Capture};

use crate::cache::Cache;
use crate::state::{CaptureState, EndpointState};
use crate::{grammar, query};

pub struct PendingFix {
    pub bilink_path: PathBuf,
    pub uuid: String,
    pub n: u8,
    pub from: Capture,
    pub to: Capture,
    /// Qué lo motivó. Sólo informativo: el fix es el mismo — una ubicación nueva.
    pub reason: &'static str,
}

impl PendingFix {
    pub fn short(&self) -> &str { &self.uuid[..8.min(self.uuid.len())] }

    pub fn description(&self) -> String {
        if self.from.file != self.to.file {
            format!("{} → {}", self.from.file, self.to.file)
        } else if self.from.query != self.to.query {
            format!("query → {}", self.to.query.as_deref().unwrap_or("(archivo entero)"))
        } else {
            format!("offset → {}", self.to.offset.as_ref()
                .map(|o| o.to_string()).unwrap_or_else(|| "(nodo entero)".into()))
        }
    }
}

/// Recorre la capa y calcula la ubicación nueva de cada endpoint que la necesite.
///
/// **Nunca deriva el fix de la cache**: re-resuelve contra git y el AST actuales, y
/// descarta el fix si el estado re-derivado no coincide con el cacheado.
pub fn scan_fixeable(layer: &Path) -> Result<Vec<PendingFix>> {
    let cache = Cache::load(layer);
    let mut fixes = Vec::new();

    for path in bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        for n in [0u8, 1u8] {
            let e = bl.endpoint.get(n);
            let Some(cap_id) = e.link.capture_id() else { continue };
            let Ok(cap) = Capture::load_in(layer, cap_id) else { continue };

            let (state, _) = crate::check::resolve_capture(
                layer, &cap, e.accepted.as_ref(), cache.commit(uuid, n))?;
            let endpoint_state = cache.endpoint_state(uuid, n);

            let (to, reason) = match (state, endpoint_state) {
                (CaptureState::Moved, _) => match compute_moved(layer, &cap)? {
                    Some(c) => (c, "MOVED"),
                    None => continue,
                },
                (CaptureState::Reanchored, _) => match compute_reanchored(layer, &bl, uuid, n, &cap)? {
                    Some(c) => (c, "REANCHORED"),
                    None => continue,
                },
                (CaptureState::Resolved, Some(cached)) if cached.has_fix() => {
                    // **Validación de frescura.** El estado cacheado lo escribió el
                    // último `check`, y el archivo pudo cambiar después: aplicar un
                    // fix derivado de la cache es corregir contra una foto vieja.
                    let Some(accepted) = e.accepted.as_ref() else { continue };
                    let cached_commit = cache.commit(uuid, n).map(str::to_string);
                    let range = cache.capture_range(cap_id);
                    let mut derive = || cached_commit.clone().or_else(
                        || crate::capture::derive_commit(layer, &cap, &accepted.hash));
                    let mut src = crate::check::CommitSource {
                        cached: cached_commit.as_deref(),
                        derive: &mut derive,
                    };
                    let fresh = crate::check::compare_content(
                        layer, &cap, accepted, range.as_ref(), &mut src, None)?;

                    if fresh == EndpointState::Ok {
                        // El fix ya no hace falta. Se omite en silencio: que algo se
                        // haya arreglado solo no es una anomalía que reportar.
                        continue;
                    }
                    if fresh != cached {
                        eprintln!(
                            "warn: {}… endpoint.{n}: la cache dice {cached} y la \
                             resolución actual da {fresh}\n                                   — fix descartado. Correr `bilinker check`.",
                            &uuid[..8.min(uuid.len())]);
                        continue;
                    }
                    match compute_offset(layer, &bl, uuid, n, &cap, cached) {
                        Ok(c) => (c, if cached == EndpointState::Expanded { "EXPANDED" } else { "DISPLACED" }),
                        Err(why) => {
                            eprintln!("warn: {}… endpoint.{n}: {why}",
                                      &uuid[..8.min(uuid.len())]);
                            continue;
                        }
                    }
                }
                _ => continue,
            };

            // Un fix que no mueve nada es un no-op.
            if to.id() == cap.id() { continue; }

            fixes.push(PendingFix {
                bilink_path: path.clone(),
                uuid: uuid.to_string(),
                n, from: cap, to, reason,
            });
        }
    }
    Ok(fixes)
}

/// Acuña el capture nuevo y repunta el `link`. **No toca `accepted`.**
pub fn apply_fix(layer: &Path, pf: &PendingFix) -> Result<Vec<PathBuf>> {
    let (id, cap_path, _existed) = pf.to.write_in(layer)?;

    let mut bl = BiLink::load(&pf.bilink_path)?;
    bl.endpoint.get_mut(pf.n).link = format!("capture {id}").parse()?;
    bl.write(&pf.bilink_path)?;

    // Repuntar no aprueba: el endpoint queda pidiendo una decisión humana.
    let mut cache = Cache::load(layer);
    cache.set_endpoint_state(&pf.uuid, pf.n, EndpointState::Relocated);
    cache.save(layer)?;

    Ok(vec![cap_path, pf.bilink_path.clone()])
}

// ─── cálculo de la ubicación nueva ────────────────────────────────────────────

/// MOVED: el índice de renames de git.
fn compute_moved(layer: &Path, cap: &Capture) -> Result<Option<Capture>> {
    let Some(new_file) = crate::check::git_renamed_to(layer, &cap.file) else {
        // Tres cosas distintas terminan acá y conviene no confundirlas: que el
        // destino no esté trackeado, que el anchor se haya renombrado también, o que
        // el fragmento no esté en ninguna parte.
        bail!("MOVED: git no reporta un rename de '{}'. Si el archivo nuevo no está \
               trackeado, `git add` y volver a correr.", cap.file);
    };
    let moved = Capture { file: new_file, ..cap.clone() };
    // Verificar que la referencia siga resolviendo en el path nuevo. Sin aceptación:
    // lo que se pregunta es si el anchor está ahí, no si dice lo que se aprobó.
    let (state, _) = crate::check::resolve_capture(layer, &moved, None, None)?;
    if !state.is_resolved() {
        bail!("MOVED: el archivo se movió a '{}', pero el anchor ya no está ahí ({state}). \
               Repuntar con `bilinker recapture`.", moved.file);
    }
    Ok(Some(moved))
}

/// REANCHORED: la query relajada, con el nombre nuevo.
fn compute_reanchored(
    layer: &Path, bl: &BiLink, uuid: &str, n: u8, cap: &Capture,
) -> Result<Option<Capture>> {
    let Some(query_str) = &cap.query else { return Ok(None) };
    let path = layer.join(&cap.file);
    if !path.exists() { return Ok(None); }

    let source   = std::fs::read_to_string(&path)?;
    let language = grammar::for_language(grammar::language_for_file(&cap.file))?;
    let accepted = bl.endpoint.get(n).accepted.as_ref();

    // Los **dos** hacen falta: el hash dice qué texto buscar y el commit dice de
    // dónde sacarlo. Con uno solo, `accepted_text` no devuelve nada y no hay contra
    // qué puntuar los candidatos.
    let mut cache = Cache::load(layer);
    let commit = accepted.and_then(|a| cache.commit_or_derive(layer, uuid, n, cap, &a.hash));
    let Some((new_name, score)) = crate::check::find_renamed_anchor(
        layer, language, &source, query_str, cap,
        accepted.map(|a| a.hash.as_str()), commit.as_deref())?
    else {
        bail!("REANCHORED: el anchor ya no se localiza — correr `bilinker check`");
    };
    let Some(new_query) = query::rewrite_name_predicate(query_str, &new_name) else {
        bail!("REANCHORED: la query no tiene predicado de nombre para reescribir");
    };
    eprintln!("  anchor → {new_name}  (similitud {:.0}%)", score * 100.0);
    Ok(Some(Capture { query: Some(new_query), ..cap.clone() }))
}

/// DISPLACED y EXPANDED: el offset se corre o se amplía.
/// La ubicación nueva de un DISPLACED o un EXPANDED, o **por qué no la hay**.
///
/// `check` dice que hay un fix disponible; acá se produce, y a veces no se puede.
/// Devolver `None` en cada uno de esos casos dejaba a `check` reportando un estado
/// con fix y a `apply` contestando que no hay nada que hacer, sin nadie que
/// explicara la contradicción. El `Err` es el motivo, y `apply` lo imprime.
fn compute_offset(
    layer: &Path, bl: &BiLink, uuid: &str, n: u8, cap: &Capture, state: EndpointState,
) -> Result<Capture> {
    let Some(accepted) = bl.endpoint.get(n).accepted.as_ref() else {
        bail!("{state}, pero el endpoint no tiene nada aprobado que buscar");
    };
    let mut cache = Cache::load(layer);
    let Some(commit) = cache.commit_or_derive(layer, uuid, n, cap, &accepted.hash) else {
        bail!("{state}, pero el contenido aceptado no se ubica en la historia del \
               archivo — ni en la cache ni en los últimos commits");
    };
    let Some(text) = crate::capture::accepted_text(layer, cap, &commit, Some(&accepted.hash)) else {
        bail!("{state}, pero git no entrega el texto aceptado en {commit} — sin él no \
               hay qué buscar");
    };

    let source = std::fs::read_to_string(layer.join(&cap.file))?;
    let Some(q) = &cap.query else {
        bail!("{state} sobre un capture de archivo completo: no hay nodo al que \
               referir un offset");
    };
    let language = grammar::for_language(grammar::language_for_file(&cap.file))?;
    let Some((node_start, node_end, _)) = query::find_target_with_sexp(language, &source, q)? else {
        bail!("{state}, pero la query ya no resuelve — repuntar con `bilinker recapture`");
    };
    let node = &source[node_start..node_end.min(source.len())];

    let offset = match state {
        // Creció alrededor de lo aceptado: el offset abarca el nodo entero.
        EndpointState::Expanded => None,
        // Se corrió: el offset apunta a donde está ahora.
        //
        // `check` busca el texto en todo el archivo y acá se busca dentro del
        // nodo, que es donde un offset puede nombrarlo. Cuando el texto quedó
        // fuera del nodo los dos tienen razón y no hay offset que sirva.
        EndpointState::Displaced => match node.find(&text) {
            Some(p) => Some(ByteRange { start: p, end: p + text.len() }),
            None => bail!("DISPLACED, pero el texto aceptado no está dentro del nodo \
                           — un offset no puede nombrarlo. Repuntar con \
                           `bilinker recapture`"),
        },
        other => bail!("{other} no es un estado con fix de offset"),
    };
    Ok(Capture { offset, ..cap.clone() })
}
