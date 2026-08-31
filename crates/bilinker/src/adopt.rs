//! `bilinker adopt` — traer las decisiones del vecino, y ninguna de las mías para allá.
//!
//! Después de un rebase sobre `main`, el código de `main` entró a la rama; si `main`
//! aceptó algo sobre ese código, los bilinks heredados no lo tienen y van a reportar
//! drift que `main` ya resolvió.
//!
//! **No se llama `merge` a propósito.** En este diseño *merge* ya nombra un commit
//! de la ref que absorbe un commit del proyecto como segundo padre. `adopt` dice lo
//! que pasa y es asimétrico, que es la verdad.
//!
//! Son **dos commits**: `●b` absorbe el tip rebaseado —es la absorción de siempre, y
//! la precondición de fidelidad la exige igual— y `●c` trae el rango del vecino como
//! segundo padre. Uno de tres padres haría que `--first-parent` mostrara una línea
//! para dos cosas distintas.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Result};

use bilink_format::{Accepted, BiLink};

use crate::bilink_ref::Repo;

/// Qué le pasa a un campo `accepted` al adoptar. Las cuatro son las únicas
/// posibles, y salen del formato sin nada agregado: `accepted` son campos con
/// nombre, por endpoint, así que un merge a tres puntas los compara de a uno.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Row {
    /// El vecino lo cambió y acá nadie lo tocó desde la base.
    Clean,
    /// Los dos escribieron el mismo valor. **Es la convergencia**: dos personas que
    /// aceptan el mismo contenido en HEADs distintos escriben los mismos valores.
    Converged,
    /// Los dos lo cambiaron, a valores distintos.
    Conflict,
}

pub struct Change {
    pub path:      String,
    pub uuid:      String,
    pub n:         u8,
    /// `ubicación` o `contenido` — la dimensión que difiere.
    pub dimension: &'static str,
    pub row:       Row,
    pub mine:      Option<String>,
    pub theirs:    Option<String>,
}

pub struct AdoptResult {
    pub branch:    String,
    pub neighbour: String,
    pub base:      Option<String>,
    pub changes:   Vec<Change>,
    pub absorbed:  Option<String>,
    pub commits:   usize,
}

impl AdoptResult {
    pub fn conflicts(&self) -> usize {
        self.changes.iter().filter(|c| c.row == Row::Conflict).count()
    }
    pub fn adopted(&self) -> usize {
        self.changes.iter().filter(|c| c.row == Row::Clean).count()
    }
}

pub fn adopt(dir: &Path, neighbour: &str, dry_run: bool) -> Result<AdoptResult> {
    let repo = Repo::open(dir)?;
    let branch = repo.require_branch()?;
    let mine = repo.require_ref_tip(&branch)?;

    // Se nombra la rama del proyecto, no su ref: `origin/main` y `main` son lo mismo.
    let neighbour_branch = repo.resolve_branch_name(neighbour);
    if neighbour_branch == branch {
        bail!("{neighbour} es la rama actual: no hay nada que adoptar de uno mismo");
    }
    let theirs = repo.require_ref_tip(&neighbour_branch)?;

    // La base sale gratis: es la base de merge real, porque `track` puso el commit
    // heredado como **primer padre** en vez de copiar archivos.
    let base = repo.merge_base(&mine, &theirs)?;

    if base.as_deref() == Some(theirs.as_str()) {
        return Ok(AdoptResult {
            branch, neighbour: neighbour_branch, base, changes: Vec::new(),
            absorbed: None, commits: 0,
        });
    }

    let changes = diff3(&repo, base.as_deref(), &mine, &theirs)?;

    // Todo o nada: con un conflicto no se escribe ningún commit, ni siquiera el de
    // absorción. Un `accepted` en conflicto son dos decisiones humanas incompatibles
    // sobre el mismo fragmento, y resolverlo es `accept`, con una persona mirando.
    let conflicts = changes.iter().filter(|c| c.row == Row::Conflict).count();
    let nothing_to_bring = !changes.iter().any(|c| c.row == Row::Clean);
    if conflicts > 0 || dry_run || nothing_to_bring {
        return Ok(AdoptResult {
            branch, neighbour: neighbour_branch, base, changes,
            absorbed: None, commits: 0,
        });
    }

    // `●b` — absorber el tip. No hay que pedirla: escribir sobre la ref la exige.
    let absorb = crate::sync::sync(dir, false)?;

    // Las decisiones del vecino, en el árbol de trabajo.
    apply_changes(&repo, &changes, &theirs)?;

    // `●c` — el commit que las trae, con la ref del vecino como segundo padre.
    let tree = repo.build_tree(&repo.branch_tip(&branch)?)?;
    let tip_now = repo.require_ref_tip(&branch)?;
    let sha = repo.write_ref_commit(
        &branch,
        &tree,
        &[tip_now, theirs.clone()],
        &crate::refmsg::RefMessage::new(crate::refmsg::RefCommand::Adopt {
            branch: neighbour_branch.clone(),
        })
        .with_prose(format!(
            "{} endpoint(s)",
            changes.iter().filter(|c| c.row == Row::Clean).count()
        ))
        .render(),
    )?;
    repo.write_head(&branch, &sha)?;

    Ok(AdoptResult {
        branch,
        neighbour: neighbour_branch,
        base,
        changes,
        absorbed: absorb.absorbed,
        commits: usize::from(absorb.commits > 0) + 1,
    })
}

/// El merge a tres puntas, campo por campo.
///
/// La fila que **no** existe es la que hace a `adopt` asimétrico: un campo que sólo
/// yo cambié se queda como está, y no viaja para el otro lado.
pub(crate) fn diff3(repo: &Repo, base: Option<&str>, mine: &str, theirs: &str) -> Result<Vec<Change>> {
    let base_bl = base.map(|b| read_bilinks(repo, b)).transpose()?.unwrap_or_default();
    let mine_bl = read_bilinks(repo, mine)?;
    let theirs_bl = read_bilinks(repo, theirs)?;

    let mut out = Vec::new();
    for (path, theirs_one) in &theirs_bl {
        let Some(mine_one) = mine_bl.get(path) else { continue };
        let base_one = base_bl.get(path);
        let uuid = uuid_of(path);

        for n in [0u8, 1u8] {
            let t = theirs_one.endpoint.get(n).accepted.as_ref();
            let m = mine_one.endpoint.get(n).accepted.as_ref();
            let b = base_one.map(|x| x.endpoint.get(n).accepted.as_ref()).unwrap_or(None);

            if let Some(mut c) = agree_to_bring(m, t) {
                c.path = path.clone();
                c.uuid = uuid.clone();
                c.n = n;
                out.push(c);
            }

            for (dimension, get) in DIMENSIONS {
                let (tv, mv, bv) = (t.map(get).flatten(), m.map(get).flatten(), b.map(get).flatten());
                if tv == mv {
                    if tv.is_some() && bv != tv {
                        // Los dos escribieron el mismo valor: ya coincidía.
                        out.push(Change {
                            path: path.clone(), uuid: uuid.clone(), n, dimension,
                            row: Row::Converged, mine: mv, theirs: tv,
                        });
                    }
                    continue;
                }
                let row = if mv == bv {
                    Row::Clean            // sólo el vecino lo tocó
                } else if tv == bv {
                    continue;             // sólo yo lo toqué: no viaja, y no se pisa
                } else {
                    Row::Conflict         // los dos, a valores distintos
                };
                out.push(Change {
                    path: path.clone(), uuid: uuid.clone(), n, dimension,
                    row, mine: mv, theirs: tv,
                });
            }
        }
    }
    out.sort_by(|a, b| (a.uuid.as_str(), a.n).cmp(&(b.uuid.as_str(), b.n)));
    Ok(out)
}

/// Las dos dimensiones que se aprueban por separado, y por eso se adoptan por
/// separado. `hash_ast` acompaña al contenido: no es una decisión propia.
///
/// **`agree` no está acá**, y no por olvido: las dimensiones se resuelven eligiendo
/// un lado, y `agree` se resuelve **uniendo**. Va por su cuenta, en
/// [`agree_to_bring`] y en [`apply_changes`].
const DIMENSIONS: [(&str, fn(&Accepted) -> Option<String>); 2] = [
    ("ubicación", |a| a.link.as_ref().map(|l| l.to_string())),
    ("contenido", |a| Some(a.hash.clone())),
];

/// La fila que el vecino aporta sobre **quiénes aprobaron**, si aporta alguna.
///
/// **Nunca es conflicto.** Es la diferencia con `commit`, el campo que no está en
/// `accepted`: el mismo contenido aceptado en dos ramas resuelve a dos commits sin
/// forma de elegir, y acá hay una resolución correcta y única — la unión.
///
/// Sólo aplica cuando los dos lados aprobaron **los mismos valores**. Si difieren,
/// el `agree` del vecino describe otros valores y viaja con ellos por las
/// dimensiones de arriba, o no viaja.
fn agree_to_bring(mine: Option<&Accepted>, theirs: Option<&Accepted>) -> Option<Change> {
    let (m, t) = (mine?, theirs?);
    if !m.same_values(t) || t.agree.is_subset(&m.agree) {
        return None;
    }
    Some(Change {
        path: String::new(), uuid: String::new(), n: 0,
        dimension: "aprobadores",
        row: Row::Clean,
        mine: Some(m.agree.iter().cloned().collect::<Vec<_>>().join(", ")),
        theirs: Some(t.agree.iter().cloned().collect::<Vec<_>>().join(", ")),
    })
}

/// Escribe los campos que entran limpios, tomando el `accepted` entero del vecino
/// para ese endpoint: las dos dimensiones que cambiaron llegan juntas y coherentes.
pub(crate) fn apply_changes(repo: &Repo, changes: &[Change], theirs: &str) -> Result<()> {
    let theirs_bl = read_bilinks(repo, theirs)?;
    let mut touched: BTreeMap<&String, Vec<u8>> = BTreeMap::new();
    for c in changes.iter().filter(|c| c.row == Row::Clean) {
        touched.entry(&c.path).or_default().push(c.n);
    }

    for (path, ns) in touched {
        let full = repo.root.join(path);
        let mut bl = BiLink::load(&full)?;
        let theirs_one = &theirs_bl[path];
        for n in ns {
            // **Los mismos valores se unen; valores distintos viajan enteros.**
            //
            // `agree` dice quiénes aprobaron *estos* valores: si adopto los del
            // vecino, los míos describían otros y no vienen. Si los dos aprobamos lo
            // mismo, la única resolución correcta es que estemos los dos.
            let theirs_acc = theirs_one.endpoint.get(n).accepted.clone();
            let mine_acc = bl.endpoint.get(n).accepted.clone();
            bl.endpoint.get_mut(n).accepted = match (mine_acc, theirs_acc) {
                (Some(mut m), Some(t)) if m.same_values(&t) => {
                    m.agree.extend(t.agree);
                    Some(m)
                }
                (_, t) => t,
            };
        }
        bl.write(&full)?;
    }
    Ok(())
}

/// Los bilinks de un commit de la ref, por path. Se leen del árbol, no del disco:
/// el vecino no está checkouteado y nunca lo va a estar.
fn read_bilinks(repo: &Repo, commit: &str) -> Result<BTreeMap<String, BiLink>> {
    let mut out = BTreeMap::new();
    for path in repo.bilink_paths_in(commit)? {
        if !path.ends_with(".yaml") || path.contains("/capture/") {
            continue;
        }
        let text = repo.git(&["show", &format!("{commit}:{path}")])?;
        if let Ok(bl) = serde_yaml_ng::from_str::<BiLink>(&text) {
            out.insert(path, bl);
        }
    }
    Ok(out)
}

fn uuid_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).trim_end_matches(".yaml").to_string()
}
