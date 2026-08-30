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
                (CaptureState::Resolved, Some(s)) if s.has_fix() => {
                    match compute_offset(layer, &bl, uuid, n, &cap, s)? {
                        Some(c) => (c, if s == EndpointState::Expanded { "EXPANDED" } else { "DISPLACED" }),
                        None => continue,
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
    // Verificar que la referencia siga resolviendo en el path nuevo.
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
    let commit = Cache::load(layer);
    let commit = commit.commit(uuid, n);
    let Some((new_name, score)) = crate::check::find_renamed_anchor(
        layer, language, &source, query_str, cap,
        accepted.map(|a| a.hash.as_str()), commit)?
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
fn compute_offset(
    layer: &Path, bl: &BiLink, uuid: &str, n: u8, cap: &Capture, state: EndpointState,
) -> Result<Option<Capture>> {
    let Some(accepted) = bl.endpoint.get(n).accepted.as_ref() else { return Ok(None) };
    let cache = Cache::load(layer);
    let Some(commit) = cache.commit(uuid, n) else { return Ok(None) };
    let Some(text) = crate::capture::accepted_text(layer, cap, commit, Some(&accepted.hash)) else {
        return Ok(None);
    };

    let source = std::fs::read_to_string(layer.join(&cap.file))?;
    let Some(q) = &cap.query else { return Ok(None) };
    let language = grammar::for_language(grammar::language_for_file(&cap.file))?;
    let Some((node_start, node_end, _)) = query::find_target_with_sexp(language, &source, q)? else {
        return Ok(None);
    };
    let node = &source[node_start..node_end.min(source.len())];

    let offset = match state {
        // Creció alrededor de lo aceptado: el offset abarca el nodo entero.
        EndpointState::Expanded => None,
        // Se corrió: el offset apunta a donde está ahora.
        EndpointState::Displaced => {
            let pos = node.find(&text).map(|p| ByteRange { start: p, end: p + text.len() });
            match pos { Some(r) => Some(r), None => return Ok(None) }
        }
        _ => return Ok(None),
    };
    Ok(Some(Capture { offset, ..cap.clone() }))
}
