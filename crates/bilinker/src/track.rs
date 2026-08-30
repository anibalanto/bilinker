//! `bilinker track` — crear la ref de una rama que no la tiene.
//!
//! Sin él, empezar a seguir una rama nueva deja todos los endpoints en `PENDING`.
//!
//! El commit que escribe tiene la forma de cualquier otro de una ref, sólo que sus
//! dos padres vienen de lugares distintos: el **primero** es el commit del que
//! hereda los bilinks, el **segundo** es el tip de la rama, de donde saca el código.
//! Su diff contra el primer padre es vacío — `track` no decide nada.
//!
//! **La búsqueda va de la ref hacia el proyecto, nunca al revés.** Ningún commit del
//! proyecto tiene un merge a `refs/bilink/*`: la relación es exactamente la inversa,
//! y buscar en los ancestros de la rama nueva un merge hacia la ref sólo puede
//! encontrar el bug que la verificación de disyunción detecta.

use std::path::Path;

use anyhow::{bail, Result};

use crate::bilink_ref::Repo;

pub struct TrackResult {
    pub branch:    String,
    pub sha:       String,
    /// El commit de la ref del que se heredaron los bilinks. `None` en el corte.
    pub inherited: Option<String>,
    /// El commit del proyecto que ese commit tenía absorbido — el `P` que calificó.
    pub base:      Option<String>,
    pub files:     usize,
}

pub fn track(dir: &Path, branch: &str, from: Option<&str>) -> Result<TrackResult> {
    let repo = Repo::open(dir)?;

    if repo.ref_tip(branch).is_some() {
        bail!(
            "{} ya existe.\n  \
             Para ponerla al día con la rama, `bilinker sync`.",
            Repo::ref_name(branch)
        );
    }

    let tip = repo.branch_tip(branch)?;
    repo.verify_disjoint(&tip)?;

    let inherit = match from {
        // Con `--from` explícito se usa y listo.
        Some(r) => Some(resolve_from(&repo, r)?),
        None => pick_inherit(&repo, branch)?,
    };

    let (tree, parents, message) = match &inherit {
        Some(Candidate { commit, absorbed }) => (
            repo.build_tree_inheriting(&tip, commit)?,
            vec![commit.clone(), tip.clone()],
            format!("track: {branch} hereda de {} sobre {}", short(commit), short(absorbed)),
        ),
        // Ningún candidato califica: la ref nace desde cero, con el `.bilink/` del
        // árbol de trabajo y el commit de la rama como padre único.
        //
        // Es el **corte**: el único commit de la ref sin ningún commit del proyecto
        // absorbido por debajo. Su fidelidad se lee contra ese padre mismo.
        None => (
            repo.build_tree(&tip)?,
            vec![tip.clone()],
            format!("corte: los bilinks de {branch} pasan a refs/bilink/{branch}"),
        ),
    };

    repo.verify_faithful(&tree, &tip)?;
    let sha = repo.write_ref_commit(branch, &tree, &parents, &message)?;

    // Recién acá se toca el árbol: si el commit no se pudo escribir, el `.bilink/`
    // del árbol quedó como estaba.
    let files = if inherit.is_some() {
        repo.materialize(branch, &sha)?
    } else {
        repo.write_head(branch, &sha)?;
        repo.tracked_bilink_files()?.len()
    };

    Ok(TrackResult {
        branch: branch.to_string(),
        sha,
        inherited: inherit.as_ref().map(|c| c.commit.clone()),
        base: inherit.as_ref().map(|c| c.absorbed.clone()),
        files,
    })
}

/// Un commit de la ref del que se podría heredar, y el commit del proyecto que
/// tiene absorbido.
struct Candidate {
    commit:   String,
    absorbed: String,
}

/// El `M` cuyo commit absorbido `P` es el más nuevo que sigue siendo ancestro de la
/// rama nueva.
///
/// El test `--is-ancestor` es lo que traduce *"la última versión de los bilinks
/// accesible"* a algo exacto. Sin él, una ref adelantada respecto del punto de fork
/// haría heredar bilinks que describen código que la rama no tiene — `UNRESOLVED` y
/// `ALTERED` falsos desde el minuto cero.
fn pick_inherit(repo: &Repo, branch: &str) -> Result<Option<Candidate>> {
    let mut per_ref: Vec<(String, Candidate)> = Vec::new();

    for other in existing_refs(repo)? {
        if other == branch {
            continue;
        }
        // Los candidatos son los commits **propios** de la ref, en su cadena de
        // primeros padres, y se los mira por su segundo padre. La cadena se corta al
        // salir de la ref: sin ese freno, el corte deja pasar commits del proyecto,
        // y el más viejo de ellos es ancestro de cualquier rama.
        for commit in repo.ref_chain(&other)? {
            let Some(absorbed) = repo.absorbed(&commit)? else { continue };
            if !is_ancestor(repo, &absorbed, branch) {
                continue;
            }
            per_ref.push((other.clone(), Candidate { commit, absorbed }));
            break; // el primero de la cadena es el más nuevo que califica
        }
    }

    match per_ref.len() {
        0 => Ok(None),
        1 => Ok(Some(per_ref.pop().unwrap().1)),
        // El fork es ancestro de dos refs trackeadas: adivinar sería peor.
        _ => {
            let names: Vec<String> = per_ref.iter().map(|(n, _)| n.clone()).collect();
            bail!(
                "el punto de fork de {branch} es ancestro de {}.\n  \
                 Elegir con `bilinker track {branch} --from <rama>`.",
                names.join(" y ")
            )
        }
    }
}

fn existing_refs(repo: &Repo) -> Result<Vec<String>> {
    let out = repo.git(&["for-each-ref", "--format=%(refname)", "refs/bilink/"])?;
    Ok(out
        .lines()
        .filter_map(|l| l.trim().strip_prefix("refs/bilink/"))
        .map(str::to_string)
        .collect())
}

/// `--from` nombra la **rama del proyecto**, no su ref de bilinks: una sola fuente
/// de verdad, y nadie tipeando namespaces de refs. La traducción la hace acá.
fn resolve_from(repo: &Repo, from: &str) -> Result<Candidate> {
    let name = repo.resolve_branch_name(from);
    let commit = repo.require_ref_tip(&name)?;
    let absorbed = repo.absorbed(&commit)?.unwrap_or_else(|| commit.clone());
    Ok(Candidate { commit, absorbed })
}

fn is_ancestor(repo: &Repo, ancestor: &str, branch: &str) -> bool {
    repo.git(&["merge-base", "--is-ancestor", ancestor, branch]).is_ok()
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}
