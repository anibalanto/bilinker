use std::path::{Path, PathBuf};
use anyhow::{bail, Result};

/// Resuelve el id de un issue —un ítem del worklist— a su archivo, y devuelve la raíz
/// del proyecto.
///
/// Los ítems son archivos sueltos en la **capa de worklist del proyecto**, con
/// nombre `<id>.<tipo>.md`. El tipo no viaja en el endpoint, así que el archivo se
/// busca por el prefijo `<id>.` en ese único directorio: sin recursión y sin índice.
///
/// Que el tipo quede afuera es lo que hace que el endpoint sobreviva a la
/// planificación. Recolgar un ítem de otra user story cambia un campo del ítem, no el
/// nombre de su archivo, así que el vínculo no se entera.
///
/// `Ok(None)` es "no hay ítem con ese id". Dos archivos con el mismo id son un error
/// del worklist —los ids son únicos— y no una ambigüedad del formato, así que se
/// reporta en vez de elegir uno.
/// La capa de worklist de un proyecto: `.stratum/worklist*`.
///
/// **Se busca, no se sabe su nombre.** El worklist de un proyecto es suyo y lleva su
/// nombre —`worklist-accreta`—, igual que el daemon dejó de llamarse
/// `lattice-daemon` cuando resultó que no era de lattice. Hardcodear un nombre acá
/// le metería el de **un** proyecto a una herramienta que se usa en cualquiera.
///
/// El prefijo es la convención, como todo lo demás: nadie declara dónde está su
/// `.bilink/` ni su capa `impl`. Con dos candidatas no se elige una — eso es una
/// ambigüedad del proyecto y no del formato.
pub fn worklist_layer(project_root: &Path) -> Option<PathBuf> {
    let mut hits: Vec<PathBuf> = std::fs::read_dir(project_root.join(".stratum"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("worklist"))
            .unwrap_or(false))
        .collect();
    hits.sort();
    match hits.len() {
        1 => hits.pop(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn layer(root: &Path, name: &str) {
        std::fs::create_dir_all(root.join(".stratum").join(name)).unwrap();
    }

    /// Se encuentra por el prefijo, se llame como se llame: el worklist de un
    /// proyecto lleva su nombre, y la herramienta no lo sabe de antemano.
    #[test]
    fn the_layer_is_found_by_prefix_whatever_the_project_calls_it() {
        for name in ["worklist", "worklist-accreta", "worklist-otracosa"] {
            let d = tempdir().unwrap();
            layer(d.path(), name);
            assert_eq!(worklist_layer(d.path()), Some(d.path().join(".stratum").join(name)));
        }
    }

    /// Sin capa no hay ítems, y eso no es un error: un proyecto puede no tener
    /// worklist.
    #[test]
    fn a_project_without_a_worklist_layer_has_no_items() {
        let d = tempdir().unwrap();
        assert_eq!(worklist_layer(d.path()), None);
        assert_eq!(resolve_issue_path(d.path(), "3a").unwrap().0, None);
    }

    /// Con dos no se elige una: es una ambigüedad del proyecto, no del formato.
    #[test]
    fn two_candidates_are_not_disambiguated() {
        let d = tempdir().unwrap();
        layer(d.path(), "worklist-uno");
        layer(d.path(), "worklist-dos");
        assert_eq!(worklist_layer(d.path()), None);
    }

    /// Y no se confunde con otra capa cualquiera.
    #[test]
    fn another_layer_is_not_the_worklist() {
        let d = tempdir().unwrap();
        layer(d.path(), "impl");
        assert_eq!(worklist_layer(d.path()), None);
    }
}

pub fn resolve_issue_path(layer_root: &Path, issue_id: &str) -> Result<(Option<PathBuf>, PathBuf)> {
    let project_root = project_root_of(layer_root);
    let Some(dir) = worklist_layer(&project_root) else {
        return Ok((None, project_root));
    };
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
