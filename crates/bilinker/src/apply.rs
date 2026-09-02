//! `bilinker apply` — corrige **dónde apunta** un endpoint, y nada más.
//!
//! Acuña el capture de la ubicación nueva y repunta el `link`. **No escribe
//! `accepted`**, así que el endpoint no queda `Ok`: queda en `Relocated` hasta que
//! alguien apruebe la ubicación nueva.
//!
//! > `apply` propone, `accept` dispone.
//!
//! No hay fork ni copy-on-write. Un capture es inmutable y su id es el hash de su
//! ubicación, así que corregir una ubicación **siempre** produce un capture
//! distinto: se acuña y se repunta un solo `link`. Los demás referentes no se
//! enteran, sin que haya que decidir nada.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{BiLink, Capture};

use crate::cache::Cache;
use crate::state::{CaptureState, EndpointState};
use crate::{grammar, query};

pub struct PendingFix {
    pub bilink_path: PathBuf,
    pub uuid: String,
    pub n: u8,
    pub what: Fix,
    /// Qué lo motivó. Sólo informativo: el fix es el mismo — una ubicación nueva.
    pub reason: &'static str,
}

/// Qué se repunta.
///
/// **Dos formas, porque son dos cosas distintas.** El `link` de un fragmento es un
/// capture y se reemplaza por otro; el `n.1.link` de un vecindario es un **conjunto**,
/// y lo que cambia no es sólo dónde está cada miembro — también quiénes son.
pub enum Fix {
    /// El fragmento se movió: un capture por otro.
    Fragment { from: Capture, to: Capture },
    /// El conjunto de vecinos que la firma menciona cambió.
    ///
    /// **Los captures van adentro porque hay que escribirlos.** Un miembro nuevo es
    /// un capture que todavía no existe en la capa, y repuntar `n.1.link` a un id sin
    /// archivo dejaría el vecindario apuntando al vacío.
    Neighbourhood { to: bilink_format::CaptureSet, captures: Vec<Capture> },
}

impl PendingFix {
    pub fn short(&self) -> &str { &self.uuid[..8.min(self.uuid.len())] }

    pub fn description(&self) -> String {
        match &self.what {
            Fix::Fragment { from, to } if from.file != to.file =>
                format!("{} → {}", from.file, to.file),
            Fix::Fragment { to, .. } =>
                format!("query → {}", to.query.as_deref().unwrap_or("(archivo entero)")),
            Fix::Neighbourhood { to, .. } =>
                format!("n1 → {} vecino(s): {to}", to.len()),
        }
    }
}

/// Recorre la capa y calcula la ubicación nueva de cada endpoint que la necesite.
///
/// **Nunca deriva el fix de la cache**: re-resuelve contra git y el AST actuales, y
/// descarta el fix si el estado re-derivado no coincide con el cacheado.
pub fn scan_fixeable(
    layer: &Path,
    nb: crate::neighbours::Provider<'_>,
) -> Result<Vec<PendingFix>> {
    let cache = Cache::load(layer);
    let mut fixes = Vec::new();

    for path in bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        for n in [0u8, 1u8] {
            let e = bl.endpoint.get(n);
            let Some(cap_id) = e.link.capture_id() else { continue };
            let Ok(cap) = Capture::load_in(layer, cap_id) else { continue };

            // **El estado se re-deriva; la cache no decide nada acá.**
            //
            // El estado cacheado lo escribió el último `check`, y el archivo pudo
            // cambiar después: aplicar un fix derivado de esa foto es corregir
            // contra algo que ya no está. De la cache sólo sale `commit`, que es
            // un dato de git y no una conclusión sobre el árbol actual.
            let (state, _) = crate::check::resolve_capture(
                layer, &cap, e.accepted.first(), cache.commit(uuid, n))?;

            let calculado = match state {
                CaptureState::Moved      => compute_moved(layer, &cap).map(|c| (c, "MOVED")),
                CaptureState::Reanchored =>
                    compute_reanchored(layer, &bl, uuid, n, &cap).map(|c| (c, "REANCHORED")),
                _ => continue,
            };

            // Un fix que no se puede calcular **se reporta**. Propagarlo con `?`
            // abortaba el scan entero: un solo endpoint sin arreglo dejaba sin
            // revisar a todos los demás.
            let (to, reason) = match calculado {
                Ok((Some(c), reason)) => (c, reason),
                Ok((None, _)) => continue,
                Err(why) => {
                    eprintln!("warn: {}… endpoint.{n}: {why}", &uuid[..8.min(uuid.len())]);
                    continue;
                }
            };

            // Un fix que no mueve nada es un no-op.
            if to.id() == cap.id() { continue; }

            fixes.push(PendingFix {
                bilink_path: path.clone(),
                uuid: uuid.to_string(),
                n, what: Fix::Fragment { from: cap, to }, reason,
            });
        }

        // ── el vecindario ────────────────────────────────────────────────────
        //
        // **Va aparte del bucle de arriba** porque no sale de un estado del capture:
        // el conjunto de vecinos puede haber cambiado con el fragmento intacto.
        for n in [0u8, 1u8] {
            if let Some(fix) = neighbourhood_fix(layer, &bl, uuid, n, nb)? {
                fixes.push(PendingFix {
                    bilink_path: path.clone(),
                    uuid: uuid.to_string(),
                    n, what: fix, reason: "N1",
                });
            }
        }
    }
    Ok(fixes)
}

/// El conjunto de vecinos que la firma menciona hoy, si difiere del declarado.
///
/// **Sin proveedor no hay fix**, y no es una falla: es la degradación que el eje del
/// vecindario tiene en todos lados. `apply` arregla lo del fragmento con git y dice
/// que no pudo tocar esto.
///
/// Y **descubrir el conjunto es lo único que necesita el proveedor**. Que un vecino se
/// haya mudado de archivo lo resuelve git como cualquier `MOVED`; lo que git no puede
/// saber es que la firma ahora menciona un tipo más.
fn neighbourhood_fix(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    nb: crate::neighbours::Provider<'_>,
) -> Result<Option<Fix>> {
    let Some(p) = nb else { return Ok(None) };
    let e = bl.endpoint.get(n);
    let Some(cap_id) = e.link.capture_id() else { return Ok(None) };
    let Ok(cap) = Capture::load_in(layer, cap_id) else { return Ok(None) };

    let cache = Cache::load(layer);
    let Some(range) = cache.capture_ranges(cap_id)
        .or_else(|| crate::check::resolve_capture(layer, &cap, e.accepted.first(),
                                                  cache.commit(uuid, n)).ok()?.1)
        else { return Ok(None) };

    // Que el fragmento **tenga** vecindario alcanzable se sabe con la gramática, sin
    // proveedor. Sin eso no hay conjunto que declarar y no hay nada que arreglar.
    let crate::neighbours::Reach::At(at) = crate::neighbours::reach(layer, &cap.file, &range)
        else { return Ok(None) };
    let Some(locs) = p.of(layer, &cap.file, &at)? else { return Ok(None) };
    let Some(f) = crate::neighbours::fold(layer, &locs)? else { return Ok(None) };

    let declarado = e.n.as_ref().and_then(|d| d.level(1)).map(|l| l.link.ids()).unwrap_or(&[]);
    if declarado == f.n.link.ids() { return Ok(None); }
    Ok(Some(Fix::Neighbourhood { to: f.n.link, captures: f.captures }))
}

/// Acuña el capture nuevo y repunta el `link`. **No toca `accepted`.**
pub fn apply_fix(layer: &Path, pf: &PendingFix) -> Result<Vec<PathBuf>> {
    let mut tocados = Vec::new();
    let mut bl = BiLink::load(&pf.bilink_path)?;

    match &pf.what {
        Fix::Fragment { to, .. } => {
            let (id, cap_path, _existed) = to.write_in(layer)?;
            bl.endpoint.get_mut(pf.n).link = format!("capture {id}").parse()?;
            tocados.push(cap_path);
        }
        Fix::Neighbourhood { to, captures } => {
            // Los captures primero: un `n.1.link` que nombra un archivo que no está
            // es un vecindario apuntando al vacío.
            for c in captures {
                let (_, p, _) = c.write_in(layer)?;
                tocados.push(p);
            }
            bl.endpoint.get_mut(pf.n).n =
                Some(bilink_format::DeclaredN::of_level_1(to.clone()));
        }
    }
    bl.write(&pf.bilink_path)?;

    // Repuntar no aprueba: el endpoint queda pidiendo una decisión humana.
    let mut cache = Cache::load(layer);
    cache.set_endpoint_state(&pf.uuid, pf.n, EndpointState::Relocated);
    cache.save(layer)?;

    tocados.push(pf.bilink_path.clone());
    Ok(tocados)
}

// ─── cálculo de la ubicación nueva ────────────────────────────────────────────

/// MOVED: el índice de renames de git.
///
/// **Tres cosas distintas terminan en "no se pudo", y cada una manda a mirar un
/// lugar distinto.** Confundirlas es lo que hacía que el mensaje culpara a git —el
/// índice de renames— cuando el que había cambiado era el anchor:
///
/// | Lo que pasó | Estado | Quién lo explica |
/// |---|---|---|
/// | el destino está, pero el anchor se renombró también | `MOVED` | **acá** |
/// | git no detectó el rename: el destino no está trackeado | sin fix | [`get`](crate::get) |
/// | el fragmento no está en ninguna parte | sin fix | [`get`](crate::get) |
///
/// Las dos últimas **no llegan a `apply`**: dejan el capture en un estado sin fix, y
/// `apply` no los toca. Que el mensaje de acá pretendiera cubrirlas era parte del
/// mismo error — describía condiciones que este camino no puede observar.
fn compute_moved(layer: &Path, cap: &Capture) -> Result<Option<Capture>> {
    let Some(new_file) = crate::check::git_renamed_to(layer, &cap.file) else {
        // **Casi inalcanzable, y por eso el mensaje no diagnostica nada.** Se entra
        // acá sólo con `CaptureState::Moved`, que ya implica que git reportó el
        // rename; llegar significa que el árbol cambió entre el `check` y esta
        // línea. Las otras dos causas —destino sin trackear, fragmento en ninguna
        // parte— **no pasan por acá**: dejan el capture en un estado sin fix, y
        // quien las explica es [`get`](crate::get), que es donde se pregunta qué
        // pasó con un endpoint que no resuelve.
        bail!(
            "MOVED: git ya no reporta el rename de '{}' — el árbol cambió mientras \
             se calculaba.\n      Correr `bilinker check .` de nuevo.",
            cap.file
        );
    };
    let moved = Capture { file: new_file, ..cap.clone() };
    // Verificar que la referencia siga resolviendo en el path nuevo. Sin aceptación:
    // lo que se pregunta es si el anchor está ahí, no si dice lo que se aprobó.
    let (state, _) = crate::check::resolve_capture(layer, &moved, None, None)?;
    if !state.is_resolved() {
        // **Nombrar el anchor, no el estado.** Es el caso de MOVED y REANCHORED a la
        // vez, que ningún estado expresa porque los dos son de resolución y el
        // capture guarda uno solo. Y no hay auto-fix: dónde quedó el fragmento
        // adentro del archivo destino es una inferencia que `apply` no debería hacer
        // sola. Lo que sí puede es decir qué comando la hace.
        bail!(
            "MOVED: el archivo se movió a '{}', pero el anchor {} ya no está ahí \
             ({state}).\n      Repuntar con `bilinker recapture`.",
            moved.file,
            named(cap),
        );
    }
    Ok(Some(moved))
}

/// El anchor entre backticks, o una frase que no finge saber su nombre.
fn named(cap: &Capture) -> String {
    match cap.query.as_deref().and_then(query::anchor_name) {
        Some(a) => format!("`{a}`"),
        None => "capturado".to_string(),
    }
}

/// REANCHORED: la query relajada, con el nombre nuevo.
fn compute_reanchored(
    layer: &Path, bl: &BiLink, uuid: &str, n: u8, cap: &Capture,
) -> Result<Option<Capture>> {
    let Some(query_str) = &cap.query else { return Ok(None) };
    let path = layer.join(&cap.file);
    if !path.exists() { return Ok(None); }

    let source   = std::fs::read_to_string(&path)?;
    let language = grammar::for_language(grammar::language_for_file(&cap.file))?;
    let accepted = bl.endpoint.get(n).accepted.first();

    // Los **dos** hacen falta: el hash dice qué texto buscar y el commit dice de
    // dónde sacarlo. Con uno solo, `accepted_text` no devuelve nada y no hay contra
    // qué puntuar los candidatos.
    let mut cache = Cache::load(layer);
    let commit = accepted.and_then(|a| cache.commit_or_derive(layer, uuid, n, cap, &a.hash));
    let Some((new_name, score)) = crate::check::find_renamed_anchor(
        layer, language, &source, query_str, cap,
        accepted.map(|a| a.hash.as_str()), commit.as_deref())?
    else {
        bail!("REANCHORED: el anchor ya no se localiza — correr `bilinker check`");
    };
    let Some(new_query) = query::rewrite_name_predicate(query_str, &new_name) else {
        bail!("REANCHORED: la query no tiene predicado de nombre para reescribir");
    };
    eprintln!("  anchor → {new_name}  (similitud {:.0}%)", score * 100.0);
    Ok(Some(Capture { query: Some(new_query), ..cap.clone() }))
}


#[cfg(test)]
mod neighbourhood_fix_tests {
    use super::*;
    use crate::neighbours::{Location, Neighbours};
    use bilink_format::{CaptureSet, DeclaredN, LinkEndpoint, Ranges};
    use std::cell::Cell;
    use tempfile::tempdir;

    struct Fake { locs: Option<Vec<Location>>, asked: Cell<usize> }
    impl Neighbours for Fake {
        fn of(&self, _l: &Path, _f: &str, _at: &[usize]) -> Result<Option<Vec<Location>>> {
            self.asked.set(self.asked.get() + 1);
            Ok(self.locs.clone())
        }
    }

    /// Un método con firma, y un DTO al lado.
    fn layer() -> (tempfile::TempDir, String, String) {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Svc.rs"),
            "pub struct Dto { pub x: u8 }\n\npub fn get(d: Dto) -> Dto { todo!() }\n").unwrap();
        for args in [vec!["init","-q"], vec!["config","user.email","t@t"],
                     vec!["config","user.name","t"], vec!["add","-A"], vec!["commit","-qm","i"]] {
            std::process::Command::new("git").current_dir(d.path()).args(&args).output().unwrap();
        }
        let (c, _, _) = crate::capture::compute(
            d.path(), "Svc.rs", &[((3,1),(3,1))], None).unwrap();
        let id = c.id();
        c.write_in(d.path()).unwrap();

        let uuid = "44444444-4444-4444-8444-444444444444".to_string();
        let mut bl = BiLink::new(format!("capture {id}").parse().unwrap(), LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).r#as = Some("interface".into());
        bl.write(&BiLink::path_in(d.path(), &uuid)).unwrap();
        // El `check` puebla el `range` que el scan necesita.
        let _ = crate::check::check_with(d.path(), d.path(), None);
        (d, uuid, id)
    }

    /// **Sin proveedor no hay fix de vecindario, y no es una falla.**
    ///
    /// Es la degradación que el eje tiene en todos lados: `apply` arregla lo del
    /// fragmento con git y no toca esto.
    #[test]
    fn without_a_provider_the_neighbourhood_is_left_alone() {
        let (d, ..) = layer();
        let fixes = scan_fixeable(d.path(), None).unwrap();
        assert!(fixes.is_empty(), "sin proveedor, nada que proponer");
    }

    /// **Con proveedor, propone el conjunto que la firma menciona hoy.**
    #[test]
    fn with_a_provider_it_proposes_the_declared_set() {
        let (d, ..) = layer();
        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };

        let fixes = scan_fixeable(d.path(), Some(&p)).unwrap();
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. }))
            .expect("propone el vecindario");
        let Fix::Neighbourhood { to, captures } = &fix.what else { unreachable!() };
        assert_eq!(to.len(), 1, "un vecino: {to}");
        assert_eq!(captures.len(), 1, "y su capture, para poder escribirlo");
    }

    /// **Y no aprueba nada**: aplicar deja el endpoint en `RELOCATED`.
    #[test]
    fn applying_it_does_not_accept() {
        let (d, uuid, _) = layer();
        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };

        let fixes = scan_fixeable(d.path(), Some(&p)).unwrap();
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. })).unwrap();
        apply_fix(d.path(), fix).unwrap();

        let bl = BiLink::load(&BiLink::path_in(d.path(), &uuid)).unwrap();
        assert!(bl.endpoint.get(0).n.is_some(), "el conjunto quedó declarado");
        assert!(bl.endpoint.get(0).accepted.is_empty(), "y no aprobó nada");
        let _ = (DeclaredN::of_level_1(CaptureSet::new(vec![])), Ranges::one(0, 1));
    }

    /// Y si el conjunto ya coincide, no hay fix: un no-op no se propone.
    #[test]
    fn a_matching_set_is_not_a_fix() {
        let (d, uuid, _) = layer();
        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };

        let fixes = scan_fixeable(d.path(), Some(&p)).unwrap();
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. })).unwrap();
        apply_fix(d.path(), fix).unwrap();
        let _ = uuid;

        let otra = scan_fixeable(d.path(), Some(&p)).unwrap();
        assert!(!otra.iter().any(|f| matches!(f.what, Fix::Neighbourhood { .. })),
                "ya está declarado: no hay nada que repuntar");
    }
}
