//! `bilinker history` — qué le pasó a un bilink.
//!
//! Los demás comandos miran el presente; éste mira [la ref](crate::bilink_ref), que
//! es donde vive el registro de decisiones.
//!
//! **No persiste nada nuevo: arma una vista.** Todos los datos ya están ahí, y salen
//! del DAG y del diff — el [comando canónico](crate::refmsg) le da a cada acto su
//! nombre sin heurísticas, pero todo lo demás es derivable sin él. Por eso la vista
//! sirve sobre la historia que ya existe.

use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;

use bilink_format::{Accepted, BiLink, Capture};

use crate::bilink_ref::{Act, Repo};
use crate::refmsg::{self, Read};

/// Un acto sobre un bilink.
#[derive(Debug, Serialize)]
pub struct Deed {
    pub commit: String,
    pub author: String,
    pub date: String,
    /// `absorción` · `decisión` · `sincronización` · `corte`, o `?` si no se pudo
    /// decidir. Sale de los padres, no del mensaje.
    pub kind: String,
    /// El comando canónico. **`None` en un acto anterior a la gramática**, y ahí se
    /// queda en `None`: adivinarlo del texto libre sería fabricar precisión.
    pub command: Option<String>,
    /// El commit del proyecto contra el que se calculó.
    pub against: Option<String>,
    pub changes: Vec<Change>,
}

/// Qué cambió de un endpoint, campo por campo.
#[derive(Debug, Serialize)]
pub struct Change {
    pub n: u8,
    pub field: &'static str,
    pub before: Option<String>,
    pub after: Option<String>,
    /// Para un cambio de `link`: los dos captures, con su `{file, query}`, leídos del
    /// árbol de **este** commit — así se siguen leyendo aunque `prune` los borre.
    pub captures: Vec<CaptureView>,
}

#[derive(Debug, Serialize)]
pub struct CaptureView {
    pub id: String,
    pub file: String,
    pub query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct History {
    pub uuid: String,
    pub path: String,
    /// `false` en un repo que todavía no cortó: la historia sale de la rama del
    /// proyecto y **no incluye** los actos que la ref registraría.
    pub from_ref: bool,
    pub deeds: Vec<Deed>,
}

pub fn history(layer: &Path, uuid_prefix: &str, endpoint: Option<u8>) -> Result<History> {
    let path = crate::accept::find_bilink_path(layer, uuid_prefix)?;
    let uuid = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(uuid_prefix)
        .to_string();

    let repo = Repo::open(layer)?;
    let rel = path
        .strip_prefix(&repo.root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();

    // Sin ref, la historia es la de la rama del proyecto — y se dice, porque callar
    // la diferencia haría parecer completa una vista que no lo es.
    let branch = repo.branch();
    let (start, from_ref) = match branch.as_deref().and_then(|b| repo.ref_tip(b)) {
        Some(tip) => (tip, true),
        None => (repo.git(&["rev-parse", "HEAD"])?.trim().to_string(), false),
    };

    let log = repo.git(&[
        "log", "--first-parent", "--format=%H", &start, "--", &rel,
    ])?;

    let mut deeds = Vec::new();
    for commit in log.lines() {
        deeds.push(deed(&repo, commit, &rel, endpoint, from_ref)?);
    }

    Ok(History { uuid, path: rel, from_ref, deeds })
}

fn deed(
    repo: &Repo,
    commit: &str,
    rel: &str,
    endpoint: Option<u8>,
    from_ref: bool,
) -> Result<Deed> {
    let meta = repo.git(&["log", "-1", "--format=%an%x00%aI%x00%B", commit])?;
    let mut parts = meta.splitn(3, '\0');
    let author = parts.next().unwrap_or("").trim().to_string();
    let date = parts.next().unwrap_or("").trim().to_string();
    let message = parts.next().unwrap_or("");

    // **El comando no se adivina.** Un mensaje viejo que empieza con `accept` no es
    // un `accept <uuid>.<N>`, y tratarlo como si lo fuera fabricaría precisión.
    let command = match refmsg::read(message) {
        Ok(Read::Parsed(m)) => Some(m.command.line()),
        _ => None,
    };

    let kind = if from_ref {
        match repo.classify(commit) {
            Ok(Act::Absorption { .. }) => "absorción",
            Ok(Act::Decision) => "decisión",
            Ok(Act::Synchronization) => "sincronización",
            Ok(Act::Cut { .. }) => "corte",
            Err(_) => "?",
        }
    } else {
        "commit"
    };

    let against = from_ref.then(|| repo.absorbed(commit).ok().flatten()).flatten();

    let parents = repo.parents(commit)?;
    let before = parents
        .first()
        .and_then(|p| repo.bilink_at(p, rel).ok());
    let after = repo.bilink_at(commit, rel).ok();

    let changes = match &after {
        Some(a) => diff(repo, commit, before.as_ref(), a, endpoint),
        None => Vec::new(),
    };

    Ok(Deed {
        commit: commit.to_string(),
        author,
        date,
        kind: kind.to_string(),
        command,
        against,
        changes,
    })
}

/// Los campos que cambiaron, endpoint por endpoint.
///
/// **`link` y `accepted.link` son las dos dimensiones**: cuándo se *propuso* una
/// ubicación y cuándo se *aprobó*. Se leen por separado porque son dos actos con dos
/// autores posibles.
fn diff(
    repo: &Repo,
    commit: &str,
    before: Option<&BiLink>,
    after: &BiLink,
    only: Option<u8>,
) -> Vec<Change> {
    let mut out = Vec::new();
    for n in [0u8, 1u8] {
        if only.is_some_and(|k| k != n) {
            continue;
        }
        let a = after.endpoint.get(n);
        let b = before.map(|bl| bl.endpoint.get(n));

        push_change(&mut out, repo, commit, n, "link",
                    b.map(|e| e.link.to_string()), Some(a.link.to_string()));

        let (ba, aa) = (b.and_then(|e| e.accepted.as_ref()), a.accepted.as_ref());
        push_change(&mut out, repo, commit, n, "accepted.link",
                    ba.and_then(link_of), aa.and_then(link_of));
        push_change(&mut out, repo, commit, n, "hash",
                    ba.map(|x| x.hash.clone()), aa.map(|x| x.hash.clone()));
        push_change(&mut out, repo, commit, n, "hash_ast",
                    ba.and_then(|x| x.hash_ast.clone()), aa.and_then(|x| x.hash_ast.clone()));
        push_change(&mut out, repo, commit, n, "agree",
                    ba.map(agree_of), aa.map(agree_of));
    }
    out
}

fn push_change(
    out: &mut Vec<Change>,
    repo: &Repo,
    commit: &str,
    n: u8,
    field: &'static str,
    before: Option<String>,
    after: Option<String>,
) {
    if before == after {
        return;
    }
    // Para una ubicación, los dos captures con su `{file, query}`. Se leen del árbol
    // de **este** commit, así que siguen estando aunque `prune` los haya borrado del
    // presente.
    let captures = if field.ends_with("link") {
        [before.as_deref(), after.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(|l| l.strip_prefix("capture "))
            .filter_map(|id| capture_at(repo, commit, id))
            .collect()
    } else {
        Vec::new()
    };
    out.push(Change { n, field, before, after, captures });
}

/// Un capture leído del árbol de un commit.
///
/// **Es lo que hace que `prune` no sea destructivo para la arqueología**: todo commit
/// que tenía ese capture lo sigue teniendo, así que la ubicación que alguien aprobó
/// se lee aunque ya no esté en el tip.
fn capture_at(repo: &Repo, commit: &str, id: &str) -> Option<CaptureView> {
    for path in repo.bilink_paths_in(commit).ok()? {
        if !path.ends_with(&format!("capture/{id}.yaml")) {
            continue;
        }
        let text = repo.git(&["show", &format!("{commit}:{path}")]).ok()?;
        let cap: Capture = serde_yaml_ng::from_str(&text).ok()?;
        return Some(CaptureView { id: id.to_string(), file: cap.file, query: cap.query });
    }
    None
}

fn link_of(a: &Accepted) -> Option<String> { a.link.as_ref().map(|l| l.to_string()) }

fn agree_of(a: &Accepted) -> String {
    a.agree.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// `<uuid>` o `<uuid>.<N>`.
pub fn parse_target(target: &str) -> Result<(String, Option<u8>)> {
    match target.rsplit_once('.') {
        Some((u, "0")) => Ok((u.to_string(), Some(0))),
        Some((u, "1")) => Ok((u.to_string(), Some(1))),
        Some((_, other)) if other.chars().all(|c| c.is_ascii_digit()) => {
            bail!("'{other}' no es un índice de endpoint: son 0 o 1")
        }
        _ => Ok((target.to_string(), None)),
    }
}
