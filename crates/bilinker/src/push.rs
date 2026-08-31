//! `bilinker push` — publicar la ref.
//!
//! **Ninguna interacción con `refs/bilink/*` se hace tipeando git.** La ref vive
//! fuera de `refs/heads/`, así que `git push` a secas no la empuja y hay que
//! nombrarla con un refspec — y hacer que alguien tipee un refspec es exactamente
//! lo que el diseño de la ref evita al decir que *los refspecs los pone bilinker;
//! nadie los tipea*. Que el usuario tenga que escribir
//! `refs/bilink/main:refs/bilink/main` una sola vez ya es una fuga del namespace
//! hacia afuera, y a la segunda ya es una convención que alguien copia mal.
//!
//! Es su propio comando y no una flag de [`sync`](crate::sync): alinear la ref con
//! la rama y publicarla son dos actos, y quien trabaja en una rama propia hace el
//! primero muchas veces antes del segundo.

use std::path::Path;

use anyhow::{bail, Result};

use crate::bilink_ref::Repo;

pub struct PushReport {
    pub branch: String,
    pub remote: String,
    /// El tip que quedó publicado.
    pub tip:    String,
    /// `false` cuando el remoto ya lo tenía.
    pub moved:  bool,
}

/// A qué remoto se le habla.
///
/// Con uno solo, ése. Con varios gana `origin`; si no está, se pide elegir —
/// adivinar sería adivinar **con quién** sincronizás, que es la clase de cosa que no
/// se adivina. Lo comparten `push` y [`pull`](crate::pull), que hablan con el mismo.
pub fn pick_remote(repo: &Repo, remote: Option<&str>) -> Result<String> {
    if let Some(r) = remote {
        return Ok(r.to_string());
    }
    match crate::config::remotes(&repo.root)?.as_slice() {
        [one] => Ok(one.clone()),
        [] => bail!("el repo no tiene ningún remoto"),
        many if many.iter().any(|r| r == "origin") => Ok("origin".to_string()),
        many => bail!(
            "hay más de un remoto ({}) y ninguno es `origin`.\n  \
             Elegir con `--remote <nombre>`.",
            many.join(", ")
        ),
    }
}

/// Empuja `refs/bilink/<branch>` al remoto.
///
/// Sin rama, la actual. El push es **siempre fast-forward** porque la ref es
/// append-only por diseño: nunca se rebasea ni se cherry-pickea. Si el remoto lo
/// rechaza, eso no es un caso a forzar sino algo que mirar — alguien reescribió una
/// historia que no se reescribe.
pub fn push(dir: &Path, branch: Option<&str>, remote: Option<&str>) -> Result<PushReport> {
    let repo = Repo::open(dir)?;
    let branch = match branch {
        Some(b) => repo.resolve_branch_name(b),
        None    => repo.require_branch()?,
    };
    let tip = repo.require_ref_tip(&branch)?;

    let remote = pick_remote(&repo, remote)?;
    let refname = Repo::ref_name(&branch);
    let before = remote_tip(&repo, &remote, &refname);

    // El refspec lo arma bilinker. Es el mismo `+` que `init` deja en el config: la
    // ref es append-only, así que un avance siempre lo es de verdad.
    repo.git(&["push", &remote, &format!("+{refname}:{refname}")])?;

    Ok(PushReport {
        branch,
        remote,
        moved: before.as_deref() != Some(tip.as_str()),
        tip,
    })
}

fn remote_tip(repo: &Repo, remote: &str, refname: &str) -> Option<String> {
    let out = repo.git(&["ls-remote", remote, refname]).ok()?;
    out.split_whitespace().next().map(str::to_string)
}
