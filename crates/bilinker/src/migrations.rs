//! Migraciones de formato de los metadatos de bilinker.
//!
//! Cada una es idempotente: correrla sobre una capa ya migrada no hace nada.
//! El registro de cuáles se aplicaron lo lleva `accreta-migrate` en el ledger
//! del repo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::Result;
use accreta_migrate::{Migration, Outcome};

use crate::bilink::{walkdir, BiLinkFile};
use crate::capture::CaptureFile;
use crate::link::LinkEndpoint;

/// Las migraciones de bilinker, en orden.
pub fn all() -> Vec<Migration> {
    vec![
        Migration {
            id:          "bilinker-001-capture-split",
            description: "extrae la ubicación de cada endpoint estructural a un .capture",
            run:         capture_split,
        },
    ]
}

/// Convierte los endpoints `file :: query :: offset` embebidos en el `.bilink`
/// a referencias `capture <uuid>` con su archivo aparte.
///
/// Es una transformación puramente sintáctica: no resuelve queries ni consulta
/// git. El `range` se copia tal cual estaba y el `state` del capture queda
/// vacío, para que un `check` posterior los refresque. Una migración que
/// resuelve puede fallar por motivos ajenos al formato —un archivo que se movió,
/// una query rota— y dejar la capa a mitad de camino.
fn capture_split(layer: &Path, dry_run: bool) -> Result<Outcome> {
    let bilink_dir = layer.join(".bilink");
    if !bilink_dir.exists() {
        return Ok(Outcome::default());
    }

    let mut out = Outcome::default();
    // Deduplicación: referencias idénticas describen la misma ubicación, así que
    // comparten capture. Sin esto la duplicación que ya existe queda congelada,
    // porque nada la fusionaría después.
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut created = 0usize;
    let mut reused  = 0usize;
    let mut dropped_subgraph = 0usize;

    let files: Vec<PathBuf> = walkdir(&bilink_dir)?.into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("bilink"))
        .filter(|p| !p.file_name().and_then(|n| n.to_str())
                      .map(|n| n.starts_with('.')).unwrap_or(false))
        .collect();

    for path in files {
        // Detectar campos obsoletos antes de reescribir, para poder reportarlos:
        // el parser los descarta en silencio.
        if let Ok(text) = std::fs::read_to_string(&path) {
            dropped_subgraph += text.lines()
                .filter(|l| l.starts_with("subgraph.0:") || l.starts_with("subgraph.1:"))
                .count();
        }

        let mut bl = BiLinkFile::load(&path)?;
        let mut changed = false;

        for n in [0u8, 1u8] {
            let Some(sref) = bl.link(n).legacy_sref().cloned() else { continue };

            let key = format!("{}\u{1}{}\u{1}{}",
                sref.file,
                sref.query.as_deref().unwrap_or(""),
                sref.range.as_ref().map(|r| r.to_string()).unwrap_or_default(),
            );

            let uuid = match seen.get(&key) {
                Some(existing) => { reused += 1; existing.clone() }
                None => {
                    let uuid = uuid::Uuid::new_v4().to_string();
                    let cap = CaptureFile {
                        uuid:        uuid.clone(),
                        sref,
                        // El range absoluto se copia del bilink; `check` lo refresca.
                        range:       if n == 0 { bl.range0.clone() } else { bl.range1.clone() },
                        state:       None,
                        resolved_at: None,
                    };
                    let cap_path = if dry_run {
                        CaptureFile::path_in(layer, &uuid)
                    } else {
                        cap.write_in(layer)?
                    };
                    out.changed.push(cap_path);
                    seen.insert(key, uuid.clone());
                    created += 1;
                    uuid
                }
            };

            *bl.link_mut(n) = LinkEndpoint::Capture(uuid);
            changed = true;
        }

        if changed {
            // `range.N` deja de existir en el bilink: la ubicación vive en el capture.
            bl.range0 = None;
            bl.range1 = None;
            if !dry_run {
                bl.write(&path)?;
            }
            out.changed.push(path);
        }
    }

    if created > 0 || reused > 0 {
        out.notes.push(format!(
            "{}: {created} capture(s) creado(s), {reused} endpoint(s) reusaron uno existente",
            layer.display(),
        ));
    }
    if dropped_subgraph > 0 {
        out.notes.push(format!(
            "{}: {dropped_subgraph} campo(s) subgraph.N descartado(s) — eliminados del formato",
            layer.display(),
        ));
    }

    Ok(out)
}
