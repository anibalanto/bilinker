use std::path::{Path, PathBuf};
use anyhow::{bail, Result};

/// Resuelve el id de un issue —un ítem del worklist— a su archivo, y devuelve la raíz
/// del proyecto.
///
/// Los ítems son archivos sueltos en `<project-root>/.stratum/worklist/`, con nombre
/// `<id>.<tipo>.md`. El tipo no viaja en el endpoint, así que el archivo se busca por
/// el prefijo `<id>.` en ese único directorio: sin recursión y sin índice.
///
/// Que el tipo quede afuera es lo que hace que el endpoint sobreviva a la
/// planificación. Recolgar un ítem de otra user story cambia un campo del ítem, no el
/// nombre de su archivo, así que el vínculo no se entera.
///
/// `Ok(None)` es "no hay ítem con ese id". Dos archivos con el mismo id son un error
/// del worklist —los ids son únicos— y no una ambigüedad del formato, así que se
/// reporta en vez de elegir uno.
pub fn resolve_issue_path(layer_root: &Path, issue_id: &str) -> Result<(Option<PathBuf>, PathBuf)> {
    let project_root = project_root_of(layer_root);
    let dir = project_root.join(".stratum").join("worklist");
    let prefix = format!("{issue_id}.");

    let mut hits: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(".md"))
                .unwrap_or(false)
        })
        .collect();
    hits.sort();

    match hits.len() {
        0 => Ok((None, project_root)),
        1 => Ok((hits.pop(), project_root)),
        _ => {
            let names: Vec<String> = hits.iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
            bail!(
                "el id de issue '{issue_id}' matchea {} archivos en {}: {}",
                names.len(), dir.display(), names.join(", ")
            );
        }
    }
}

/// La raíz del proyecto, contando la profundidad Stratum de la capa.
///
/// Cada nivel Stratum son dos componentes de path (`.stratum/<name>`), así que subir
/// `depth * 2` desde la raíz de la capa da la raíz del proyecto.
fn project_root_of(layer_root: &Path) -> PathBuf {
    let d = stratum::depth(layer_root);
    let canonical = layer_root.canonicalize().unwrap_or_else(|_| layer_root.to_path_buf());
    canonical.ancestors()
        .nth(d * 2)
        .unwrap_or(&canonical)
        .to_path_buf()
}
