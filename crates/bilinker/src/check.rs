//! `bilinker check` — verifica, y **no escribe ni un byte en git**.
//!
//! Opera en dos pasos y sobre **dos dimensiones**. Primero resuelve el capture
//! —dónde está el fragmento—, después compara contra `accepted` —dónde se aprobó
//! que estuviera, y qué se aprobó que dijera. Todo lo que produce va a la
//! [cache](crate::cache).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use bilink_format::bilink::bilink_files;
use bilink_format::{BiLink, Capture, LinkEndpoint, Ranges};

use crate::cache::Cache;
use crate::dimension::{self, DimensionStates};
use crate::state::{CaptureState, EndpointState};
use crate::{grammar, hash, query};

/// Lo que `check` encontró: lo que verificó, y lo que no pudo leer.
///
/// **Las dos listas viajan juntas porque el conteo de una no es el total.** Un
/// `check` que verificó 203 de 206 no puede decir `all clean (206)` ni
/// `all clean (203)` a secas: el primero miente sobre lo que miró, el segundo
/// esconde lo que no pudo mirar.
#[derive(Debug)]
pub struct Checked {
    pub results: Vec<CheckResult>,
    pub unreadable: Vec<Unreadable>,
    /// Los endpoints con nivel 1 adquirido que entraron en esta corrida.
    pub n1: crate::neighbours::Demand,
}

/// Un bilink que no parsea. **Es un estado, no una ausencia.**
///
/// Saltearlo para no abortar el recorrido de los demás está bien; saltearlo en
/// silencio hace que *"no pude leer 206"* salga igual que *"no hay ninguno"*.
#[derive(Debug)]
pub struct Unreadable {
    /// Relativo a la capa: es lo que se imprime.
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug)]
pub struct CheckResult {
    pub uuid: String,
    pub state0: EndpointState,
    pub state1: EndpointState,
    /// Las dimensiones que califican al estado de cada endpoint: las que no están
    /// `OK`. Vacío sin dimensiones, o con todas limpias.
    pub dimensions: [DimensionStates; 2],
}

impl CheckResult {
    /// Los dos endpoints en OK. **Decide qué se imprime.**
    pub fn all_ok(&self) -> bool { self.state0.is_ok() && self.state1.is_ok() }

    /// Nada que exija una decisión humana. **Decide el código de salida.**
    pub fn is_clean(&self) -> bool { self.state0.is_clean() && self.state1.is_clean() }
}

/// Verifica una capa y deja el resultado en la cache.
pub fn check(root: &Path, path: &Path) -> Result<Checked> {
    check_with(root, path, None)
}

/// Como [`check`], con quien resuelva el vecindario de las firmas.
///
/// **Sin proveedor no le pregunta a nadie**, y lo que no confirma es
/// `OK_N1_UNCONFIRMED`. **Con proveedor y nivel 1 adquirido, el proveedor tiene que
/// estar**: si no, falla antes de verificar nada. Y si falla en el medio, lo que se
/// alcanzó a verificar queda en la cache.
pub fn check_with(
    root: &Path, path: &Path, nb: crate::neighbours::Provider<'_>,
) -> Result<Checked> {
    let layer = if path.join(".bilink").is_dir() { path.to_path_buf() } else { root.to_path_buf() };
    let scope = Scope::of(&layer, path)?;

    // **La versión de la capa se compara antes de abrir un bilink.** Un archivo de
    // formato viejo puede parsear bien y significar otra cosa, así que deducirlo del
    // parseo no alcanza: la versión es el único dato que discrimina en esa dirección.
    //
    // Y se pregunta **cuando hay archivos del formato**, que es lo que hace que la
    // versión importe. Sin `.bilink/`, o con uno que sólo tiene la cache y el
    // `.gitignore`, no hay nada que se pueda leer con el parser equivocado: `0
    // bilink(s)` es cierto, y negarse ahí volvería `check` inusable fuera de una capa
    // y adentro de una recién declarada. Con archivos y sin `version` sí hay algo, y
    // es formato 1.
    if bilink_format::has_format_files(&layer) {
        bilink_format::ensure_readable(&layer)?;
    }
    // **La cache se invalida sola al cambiar de rama.** Sin esto una capa devuelve
    // estados de la rama anterior en silencio: `git checkout` no toca `.bilink/`, y
    // los estados cacheados describen bilinks que ya no están en el árbol.
    let ref_commit = Cache::ref_commit_of(&layer);
    let mut cache = Cache::load_for(&layer, ref_commit.as_deref());
    cache.ref_commit = ref_commit;
    let mut out = Vec::new();
    let mut unreadable = Vec::new();

    // Un mismo capture se resuelve **una sola vez**, aunque lo referencien varios
    // endpoints. La comparación contra `accepted` sí corre por endpoint, porque
    // cada uno tiene el suyo.
    let mut resolved: HashMap<String, (CaptureState, Option<Ranges>)> = HashMap::new();

    // **Primero se lee todo, y se sabe cuánto nivel 1 hay.** Sin proveedor que conteste
    // no se verifica nada: a medio camino, lo que no tiene nivel 1 saldría verde y lo
    // que sí, sin terminar.
    let mut bilinks: Vec<(String, BiLink)> = Vec::new();
    for path in bilink_files(&layer.join(".bilink")) {
        if let Scope::Bilink(only) = &scope {
            if path.file_stem() != Some(only.as_os_str()) { continue; }
        }
        // Se saltea para no abortar el recorrido de los demás —igual que un
        // directorio que no se puede leer— pero **se cuenta y se nombra**: un archivo
        // roto no es razón para dejar de decir lo que se sabe del resto, ni para
        // dejar de decir que está roto.
        let bl = match BiLink::load(&path) {
            Ok(bl) => bl,
            Err(e) => {
                unreadable.push(Unreadable {
                    path: path.strip_prefix(&layer).unwrap_or(&path).to_path_buf(),
                    error: e.root_cause().to_string(),
                });
                continue;
            }
        };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if let Scope::Under(dir) = &scope {
            if !scope_covers(&layer, &bl, dir) { continue; }
        }
        bilinks.push((uuid.to_string(), bl));
    }

    let n1 = n1_demand(&layer, &bilinks);
    crate::neighbours::require(nb, &layer, n1.clone())?;

    for (uuid, bl) in &bilinks {
        let uuid = uuid.as_str();
        let mut states = [EndpointState::Pending; 2];
        let mut dimensions: [DimensionStates; 2] = Default::default();
        for n in [0u8, 1u8] {
            let (state, dims) = match check_endpoint(&layer, bl, uuid, n, &mut resolved, &mut cache, nb) {
                Ok(s) => s,
                // **Lo que se alcanzó a verificar queda**, como en cualquier check
                // parcial: el resto de la capa conserva lo que tenía.
                Err(e) => {
                    for (id, (state, range)) in &resolved {
                        cache.set_capture(id, *state, range.as_ref());
                    }
                    cache.save(&layer)?;
                    return Err(e);
                }
            };
            states[n as usize] = state;
            cache.set_endpoint_state(uuid, n, states[n as usize]);
            cache.set_endpoint_dimensions(uuid, n, &dims);
            dimensions[n as usize] = dims;
            // El alias sale del mismo trabajo: resolver el capture es lo que `check`
            // ya hizo para escribir `range`. `None` **borra** el que hubiera — un
            // `as` que se sacó no puede dejar el rótulo viejo colgado.
            let alias = alias_de(&layer, bl, n, &resolved);
            cache.set_alias(uuid, n, alias);
        }
        out.push(CheckResult { uuid: uuid.to_string(), state0: states[0], state1: states[1], dimensions });
    }

    for (id, (state, range)) in &resolved {
        cache.set_capture(id, *state, range.as_ref());
    }
    cache.save(&layer)?;
    out.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    Ok(Checked { results: out, unreadable, n1 })
}

/// Los endpoints con un nivel 1 adquirido y una sola decisión: los que `check` le
/// pregunta al proveedor.
///
/// Sobre-cuenta a propósito los que no van a llegar a preguntar —un fragmento que
/// cambió no mira su vecindario—: saberlo pide verificar, y esto va antes.
fn n1_demand(layer: &Path, bilinks: &[(String, BiLink)]) -> crate::neighbours::Demand {
    let mut d = crate::neighbours::Demand::default();
    for (_, bl) in bilinks {
        for n in [0u8, 1u8] {
            let e = bl.endpoint.get(n);
            let [accepted] = e.accepted.as_slice() else { continue };
            if accepted.n.as_ref().and_then(|x| x.level(1)).is_none() { continue }
            let Some(id) = e.link.capture_id() else { continue };
            let Ok(cap) = Capture::load_in(layer, id) else { continue };
            d.add(&cap.file);
        }
    }
    d
}

/// Qué parte de la capa verifica un `check <path>`.
enum Scope {
    Layer,
    /// Un bilink, por su UUID.
    Bilink(std::ffi::OsString),
    /// Los bilinks con un endpoint cuyo capture cae bajo este path, relativo a la capa.
    Under(PathBuf),
}

impl Scope {
    fn of(layer: &Path, path: &Path) -> Result<Scope> {
        if path.join(".bilink").is_dir() { return Ok(Scope::Layer); }
        // **Un path que no existe es un error, no un alcance vacío.** Con un typo,
        // "no hay nada no-OK" se leería como que todo está bien.
        let Ok(abs) = path.canonicalize() else {
            anyhow::bail!("{} no existe", path.display());
        };
        let layer = layer.canonicalize()?;
        let Ok(rel) = abs.strip_prefix(&layer) else {
            anyhow::bail!("{} no está adentro de la capa {}", path.display(), layer.display());
        };
        if rel.as_os_str().is_empty() { return Ok(Scope::Layer); }
        if rel.parent() == Some(Path::new(".bilink")) && rel.extension().is_some_and(|e| e == "yaml") {
            return Ok(Scope::Bilink(rel.file_stem().unwrap_or_default().to_os_string()));
        }
        Ok(Scope::Under(rel.to_path_buf()))
    }
}

/// ¿Algún endpoint de `bl` apunta a un capture bajo `dir`?
///
/// Cuenta el `link`, la ubicación vigente: un vecino del vecindario no mete a su
/// bilink en el alcance, porque el archivo de un DTO no es el fragmento de nadie.
fn scope_covers(layer: &Path, bl: &BiLink, dir: &Path) -> bool {
    [0u8, 1u8].iter().any(|&n| {
        bl.endpoint.get(n).link.capture_id()
            .and_then(|id| Capture::load_in(layer, id).ok())
            .is_some_and(|cap| Path::new(&cap.file).starts_with(dir))
    })
}

/// Cómo se llama este endpoint, si su generador sabe nombrarlo.
///
/// **Sin `as` no hay alias**, y eso no es una falla: es lo que dice todo bilink
/// escrito antes de que el campo existiera. Un `as` que nombra un generador que no
/// está instalado tampoco falla — es un dato que no se pudo usar.
fn alias_de(
    layer: &Path,
    bl: &BiLink,
    n: u8,
    resolved: &HashMap<String, (CaptureState, Option<Ranges>)>,
) -> Option<String> {
    let e = bl.endpoint.get(n);
    let g = crate::capture::generator_named(e.r#as.as_deref()?).ok()?;
    let cap = crate::capture::capture_of(layer, &e.link).ok()??;
    let (_, ranges) = resolved.get(e.link.capture_id()?)?;
    let source = std::fs::read_to_string(layer.join(&cap.file)).ok()?;
    g.alias(&source, ranges.as_ref()?, cap.query.as_deref()?)
}

fn check_endpoint(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    resolved: &mut HashMap<String, (CaptureState, Option<Ranges>)>,
    cache: &mut Cache,
    nb: crate::neighbours::Provider<'_>,
) -> Result<(EndpointState, DimensionStates)> {
    let e = bl.endpoint.get(n);
    let word = |s: Result<EndpointState>| s.map(|s| (s, Vec::new()));
    match &e.link {
        LinkEndpoint::Path(p)   => word(check_path(layer, p, uuid, e.accepted.first())),
        LinkEndpoint::Issue(id) => word(check_issue(layer, id, e.accepted.first())),
        LinkEndpoint::Repo(alias) => word(check_repo(layer, alias, uuid, e.accepted.first())),
        // Constante: no hay contra qué comparar. Nunca pide acción.
        LinkEndpoint::Abstract  => Ok((EndpointState::Open, Vec::new())),
        LinkEndpoint::Capture(cap_id) => {
            let cap = match Capture::load_in(layer, cap_id) {
                Ok(c) => c,
                // El capture que el link nombra no está: no hay ubicación que evaluar.
                Err(_) => return Ok((EndpointState::Unresolved, Vec::new())),
            };

            // La resolución se cachea por capture, pero la aceptación es por
            // endpoint: dos endpoints sobre el mismo capture pueden haber aprobado
            // contenidos distintos, y el que resuelva primero es el que aporta el
            // texto para puntuar un reanclaje. Es una aproximación consciente —
            // resolver una vez por capture es lo que la spec pide— y sólo afecta a
            // qué candidato gana en un caso ya ambiguo.
            let (state, range) = match resolved.get(cap_id) {
                Some(v) => v.clone(),
                None => {
                    let v = resolve_capture(layer, &cap, e.accepted.first(), cache.commit(uuid, n))?;
                    resolved.insert(cap_id.clone(), v.clone());
                    v
                }
            };
            if !state.is_resolved() {
                return Ok((EndpointState::Unresolved, Vec::new()));
            }

            // **La lista decide antes que cualquier eje.**
            //
            // Vacía es `PENDING`. Con más de una entrada hay dos decisiones humanas
            // incompatibles, y **no hay un valor contra el cual comparar**: evaluar
            // ubicación o contenido exigiría elegir una, que es exactamente lo que
            // nadie hizo. Así que se reporta el desacuerdo y se corta.
            let accepted = match e.accepted.as_slice() {
                []      => return Ok((EndpointState::Pending, Vec::new())),
                [one]   => one,
                [_, ..] => return Ok((EndpointState::ConsensusDiverged, Vec::new())),
            };

            // ── dimensión de ubicación ────────────────────────────────────────
            //
            // Dos ids: no abre ningún archivo. Por eso se decide **siempre**, incluso
            // donde la otra dimensión degrada por no poder recuperar el texto aceptado.
            if accepted.link.as_ref() != Some(&e.link) {
                return Ok((EndpointState::Relocated, Vec::new()));
            }

            // ── dimensión de contenido ────────────────────────────────────────
            //
            // El commit se deriva si la cache no lo tiene. Sin él, `accepted.hash`
            // es un hash que no se puede resolver a texto, y sin el texto aceptado
            // EXPANDED, DISPLACED y REANCHORED degradan todos a ALTERED — o sea,
            // un clon fresco perdería las tres distinciones.
            let cached_commit = cache.commit(uuid, n).map(str::to_string);
            let mut derived: Option<Option<String>> = None;
            let state = {
                let mut derive = || -> Option<String> {
                    derived
                        .get_or_insert_with(|| match &cached_commit {
                            Some(c) => Some(c.clone()),
                            None => crate::capture::derive_commit(layer, &cap, &accepted.hash),
                        })
                        .clone()
                };
                let mut src = CommitSource { derive: &mut derive };
                // **Con dimensiones, el estado sale de ellas**, y el fragmento entero
                // deja de decidir: lo que cambia afuera de toda parte no lo pidió
                // vigilar nadie. La palabra es la de la parte más severa, y las que
                // no están OK van al lado.
                if e.dimensions.is_empty() && accepted.dimensions.is_empty() {
                    (compare_content(layer, &cap, accepted, range.as_ref(), &mut src)?, Vec::new())
                } else {
                    let Some(r) = range.as_ref() else {
                        return Ok((EndpointState::Unresolved, Vec::new()));
                    };
                    let dims = dimension::compare(layer, &cap, &e.dimensions, &accepted.dimensions, r, &mut src)?;
                    (dimension::word(&dims), dimension::qualifying(dims))
                }
            };
            let (state, dims) = state;
            // Lo derivado se guarda: el walk cuesta un `git show` por commit y el
            // mismo endpoint se consulta más de una vez en una corrida.
            if let Some(Some(c)) = &derived {
                if cached_commit.is_none() { cache.set_commit(uuid, n, c); }
            }

            // ── el eje del vecindario ─────────────────────────────────────────
            //
            // **Sólo si el del contenido dice OK.** Un endpoint tiene un estado y no
            // dos: si el fragmento mismo cambió, eso se reporta y alguien va a mirar
            // igual. Lo que este eje aporta es el caso donde el fragmento no cambió
            // y aun así el contrato se movió.
            if state == EndpointState::Ok {
                if let Some(s) = compare_contract(layer, &cap, e.n.as_ref(), accepted, range.as_ref(), nb)? {
                    return Ok((s, Vec::new()));
                }
            }
            Ok((state, dims))
        }
    }
}

// ─── el eje del vecindario ────────────────────────────────────────────────────

/// Qué dicen hoy los tipos que la firma menciona, contra lo que se aprobó.
///
/// `Ok(None)` es *"nada que decir"*: sin vecindario aceptado —la inmensa mayoría— o
/// con todo confirmado.
///
/// **Lo probado le gana a lo sospechado.** Primero el contenido de los vecinos, con
/// sus captures; después la ubicación, declarada y la de hoy; y por último si alguien
/// confirmó que los nombres de la firma siguen resolviendo a esos vecinos.
fn compare_contract(
    layer:    &Path,
    cap:      &Capture,
    declared: Option<&bilink_format::DeclaredN>,
    accepted: &bilink_format::Accepted,
    range:    Option<&Ranges>,
    nb:       crate::neighbours::Provider<'_>,
) -> Result<Option<EndpointState>> {
    // Sólo un vecindario **adquirido** se compara: una renuncia no tiene con qué,
    // y la ausencia significa que el fragmento no tiene firma resoluble.
    let Some(expected) = accepted.n.as_ref().and_then(|n| n.level(1)) else { return Ok(None) };

    // ── contenido, con los captures de los vecinos ────────────────────────────
    //
    // **No le pregunta a nadie**: los captures se resuelven con tree-sitter y se
    // pliegan con el mismo fold que calculó `accept`. Un cambio acá es drift probado.
    // Un capture que ya no resuelve es la declaración aceptada que no está donde
    // estaba, y eso es un cambio, no una ausencia.
    if let Some(ids) = expected.link.known_ids() {
        let Some(hoy) = crate::neighbours::fold_captures(layer, ids)? else {
            return Ok(Some(EndpointState::ContractAltered));
        };
        if let Some(s) = contract_content(&hoy.n, expected) { return Ok(Some(s)) }
    }

    // ── ubicación declarada ───────────────────────────────────────────────────
    //
    // **Dos listas de ids.** Separa cuatro casos que el fold solo no distingue: un
    // vecino que entró, uno que salió, uno que se mudó de archivo y uno que se
    // renombró. Con `unknown` de cualquiera de los dos lados no hay ids que comparar,
    // y no poder compararlos no es que coincidan: eso es `ContractUnlocated`.
    let declarado = declared.and_then(|d| d.level(1)).map(|l| &l.link);
    let sin_ubicacion = declarado.is_some_and(|l| l.is_unknown()) || expected.link.is_unknown();
    if !sin_ubicacion {
        let hoy = declarado.and_then(|l| l.known_ids()).unwrap_or(&[]);
        if hoy != expected.link.known_ids().unwrap_or(&[]) {
            return Ok(Some(EndpointState::ContractRelocated));
        }
    }

    // ── resolución de los nombres ─────────────────────────────────────────────
    //
    // Lo único que los captures no pueden decir: que la firma siga nombrando a esos
    // vecinos. Depende de los imports y del build, y lo contesta el proveedor.
    let Some(p) = nb else {
        return Ok(Some(if sin_ubicacion {
            EndpointState::ContractUnlocated
        } else {
            EndpointState::OkN1Unconfirmed
        }));
    };
    let Some(range) = range else { anyhow::bail!("{}: el fragmento está OK y no tiene rango", cap.file) };
    let locs = match crate::neighbours::reach(layer, &cap.file, range) {
        crate::neighbours::Reach::At(at) => crate::neighbours::ask(p, layer, &cap.file, &at)?,
        // La firma no menciona tipos: el conjunto de hoy es vacío, sin preguntar.
        crate::neighbours::Reach::None => Vec::new(),
        crate::neighbours::Reach::Unreachable { what } => anyhow::bail!(
            "{}: el fragmento {what}, y tiene un nivel 1 aceptado que no se puede volver a \
             preguntar", cap.file),
    };
    // Un vecino de hoy que no se puede capturar no es ninguno de los aceptados.
    let Some(hoy) = crate::neighbours::fold(layer, &locs)? else {
        return Ok(Some(EndpointState::ContractRelocated));
    };

    // Sin ubicación, el `hash` conservado es lo único con qué comparar el conjunto de
    // hoy, y un cambio real de contrato le gana a la ubicación faltante.
    if sin_ubicacion {
        return Ok(Some(contract_content(&hoy.n, expected).unwrap_or(EndpointState::ContractUnlocated)));
    }
    let ids_hoy = hoy.n.link.known_ids().unwrap_or(&[]);
    if ids_hoy != expected.link.known_ids().unwrap_or(&[]) {
        return Ok(Some(EndpointState::ContractRelocated));
    }
    Ok(None)
}

/// El contenido de un vecindario contra el aceptado: `None` si coincide.
fn contract_content(
    hoy: &bilink_format::Neighbourhood,
    expected: &bilink_format::Neighbourhood,
) -> Option<EndpointState> {
    if hoy.hash == expected.hash { return None }
    // Sólo formato en el vecindario: el texto difiere y las s-expressions no. La
    // pregunta sólo se hace donde los dos lados tienen `hash_ast`.
    if let (Some(a), Some(b)) = (&hoy.hash_ast, &expected.hash_ast) {
        if a == b { return Some(EndpointState::ContractRestyled) }
    }
    Some(EndpointState::ContractAltered)
}

// ─── dimensión 1: ¿dónde está? ────────────────────────────────────────────────

/// Resuelve un capture contra el árbol actual.
///
/// Recibe `accepted` porque **REANCHORED lo necesita**: para decidir si un nodo con
/// otro nombre es el mismo fragmento hay que compararlo contra el texto aceptado, y
/// ese texto se recupera de git con `(hash, commit)`. Sin eso el anchor renombrado
/// se reporta como UNANCHORED —"no está"— en vez de "está, con otro nombre".
///
/// Es la única cosa de la aceptación que la dimensión de ubicación mira, y sólo para
/// puntuar: el estado que devuelve sigue siendo sobre dónde está el fragmento.
pub(crate) fn resolve_capture(
    layer: &Path,
    cap: &Capture,
    accepted: Option<&bilink_format::Accepted>,
    commit: Option<&str>,
) -> Result<(CaptureState, Option<Ranges>)> {
    let path = layer.join(&cap.file);

    if !path.exists() {
        if git_renamed_to(layer, &cap.file).is_some() {
            return Ok((CaptureState::Moved, None));
        }
        if git_knows_file(layer, &cap.file) {
            return Ok((CaptureState::Deleted, None));
        }
        return Ok((CaptureState::Broken, None));
    }

    let Ok(source) = std::fs::read_to_string(&path) else {
        return Ok((CaptureState::Broken, None));
    };

    // Sin query, el capture es el archivo entero.
    let Some(query_str) = &cap.query else {
        return Ok((CaptureState::Resolved, Some(Ranges::one(0, source.len()))));
    };

    let lang     = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;

    let Some(fragment) = query::find_fragment(language.clone(), &source, query_str)?
    else {
        // La query no matchea: ¿el anchor se renombró, o el fragmento desapareció?
        let hash = accepted.map(|a| a.hash.as_str());
        if find_renamed_anchor(layer, language, &source, query_str, cap, hash, commit)?.is_some() {
            return Ok((CaptureState::Reanchored, None));
        }
        if git_fragment_vanished(layer, &cap.file, hash) {
            return Ok((CaptureState::Deleted, None));
        }
        return Ok((CaptureState::Unanchored, None));
    };

    Ok((CaptureState::Resolved, Some(fragment.ranges)))
}

// ─── dimensión 2: ¿coincide con lo aceptado? ──────────────────────────────────

/// De dónde sale el commit del contenido aceptado, cuando hace falta.
///
/// Es **perezoso a propósito**: derivarlo antes de saber si hace falta lo cobraría
/// también sobre los endpoints OK, que son los que nunca lo necesitan —el hash
/// decide antes—, y el costo dejaría de estar acotado por lo que está roto.
///
/// Adentro puede salir de la cache o de un walk por la historia; a quien compara no
/// le cambia nada, y por eso ya no son dos campos. Lo fueron mientras el cacheado
/// habilitaba un fast-path, que se sacó por infundado.
pub(crate) struct CommitSource<'a> {
    /// Se llama a lo sumo una vez, y sólo después de que el hash dijo que hay drift.
    pub derive: &'a mut dyn FnMut() -> Option<String>,
}

/// Qué dice el fragmento contra lo que se aprobó.
///
/// **No hay fast-path, y no puede haberlo por el camino que parecía obvio.** Hubo
/// uno: si el archivo no cambió desde el commit donde vivía el contenido aceptado,
/// conservar el `OK` cacheado sin volver a hashear. Su premisa es un *proxy* —"¿el
/// archivo cambió?"— de la pregunta real —"¿el fragmento sigue hasheando a lo
/// aceptado?"—, y las dos dejan de coincidir apenas cambia **cómo se resuelve el
/// rango**: el mismo archivo produce otro fragmento, y el proxy no se entera nunca.
///
/// Pasó, y quedó escondido. La migración `18` cambió los bordes del rango; los
/// endpoints aceptados antes quedaron con un `accepted.hash` que ya no coincide, y
/// el fast-path los reportó `OK` durante toda una sesión — con `accept` creyéndole
/// y no aceptando nada, que es la falla que `cache.md` llama *"una decisión
/// perdida"*.
///
/// Y no compraba nada: el `git diff` que corría cuesta lo mismo que leer el archivo
/// y hashearlo, porque es un subproceso contra una lectura. Se pagaba un proceso
/// para ahorrarse un `read`.
pub(crate) fn compare_content(
    layer: &Path,
    cap: &Capture,
    accepted: &bilink_format::Accepted,
    range: Option<&Ranges>,
    commit: &mut CommitSource<'_>,
) -> Result<EndpointState> {
    let source = std::fs::read_to_string(layer.join(&cap.file))?;
    let Some(r) = range else { return Ok(EndpointState::Unresolved) };
    let fragment = r.text(&source);
    let fragment = fragment.as_str();

    if hash::sha256(fragment.as_bytes()) == accepted.hash {
        return Ok(EndpointState::Ok);
    }

    // El texto aceptado, recuperado de git y verificado contra `accepted.hash`. Con
    // él, la frontera entre EXPANDED y DISPLACED es un test de subcadena y no un
    // umbral:
    //
    //   fragmento ⊃ aceptado          → creció alrededor        → EXPANDED
    //   fragmento ⊅ aceptado, nodo sí → se corrió, sigue igual  → DISPLACED
    let text = (commit.derive)()
        .and_then(|c| crate::capture::accepted_text(layer, cap, &c, Some(&accepted.hash)));

    if let Some(t) = text.as_deref() {
        if !t.is_empty() && fragment.len() > t.len() && fragment.contains(t) {
            return Ok(EndpointState::Expanded);
        }
    }

    // Sólo formato: el texto difiere y el AST no.
    //
    // La pregunta la decide la gramática, no el archivo: donde el AST no
    // discrimina contenido —prosa— el sexp de una sección es el mismo con
    // cualquier texto adentro, y compararlo diría RESTYLED de una reescritura
    // entera. Se consulta la gramática antes que `accepted`, así que un
    // `hash_ast` guardado por una versión anterior queda inerte en vez de mentir.
    let lang = grammar::language_for_file(&cap.file);
    if grammar::ast_discriminates_content(lang) {
        if let (Some(expected_ast), Some(q)) = (&accepted.hash_ast, &cap.query) {
            let language = grammar::for_language(lang)?;
            if let Some(f) = query::find_fragment(language, &source, q)? {
                if hash::sha256(f.sexp.as_bytes()) == *expected_ast {
                    return Ok(EndpointState::Restyled);
                }
            }
        }
    }

    Ok(EndpointState::Altered)
}

// ─── endpoints que no son estructurales ───────────────────────────────────────

/// Un endpoint `path` copia los **dos** valores aceptados de su vecino.
fn check_path(
    layer: &Path,
    p: &bilink_format::link::StratumPath,
    uuid: &str,
    accepted: Option<&bilink_format::Accepted>,
) -> Result<EndpointState> {
    let Ok(target) = stratum::resolve(layer, layer, p.tokens()) else {
        return Ok(absent_layer(layer, None, accepted));
    };
    let dir = layer.join(&target);
    if !dir.is_dir() {
        return Ok(absent_layer(layer, Some(&target), accepted));
    }

    // La capa está y el bilink del uuid no: **es una regresión**, no una ausencia.
    let adj_path = dir.join(".bilink").join(format!("{uuid}.yaml"));
    if !adj_path.exists() {
        return Ok(if accepted.is_none() { EndpointState::Todo } else { EndpointState::Broken });
    }

    let Ok(adj) = BiLink::load(&adj_path) else { return Ok(EndpointState::Broken) };
    let Some(adj_accepted) = adj.structural_accepted() else {
        // El vecino existe y nunca se aceptó: no hay contra qué comparar.
        return Ok(EndpointState::Pending);
    };
    let Some(mine) = accepted else { return Ok(EndpointState::Pending) };

    // Los dos valores, no uno: la ubicación aprobada del vecino y su contenido.
    let same = mine.hash == adj_accepted.hash && mine.link == adj_accepted.link;
    Ok(if same { EndpointState::Ok } else { EndpointState::ChainDirty })
}

/// Las tres ausencias de una capa, que se arreglan distinto y por eso no comparten
/// nombre.
///
/// Un solo `UNREACHABLE` no distinguía *"me falta traer algo"* de *"algo se rompió"*,
/// que es la diferencia que decide si alguien tiene que mirar. Y separar en dos no
/// alcanza: a una capa **declarada** le falta traerla, a una sin declarar le falta
/// declararla, y las dos se arreglan con comandos distintos.
fn absent_layer(
    layer: &Path,
    target: Option<&Path>,
    accepted: Option<&bilink_format::Accepted>,
) -> EndpointState {
    let declared = target
        .and_then(|t| t.file_name())
        .map(|name| {
            let parent = target.and_then(|t| t.parent()).unwrap_or(Path::new(""));
            layer.join(parent).join(format!(".{}.toml", name.to_string_lossy())).exists()
        })
        .unwrap_or(false);

    match (declared, accepted.is_some()) {
        // Declarada y ausente: falta traerla, y eso es normal.
        (true, _)      => EndpointState::LayerUnreachable,
        // Ni declarada ni presente, con aceptación previa: falta la declaración.
        (false, true)  => EndpointState::LayerUnconfigured,
        // Sin aceptación previa es una intención, no una ausencia.
        (false, false) => EndpointState::Todo,
    }
}

/// Un endpoint repo: el `path` con la dirección resuelta por alias, y **sin red**.
///
/// Se leen dos cosas del proveedor y son dos hechos distintos: si su punta sigue
/// siendo `abstract`, y qué aceptó. Mezclarlos en el mismo token perdería cuál de
/// los dos pasó.
fn check_repo(
    layer: &Path,
    alias: &str,
    uuid: &str,
    accepted: Option<&bilink_format::Accepted>,
) -> Result<EndpointState> {
    use crate::frontier::Resolution;

    // El paso de la versión **no devuelve un estado**: no poder leer los archivos no
    // es drift, y reportar cualquier estado sobre eso sería inventar.
    match crate::frontier::resolve(layer, alias, uuid)? {
        Resolution::NotCloned  => Ok(EndpointState::RemoteUnreachable),
        Resolution::BilinkGone => Ok(EndpointState::Broken),
        Resolution::Found(view) => {
            if !view.still_abstract {
                return Ok(EndpointState::Rejected);
            }
            let Some(theirs) = view.accepted else {
                // El proveedor nunca aceptó lo que publica.
                return Ok(EndpointState::Pending);
            };
            let Some(mine) = accepted else { return Ok(EndpointState::Pending) };

            let same = mine.hash == theirs.hash && mine.link == theirs.link;
            Ok(if same { EndpointState::Ok } else { EndpointState::ChainDirty })
        }
    }
}

/// Un endpoint `issue` se hashea como el contenido del archivo del ítem.
fn check_issue(layer: &Path, id: &str, accepted: Option<&bilink_format::Accepted>) -> Result<EndpointState> {
    let (item, _) = crate::issue::resolve_issue_path(layer, id)?;
    let Some(item) = item else {
        return Ok(if accepted.is_none() { EndpointState::Todo } else { EndpointState::Broken });
    };
    let Some(accepted) = accepted else { return Ok(EndpointState::Pending) };
    let Ok(text) = std::fs::read_to_string(&item) else { return Ok(EndpointState::Broken) };

    Ok(if hash::sha256(text.as_bytes()) == accepted.hash {
        EndpointState::Ok
    } else {
        EndpointState::Altered
    })
}

// ─── git ──────────────────────────────────────────────────────────────────────

/// Nueva ruta del archivo si git detecta un rename (≥ 50% de similitud).
///
/// Sin pathspec: filtrar por el path viejo puede impedir que git detecte el
/// rename, porque el destino queda fuera del filtro.
pub(crate) fn git_renamed_to(layer_root: &Path, file: &str) -> Option<String> {
    for args in [
        &["diff", "-M", "--name-status", "HEAD"][..],
        &["diff", "-M", "--name-status", "--cached"][..],
    ] {
        let out = std::process::Command::new("git")
            .args(["-C", &layer_root.to_string_lossy()])
            .args(args)
            .output()
            .ok()?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if !line.starts_with('R') { continue; }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() == 3 && parts[1] == file && layer_root.join(parts[2]).exists() {
                return Some(parts[2].to_string());
            }
        }
    }
    None
}

/// ¿Git tiene historial de este archivo?
///
/// Distingue "el archivo se borró" de "esta referencia nunca apuntó a nada".
fn git_knows_file(layer_root: &Path, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &layer_root.to_string_lossy(), "log", "--oneline", "-1", "--", file])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Umbral de similitud para dar por reanclado un fragmento.
///
/// Es el mismo 50% que usa `git diff -M` para renames de archivos: la analogía
/// es exacta —encontrar a dónde se fue algo que cambió de nombre— y usar el
/// mismo número evita dos criterios distintos para la misma pregunta.
const REANCHOR_THRESHOLD: f64 = 0.5;

/// Margen mínimo sobre el segundo candidato.
///
/// Sin esto, un archivo con varias funciones de forma parecida produciría un
/// REANCHORED arbitrario. Ante un empate es preferible UNANCHORED: que un humano
/// mire es mejor que reanclar al nodo equivocado.
const REANCHOR_MARGIN: f64 = 0.15;

/// Busca a dónde se fue un fragmento cuyo anchor cambió de nombre.
///
/// No compara hashes: `hash.N` es exacto, y renombrar un anchor casi siempre
/// cambia el fragmento —el nombre suele estar *dentro* de lo capturado—, así que
/// una comparación exacta no dispararía nunca. En su lugar recupera el texto
/// aceptado desde git (`commit.N` + el range guardado, igual que `get --diff`) y
/// puntúa cada candidato por similitud.
pub(crate) fn find_renamed_anchor(
    root:      &Path,
    language:  tree_sitter::Language,
    source:    &str,
    query_str: &str,
    cap:       &Capture,
    hash:      Option<&str>,
    commit:    Option<&str>,
) -> Result<Option<(String, f64)>> {
    let Some(old_text) = commit.and_then(|c| crate::capture::accepted_text(root, cap, c, hash)) else {
        return Ok(None);
    };

    let relaxed = query::relax_name_predicates(query_str);
    let Ok(matches) = query::find_all_targets(language, source, &relaxed) else {
        return Ok(None); // la query relajada puede no ser válida; no es un error
    };

    let mut scored: Vec<(String, f64)> = Vec::new();
    for m in matches {
        let Some(name) = m.name.clone() else { continue };
        if m.fragment.ranges.end() > source.len() { continue; }
        scored.push((name, hash::similarity(&old_text, &m.fragment.ranges.text(source))));
    }

    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let Some((name, best)) = scored.first().cloned() else { return Ok(None) };
    if best < REANCHOR_THRESHOLD { return Ok(None); }

    let second = scored.get(1).map(|(_, s)| *s).unwrap_or(0.0);
    if best - second < REANCHOR_MARGIN { return Ok(None); }

    Ok(Some((name, best)))
}

/// ¿El fragmento aceptado existió alguna vez en el historial de este archivo?
///
/// `git log -S` busca commits que agreguen o quiten esa cadena. Si aparece,
/// hubo un commit que se llevó el fragmento — eso es DELETED, rastreable. Si no
/// aparece nunca, la referencia nunca ancló a algo que git haya visto.
fn git_fragment_vanished(layer_root: &Path, file: &str, hash: Option<&str>) -> bool {
    let Some(hash) = hash else { return false };
    std::process::Command::new("git")
        .args(["-C", &layer_root.to_string_lossy(), "log", "--oneline", "-1",
               "-S", hash, "--", file])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Finds all bilinks referencing `file_path` across all layers under `root`.
/// Returns `(bilink_path, endpoint_index, absolute_range)`.
/// Uses `.bilink/.index` per layer when valid; falls back to O(N) scan.
/// Los endpoints que referencian un archivo y tienen rango en la cache.
///
/// El rango es un derivado: con la cache fría no está, y el endpoint no sale.
/// Quien necesite saber que faltan usa [`find_by_file_unranged`].
pub fn find_by_file(root: &Path, file_path: &Path) -> Result<Vec<(PathBuf, u8, Ranges)>> {
    Ok(find_by_file_unranged(root, file_path)?.into_iter()
        .filter_map(|(path, n, range)| Some((path, n, range?)))
        .collect())
}

/// Los endpoints que referencian un archivo, con el rango que la cache tenga.
///
/// Qué endpoints referencian el archivo lo dicen los bilinks, y el rango es un
/// derivado: con la cache fría el endpoint sale igual, sin rango, en vez de
/// desaparecer. Quien necesite el rango corre `check` primero.
pub fn find_by_file_unranged(root: &Path, file_path: &Path) -> Result<Vec<(PathBuf, u8, Option<Ranges>)>> {
    let mut results = Vec::new();
    for layer_root in crate::index::layer_roots(root) {
        let Ok(rel) = file_path.strip_prefix(&layer_root) else { continue };
        let Some(rel_str) = rel.to_str() else { continue };

        let cache = Cache::load(&layer_root);
        let bilink_dir = layer_root.join(".bilink");

        for (uuid, n) in crate::index::lookup(&layer_root, rel_str)? {
            let bilink_path = bilink_dir.join(format!("{uuid}.yaml"));
            let Ok(bl) = BiLink::load(&bilink_path) else { continue };
            let Some(id) = bl.endpoint.get(n).link.capture_id() else { continue };
            results.push((bilink_path, n, cache.capture_ranges(id)));
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bilink_format::Accepted;
    use tempfile::tempdir;

    const QUERY: &str = r#"(section (atx_heading (inline) @n0 (#eq? @n0 "Spec"))) @target"#;

    fn sexp_hash(source: &str, query: &str) -> String {
        let language = grammar::for_language("markdown").unwrap();
        let f = query::find_fragment(language, source, query)
            .unwrap()
            .expect("la query debería resolver");
        hash::sha256(f.sexp.as_bytes())
    }

    /// Sobre prosa, un `hash_ast` guardado no se consulta aunque coincida.
    ///
    /// Una versión anterior lo escribía también para markdown. La gramática se
    /// consulta antes que `accepted`, así que el residuo queda inerte: acá el
    /// hash es el del contenido actual —coincide exacto— y aun así el estado es
    /// ALTERED, porque en prosa la pregunta no se hace.
    #[test]
    fn a_stored_ast_hash_over_prose_is_never_consulted() {
        let d = tempdir().unwrap();
        let file = "spec.md";
        let before = "# Spec\n\nLo que decía antes.\n";
        let after  = "# Spec\n\nOtra cosa completamente distinta.\n";
        std::fs::write(d.path().join(file), after).unwrap();

        let cap = Capture { file: file.into(), query: Some(QUERY.into()) };
        let accepted = Accepted {
            agree: Default::default(),
            link: None,
            hash: hash::sha256(before.as_bytes()),
            hash_ast: Some(sexp_hash(after, QUERY)),   // coincidiría, si se mirara
            n: None,
            dimensions: Default::default(),
        };
        let range = Ranges::one(0, after.len());

        let mut derive = || None;
        let mut src = CommitSource { derive: &mut derive };
        let state = compare_content(d.path(), &cap, &accepted, Some(&range), &mut src).unwrap();
        assert_eq!(state, EndpointState::Altered,
                   "en prosa el AST no discrimina contenido: no hay RESTYLED que dar");
    }

    // ─── lo que no se puede leer ──────────────────────────────────────────────

    /// **Una capa de otro major no se verifica: se rechaza.**
    ///
    /// Un archivo de formato viejo puede parsear bien y significar otra cosa, así que
    /// leerlo y reportar estados sería inventar.
    #[test]
    fn a_layer_of_another_major_is_refused_before_reading_anything() {
        let (d, cap, _, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let bl = bilink_format::BiLink::new(
            format!("capture {}", cap.id()).parse().unwrap(),
            bilink_format::LinkEndpoint::Abstract);
        cap.write_in(d.path()).unwrap();
        bl.write(&bilink_format::BiLink::path_in(
            d.path(), "44444444-4444-4444-8444-444444444444")).unwrap();
        bilink_format::write_version(d.path(), "0.0.1").unwrap();

        let e = check_with(d.path(), d.path(), None).unwrap_err();
        let m = e.downcast_ref::<bilink_format::Mismatch>().expect("es un Mismatch");
        assert_eq!(m.declared.as_deref(), Some("0.0.1"));
    }

    /// **Sin archivos del formato no hay versión que comparar.**
    ///
    /// Ni afuera de una capa ni adentro de una que sólo tiene su cache: negarse ahí
    /// volvería `check` inusable justo donde no hay nada que malinterpretar.
    #[test]
    fn a_layer_with_no_format_files_is_not_a_version_problem() {
        let d = tempdir().unwrap();
        let r = check_with(d.path(), d.path(), None).unwrap();
        assert!(r.results.is_empty() && r.unreadable.is_empty());

        // Y con `.bilink/` declarado, pero todavía vacío — el caso del consumidor
        // que puso el `.toml` del alias y nada más.
        std::fs::create_dir_all(d.path().join(".bilink/cache")).unwrap();
        std::fs::write(d.path().join(".bilink/.hsi.toml"), "remote = \"x\"\n").unwrap();
        let r = check_with(d.path(), d.path(), None).unwrap();
        assert!(r.results.is_empty(), "0 bilink(s) es cierto");
    }

    /// **Un archivo que no parsea se cuenta, y el resto se evalúa igual.**
    ///
    /// Es la diferencia entre *"no pude leer uno"* y *"no hay ninguno"*, que es la
    /// que se perdía al saltearlo en silencio.
    #[test]
    fn an_unreadable_bilink_is_counted_and_the_rest_is_still_checked() {
        let (d, cap, _, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let sano = "55555555-5555-4555-8555-555555555555";
        let roto = "66666666-6666-4666-8666-666666666666";
        let bl = bilink_format::BiLink::new(
            format!("capture {}", cap.id()).parse().unwrap(),
            bilink_format::LinkEndpoint::Abstract);
        cap.write_in(d.path()).unwrap();
        bl.write(&bilink_format::BiLink::path_in(d.path(), sano)).unwrap();
        std::fs::write(bilink_format::BiLink::path_in(d.path(), roto),
                       "endpoint:\n  0:\n    link: capture x\n    campo_que_no_existe: 1\n").unwrap();
        bilink_format::ensure_version(d.path()).unwrap();

        let r = check_with(d.path(), d.path(), None).unwrap();
        assert_eq!(r.results.len(), 1, "el sano se evalúa igual");
        assert_eq!(r.results[0].uuid, sano);
        assert_eq!(r.unreadable.len(), 1, "y el roto no desaparece");
        // El path que se imprime es relativo a la capa, y el error dice qué pasó.
        assert!(r.unreadable[0].path.to_string_lossy().contains(roto));
        assert!(!r.unreadable[0].error.is_empty());
    }

    // ─── la divergencia y la ubicación del vecindario ─────────────────────────

    /// **Dos decisiones incompatibles son un estado, y no se evalúa nada más.**
    ///
    /// Evaluar ubicación o contenido exigiría elegir una de las dos, que es
    /// exactamente lo que nadie hizo.
    #[test]
    fn two_decisions_are_reported_as_divergence() {
        let (d, cap, range, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let uuid = "33333333-3333-4333-8333-333333333333";
        let entrada = |h: &str| bilink_format::Accepted {
            agree: Default::default(),
            link: Some(format!("capture {}", cap.id()).parse().unwrap()),
            hash: h.into(), hash_ast: None, n: None,
            dimensions: Default::default(),
        };
        let mut bl = bilink_format::BiLink::new(
            format!("capture {}", cap.id()).parse().unwrap(),
            bilink_format::LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).accepted = vec![entrada("h1"), entrada("h2")];
        cap.write_in(d.path()).unwrap();
        bl.write(&bilink_format::BiLink::path_in(d.path(), uuid)).unwrap();
        bilink_format::ensure_version(d.path()).unwrap();
        let _ = range;

        let r = check_with(d.path(), d.path(), None).unwrap();
        let it = r.results.iter().find(|x| x.uuid == uuid).expect("el bilink está");
        assert_eq!(it.state0, EndpointState::ConsensusDiverged, "{:?}", it.state0);
        assert!(!it.state0.is_clean(), "divergido no es limpio: check tiene que fallar");
    }

    // ─── las partes del contenido ─────────────────────────────────────────────

    /// Una capa con un bilink cuyo endpoint 0 vigila el cuerpo y los parámetros de
    /// `a`, aprobados sobre `before`, y el archivo reescrito con `today`.
    fn dimensioned_layer(before: &str, today: &str) -> (tempfile::TempDir, String) {
        const FN_A: &str = r#"(function_item name: (identifier) @n0 (#eq? @n0 "a")) @target"#;
        let parts = [("body", "(function_item body: (block) @target)"),
                     ("parameters", "(function_item parameters: (parameters) @target)")];
        let d = tempdir().unwrap();
        let language = grammar::for_language("rust").unwrap();
        let whole = query::find_fragment(language.clone(), before, FN_A).unwrap().unwrap();
        let within = bilink_format::ByteRange { start: whole.ranges.start(), end: whole.ranges.end() };

        let cap = Capture { file: "lib.rs".into(), query: Some(FN_A.into()) };
        cap.write_in(d.path()).unwrap();
        let link: LinkEndpoint = format!("capture {}", cap.id()).parse().unwrap();
        let mut bl = BiLink::new(link.clone(), LinkEndpoint::Abstract);
        let e = bl.endpoint.get_mut(0);
        let mut approved = std::collections::BTreeMap::new();
        for (name, q) in parts {
            e.dimensions.insert(name.into(), bilink_format::DeclaredDimension { query: q.into() });
            let f = query::find_fragment_within(language.clone(), before, q, &within).unwrap().unwrap();
            approved.insert(name.to_string(), bilink_format::AcceptedDimension {
                hash: hash::sha256(f.ranges.text(before).as_bytes()),
                hash_ast: Some(hash::sha256(f.sexp.as_bytes())),
            });
        }
        e.accepted = vec![Accepted {
            agree: Default::default(), link: Some(link),
            hash: hash::sha256(whole.ranges.text(before).as_bytes()), hash_ast: None,
            n: None, dimensions: approved,
        }];
        let uuid = "44444444-4444-4444-8444-444444444444".to_string();
        bl.write(&BiLink::path_in(d.path(), &uuid)).unwrap();
        bilink_format::ensure_version(d.path()).unwrap();
        std::fs::write(d.path().join("lib.rs"), today).unwrap();
        (d, uuid)
    }

    const LIB: &str = "fn a(x: u8) -> u8 { x + 1 }\n";

    /// **La palabra es la de siempre, y las partes van al lado**: el cuerpo sólo
    /// reformateado y un parámetro cambiado dan `ALTERED`, calificado con las dos.
    #[test]
    fn the_state_is_one_word_qualified_by_its_parts() {
        let (d, uuid) = dimensioned_layer(LIB, "fn a(x: u32) -> u8 {\n    x + 1\n}\n");
        let r = check_with(d.path(), d.path(), None).unwrap();
        let it = r.results.iter().find(|x| x.uuid == uuid).unwrap();
        assert_eq!(it.state0, EndpointState::Altered);
        assert_eq!(it.dimensions[0], vec![("body".to_string(), EndpointState::Restyled),
                                          ("parameters".to_string(), EndpointState::Altered)]);
        assert!(it.dimensions[1].is_empty());
        assert!(!it.is_clean());

        // La cache guarda la palabra sola, y las partes aparte.
        let cache = Cache::load(d.path());
        assert_eq!(cache.endpoint_state(&uuid, 0), Some(EndpointState::Altered));
        assert_eq!(cache.endpoint_dimensions(&uuid, 0), it.dimensions[0]);
    }

    /// Con las partes intactas, un cambio afuera de ellas no avisa.
    #[test]
    fn with_parts_the_whole_fragment_no_longer_decides() {
        let (d, uuid) = dimensioned_layer(LIB, "fn a(x: u8) -> u16 { x + 1 }\n");
        let r = check_with(d.path(), d.path(), None).unwrap();
        let it = r.results.iter().find(|x| x.uuid == uuid).unwrap();
        assert_eq!(it.state0, EndpointState::Ok, "el retorno no se vigila");
        assert!(it.dimensions[0].is_empty());
    }

    /// Una sola parte reformateada no hace fallar, y se lista igual.
    #[test]
    fn a_restyled_part_alone_is_restyled_and_clean() {
        let (d, uuid) = dimensioned_layer(LIB, "fn a(x: u8) -> u8 {\n    x + 1\n}\n");
        let r = check_with(d.path(), d.path(), None).unwrap();
        let it = r.results.iter().find(|x| x.uuid == uuid).unwrap();
        assert_eq!(it.state0, EndpointState::Restyled);
        assert_eq!(it.dimensions[0], vec![("body".to_string(), EndpointState::Restyled)]);
        assert!(it.is_clean());
    }

    // ─── el eje del vecindario ────────────────────────────────────────────────

    use crate::neighbours::{Location, Neighbours};

    /// Un proveedor de mentira: contesta lo que se le dijo, o falla.
    struct Fake(Option<Vec<Location>>);
    impl Neighbours for Fake {
        fn available(&self, _l: &std::path::Path) -> bool { true }
        fn of(&self, _l: &std::path::Path, _f: &str, _at: &[usize]) -> Result<Vec<Location>> {
            self.0.clone().ok_or_else(|| anyhow::anyhow!("el language server se cayó"))
        }
    }

    /// Uno que no está: `available` dice que no, y preguntarle es un error del test.
    struct Apagado;
    impl Neighbours for Apagado {
        fn available(&self, _l: &std::path::Path) -> bool { false }
        fn of(&self, _l: &std::path::Path, _f: &str, _at: &[usize]) -> Result<Vec<Location>> {
            panic!("no se le pregunta a un proveedor que no está")
        }
    }

    const FIRMA: &str = "pub fn get() -> Dto { todo!() }";

    /// El fragmento es **la firma**, y el DTO es su vecino.
    ///
    /// Un DTO no tiene firma, y por eso no tiene vecindario que comparar.
    fn dto_layer(body: &str) -> (tempfile::TempDir, Capture, Ranges, Vec<Location>) {
        let d = tempdir().unwrap();
        let (cap, range, locs) = rewrite(d.path(), body);
        (d, cap, range, locs)
    }

    /// Reescribe la capa con otro DTO, en el mismo lugar: los captures aceptados se
    /// resuelven contra lo de hoy.
    fn rewrite(layer: &Path, body: &str) -> (Capture, Ranges, Vec<Location>) {
        let source = format!("{body}\n\n{FIRMA}\n");
        std::fs::write(layer.join("Svc.rs"), &source).unwrap();
        let cap = Capture { file: "Svc.rs".into(), query: None };
        let firma = source.find("pub fn get").unwrap();
        let range = Ranges::one(firma, source.len() - 1);
        let at = body.find("struct ").map(|i| i + 7).unwrap_or(0);
        let locs = vec![Location {
            file: "Svc.rs".into(), symbol: "Dto".into(), start: at, end: at + 3,
        }];
        (cap, range, locs)
    }

    /// El nivel 1 aceptado sobre estos vecinos, con sus captures escritos.
    fn accepted_over(layer: &Path, locs: &[Location]) -> Accepted {
        let f = crate::neighbours::fold(layer, locs).unwrap().expect("los vecinos se capturan");
        for c in &f.captures { c.write_in(layer).unwrap(); }
        Accepted {
            agree: Default::default(), link: None,
            hash: String::new(), hash_ast: None,
            n: Some(bilink_format::N::of_level_1(f.n)),
            dimensions: Default::default(),
        }
    }

    fn declared_as(acc: &Accepted) -> bilink_format::DeclaredN {
        let n1 = acc.n.as_ref().unwrap().level(1).unwrap();
        bilink_format::DeclaredN::of_level_1(n1.link.clone())
    }

    fn contract(layer: &Path, cap: &Capture, declared: Option<&bilink_format::DeclaredN>,
                acc: &Accepted, range: &Ranges, nb: crate::neighbours::Provider<'_>)
        -> Result<Option<EndpointState>> {
        compare_contract(layer, cap, declared, acc, Some(range), nb)
    }

    /// Sin vecindario aceptado no hay nada que preguntar — y es el caso de casi todos
    /// los endpoints, así que tampoco se le pregunta al proveedor.
    #[test]
    fn an_endpoint_without_a_neighbourhood_is_not_asked() {
        let (d, cap, range, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = Accepted { agree: Default::default(), link: None, hash: String::new(), hash_ast: None, n: None, dimensions: Default::default() };
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, Some(&Apagado)).unwrap(), None);
    }

    /// **Sin preguntar, un vecindario intacto es `OK_N1_UNCONFIRMED`.** Los captures
    /// dicen que las declaraciones no cambiaron; que la firma las siga nombrando no lo
    /// preguntó nadie.
    #[test]
    fn without_asking_an_intact_neighbourhood_is_unconfirmed() {
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let declared = declared_as(&acc);
        assert_eq!(contract(d.path(), &cap, Some(&declared), &acc, &range, None).unwrap(),
                   Some(EndpointState::OkN1Unconfirmed));
    }

    /// Y el vacío también: un import nuevo puede meterle un vecino.
    #[test]
    fn an_empty_acquired_level_is_also_unconfirmed() {
        let (d, cap, range, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &[]);
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, None).unwrap(),
                   Some(EndpointState::OkN1Unconfirmed));
    }

    /// **Con el daemon, un vecindario intacto y los mismos vecinos no dice nada.**
    #[test]
    fn a_confirmed_neighbourhood_says_nothing() {
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let declared = declared_as(&acc);
        assert_eq!(contract(d.path(), &cap, Some(&declared), &acc, &range, Some(&Fake(Some(locs)))).unwrap(), None);
    }

    /// **El caso que motivó todo, sin preguntarle a nadie:** el fragmento intacto y el
    /// DTO con un campo más. Es drift probado, y sale igual con `--no-ask-n1`.
    #[test]
    fn a_field_added_to_the_dto_moves_the_contract_without_asking() {
        let (d, cap, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let (_, range, _) = rewrite(d.path(), "pub struct Dto { pub x: u8, pub y: u8 }");
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, None).unwrap(),
                   Some(EndpointState::ContractAltered));
    }

    /// Reformatearlo mueve el texto y no el AST: es del vecindario y es sólo formato.
    #[test]
    fn a_reformatted_neighbourhood_is_restyled_without_asking() {
        let (d, cap, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let (_, range, _) = rewrite(d.path(), "pub struct Dto {\n    pub x: u8\n}");
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, None).unwrap(),
                   Some(EndpointState::ContractRestyled));
    }

    /// **Un vecino cuyo capture ya no resuelve es un cambio, no una ausencia.**
    #[test]
    fn a_neighbour_that_no_longer_resolves_is_altered() {
        let (d, cap, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let (_, range, _) = rewrite(d.path(), "pub struct Otro { pub x: u8 }");
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, Some(&Fake(Some(vec![])))).unwrap(),
                   Some(EndpointState::ContractAltered));
    }

    /// **El eje de ubicación del vecindario se decide sin proveedor**, porque son dos
    /// listas de ids.
    #[test]
    fn the_neighbourhood_location_axis_needs_no_provider() {
        use bilink_format::{CaptureSet, DeclaredN};
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        // Lo declarado hoy nombra otro vecino: entró, salió, se mudó o se renombró.
        let hoy = DeclaredN::of_level_1(CaptureSet::new(vec!["b".repeat(32)]));
        assert_eq!(contract(d.path(), &cap, Some(&hoy), &acc, &range, None).unwrap(),
                   Some(EndpointState::ContractRelocated),
                   "sin proveedor y aun así decidido");
    }

    /// **Lo probado le gana a lo sospechado:** un vecino que cambió se nombra antes que
    /// un conjunto declarado distinto.
    #[test]
    fn a_changed_neighbour_wins_over_a_different_declared_set() {
        use bilink_format::{CaptureSet, DeclaredN};
        let (d, cap, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let (_, range, _) = rewrite(d.path(), "pub struct Dto { pub x: u16 }");
        let hoy = DeclaredN::of_level_1(CaptureSet::new(vec!["b".repeat(32)]));
        assert_eq!(contract(d.path(), &cap, Some(&hoy), &acc, &range, None).unwrap(),
                   Some(EndpointState::ContractAltered));
    }

    /// **Lo que los captures no ven, lo ve el daemon:** las declaraciones aceptadas no
    /// cambiaron, y la firma ahora nombra otra.
    #[test]
    fn the_daemon_resolving_to_other_neighbours_is_relocated() {
        let (d, cap, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let declared = declared_as(&acc);
        let body = "pub struct Dto { pub x: u8 }\npub struct Otro { pub y: u8 }";
        let (_, range, _) = rewrite(d.path(), body);
        let otro = body.find("Otro").unwrap();
        let hoy = vec![Location { file: "Svc.rs".into(), symbol: "Otro".into(), start: otro, end: otro + 4 }];
        assert_eq!(contract(d.path(), &cap, Some(&declared), &acc, &range, None).unwrap(),
                   Some(EndpointState::OkN1Unconfirmed), "sin preguntar no se ve");
        assert_eq!(contract(d.path(), &cap, Some(&declared), &acc, &range, Some(&Fake(Some(hoy)))).unwrap(),
                   Some(EndpointState::ContractRelocated));
    }

    /// **Un proveedor que falla hace fallar la comparación**, y la falla se distingue:
    /// no hay un estado para *"pregunté y no pude"*.
    #[test]
    fn a_provider_that_fails_fails_the_check() {
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = accepted_over(d.path(), &locs);
        let declared = declared_as(&acc);
        let e = contract(d.path(), &cap, Some(&declared), &acc, &range, Some(&Fake(None))).unwrap_err();
        assert!(crate::neighbours::is_provider_error(&e), "{e:#}");
    }

    /// Un nivel con el contrato conservado y sin ubicación, que es lo que deja una
    /// restitución.
    fn unlocated(hash: &str, hash_ast: Option<String>) -> Accepted {
        Accepted {
            agree: Default::default(), link: None,
            hash: String::new(), hash_ast: None,
            n: Some(bilink_format::N::of_level_1(bilink_format::Neighbourhood {
                link: bilink_format::LevelLink::Unknown,
                hash: hash.into(), hash_ast,
            })),
            dimensions: Default::default(),
        }
    }

    /// **Sin ubicación se contesta igual sin proveedor, y no queda limpio.**
    #[test]
    fn an_unlocated_level_answers_without_a_provider() {
        let (d, cap, range, _) = dto_layer("pub struct Dto { pub x: u8 }");
        let acc = unlocated("el-contrato", None);
        let s = contract(d.path(), &cap, None, &acc, &range, None).unwrap();
        assert_eq!(s, Some(EndpointState::ContractUnlocated));
        assert!(!s.unwrap().is_clean(), "hay captures que alguien tiene que acuñar");
    }

    /// Y con el contrato intacto **sigue sin ubicación**: no puede salir `Ok`.
    #[test]
    fn an_intact_contract_without_its_location_is_not_ok() {
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let hoy = crate::neighbours::fold(d.path(), &locs).unwrap().unwrap().n;
        let acc = unlocated(&hoy.hash, hoy.hash_ast.clone());
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, Some(&Fake(Some(locs)))).unwrap(),
                   Some(EndpointState::ContractUnlocated));
    }

    /// **Un cambio real de contrato le gana a la ubicación faltante.**
    #[test]
    fn a_real_contract_change_wins_over_the_missing_location() {
        let (d, cap, range, locs) = dto_layer("pub struct Dto { pub x: u8, pub y: u8 }");
        let acc = unlocated("el-contrato-de-antes", None);
        assert_eq!(contract(d.path(), &cap, None, &acc, &range, Some(&Fake(Some(locs)))).unwrap(),
                   Some(EndpointState::ContractAltered));
    }

    /// Una capa con un bilink cuya punta tiene nivel 1 adquirido.
    fn layer_with_an_acquired_level() -> (tempfile::TempDir, String) {
        let (d, _, _, locs) = dto_layer("pub struct Dto { pub x: u8 }");
        let source = std::fs::read_to_string(d.path().join("Svc.rs")).unwrap();
        let cap = Capture { file: "Svc.rs".into(), query: None };
        cap.write_in(d.path()).unwrap();
        let mut acc = accepted_over(d.path(), &locs);
        acc.link = Some(format!("capture {}", cap.id()).parse().unwrap());
        acc.hash = hash::sha256(source.as_bytes());
        let uuid = "77777777-7777-4777-8777-777777777777".to_string();
        let mut bl = bilink_format::BiLink::new(
            format!("capture {}", cap.id()).parse().unwrap(), LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).n = Some(declared_as(&acc));
        bl.endpoint.get_mut(0).accepted = vec![acc];
        bl.write(&bilink_format::BiLink::path_in(d.path(), &uuid)).unwrap();
        bilink_format::ensure_version(d.path()).unwrap();
        (d, uuid)
    }

    /// **Con nivel 1 adquirido y sin nadie que conteste, `check` falla antes de
    /// verificar nada**, y dice cuántos y de qué lenguajes.
    #[test]
    fn without_a_daemon_check_fails_before_checking() {
        let (d, _) = layer_with_an_acquired_level();
        let e = check_with(d.path(), d.path(), Some(&Apagado)).unwrap_err();
        let crate::neighbours::NoProvider(demand) = e.downcast_ref().expect("es NoProvider");
        assert_eq!(demand.endpoints, 1);
        assert_eq!(demand.languages.iter().copied().collect::<Vec<_>>(), vec!["rust"]);
        assert!(Cache::load(d.path()).is_cold(), "no verificó nada");
    }

    /// **Sin preguntar, verifica y no confirma.**
    #[test]
    fn without_asking_check_verifies_and_leaves_it_unconfirmed() {
        let (d, uuid) = layer_with_an_acquired_level();
        let r = check_with(d.path(), d.path(), None).unwrap();
        let it = r.results.iter().find(|x| x.uuid == uuid).unwrap();
        assert_eq!(it.state0, EndpointState::OkN1Unconfirmed);
        assert!(it.is_clean());
        assert_eq!(r.n1.endpoints, 1);
    }

    /// Una capa sin nivel 1 adquirido no pide a nadie.
    #[test]
    fn a_layer_without_an_acquired_level_does_not_need_a_daemon() {
        let d = tempdir().unwrap();
        assert!(check_with(d.path(), d.path(), Some(&Apagado)).is_ok());
    }
}
