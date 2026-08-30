//! `bilinker sync` — alinear la ref con la rama, sin verificar nada.
//!
//! Cubre el caso en que **el proyecto avanzó y nadie aceptó nada**. No corre
//! tree-sitter, no resuelve captures y no toca `cache/state` — de ahí el nombre;
//! `update` sugeriría que recalcula estados, que es lo que no hace.
//!
//! Su commit es el único de la ref cuyo diff contra el primer padre es **vacío**, y
//! por eso el único que no registra ninguna decisión: alinea la foto y nada más.

use std::path::Path;

use anyhow::Result;

use crate::bilink_ref::{Commit, Repo};

pub struct SyncResult {
    pub branch:      String,
    pub from:        String,
    pub to:          String,
    pub absorbed:    Option<String>,
    pub commits:     usize,
    pub pushed:      bool,
}

pub fn sync(dir: &Path, dry_run: bool, push: bool) -> Result<SyncResult> {
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
            absorbed: None,
            commits: 0,
            pushed: false,
        });
    }

    if dry_run {
        repo.verify_disjoint(&project_tip)?;
        return Ok(SyncResult {
            branch,
            from: ref_tip.clone(),
            to: ref_tip,
            absorbed: Some(project_tip),
            commits: 1,
            pushed: false,
        });
    }

    let short = &project_tip[..project_tip.len().min(7)];
    let Commit { sha, absorbed: got, wrote } =
        repo.commit(&branch, &format!("sync: {branch} hasta {short}"))?;

    let pushed = if push && wrote {
        let refspec = format!("{0}:{0}", Repo::ref_name(&branch));
        repo.git(&["push", "origin", &refspec])?;
        true
    } else {
        false
    };

    Ok(SyncResult {
        branch,
        from: ref_tip,
        to: sha,
        absorbed: got,
        commits: usize::from(wrote),
        pushed,
    })
}
