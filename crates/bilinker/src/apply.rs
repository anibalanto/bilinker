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

/// Un endpoint que no se pudo mirar, y por qué.
///
/// **No es un fix que falta: es una respuesta que falta.** Un endpoint acá no entra en
/// la cuenta de *"no hay nada que arreglar"*, porque sobre él no se sabe.
pub struct Unlooked {
    pub uuid: String,
    pub n: u8,
    /// Qué falló. Es para el reporte, no para decidir: un *"no se pudo"* sin decir qué
    /// falló es la misma respuesta vacía en otro lugar.
    pub why: &'static str,
}

impl Unlooked {
    pub fn short(&self) -> &str { &self.uuid[..8.min(self.uuid.len())] }
}

/// Qué salió de recorrer la capa.
///
/// **Dos formas y no una lista con un flag.** Una capa fría no tiene una lista de fixes
/// vacía: no tiene lista. Que el tipo lo diga es lo que impide imprimir
/// `Pending fixes (0)` sobre algo que nadie miró — el caso que [`is_cold`] describe.
///
/// [`is_cold`]: crate::cache::Cache::is_cold
pub enum Scan {
    /// La capa no tiene estado calculado. Lo llena `check`, y `apply` lo nombra en vez
    /// de suplirlo: hacerlo acá escondería el costo de verificar la capa entera adentro
    /// de un comando que dice *"propone fixes"*.
    Cold { bilinks: usize },
    /// La capa se miró. Los que no se pudieron van aparte de los fixes, **no
    /// mezclados con los que no tenían nada**.
    Looked { fixes: Vec<PendingFix>, unlooked: Vec<Unlooked> },
}

/// Qué salió de mirar el vecindario de **un** endpoint.
///
/// **Tres valores y no un `Option`**, que es lo que separa *"no hay"* de *"no pude"* —
/// la misma figura que [`Reach`](crate::neighbours::Reach) tiene un nivel más abajo, y
/// por el mismo motivo: el tercer valor es el que hace honestos a los otros dos.
enum Looked {
    /// Hay conjunto nuevo que proponer.
    Fix(Fix),
    /// Se miró, y no hay nada que arreglar.
    Nada,
    /// No se pudo mirar.
    NoSePudo(&'static str),
}

/// Recorre la capa y calcula la ubicación nueva de cada endpoint que la necesite.
///
/// **Nunca deriva el fix de la cache**: re-resuelve contra git y el AST actuales, y
/// descarta el fix si el estado re-derivado no coincide con el cacheado.
///
/// Lo que sí le pide a la cache es la prueba de que la capa **se miró alguna vez**. No
/// es lo mismo que heredar una conclusión: un estado cacheado dice *"esto estaba
/// `MOVED`"* y eso se re-deriva; una cache fría dice *"nadie preguntó"*, y de eso no hay
/// nada que re-derivar. Ver `commands/apply.md` § "Pero la capa tiene que estar mirada".
pub fn scan_fixeable(
    layer: &Path,
    nb: crate::neighbours::Provider<'_>,
) -> Result<Scan> {
    let cache = Cache::load(layer);
    let bilinks = bilink_files(&layer.join(".bilink"));

    // **Paso 0: la capa tiene que estar mirada.** Una capa sin bilinks no cuenta —ahí
    // la cache vacía es la correcta y no hay nada que preguntar—; con bilinks y sin
    // estado, lo que falta no son fixes sino el `check` que nadie corrió.
    if !bilinks.is_empty() && cache.is_cold() {
        return Ok(Scan::Cold { bilinks: bilinks.len() });
    }

    let mut fixes = Vec::new();
    let mut unlooked = Vec::new();

    for path in bilinks {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Qué endpoints de este bilink cambian de ubicación en esta corrida. Lo usa el
        // bucle del vecindario para no llamar agujero a lo que es una espera.
        let mut se_repunta: Vec<u8> = Vec::new();

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

            se_repunta.push(n);
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
            match neighbourhood_fix(layer, &bl, uuid, n, nb)? {
                Looked::Fix(fix) => fixes.push(PendingFix {
                    bilink_path: path.clone(),
                    uuid: uuid.to_string(),
                    n, what: fix, reason: "N1",
                }),
                Looked::Nada => {}
                // **Un endpoint cuyo fragmento se está por repuntar no está sin mirar:
                // está sin poder preguntarse todavía.** El vecindario se pregunta desde
                // el rango del fragmento, y el rango que vale es el de la ubicación
                // nueva — la que este mismo scan propone. Contarlo como agujero pondría
                // un *"no se sabe"* en cada `MOVED`, que es ruido y encima al lado del
                // renglón que dice que se arregló.
                Looked::NoSePudo(_) if se_repunta.contains(&n) => {}
                // El caso que el `.ok()?` se tragaba: acá se cuenta, y por eso el
                // resumen deja de afirmar que no había nada.
                Looked::NoSePudo(why) => unlooked.push(Unlooked {
                    uuid: uuid.to_string(), n, why,
                }),
            }
        }
    }
    Ok(Scan::Looked { fixes, unlooked })
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
) -> Result<Looked> {
    // Sin proveedor el eje entero queda sin mirar, y eso ya se reporta una vez para la
    // corrida —no una por endpoint— porque es la degradación declarada del comando y no
    // un accidente de este bilink.
    let Some(p) = nb else { return Ok(Looked::Nada) };
    let e = bl.endpoint.get(n);
    // Un endpoint que no es un capture —`path`, `issue`— no tiene vecindario. No es que
    // no se haya podido: no lo tiene.
    let Some(cap_id) = e.link.capture_id() else { return Ok(Looked::Nada) };
    let Ok(cap) = Capture::load_in(layer, cap_id) else {
        return Ok(Looked::NoSePudo("el capture del endpoint no se pudo leer"));
    };

    // **El rango es la puerta de todo lo de abajo**, y no tenerlo no es una respuesta
    // sobre el vecindario: es no haber llegado a preguntar. Con la capa mirada sale de
    // la cache; que igual se re-resuelva es lo que cubre al endpoint suelto cuyo capture
    // no resolvió en el último `check`.
    let cache = Cache::load(layer);
    let range = match cache.capture_ranges(cap_id) {
        Some(r) => Some(r),
        None => crate::check::resolve_capture(
            layer, &cap, e.accepted.first(), cache.commit(uuid, n))?.1,
    };
    let Some(range) = range else {
        return Ok(Looked::NoSePudo(
            "el capture no resolvió — no hay rango desde donde preguntar por el vecindario"));
    };

    // Que el fragmento **tenga** vecindario alcanzable se sabe con la gramática, sin
    // proveedor. Sin eso no hay conjunto que declarar y no hay nada que arreglar — las
    // dos formas de eso son respuestas sobre el árbol, no ausencias de respuesta.
    let crate::neighbours::Reach::At(at) = crate::neighbours::reach(layer, &cap.file, &range)
        else { return Ok(Looked::Nada) };
    // Un `None` del proveedor es *"no pude mirar"* y no *"no hay vecinos"*: el daemon no
    // contesta, o una posición no resolvió. Leerlo como vacío es el mismo error que
    // `concepts/language-servers.md` describe del otro lado de la frontera.
    let Some(locs) = p.of(layer, &cap.file, &at)? else {
        return Ok(Looked::NoSePudo("el proveedor de vecindario no pudo contestar"));
    };
    let Some(f) = crate::neighbours::fold(layer, &locs)? else { return Ok(Looked::Nada) };

    let Some(hoy) = f.n.link.captures() else { return Ok(Looked::Nada) };

    // Tres declaraciones posibles, y cada una compara distinto contra lo de hoy.
    let hay_fix = match e.n.as_ref().and_then(|d| d.level(1)).map(|l| &l.link) {
        // **`unknown` es incomparable, así que cualquier conjunto es el fix.** No hay
        // ids de un lado: dos `unknown` no coinciden entre sí ni con una lista. Es la
        // otra forma de ganar miembros —ahí faltaba uno, acá falta la lista entera— y
        // el vacío **también** es un fix, porque *"se miró y no hay vecinos"* es una
        // respuesta distinta de *"de qué vecinos salió no se sabe"*.
        Some(l) if l.is_unknown() => true,
        // Un conjunto declarado se compara por ids, que es la identidad del vecino.
        Some(l) => l.known_ids().is_some_and(|ids| ids != hoy.ids()),
        // Sin nivel declarado, sólo un conjunto no vacío agrega algo.
        None => !hoy.is_empty(),
    };
    if !hay_fix { return Ok(Looked::Nada); }
    Ok(Looked::Fix(Fix::Neighbourhood { to: hoy.clone(), captures: f.captures }))
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

    /// Los fixes de un scan que **miró** la capa.
    ///
    /// Un `Cold` acá es un fixture mal armado —`layer()` corre `check`— y por eso
    /// revienta en vez de devolver una lista vacía: confundir las dos cosas es
    /// exactamente el defecto que estos tests cuidan.
    fn scan(layer_dir: &Path, nb: crate::neighbours::Provider<'_>) -> Vec<PendingFix> {
        match scan_fixeable(layer_dir, nb).unwrap() {
            Scan::Looked { fixes, .. } => fixes,
            Scan::Cold { bilinks } => panic!("la capa quedó fría con {bilinks} bilink(s)"),
        }
    }

    /// **Sin proveedor no hay fix de vecindario, y no es una falla.**
    ///
    /// Es la degradación que el eje tiene en todos lados: `apply` arregla lo del
    /// fragmento con git y no toca esto.
    #[test]
    fn without_a_provider_the_neighbourhood_is_left_alone() {
        let (d, ..) = layer();
        let fixes = scan(d.path(), None);
        assert!(fixes.is_empty(), "sin proveedor, nada que proponer");
    }

    /// **Una capa fría no es una lista de fixes vacía.**
    ///
    /// Es la secuencia del que clona y corre `apply` de una: sin `check` previo no hay
    /// rango desde donde preguntar, así que el comando no llega a mirar. Contestarle
    /// *"no hay nada que arreglar"* es afirmar algo sobre un árbol que nadie leyó.
    #[test]
    fn a_cold_layer_is_not_an_empty_list_of_fixes() {
        let (d, ..) = layer();
        // Lo que `layer()` calentó con su `check`, deshecho: el estado de todo clon
        // fresco, toda rama nueva y toda máquina nueva.
        std::fs::remove_file(Cache::path_in(d.path())).unwrap();

        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };
        match scan_fixeable(d.path(), Some(&p)).unwrap() {
            Scan::Cold { bilinks } => assert_eq!(bilinks, 1, "los bilinks que no se miraron"),
            Scan::Looked { .. } => panic!("una capa sin estado calculado no se miró"),
        }
        assert_eq!(p.asked.get(), 0, "y no se le preguntó nada al proveedor");
    }

    /// **Y un endpoint que no se pudo mirar no se cuenta como revisado.**
    ///
    /// El proveedor que no contesta —el daemon caído, una posición sin resolver— devuelve
    /// `None`, y leerlo como *"no hay vecinos"* es el mismo vacío en otro lugar.
    #[test]
    fn an_endpoint_that_could_not_be_looked_at_is_not_nothing() {
        let (d, ..) = layer();
        // `None` es *"no pude mirar"*; `Some(vec![])` sería *"miré y no hay"*.
        let p = Fake { locs: None, asked: Cell::new(0) };
        let Scan::Looked { fixes, unlooked } = scan_fixeable(d.path(), Some(&p)).unwrap()
            else { panic!("la capa está mirada") };

        assert!(!fixes.iter().any(|f| matches!(f.what, Fix::Neighbourhood { .. })),
                "no hay nada que proponer");
        assert!(!unlooked.is_empty(),
                "y eso no es lo mismo que no haber encontrado nada");
    }

    /// **Con proveedor, propone el conjunto que la firma menciona hoy.**
    #[test]
    fn with_a_provider_it_proposes_the_declared_set() {
        let (d, ..) = layer();
        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };

        let fixes = scan(d.path(), Some(&p));
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. }))
            .expect("propone el vecindario");
        let Fix::Neighbourhood { to, captures } = &fix.what else { unreachable!() };
        assert_eq!(to.len(), 1, "un vecino: {to}");
        assert_eq!(captures.len(), 1, "y su capture, para poder escribirlo");
    }

    /// **Un `unknown` declarado se llena**, que es lo que la `003` dejó pendiente en 139
    /// niveles.
    ///
    /// No hay conjunto contra el que comparar —dos `unknown` no coinciden entre sí ni
    /// con una lista— así que cualquier conjunto que el proveedor alcance es el fix.
    #[test]
    fn an_unknown_level_gets_filled() {
        let (d, uuid, _) = layer();
        // La declaración dice "el contrato está y de qué vecinos salió no se sabe".
        let path = BiLink::path_in(d.path(), &uuid);
        let mut bl = BiLink::load(&path).unwrap();
        bl.endpoint.get_mut(0).n = Some(bilink_format::DeclaredN::of_level_1(bilink_format::LevelLink::Unknown));
        bl.write(&path).unwrap();

        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };
        let fixes = scan(d.path(), Some(&p));
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. }))
            .expect("un `unknown` tiene fix: llenarlo");
        let Fix::Neighbourhood { to, captures } = &fix.what else { unreachable!() };
        assert_eq!(to.len(), 1, "el vecino que el proveedor alcanzó: {to}");
        assert_eq!(captures.len(), 1, "con su capture, para poder escribirlo");

        // Y llenarlo **no toca `accepted`**: el contrato conservado sigue donde estaba,
        // así que el endpoint sigue pidiendo una decisión.
        apply_fix(d.path(), fix).unwrap();
        let bl = BiLink::load(&path).unwrap();
        assert!(!bl.endpoint.get(0).n.as_ref().unwrap().level(1).unwrap().link.is_unknown(),
                "la declaración dejó de decir `unknown`");
    }

    /// **Y el vacío también es un fix sobre un `unknown`.**
    ///
    /// *"Se miró y no hay vecinos"* es una respuesta distinta de *"de qué vecinos salió
    /// no se sabe"*, así que quedarse en `unknown` sería perder la que sí se consiguió.
    #[test]
    fn an_unknown_level_gets_filled_even_when_the_answer_is_empty() {
        let (d, uuid, _) = layer();
        let path = BiLink::path_in(d.path(), &uuid);
        let mut bl = BiLink::load(&path).unwrap();
        bl.endpoint.get_mut(0).n = Some(bilink_format::DeclaredN::of_level_1(bilink_format::LevelLink::Unknown));
        bl.write(&path).unwrap();

        // El proveedor miró y no alcanzó nada de esta capa: `Some(vec![])`.
        let p = Fake { locs: Some(vec![]), asked: Cell::new(0) };
        let fixes = scan(d.path(), Some(&p));
        assert!(fixes.iter().any(|f| matches!(f.what, Fix::Neighbourhood { .. })),
                "el vacío reemplaza al `unknown`: son dos respuestas distintas");
    }

    /// **Y sobre un conjunto ya declarado, el vacío no es un fix.** Ahí sí hay con qué
    /// comparar, y comparar es lo que decide.
    #[test]
    fn an_already_empty_declared_set_is_not_a_fix() {
        let (d, ..) = layer();
        let p = Fake { locs: Some(vec![]), asked: Cell::new(0) };
        let fixes = scan(d.path(), Some(&p));
        assert!(!fixes.iter().any(|f| matches!(f.what, Fix::Neighbourhood { .. })),
                "sin nivel declarado y sin vecinos, no hay nada que proponer");
    }

    /// **Y no aprueba nada**: aplicar deja el endpoint en `RELOCATED`.
    #[test]
    fn applying_it_does_not_accept() {
        let (d, uuid, _) = layer();
        let dto = Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 };
        let p = Fake { locs: Some(vec![dto]), asked: Cell::new(0) };

        let fixes = scan(d.path(), Some(&p));
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

        let fixes = scan(d.path(), Some(&p));
        let fix = fixes.iter().find(|f| matches!(f.what, Fix::Neighbourhood { .. })).unwrap();
        apply_fix(d.path(), fix).unwrap();
        let _ = uuid;

        let otra = scan(d.path(), Some(&p));
        assert!(!otra.iter().any(|f| matches!(f.what, Fix::Neighbourhood { .. })),
                "ya está declarado: no hay nada que repuntar");
    }
}
