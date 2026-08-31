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
use bilink_format::{BiLink, Capture};

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
        } else {
            format!("query → {}", self.to.query.as_deref().unwrap_or("(archivo entero)"))
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

            // **El estado se re-deriva; la cache no decide nada acá.**
            //
            // El estado cacheado lo escribió el último `check`, y el archivo pudo
            // cambiar después: aplicar un fix derivado de esa foto es corregir
            // contra algo que ya no está. De la cache sólo sale `commit`, que es
            // un dato de git y no una conclusión sobre el árbol actual.
            let (state, _) = crate::check::resolve_capture(
                layer, &cap, e.accepted.as_ref(), cache.commit(uuid, n))?;

            let calculado = match state {
                CaptureState::Moved      => compute_moved(layer, &cap).map(|c| (c, "MOVED")),
                CaptureState::Reanchored =>
                    compute_reanchored(layer, &bl, uuid, n, &cap).map(|c| (c, "REANCHORED")),
                _ => continue,
            };

            // Un fix que no se puede calcular **se reporta**. Propagarlo con `?`
            // abortaba el scan entero: un solo endpoint sin arreglo dejaba sin
            // revisar a todos los demás.
            let (to, reason) = match calculado {
                Ok((Some(c), reason)) => (c, reason),
                Ok((None, _)) => continue,
                Err(why) => {
                    eprintln!("warn: {}… endpoint.{n}: {why}", &uuid[..8.min(uuid.len())]);
                    continue;
                }
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
///
/// **Tres cosas distintas terminan en "no se pudo", y cada una manda a mirar un
/// lugar distinto.** Confundirlas es lo que hacía que el mensaje culpara a git —el
/// índice de renames— cuando el que había cambiado era el anchor:
///
/// | Lo que pasó | Estado | Quién lo explica |
/// |---|---|---|
/// | el destino está, pero el anchor se renombró también | `MOVED` | **acá** |
/// | git no detectó el rename: el destino no está trackeado | sin fix | [`get`](crate::get) |
/// | el fragmento no está en ninguna parte | sin fix | [`get`](crate::get) |
///
/// Las dos últimas **no llegan a `apply`**: dejan el capture en un estado sin fix, y
/// `apply` no los toca. Que el mensaje de acá pretendiera cubrirlas era parte del
/// mismo error — describía condiciones que este camino no puede observar.
fn compute_moved(layer: &Path, cap: &Capture) -> Result<Option<Capture>> {
    let Some(new_file) = crate::check::git_renamed_to(layer, &cap.file) else {
        // **Casi inalcanzable, y por eso el mensaje no diagnostica nada.** Se entra
        // acá sólo con `CaptureState::Moved`, que ya implica que git reportó el
        // rename; llegar significa que el árbol cambió entre el `check` y esta
        // línea. Las otras dos causas —destino sin trackear, fragmento en ninguna
        // parte— **no pasan por acá**: dejan el capture en un estado sin fix, y
        // quien las explica es [`get`](crate::get), que es donde se pregunta qué
        // pasó con un endpoint que no resuelve.
        bail!(
            "MOVED: git ya no reporta el rename de '{}' — el árbol cambió mientras \
             se calculaba.\n      Correr `bilinker check .` de nuevo.",
            cap.file
        );
    };
    let moved = Capture { file: new_file, ..cap.clone() };
    // Verificar que la referencia siga resolviendo en el path nuevo. Sin aceptación:
    // lo que se pregunta es si el anchor está ahí, no si dice lo que se aprobó.
    let (state, _) = crate::check::resolve_capture(layer, &moved, None, None)?;
    if !state.is_resolved() {
        // **Nombrar el anchor, no el estado.** Es el caso de MOVED y REANCHORED a la
        // vez, que ningún estado expresa porque los dos son de resolución y el
        // capture guarda uno solo. Y no hay auto-fix: dónde quedó el fragmento
        // adentro del archivo destino es una inferencia que `apply` no debería hacer
        // sola. Lo que sí puede es decir qué comando la hace.
        bail!(
            "MOVED: el archivo se movió a '{}', pero el anchor {} ya no está ahí \
             ({state}).\n      Repuntar con `bilinker recapture`.",
            moved.file,
            named(cap),
        );
    }
    Ok(Some(moved))
}

/// El anchor entre backticks, o una frase que no finge saber su nombre.
fn named(cap: &Capture) -> String {
    match cap.query.as_deref().and_then(query::anchor_name) {
        Some(a) => format!("`{a}`"),
        None => "capturado".to_string(),
    }
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

