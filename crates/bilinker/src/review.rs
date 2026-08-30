//! La superficie de revisión: `status`, `diff` y `log` sobre el índice y la ref
//! propios.
//!
//! **La forja no muestra la ref**, así que esto pasa a ser parte del producto y no
//! una comodidad. Es lo que contesta por qué la aceptación no necesita un archivo
//! propio para ser revisable: el artefacto revisable es el commit, y el registro de
//! decisiones es `log --first-parent`.
//!
//! Sin el índice propio nada de esto existiría: los cambios que escribe `accept`
//! son, para el índice del proyecto, archivos ignorados, y la ref donde cuentan no
//! está checkouteada. Serían invisibles de los dos lados.

use std::path::Path;

use anyhow::Result;

use crate::bilink_ref::Repo;

/// Qué le pasó a un archivo de `.bilink/` respecto del commit de la ref.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
}

impl Change {
    pub fn letter(self) -> char {
        match self { Self::Added => 'A', Self::Modified => 'M', Self::Deleted => 'D' }
    }
}

/// El `git status` de bilinker: qué cambió en `.bilink/` desde el commit que
/// [`head`](crate::bilink_ref) nombra.
///
/// Es contra `head` y no contra el tip de la ref a propósito: la pregunta es *"¿qué
/// escribí yo que todavía no está en ningún commit?"*, y lo que el árbol tiene
/// enfrente es el commit del que salió.
pub fn status(dir: &Path) -> Result<Vec<(Change, String)>> {
    let repo = Repo::open(dir)?;
    let Some(head) = repo.read_head() else { return Ok(Vec::new()) };

    let want = repo.bilink_paths_in(&head.commit)?;
    let have = repo.tracked_bilink_files()?;

    let mut out = Vec::new();
    for path in &have {
        if !want.contains(path) {
            out.push((Change::Added, path.clone()));
        }
    }
    for path in repo.dirty_against(&head.commit)? {
        if want.contains(&path) && have.contains(&path) {
            out.push((Change::Modified, path));
        }
    }
    for path in &want {
        if !have.contains(path) {
            out.push((Change::Deleted, path.clone()));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// El diff de `.bilink/` contra un commit de la ref. Por defecto, contra `head`.
///
/// Va por el índice propio: `git diff` del proyecto no lo mostraría, porque para él
/// esos archivos están excluidos.
pub fn diff(dir: &Path, against: Option<&str>) -> Result<String> {
    let repo = Repo::open(dir)?;
    let base = match against {
        Some(r) => r.to_string(),
        None => match repo.read_head() {
            Some(h) => h.commit,
            None => return Ok(String::new()),
        },
    };

    // El scratch va en `.git/bilink/`, al lado del índice propio: es por clon y no se
    // versiona, y evita que un comando de lectura dependa de un temporal del sistema.
    let scratch = repo.index_path()?.with_file_name("diff");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;

    let mut out = String::new();
    for (change, path) in status(dir)? {
        let (old_side, new_side) = match change {
            Change::Added   => ("/dev/null".to_string(), path.clone()),
            Change::Deleted => (blob_to_file(&repo, &base, &path, &scratch)?, "/dev/null".into()),
            Change::Modified => (blob_to_file(&repo, &base, &path, &scratch)?, path.clone()),
        };
        // `--no-index` sale con 1 cuando los archivos difieren, que es siempre acá:
        // el código de salida no significa falla.
        out.push_str(&format!("{} {}\n", change.letter(), path));
        out.push_str(&repo.git_lenient(&["diff", "--no-index", "--", &old_side, &new_side]));
    }
    Ok(out)
}

/// El contenido de un blob del commit, en un archivo suelto para poder diffear.
fn blob_to_file(repo: &Repo, commit: &str, path: &str, dir: &Path) -> Result<String> {
    let blob = repo.git(&["show", &format!("{commit}:{path}")])?;
    let dest = dir.join(path.replace('/', "_"));
    std::fs::write(&dest, blob)?;
    Ok(dest.to_string_lossy().into_owned())
}

/// El registro de decisiones: los commits **propios** de la ref, del más nuevo al
/// corte.
///
/// `git log --first-parent` sobre la ref no alcanza — al llegar al corte sigue por
/// la historia del proyecto. Ver [`Repo::ref_chain`].
///
/// Con `excluding`, sólo los que `branch` tiene y esa otra rama no: es el
/// `log --first-parent <suya> ^<mía>` que contesta *"¿qué actos hubo del otro
/// lado?"* antes de un [`adopt`](crate::adopt).
pub fn log(dir: &Path, branch: Option<&str>, excluding: Option<&str>) -> Result<Vec<String>> {
    let repo = Repo::open(dir)?;
    let branch = match branch {
        Some(b) => repo.resolve_branch_name(b),
        None    => repo.require_branch()?,
    };

    let already: Vec<String> = match excluding {
        Some(other) => repo.ref_chain(&repo.resolve_branch_name(other))?,
        None => Vec::new(),
    };

    let mut out = Vec::new();
    for commit in repo.ref_chain(&branch)? {
        if already.contains(&commit) {
            continue;
        }
        out.push(repo.git(&["log", "-1", "--format=%h %an  %s", &commit])?.trim().to_string());
    }
    Ok(out)
}
