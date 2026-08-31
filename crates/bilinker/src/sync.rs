//! `bilinker sync` — alinear la ref con la rama, sin verificar nada.
//!
//! Cubre el caso en que **el proyecto avanzó y nadie aceptó nada**. No corre
//! tree-sitter, no resuelve captures y no toca `cache/state` — de ahí el nombre;
//! `update` sugeriría que recalcula estados, que es lo que no hace.
//!
//! Su commit es el único de la ref cuyo diff contra el primer padre es **vacío**, y
//! por eso el único que no registra ninguna decisión: alinea la foto y nada más.
//!
//! **No publica.** Alinear la ref con la rama y publicarla son dos actos, y quien
//! trabaja en una rama propia hace el primero muchas veces antes del segundo. Para
//! publicar está [`push`](crate::push), que es un comando y no una flag.

use std::path::Path;

use anyhow::Result;

use crate::bilink_ref::{Commit, Repo};

pub struct SyncResult {
    pub branch:      String,
    pub from:        String,
    pub to:          String,
    /// El commit del proyecto que **este** acto absorbió.
    pub absorbed:    Option<String>,
    /// Contra qué commit del proyecto quedó la ref. Con `absorbed` en `None`, es el
    /// que ya tenía.
    pub at:          Option<String>,
    pub commits:     usize,
}

pub fn sync(dir: &Path, dry_run: bool) -> Result<SyncResult> {
    let repo = Repo::open(dir)?;
    let branch = repo.require_branch()?;
    let ref_tip = repo.require_ref_tip(&branch)?;
    let project_tip = repo.branch_tip(&branch)?;
    let absorbed = repo.absorbed(&ref_tip)?.unwrap_or_else(|| ref_tip.clone());

    // Con el tip ya absorbido no se escribe nada: un merge con el mismo segundo
    // padre y el mismo `.bilink/` no dice nada que la ref no diga ya.
    if absorbed == project_tip {
        return Ok(SyncResult {
            branch,
            from: ref_tip.clone(),
            to: ref_tip,
            // `None` es "no se absorbió nada acá"; `at` es contra qué commit del
            // proyecto la ref ya estaba, que es lo que hay que reportar.
            absorbed: None,
            at: Some(project_tip),
            commits: 0,
        });
    }

    if dry_run {
        repo.verify_disjoint(&project_tip)?;
        return Ok(SyncResult {
            branch,
            from: ref_tip.clone(),
            to: ref_tip,
            absorbed: Some(project_tip.clone()),
            at: Some(project_tip),
            commits: 1,
        });
    }

    let short = &project_tip[..project_tip.len().min(7)];
    let Commit { sha, absorbed: got, wrote } =
        repo.commit(&branch, &format!("sync: {branch} hasta {short}"))?;

    Ok(SyncResult {
        branch,
        from: ref_tip,
        to: sha,
        absorbed: got.clone(),
        at: got.or(Some(project_tip)),
        commits: usize::from(wrote),
    })
}
