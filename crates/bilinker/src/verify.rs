//! `bilinker verify-ref` — que la ref tenga la forma que promete.
//!
//! La misma verificación en dos lugares que no se parecen: **el servidor**, donde
//! puede rechazar un push, y **el que recibe una ref ajena**, donde puede avisar
//! antes de calcular drift contra un árbol fabricado.
//!
//! **Nada de acá necesita tree-sitter ni resolver una query.** Son comparaciones de
//! tree oids, parseo de YAML y hashes — que es lo que permite correrlo en un
//! servidor que no adoptó bilinker. Y **no escribe nada**, ni siquiera cache: es lo
//! único que un hook puede correr sin efectos.
//!
//! Lo que no verifica, a propósito: **si los bilinks están en `OK`.** Una ref con
//! drift es normal —es el estado que el método existe para reportar— y exigirlo
//! haría imposible `track`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

use bilink_format::{Accepted, BiLink, Capture};

use crate::bilink_ref::{Act, Repo};
use crate::refmsg::{self, Read};

/// Lo que se le reprocha a un commit. Vacío es que pasó.
pub struct Verdict {
    pub commit: String,
    pub faults: Vec<String>,
    /// Anterior a la gramática: su forma no se verifica, y eso no es un error.
    pub pre_grammar: bool,
}

pub struct Report {
    pub refname: String,
    pub verdicts: Vec<Verdict>,
    /// `false` cuando no había allowlist: la firma **no se verificó**, y eso no es
    /// lo mismo que haberla verificado y que esté bien.
    pub signatures_checked: bool,
}

impl Report {
    pub fn rejected(&self) -> usize {
        self.verdicts.iter().filter(|v| !v.faults.is_empty()).count()
    }
    pub fn pre_grammar(&self) -> usize {
        self.verdicts.iter().filter(|v| v.pre_grammar).count()
    }
    pub fn ok(&self) -> bool { self.rejected() == 0 }
}

/// Verifica un rango de la ref.
///
/// `old` en `None` es *"toda la ref"*: los commits propios, del corte para acá.
pub fn verify(
    dir: &Path,
    refname: &str,
    old: Option<&str>,
    new: &str,
    signers: Option<&Path>,
) -> Result<Report> {
    let repo = Repo::open(dir)?;

    // ── del rango ────────────────────────────────────────────────────────────
    if is_zero(new) {
        return Ok(Report {
            refname: refname.to_string(),
            verdicts: vec![Verdict {
                commit: new.to_string(),
                faults: vec![
                    "borrar la ref no está permitido: sin esto, \"sólo avanza\" se \
                     esquiva borrándola y empujándola de nuevo".into(),
                ],
                pre_grammar: false,
            }],
            signatures_checked: signers.is_some(),
        });
    }

    let old = old.filter(|o| !is_zero(o));
    if let Some(o) = old {
        // Append-only: un no-fast-forward es una reescritura, no un avance.
        if !repo.is_ancestor(o, new)? {
            return Ok(Report {
                refname: refname.to_string(),
                verdicts: vec![Verdict {
                    commit: new.to_string(),
                    faults: vec![format!(
                        "no es fast-forward sobre {}: la ref es append-only, y \
                         reescribirla deja sin baseline a toda aceptación del repo",
                        short(o)
                    )],
                    pre_grammar: false,
                }],
                signatures_checked: signers.is_some(),
            });
        }
    }

    // ── de cada commit, del más viejo al más nuevo ───────────────────────────
    //
    // El orden importa: **la gramática no vuelve para atrás**, y eso sólo se puede
    // decidir sabiendo si algún antepasado del rango ya la llevaba.
    let commits = range(&repo, old, new)?;
    let mut seen_grammar = match old {
        Some(o) => carries_grammar(&repo, o)?,
        None => false,
    };

    let mut verdicts = Vec::new();
    for commit in commits {
        let mut v = verify_commit(&repo, &commit, signers, seen_grammar)?;
        if !v.pre_grammar {
            seen_grammar = true;
        }
        v.commit = commit;
        verdicts.push(v);
    }

    Ok(Report {
        refname: refname.to_string(),
        verdicts,
        signatures_checked: signers.is_some(),
    })
}

fn verify_commit(
    repo: &Repo,
    commit: &str,
    signers: Option<&Path>,
    seen_grammar: bool,
) -> Result<Verdict> {
    let message = repo.git(&["log", "-1", "--format=%B", commit])?;
    let mut faults = Vec::new();

    let parsed = match refmsg::read(&message) {
        Ok(Read::PreGrammar) => {
            // Pasa **una vez**, y sólo mientras nadie abajo lo haya llevado. Un
            // commit sin trailer encima de uno que lo tiene no es historia vieja:
            // es alguien esquivando la verificación.
            if seen_grammar {
                faults.push(
                    "no lleva Bilinker-Version y su antepasado sí: la gramática no \
                     vuelve para atrás"
                        .into(),
                );
                return Ok(Verdict { commit: String::new(), faults, pre_grammar: false });
            }
            return Ok(Verdict { commit: String::new(), faults, pre_grammar: true });
        }
        Ok(Read::Parsed(m)) => Some(m),
        Err(e) => {
            faults.push(format!("el mensaje no parsea: {e}"));
            None
        }
    };
    let _ = parsed;

    // ── un commit hace una cosa ──────────────────────────────────────────────
    let act = match repo.classify(commit) {
        Ok(a) => Some(a),
        Err(e) => {
            faults.push(e.to_string());
            None
        }
    };

    // ── disyunción y fidelidad ───────────────────────────────────────────────
    if let Some(Act::Absorption { project }) = &act {
        if let Err(e) = repo.verify_disjoint(project) {
            faults.push(first_line(&e.to_string()));
        }
        let tree = repo.git(&["rev-parse", &format!("{commit}^{{tree}}")])?;
        if let Err(e) = repo.verify_faithful(tree.trim(), project) {
            faults.push(first_line(&e.to_string()));
        }
    }

    // ── lo que el commit escribió en `.bilink/` ──────────────────────────────
    let parents = repo.parents(commit)?;
    let base = parents.first().map(String::as_str);
    faults.extend(verify_files(repo, base, commit)?);
    faults.extend(verify_agree(repo, base, commit)?);

    // ── la firma ─────────────────────────────────────────────────────────────
    if let Some(file) = signers {
        if let Err(e) = repo.verify_signature(commit, file) {
            faults.push(first_line(&e.to_string()));
        }
    }

    Ok(Verdict { commit: String::new(), faults, pre_grammar: false })
}

/// Los archivos del formato que el commit agrega o modifica.
///
/// **Un capture sólo se agrega.** Su id es el hash de su ubicación, así que
/// modificarlo le cambiaría el nombre: un capture con el mismo nombre y otro
/// contenido es una ubicación reescrita bajo una identidad ajena.
fn verify_files(repo: &Repo, base: Option<&str>, commit: &str) -> Result<Vec<String>> {
    let mut faults = Vec::new();

    for (status, path) in repo.changed_bilink_files(base, commit)? {
        let es_capture = path.contains("/capture/");

        if es_capture && status != 'A' {
            faults.push(format!(
                "{path}: un capture no se {}. Su id es el hash de su ubicación, así \
                 que cambiarlo le cambia el nombre",
                if status == 'D' { "borra" } else { "modifica" }
            ));
            continue;
        }
        if status == 'D' {
            continue;
        }

        let text = repo.git(&["show", &format!("{commit}:{path}")])?;
        if es_capture {
            match serde_yaml_ng::from_str::<Capture>(&text) {
                Ok(cap) => {
                    let esperado = cap.id();
                    let nombre = file_stem(&path);
                    if nombre != esperado {
                        faults.push(format!(
                            "{path}: el nombre no es su hash — {} para esa ubicación",
                            &esperado[..8]
                        ));
                    }
                }
                Err(e) => faults.push(format!("{path}: no valida contra el formato: {e}")),
            }
        } else if path.ends_with("/version") {
            if let Some(fault) = version_too_new(&text) {
                faults.push(format!("{path}: {fault}"));
            }
        } else if path.ends_with(".yaml") {
            if let Err(e) = serde_yaml_ng::from_str::<BiLink>(&text) {
                faults.push(format!("{path}: no valida contra el formato: {e}"));
            }
        }
    }
    Ok(faults)
}

/// **A `agree` sólo se agrega el autor del commit.**
///
/// Es la fila que convierte `agree` de atribución en atestación, y la que hace que
/// no haga falta ningún mapeo de nombres a claves: la firma ata el commit a su
/// autor, y esto ata los nombres agregados a ese mismo autor.
///
/// **Sacar no está restringido**, y no puede estarlo: es lo que hace `adopt` al
/// traer valores distintos, y lo que hace un `accept` cuando los valores cambian y
/// la lista se vacía. Lo que afirma algo sobre otra persona es agregar.
fn verify_agree(repo: &Repo, base: Option<&str>, commit: &str) -> Result<Vec<String>> {
    let autor = repo.git(&["log", "-1", "--format=%an", commit])?.trim().to_string();
    let mut faults = Vec::new();

    for (status, path) in repo.changed_bilink_files(base, commit)? {
        if status == 'D' || path.contains("/capture/") || !path.ends_with(".yaml") {
            continue;
        }
        let Ok(nuevo) = repo.bilink_at(commit, &path) else { continue };
        let viejo = base.and_then(|b| repo.bilink_at(b, &path).ok());

        for n in [0u8, 1u8] {
            let antes: BTreeSet<String> = viejo
                .as_ref()
                .and_then(|bl| bl.endpoint.get(n).accepted.as_ref().map(agree_of))
                .unwrap_or_default();
            let ahora = nuevo.endpoint.get(n).accepted.as_ref().map(agree_of).unwrap_or_default();

            for nombre in ahora.difference(&antes) {
                if *nombre != autor {
                    faults.push(format!(
                        "agrega `- {nombre}` a {}.{n} y el autor es {autor}: \
                         nadie aprueba en nombre de otro",
                        file_stem(&path).get(..8).unwrap_or("?")
                    ));
                }
            }
        }
    }
    Ok(faults)
}

fn agree_of(a: &Accepted) -> BTreeSet<String> { a.agree.clone() }

/// Que el formato declarado no sea más nuevo que el que este binario entiende.
fn version_too_new(declared: &str) -> Option<String> {
    let declarada = declared.trim();
    let nuestra = bilink_format::VERSION;
    let n = |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse().ok()).collect() };
    (n(declarada) > n(nuestra))
        .then(|| format!("declara el formato {declarada} y este binario entiende {nuestra}"))
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Los commits **propios** de la ref en el rango, del más viejo al más nuevo.
fn range(repo: &Repo, old: Option<&str>, new: &str) -> Result<Vec<String>> {
    let spec = match old {
        Some(o) => format!("{o}..{new}"),
        None => new.to_string(),
    };
    let out = repo.git(&["rev-list", "--reverse", "--first-parent", &spec])?;
    let todos: Vec<String> = out.lines().map(str::to_string).collect();

    // Sin `old`, `rev-list` se sale de la ref al llegar al corte y sigue por la
    // historia del proyecto. El freno es el de siempre: los commits de la ref
    // llevan `.bilink/` en su árbol y los del proyecto no.
    if old.is_some() {
        return Ok(todos);
    }
    let mut propios = Vec::new();
    for c in todos.into_iter().rev() {
        if !repo.tree_has_any_bilink(&c)? {
            break;
        }
        propios.push(c);
    }
    propios.reverse();
    Ok(propios)
}

fn carries_grammar(repo: &Repo, commit: &str) -> Result<bool> {
    let message = repo.git(&["log", "-1", "--format=%B", commit])?;
    Ok(!matches!(refmsg::read(&message), Ok(Read::PreGrammar)))
}

fn is_zero(sha: &str) -> bool { sha.chars().all(|c| c == '0') }

fn file_stem(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).trim_end_matches(".yaml").to_string()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

fn short(sha: &str) -> &str { &sha[..sha.len().min(7)] }

/// Lee las líneas `<viejo> <nuevo> <ref>` de un `pre-receive`.
///
/// **Las refs que no son `refs/bilink/*` se ignoran**: este hook no opina sobre las
/// ramas del proyecto, y opinar sería exceder lo que se le pidió.
pub fn parse_stdin(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            match (p.next(), p.next(), p.next()) {
                (Some(o), Some(n), Some(r)) if r.starts_with("refs/bilink/") => {
                    Some((o.to_string(), n.to_string(), r.to_string()))
                }
                _ => None,
            }
        })
        .collect()
}

/// Resuelve el argumento de la línea de comandos a `(refname, old, new)`.
pub fn target(dir: &Path, arg: Option<&str>) -> Result<(String, Option<String>, String)> {
    let repo = Repo::open(dir)?;
    let spec = match arg {
        Some(a) => a.to_string(),
        None => {
            let branch = repo.require_branch()?;
            Repo::ref_name(&branch)
        }
    };

    if let Some((o, n)) = spec.split_once("..") {
        let old = repo.git(&["rev-parse", o]).context("el extremo viejo del rango")?;
        let new = repo.git(&["rev-parse", n]).context("el extremo nuevo del rango")?;
        return Ok((spec.clone(), Some(old.trim().to_string()), new.trim().to_string()));
    }

    let new = repo
        .git(&["rev-parse", &spec])
        .with_context(|| format!("{spec} no existe"))?;
    Ok((spec, None, new.trim().to_string()))
}
