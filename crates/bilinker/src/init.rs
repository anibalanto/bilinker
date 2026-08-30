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
