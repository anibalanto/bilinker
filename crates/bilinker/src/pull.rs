//! `bilinker pull` — traer lo que otro aceptó **en la misma rama**.
//!
//! Es el caso 3.b de la taxonomía: sincronización de decisiones donde los dos lados
//! cuelgan de la **misma absorción**. [`adopt`](crate::adopt) cubre el 3.a —otra
//! rama, otra absorción— y no aplica acá, porque no hay otra rama que nombrar.
//!
//! **Es el más simple de los dos.** Los dos lados describen el mismo código, así que
//! el árbol no se elige: sale del primer padre. Lo único que se fusiona es
//! `.bilink/`, campo por campo.
//!
//! **Ninguna aceptación se pierde**, y no por cuidado sino por construcción: los dos
//! commits son los dos padres del resultado, así que siguen alcanzables.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::adopt::{apply_changes, diff3, Change, Row};
use crate::bilink_ref::Repo;
use crate::refmsg::{RefCommand, RefMessage};

pub struct PullResult {
    pub branch:  String,
    pub remote:  String,
    pub base:    Option<String>,
    pub changes: Vec<Change>,
    /// El commit escrito, o `None` si no hubo nada que traer.
    pub sha:     Option<String>,
    /// La ref del remoto ya estaba contenida en la local.
    pub up_to_date: bool,
    /// Sólo hubo que avanzar: la local era antepasado de la del remoto.
    pub fast_forward: bool,
}

impl PullResult {
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|c| c.row == Row::Conflict).count()
    }
    pub fn brought(&self) -> usize {
        self.changes.iter().filter(|c| c.row == Row::Clean).count()
    }
}

pub fn pull(dir: &Path, remote: Option<&str>, dry_run: bool) -> Result<PullResult> {
    let repo = Repo::open(dir)?;
    let branch = repo.require_branch()?;
    let mine = repo.require_ref_tip(&branch)?;
    let remote = crate::push::pick_remote(&repo, remote)?;

    let theirs = fetch_theirs(&repo, &remote, &branch)?;

    // Lo que el remoto tiene ya está acá.
    if repo.is_ancestor(&theirs, &mine)? {
        return Ok(PullResult {
            branch, remote, base: Some(theirs), changes: Vec::new(),
            sha: None, up_to_date: true, fast_forward: false,
        });
    }

    // Sólo hay que avanzar: no hay nada que unir, y un commit de merge diría que sí.
    if repo.is_ancestor(&mine, &theirs)? {
        if !dry_run {
            repo.git(&["update-ref", &Repo::ref_name(&branch), &theirs])?;
            repo.materialize(&branch, &theirs)?;
        }
        return Ok(PullResult {
            branch, remote, base: Some(mine), changes: Vec::new(),
            sha: Some(theirs), up_to_date: false, fast_forward: true,
        });
    }

    // Divergieron. La base es el commit donde los dos se separaron, y existe siempre
    // que nadie haya reescrito: es lo que distingue este caso de una reescritura.
    let base = repo.merge_base(&mine, &theirs)?;
    let changes = diff3(&repo, base.as_deref(), &mine, &theirs)?;

    // **Todo o nada.** Un `accepted` en conflicto son dos decisiones humanas
    // incompatibles sobre el mismo fragmento, y elegir una es `accept`, con una
    // persona mirando.
    let hay_conflicto = changes.iter().any(|c| c.row == Row::Conflict);
    if hay_conflicto || dry_run {
        return Ok(PullResult {
            branch, remote, base, changes,
            sha: None, up_to_date: false, fast_forward: false,
        });
    }

    apply_changes(&repo, &changes, &theirs)?;

    // **El árbol de código sale del primer padre y no se fusiona.** Los dos lados
    // cuelgan de la misma absorción, así que describen el mismo código: no hay nada
    // que elegir, y construirlo desde el absorbido lo deja idéntico por definición.
    let absorbido = repo
        .absorbed(&mine)?
        .context("la ref local no tiene ningún commit del proyecto absorbido")?;
    let tree = repo.build_tree(&absorbido)?;
    repo.verify_faithful(&tree, &absorbido)?;

    let message = RefMessage::new(RefCommand::Pull { remote: remote.clone() })
        .with_prose(format!("{} endpoint(s) de {remote}", brought_endpoints(&changes)));
    let sha = repo.write_ref_commit(&branch, &tree, &[mine, theirs], &message.render())?;
    repo.write_head(&branch, &sha)?;

    Ok(PullResult {
        branch, remote, base, changes,
        sha: Some(sha), up_to_date: false, fast_forward: false,
    })
}

/// Trae la ref del remoto a un namespace aparte.
///
/// **No a `refs/bilink/<branch>`**, que es la ref local y es justo la que no se
/// quiere pisar — el refspec sin `+` de `init` existe para que ese fetch falle en
/// vez de pisarla. Acá sí se trae forzando, y no hay nada que proteger: es una copia
/// de lectura del remoto, se descarta y se vuelve a traer.
fn fetch_theirs(repo: &Repo, remote: &str, branch: &str) -> Result<String> {
    let local = format!("refs/bilink-remote/{remote}/{branch}");
    let refspec = format!("+{}:{local}", Repo::ref_name(branch));

    // **`--refmap=` no es un detalle.** Sin él, `git fetch <remoto> <refspec>` aplica
    // *además* los refspecs configurados, y el de [`init`](crate::init) va sin `+`
    // justo para fallar cuando el remoto divergió. O sea que el fetch de `pull`
    // fallaría exactamente en el único caso en que `pull` existe. El refmap vacío
    // apaga esa parte y deja sólo el refspec que se pidió.
    repo.git(&["fetch", "--quiet", "--refmap=", remote, &refspec])
        .with_context(|| format!("trayendo {} de {remote}", Repo::ref_name(branch)))?;

    repo.git(&["rev-parse", &local])
        .map(|s| s.trim().to_string())
        .map_err(|_| {
            anyhow::anyhow!("{remote} no tiene {} — nada que traer", Repo::ref_name(branch))
        })
}

/// Cuántos endpoints distintos entran, que es lo que se cuenta en el mensaje: un
/// endpoint con las dos dimensiones limpias es **una** aceptación que llega, no dos.
fn brought_endpoints(changes: &[Change]) -> usize {
    let mut vistos: Vec<(&str, u8)> = changes
        .iter()
        .filter(|c| c.row == Row::Clean)
        .map(|c| (c.uuid.as_str(), c.n))
        .collect();
    vistos.sort_unstable();
    vistos.dedup();
    vistos.len()
}

/// Por qué el remoto rechazó un push: **divergencia o reescritura**, que no son lo
/// mismo y hoy se confundían.
///
/// Un non-fast-forward donde los dos lados descienden de una base común es dos
/// personas que **agregaron**: nadie reescribió nada, y la salida es unir. Sin base
/// de merge —o con el tip viejo inalcanzable— sí es una historia reescrita, y eso es
/// algo que mirar.
pub fn diagnose_rejection(dir: &Path, remote: &str) -> Result<String> {
    let repo = Repo::open(dir)?;
    let branch = repo.require_branch()?;
    let mine = repo.require_ref_tip(&branch)?;

    let theirs = match fetch_theirs(&repo, remote, &branch) {
        Ok(t) => t,
        Err(e) => bail!("{remote} rechazó el push y su ref no se pudo leer: {e}"),
    };

    match repo.merge_base(&mine, &theirs)? {
        Some(base) => Ok(format!(
            "{remote} tiene {} adelantada, y las dos historias agregaron.\\n  \
             Nadie reescribió nada: base de merge en {}.\\n  \
             Unir con `bilinker pull`.",
            Repo::ref_name(&branch),
            &base[..base.len().min(7)]
        )),
        None => Ok(format!(
            "{remote} tiene una {} que no comparte historia con la local.\\n  \
             La ref es append-only: esto es una reescritura, y es algo que mirar \\
             antes de tocar nada.",
            Repo::ref_name(&branch)
        )),
    }
}
