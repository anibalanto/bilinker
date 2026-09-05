use std::path::{Path, PathBuf};
use anyhow::{bail, Result};

/// Resuelve el id de un issue —un ítem del worklist— a su archivo, y devuelve la raíz
/// del proyecto.
///
/// Los ítems son archivos sueltos en el **panorama del worklist del proyecto**, con
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
/// El panorama del worklist de un proyecto: `.worklist/insecure/all`.
///
/// **No se busca: el nombre es de la spec, no del proyecto.** El worklist ya no es una
/// capa que lleve el nombre de quien la usa —`worklist-accreta`— sino un contenedor de
/// worktrees, y `insecure/all` se llama igual en todos porque lo nombra la spec de
/// sincronización.
///
/// **Y es el panorama, nunca una ventana.** Una ventana —`secure/sprint/<id>`— lleva
/// el subárbol de su sprint y nada más, así que resolver contra la que está abierta
/// haría que el mismo `issue 3a` resolviera o no según en qué rama esté el checkout, y
/// un bilink válido pasaría a no-OK sin que nadie toque nada.
///
/// `None` es "este proyecto no tiene worklist", que no es un error: el worklist se
/// clona aparte y un clon del proyecto no lo trae.
pub fn worklist_panorama(project_root: &Path) -> Option<PathBuf> {
    let dir = project_root.join(".worklist").join("insecure").join("all");
    dir.is_dir().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn worktree(root: &Path, branch: &str) {
        std::fs::create_dir_all(root.join(".worklist").join(branch)).unwrap();
    }

    fn item(root: &Path, branch: &str, name: &str) {
        worktree(root, branch);
        std::fs::write(root.join(".worklist").join(branch).join(name), "").unwrap();
    }

    /// El panorama está donde lo pone la spec, y no hay nombre que adivinar.
    #[test]
    fn the_panorama_is_where_the_spec_says() {
        let d = tempdir().unwrap();
        worktree(d.path(), "insecure/all");
        assert_eq!(
            worklist_panorama(d.path()),
            Some(d.path().join(".worklist").join("insecure").join("all"))
        );
    }

    /// Sin worklist no hay ítems, y eso no es un error: se clona aparte, así que un
    /// clon del proyecto no lo trae.
    #[test]
    fn a_project_without_a_worklist_has_no_items() {
        let d = tempdir().unwrap();
        assert_eq!(worklist_panorama(d.path()), None);
        assert_eq!(resolve_issue_path(d.path(), "3a").unwrap().0, None);
    }

    /// Una ventana no reemplaza al panorama. Si valiera, el mismo endpoint resolvería
    /// o no según qué sprint esté abierto.
    #[test]
    fn a_window_is_not_the_panorama() {
        let d = tempdir().unwrap();
        item(d.path(), "secure/sprint/10", "3a.task.md");
        assert_eq!(worklist_panorama(d.path()), None);
        assert_eq!(resolve_issue_path(d.path(), "3a").unwrap().0, None);
    }

    /// El ítem se encuentra por prefijo, sin que el endpoint diga el tipo.
    #[test]
    fn the_item_is_found_by_prefix_without_its_type() {
        let d = tempdir().unwrap();
        item(d.path(), "insecure/all", "3a.user-story.md");
        assert_eq!(
            resolve_issue_path(d.path(), "3a").unwrap().0,
            Some(d.path().join(".worklist/insecure/all/3a.user-story.md"))
        );
    }

    /// Y estando el panorama, un ítem que sólo vive en una ventana no se ve — el
    /// panorama los tiene todos, así que faltar ahí es no existir.
    #[test]
    fn an_item_only_in_a_window_does_not_resolve() {
        let d = tempdir().unwrap();
        worktree(d.path(), "insecure/all");
        item(d.path(), "secure/sprint/10", "3a.task.md");
        assert_eq!(resolve_issue_path(d.path(), "3a").unwrap().0, None);
    }
}

pub fn resolve_issue_path(layer_root: &Path, issue_id: &str) -> Result<(Option<PathBuf>, PathBuf)> {
    let project_root = project_root_of(layer_root);
    let Some(dir) = worklist_panorama(&project_root) else {
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
