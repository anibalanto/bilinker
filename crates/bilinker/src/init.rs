//! `bilinker init` — la puesta a punto del clon.
//!
//! Es lo primero que corre cualquiera que vaya a usar bilinker. Todo lo que la ref
//! necesita son tres cosas puestas en el clon, y ninguna viaja con él: la exclusión
//! en `.git/info/exclude`, el refspec en `.git/config`, y el `.bilink/`
//! materializado en el árbol. Son **por clon**, no por rama ni por commit.
//!
//! Es idempotente y es **por repo**, no por capa: un solo patrón `.bilink/` en el
//! exclude cubre todas las capas de ese repo, estén donde estén.

use std::path::Path;

use anyhow::Result;

use crate::bilink_ref::Repo;
use crate::config;

pub struct InitResult {
    pub excluded:  Vec<&'static str>,
    pub refspec:   Vec<String>,
    pub fetched:   Option<String>,
    pub branch:    Option<String>,
    pub outcome:   Outcome,
}

pub enum Outcome {
    /// Se materializó el `.bilink/` de la rama actual.
    Materialized { commit: String, files: usize },
    /// El árbol ya estaba al día.
    AlreadyCurrent { commit: String },
    /// Hay `.bilink/` sin `head`: no se pisa. Es lo esperado en el paso 3 del corte.
    SkippedNoProvenance,
    /// La rama no tiene ref. No es un error: de quién hereda los bilinks una rama
    /// nueva es una decisión de `track`.
    NoRef(String),
    /// `HEAD` desacoplado.
    Detached,
}

pub fn init(dir: &Path, dry_run: bool) -> Result<InitResult> {
    let repo = Repo::open(dir)?;

    let excluded = config::write_exclude(&repo.root, dry_run)?;
    let refspec = config::write_refspec(&repo.root, dry_run)?;

    let fetched = if dry_run || config::remotes(&repo.root)?.is_empty() {
        None
    } else {
        // El fetch trae `refs/bilink/*` con el refspec recién puesto. Falla blando:
        // un remoto inalcanzable no debería impedir configurar el clon.
        repo.git(&["fetch", "--quiet"]).ok().map(|_| "ok".to_string())
    };

    let branch = repo.branch();
    let outcome = materialize_step(&repo, branch.as_deref(), dry_run)?;

    Ok(InitResult { excluded, refspec, fetched, branch, outcome })
}

/// El paso 3, que **no pisa nada**.
///
/// Si hay un `.bilink/` en el árbol y no hay `head`, `init` no puede saber de dónde
/// salió, así que lo deja intacto y se limita a los pasos 1 y 2 — y lo dice. Es lo
/// que hace que el paso 3 del corte `005` pueda ser un `init` a secas: ahí el
/// `.bilink/` todavía no está en la ref, y materializar lo borraría.
fn materialize_step(repo: &Repo, branch: Option<&str>, dry_run: bool) -> Result<Outcome> {
    let Some(branch) = branch else { return Ok(Outcome::Detached) };
    let Some(tip) = repo.ref_tip(branch) else {
        return Ok(Outcome::NoRef(branch.to_string()));
    };

    let head = repo.read_head();
    let has_bilinks = !repo.bilink_dirs()?.is_empty();

    match head {
        None if has_bilinks => Ok(Outcome::SkippedNoProvenance),
        Some(h) if h.branch == branch && h.commit == tip => {
            Ok(Outcome::AlreadyCurrent { commit: tip })
        }
        Some(h) => {
            repo.guard_clean(&h)?;
            let files = if dry_run { 0 } else { repo.materialize(branch, &tip)? };
            Ok(Outcome::Materialized { commit: tip, files })
        }
        None => {
            let files = if dry_run { 0 } else { repo.materialize(branch, &tip)? };
            Ok(Outcome::Materialized { commit: tip, files })
        }
    }
}

/// Lo que corre **antes de todo comando**, salvo `init` mismo.
///
/// Dos cosas, y las dos salen de [la ref](crate::bilink_ref):
///
/// 1. **Exigir `init`** en un repo que ya cortó a la ref. Sin exclude ni refspec,
///    los bilinks del árbol no tienen procedencia y `git fetch` no trae la ref.
/// 2. **Materializar el `.bilink/` de la rama actual** si `head` no coincide.
///    `git checkout` no lo toca —para el índice del proyecto son archivos
///    ignorados— así que cambiar de rama deja el código de `B` con los bilinks de
///    `A` y nada avisa.
///
/// La corrección del punto 2 es **automática y sin ceremonia**: no hay comando de
/// más que tipear ni pregunta que contestar, porque no hay nada que decidir. Lo
/// único que la frena es la guarda: con trabajo sin commitear en `.bilink/`, se
/// para, igual que `git checkout` se niega a pisar cambios.
///
/// En un repo que todavía no cortó no hace nada: no hay ref de la cual materializar.
///
/// **Y fuera de un repo git tampoco hace nada.** La raíz se resuelve caminando hacia
/// arriba desde cwd y cae al cwd si no encuentra marcador, así que bilinker corre en
/// un proyecto nuevo sin ningún paso de inicialización — ver
/// [`configuration`](crate::config). Un preludio que exigiera git rompería eso.
pub fn prelude(dir: &Path) -> Result<Materialization> {
    let Ok(repo) = Repo::open(dir) else {
        return Ok(Materialization::NoGit);
    };
    crate::config::require_initialized(&repo.root)?;
    repo.ensure_current()
}

pub use crate::bilink_ref::Materialization;
