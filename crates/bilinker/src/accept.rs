//! `bilinker accept` — el único que escribe una decisión.
//!
//! Aceptar es decir *"revisé esto y lo apruebo"*, y hay **dos cosas que aprobar**:
//! dónde está el fragmento y qué dice. Se pueden aprobar juntas o por separado.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{Accepted, AcceptedDimension, BiLink, Capture, DeclaredDimension, LinkEndpoint, N};

use crate::cache::Cache;
use crate::state::EndpointState;
use crate::{grammar, hash, query};

/// Qué dimensiones aprueba esta aceptación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct What {
    pub place:   bool,
    pub content: bool,
    /// Aceptar renunciando al [vecindario](crate::neighbours): escribe
    /// `n: declined` en vez de los niveles, sin preguntarle a nadie.
    ///
    /// Está acá y no en un parámetro aparte porque **el vecindario es una tercera
    /// dimensión** y se comporta como las otras dos: se aprueba, o no se toca.
    ///
    /// **Renuncia del nivel 1 para arriba, no al nivel 1.** El día que exista un
    /// nivel 2 —los campos de los tipos que el 1 resuelve— queda adentro de esta
    /// misma renuncia, porque está definido a través del 1.
    ///
    /// No preguntar es otra cosa, y no va acá: es aceptar sin proveedor.
    pub decline_n1: bool,
    /// Sólo junto a `decline_n1`, y sólo donde éste **baja** un nivel 1 adquirido.
    ///
    /// Escalonado a propósito: la renuncia en una persona se tipea una vez, en un CI
    /// se escribe una vez y queda para siempre. Sin el escalón, esa línea de
    /// configuración sería una autorización permanente a bajar cobertura.
    pub force:   bool,
}

impl Default for What {
    /// Por defecto se aprueban las dos.
    fn default() -> Self { Self { place: true, content: true, decline_n1: false, force: false } }
}

impl What {
    pub fn place_only()   -> Self { Self { place: true,  content: false, ..Self::default() } }
    pub fn content_only() -> Self { Self { place: false, content: true,  ..Self::default() } }
}

pub struct AcceptResult {
    pub uuid: String,
    pub n: u8,
    pub hash: String,
    pub commit: Option<String>,
    /// Quiénes aprobaron estos valores, después de este acto.
    pub agree: BTreeSet<String>,
    /// `false` cuando el archivo quedó igual: los mismos valores y quien acepta ya
    /// estaba en el set. No hay nada que agregar, y no hay commit que escribir.
    pub wrote: bool,
}

/// Cuántos de estos endpoints tienen nivel 1 que preguntar si se aceptan así.
///
/// Es lo que se exige antes de trabajar: los que alcanzan una firma con tipos, cuando
/// se aprueba el contenido y no se renuncia. Un endpoint que no se puede leer o no
/// resuelve no cuenta: `accept` lo va a rechazar por su cuenta, y con su mensaje.
pub fn demand(layer: &Path, targets: &[(String, u8)], what: What) -> crate::neighbours::Demand {
    let mut d = crate::neighbours::Demand::default();
    if !what.content || what.decline_n1 { return d }
    let cache = Cache::load(layer);
    for (uuid, n) in targets {
        let Ok(path) = find_bilink_path(layer, uuid) else { continue };
        let Ok(bl) = BiLink::load(&path) else { continue };
        let e = bl.endpoint.get(*n);
        let Some(id) = e.link.capture_id() else { continue };
        let Ok(cap) = Capture::load_in(layer, id) else { continue };
        let full = path.file_stem().and_then(|s| s.to_str()).unwrap_or(uuid);
        let Ok((_, Some(range))) = crate::check::resolve_capture(
            layer, &cap, e.accepted.first(), cache.commit(full, *n)) else { continue };
        if matches!(crate::neighbours::reach(layer, &cap.file, &range), crate::neighbours::Reach::At(_)) {
            d.add(&cap.file);
        }
    }
    d
}

/// Acepta un endpoint.
pub fn accept(
    layer: &Path, uuid: &str, n: u8, what: What, nb: crate::neighbours::Provider<'_>,
) -> Result<AcceptResult> {
    let path = find_bilink_path(layer, uuid)?;
    let uuid = path.file_stem().and_then(|s| s.to_str())
        .context("el nombre del bilink no es un uuid")?.to_string();

    let mut bl = BiLink::load(&path)?;
    let mut cache = Cache::load(layer);

    let (mut accepted, commit) = compute(layer, &bl, &uuid, n, what, &cache, nb)?;
    if let Some(c) = &commit {
        cache.set_commit(&uuid, n, c);
    }

    // **Quiénes aprobaron *estos* valores**, y qué pasa con las otras decisiones.
    //
    // Hay exactamente dos casos, y los dos ya estaban decididos antes de que
    // `accepted` fuera una lista — lo que la lista cambia es qué pasa con lo que no
    // se acepta.
    //
    // **Coincide con una entrada**: quien acepta se **suma** a su `agree`, y las
    // demás entradas siguen ahí. Sigue divergido, y es correcto — sumarse a un lado
    // no resuelve un desacuerdo.
    //
    // **No coincide con ninguna**: se abre una entrada nueva con quien acepta solo, y
    // las que aprobaban **otros** valores se van. Quien aprobó los valores anteriores
    // no aprobó éstos, y arrastrar su nombre sería atribuirle una decisión que no
    // tomó; donde queda su aprobación es donde siempre quedó — en el commit que la
    // escribió.
    //
    // La identidad de una entrada es su **tupla entera**: `link`, `hash`, `hash_ast`,
    // `dimensions` y `n`. Dos personas que aprueban el mismo fragmento con vecindarios distintos no
    // comparten entrada: son dos contratos.
    let firmante = signer(layer)?;
    let previas = &bl.endpoint.get(n).accepted;
    let coincide = previas.iter().position(|p| p.same_values(&accepted));

    let (nuevas, wrote) = match coincide {
        Some(i) => {
            // Me sumo a la que ya estaba. Las demás no se tocan: si había
            // divergencia, sigue habiéndola.
            let mut nuevas = previas.clone();
            let sumado = nuevas[i].agree.insert(firmante);
            (nuevas, sumado)
        }
        None => {
            // **Colapsa.** Lo que se aprueba queda solo, y lo que aprobaba otra cosa
            // se va. La lista es la ventana entre dos aceptaciones, no un archivo
            // histórico.
            accepted.agree = BTreeSet::from([firmante]);
            (vec![accepted], true)
        }
    };

    let hash = nuevas[coincide.unwrap_or(0)].hash.clone();
    let agree = nuevas[coincide.unwrap_or(0)].agree.clone();
    bl.endpoint.get_mut(n).accepted = nuevas;
    bl.write(&path)?;

    // El estado cacheado describe la comparación anterior y ya no vale.
    cache.set_endpoint_state(&uuid, n, EndpointState::Ok);
    cache.save(layer)?;

    Ok(AcceptResult { uuid, n, hash, commit, agree, wrote })
}

/// Quién acepta: **el nombre que git va a poner como autor del commit.**
///
/// Que sea el mismo que el autor y el mismo que `git blame` muestra sobre la línea
/// del nombre es lo que permite cruzarlos: un `agree` que dijera una cosa y el autor
/// del commit otra no se podría verificar contra ninguna firma, y el campo quedaría
/// siendo decoración.
///
/// **Por eso se le pregunta a git en vez de leer `user.name`.** El nombre del autor
/// no siempre sale de ahí —puede venir de `GIT_AUTHOR_NAME`, de un `[includeIf]` por
/// directorio, o del sistema cuando nadie lo configuró— y leer un solo lugar acierta
/// a veces. `git var GIT_AUTHOR_IDENT` contesta lo que git realmente va a usar, con
/// el mismo orden de precedencia, y devuelve `Nombre <mail> ts tz`.
///
/// Si git no puede contestar, no se acepta: tampoco se podría commitear.
fn signer(layer: &Path) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "var", "GIT_AUTHOR_IDENT"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        // El nombre es todo lo que va antes del mail, que git siempre encierra.
        .and_then(|ident| ident.split_once(" <").map(|(n, _)| n.trim().to_string()))
        .filter(|s| !s.is_empty());

    out.context(
        "git no sabe con qué nombre firmar, y `agree` dice quién aprueba.\n       Configurarlo: `git config user.name '<nombre>'`",
    )
}

/// **No preguntar no puede reducir la cobertura de un vínculo, ni afirmar una que
/// nadie miró.**
///
/// Ver `concepts/accept.md` § "El `n` previo tiene tres valores". `folded` es lo que
/// contestó el proveedor; `None` con firma alcanzable es que no se le preguntó.
fn resolve_n1(
    cap: &Capture,
    alcance: &crate::neighbours::Reach,
    previous: Option<&Accepted>,
    folded: &Option<crate::neighbours::Neighbourhood>,
    what: What,
    content_hash: &str,
) -> Result<Option<N>> {
    // Con `--place` el contenido no se toca, y el vecindario es del contenido.
    let preserve = || previous.and_then(|a| a.n.clone());
    if !what.content {
        return Ok(preserve());
    }

    // **Qué se puede saber del vecindario se contesta con la gramática, no con el
    // proveedor**, y son tres cosas y no dos.
    use crate::neighbours::Reach;
    match alcance {
        // No hay vecindario: prosa, un DTO, un lenguaje sin tipos. La ausencia ya era
        // la correcta, y avisar acá sería ruido — un aviso que sale siempre no lo lee
        // nadie.
        Reach::None => return Ok(preserve()),

        // **Hay y no se alcanza**, que es lo que antes se escribía como si no
        // hubiera. Escribir ausencia acá le daría a la ausencia un segundo
        // significado que ningún lector puede separar del primero.
        Reach::Unreachable { what: que } => {
            if what.decline_n1 { return Ok(Some(N::declined())); }
            bail!(
                "el fragmento de {} {que}, y su vecindario no se puede alcanzar: el \
                 nivel 1 sale de una firma, y ahí no hay una que sea la suya.\n       \
                 Capturar el contrato con --as, o renunciar al vecindario con --decline-n1.",
                cap.file);
        }

        Reach::At(_) => {}
    }

    // **Una renuncia escrita es una decisión, y se lee de vuelta.** Un `declined`
    // previo no cae en el mismo casillero que no tener nada: si cayera, la renuncia
    // habría que tipearla en cada `accept`, y un pedido que sale siempre no lo lee
    // nadie.
    let had = match previous.and_then(|a| a.n.as_ref()) {
        None                       => Had::Nothing,
        Some(n) if n.is_acquired() => Had::Acquired,
        Some(_)                    => Had::Declined,
    };

    // Renunciar es una decisión, y bajar un nivel adquirido se pide entero.
    if what.decline_n1 {
        return match (had, what.force) {
            (Had::Acquired, false) => bail!(
                "--decline-n1 acá baja un vecindario que ya estaba aceptado.\n       \
                 Conservarlo: aceptar sin --decline-n1. Bajarlo a propósito: --decline-n1 --force."),
            _ => Ok(Some(N::declined())),
        };
    }

    // Se preguntó: se escribe el vecindario entero, y una renuncia anterior se levanta.
    if let Some(f) = folded {
        return Ok(Some(N::of_level_1(f.clone())));
    }

    // ── sin preguntar ─────────────────────────────────────────────────────────
    //
    // **Los nombres del conjunto los pone la firma, y la firma está en el fragmento.**
    // Con el capture de contrato el `hash` *es* el de la firma, así que un `hash`
    // quieto no agregó ni sacó nombres. Sobre un capture que arrastra el cuerpo esto
    // sobre-dispara —un refactor adentro cuenta como cambio— y erra hacia pedir que
    // alguien mire, que es el lado correcto para errar.
    let signature_changed = previous.map(|p| p.hash != content_hash).unwrap_or(true);
    let lang = crate::grammar::language_for_file(&cap.file);

    match (had, signature_changed) {
        // No había vecindario que conservar: aceptar así lo deja sin vigilar, y el
        // baseline no lo diría.
        (Had::Nothing, _) => bail!(
            "la firma de {} tiene nivel 1, y --no-ask-n1 no le pregunta a nadie.\n       \
             Aceptar así deja los tipos que la firma menciona sin vigilar, y el \
             baseline no lo diría.\n       \
             Levantar el daemon con `lspd start --wait --lang {lang}`, o renunciar al \
             nivel 1 con --decline-n1.", cap.file),

        // Una renuncia no es sobre un conjunto de vecinos, es sobre si se vigilan: que
        // la firma haya cambiado no la vuelve falsa.
        (Had::Declined, _) => Ok(preserve()),

        // Había, y la firma no se movió. Conservar es más seguro que borrar: si algún
        // vecino cambió, el valor viejo sigue ahí y el próximo `check` lo reporta.
        (Had::Acquired, false) => Ok(preserve()),

        // Había y la firma cambió: conservar sería mentir, porque el conjunto pudo
        // cambiar con ella.
        (Had::Acquired, true) => bail!(
            "ya hay un vecindario aceptado, y la firma cambió.\n       \
             Conservarlo mentiría: el conjunto de vecinos pudo cambiar con la firma, y \
             --no-ask-n1 no le pregunta a nadie.\n       \
             Levantar el daemon con `lspd start --wait --lang {lang}`, o bajarlo a \
             propósito con --decline-n1 --force."),
    }
}

/// Qué decisión había sobre el vecindario de este endpoint.
///
/// **Tres valores y no un booleano.** Una renuncia escrita no es la ausencia de una
/// decisión: es una que alguien tomó, y meterla en el mismo casillero que "no hay
/// nada" es lo que hacía que se volviera a pedir en cada `accept`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Had {
    /// No hay `n`: nadie decidió todavía.
    Nothing,
    /// `n: declined` — alguien renunció, y quedó escrito.
    Declined,
    /// Hay vecindario adquirido: cobertura que se puede perder.
    Acquired,
}

/// Calcula el bloque `accepted` para un endpoint.
fn compute(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    what: What,
    cache: &Cache,
    nb: crate::neighbours::Provider<'_>,
) -> Result<(Accepted, Option<String>)> {
    let e = bl.endpoint.get(n);
    // La primera entrada. La lista es de `3u`; acá se mantiene lo que había.
    let previous = e.accepted.first();

    match &e.link {
        LinkEndpoint::Capture(id) => {
            let cap = Capture::load_in(layer, id)?;
            let (state, range) = crate::check::resolve_capture(layer, &cap, previous, cache.commit(uuid, n))?;
            if !state.is_resolved() {
                bail!("el capture no resuelve ({state}): no se puede aprobar contenido \
                       que no se pudo localizar");
            }
            let range = range.context("el capture resolvió sin rango")?;

            // El fragmento tiene que estar commiteado.
            //
            // No es una recomendación: `commit` es el commit en que el fragmento
            // quedó con el contenido aprobado, y ese commit **no existe** si el
            // fragmento no está commiteado. Sin él no hay `git show`, y sin eso
            // `check` no puede recuperar el texto aceptado.
            if working_tree_dirty(layer, &cap.file) {
                bail!("{} tiene cambios sin commitear.\n       \
                       Aceptar fija un contenido, y ese contenido tiene que existir \
                       en la historia.", cap.file);
            }

            let source = std::fs::read_to_string(layer.join(&cap.file))?;
            let fragment = range.text(&source);
            let content_hash = hash::sha256(fragment.as_bytes());
            let ast_hash = ast_hash_of(layer, &cap, &source)?;
            // Las dimensiones salen de la gramática y no del proveedor: se calculan
            // igual con daemon o sin él, y con cualquier flag del vecindario.
            let dimensions = if what.content {
                dimensions_of(&cap, &e.dimensions, &source, &range)?
            } else {
                previous.map(|a| a.dimensions.clone()).unwrap_or_default()
            };

            // El vecindario, si hay quien lo resuelva y el fragmento tiene uno **al
            // que se llegue**. Las posiciones las pone la gramática: preguntar donde
            // arranca el fragmento devuelve `pub`, que no declara nada.
            let alcance = crate::neighbours::reach(layer, &cap.file, &range);
            let preguntar = what.content && !what.decline_n1;
            let resuelto = match (preguntar, nb, &alcance) {
                (true, Some(p), crate::neighbours::Reach::At(at)) => {
                    let locs = crate::neighbours::ask(p, layer, &cap.file, at)?;
                    // Escribirlo sin el vecino que no se puede capturar afirmaría un
                    // conjunto que no es el de la firma.
                    let Some(f) = crate::neighbours::fold(layer, &locs)? else {
                        bail!("el vecindario de {} no se puede representar: algún tipo que la \
                               firma menciona no se puede capturar.\n       \
                               Renunciar al nivel 1 con --decline-n1.", cap.file);
                    };
                    Some(f)
                }
                _ => None,
            };

            // **Los captures de los vecinos se escriben acá.**
            //
            // `fold` los calcula y no los escribe, porque `check` lo llama igual y no
            // escribe nada versionado. Escribirlos es de `accept`: es quien tiene el
            // proveedor, y sin los archivos el `n.1.link` que se está por guardar
            // apuntaría a captures que no existen.
            //
            // Que `accept` acuñe captures es nuevo —era de `apply` y de `chain new`—
            // y no rompe el reparto: el conjunto es de **sólo-agregar**, y lo que
            // sigue siendo exclusivo de `accept` es escribir `accepted`.
            if let Some(f) = &resuelto {
                for c in &f.captures { c.write_in(layer)?; }
            }
            let folded = resuelto.map(|f| f.n);
            let neighbourhood = resolve_n1(&cap, &alcance, previous, &folded, what, &content_hash)?;

            let accepted = Accepted {
                // Lo pone `accept`, no `compute`: depende de qué había antes.
                agree: BTreeSet::new(),
                // Aprobar la ubicación es escribir el link vigente en `accepted`.
                link: if what.place {
                    Some(e.link.clone())
                } else {
                    previous.and_then(|a| a.link.clone())
                },
                hash: if what.content {
                    content_hash
                } else {
                    previous.map(|a| a.hash.clone())
                        .context("no hay contenido previo que conservar: aceptar con --place \
                                  exige que el endpoint ya tuviera algo aprobado")?
                },
                // Nunca se conserva un `hash_ast` que la gramática no puede
                // producir: uno guardado por una versión anterior sobreviviría a
                // cada `accept --place` y seguiría estando ahí para mentir.
                hash_ast: if what.content {
                    ast_hash
                } else if grammar::ast_discriminates_content(grammar::language_for_file(&cap.file)) {
                    previous.and_then(|a| a.hash_ast.clone())
                } else {
                    None
                },
                // **Un campo con tres estados**, y qué se escribe lo decide
                // `resolve_n1`: adquirido, `declined`, o ausente porque el fragmento
                // no tiene firma resoluble. La regla que las gobierna es que no
                // preguntar nunca baja la cobertura.
                n: neighbourhood,
                // Todas juntas, con el contenido: `--place` conserva las que había,
                // igual que conserva el `hash`.
                dimensions,
            };

            // `commit` es el commit **del contenido**, no el HEAD de quien acepta.
            // Con el HEAD, el mismo acto daba distinto según quién y cuándo lo
            // hiciera, y el valor no describía nada del fragmento.
            let commit = what.content
                .then(|| content_commit(layer, &cap, &range))
                .flatten()
                .or_else(|| cache.commit(uuid, n).map(String::from));

            Ok((accepted, commit))
        }

        // Un endpoint `path` copia los **dos** valores del endpoint estructural de
        // su vecino: qué ubicación y qué contenido se aprobaron ahí.
        LinkEndpoint::Path(p) => {
            let target = stratum::resolve(layer, layer, p.tokens())
                .map_err(|e| anyhow::anyhow!("resolviendo el endpoint path: {e:?}"))?;
            let adj_path = layer.join(&target).join(".bilink").join(format!("{uuid}.yaml"));
            let adj = BiLink::load(&adj_path)
                .with_context(|| format!("leyendo el bilink vecino {}", adj_path.display()))?;
            let adj_accepted = adj.structural_accepted()
                .context("el vecino todavía no tiene ningún endpoint estructural aceptado; \
                          aceptarlo primero")?;
            Ok((adj_accepted.clone(), None))
        }

        // Un endpoint repo copia los mismos **dos** valores que un `path`, sólo que
        // del bilink de otro proyecto. Son dos SHA-256 opacos: se comparan, no se
        // resuelven, y de ellos no se reconstruye nada del proveedor.
        LinkEndpoint::Repo(alias) => {
            use crate::frontier::Resolution;
            match crate::frontier::resolve(layer, alias, uuid)? {
                Resolution::Found(view) => {
                    if !view.still_abstract {
                        bail!("la otra punta de '{alias}' dejó de ser `abstract`: el vínculo \
                               no se sostiene, y aceptarlo lo fijaría contra algo que ya no \
                               admite ser ampliado");
                    }
                    let accepted = view.accepted.context(
                        "el proveedor todavía no aceptó lo que publica; no hay qué copiar",
                    )?;
                    Ok((accepted, None))
                }
                // Aceptar exige leer al proveedor, así que acá sí falta el clon —a
                // diferencia de `check`, que lo reporta y sigue.
                Resolution::NotCloned => bail!(
                    "el repo '{alias}' no está clonado. Traerlo primero: `bilinker fetch {alias}`."
                ),
                Resolution::BilinkGone => bail!(
                    "el bilink {uuid} no está en el repo '{alias}': el proveedor lo removió"
                ),
            }
        }

        // Una punta `abstract` no se acepta nunca: no hay nada que bendecir del lado
        // abierto. `accept .` la saltea sola, y pedirla por nombre es un error.
        LinkEndpoint::Abstract => bail!(
            "un endpoint `abstract` no se acepta: es la punta abierta, y su estado es \
             OPEN siempre"
        ),

        // Un `issue` no lleva `accepted.link`: la ubicación de un ítem es su id.
        LinkEndpoint::Issue(id) => {
            let (item, root) = crate::issue::resolve_issue_path(layer, id)?;
            let item = item.with_context(|| format!("no hay ítem de worklist con id '{id}'"))?;
            let text = std::fs::read_to_string(&item)
                .with_context(|| format!("leyendo {}", item.display()))?;
            let rel = item.strip_prefix(&root).unwrap_or(&item).display().to_string();
            Ok((
                Accepted {
                    agree: BTreeSet::new(),
                    link: None,
                    hash: hash::sha256(text.as_bytes()),
                    hash_ast: None,
                    // Un ítem de worklist no tiene firma: no hay tipos que resolver.
                    // La ausencia de `n1` dice exactamente eso, y no una renuncia.
                    n: None,
                    dimensions: Default::default(),
                },
                crate::git::try_head_commit_for_file(&root, &rel),
            ))
        }
    }
}

/// Los endpoints de la capa que necesitan atención, en orden de archivo.
///
/// Es lo que `accept .` va a aprobar, **enumerado antes de aprobar nada**. Se separa
/// del bucle porque cada aceptación cierra con su propio commit sobre la ref, y la
/// absorción que las precede a todas se escribe una sola vez: quien recorre esta
/// lista es quien commitea, no esta función.
///
/// Existe para el caso en que ya se revisó todo, no para el caso en que no se
/// revisó nada: cada estado no-OK es un puntero al fragmento que hay que mirar.
pub fn pending(layer: &Path) -> Vec<(String, u8)> {
    let cache = Cache::load(layer);
    let mut out = Vec::new();

    for path in bilink_files(&layer.join(".bilink")) {
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Ok(bl) = BiLink::load(&path) else { continue };

        for n in [0u8, 1u8] {
            // `accept .` **nunca toca una punta `abstract`.** Su estado es OPEN,
            // constante y sano: no hay nada que aprobar del lado abierto, y
            // saltearla acá es lo que evita que un bulk la convierta en otra cosa.
            if bl.endpoint.get(n).link.is_abstract() { continue; }

            // `OK_N1_UNCONFIRMED` no pide ninguna decisión: se pidió no preguntar.
            let needs = match cache.endpoint_state(uuid, n) {
                Some(s) => s.is_listed(),
                None    => bl.endpoint.get(n).accepted.is_empty(),
            };
            if needs { out.push((uuid.to_string(), n)); }
        }
    }
    out
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// ¿El archivo tiene cambios sin commitear?
fn working_tree_dirty(layer: &Path, file: &str) -> bool {
    std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "status", "--porcelain", "--", file])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// El commit en que el fragmento quedó con este contenido.
///
/// `git log -L` recorre la historia de un rango de líneas: su primer commit es
/// aquel en que las líneas quedaron como están. Es nativo y offline.
///
/// Con varias partes se pregunta por **el tramo que las abarca a todas**, que es un
/// superconjunto del fragmento. Da un commit igual o más nuevo que el del fragmento
/// solo, y sirve igual: lo que se necesita es un commit desde el cual el fragmento
/// no cambió, y si el tramo entero no cambió, el fragmento tampoco.
fn content_commit(layer: &Path, cap: &Capture, range: &bilink_format::Ranges) -> Option<String> {
    let source = std::fs::read_to_string(layer.join(&cap.file)).ok()?;
    let line_of = |byte: usize| source[..byte.min(source.len())].lines().count().max(1);
    let (a, b) = (line_of(range.start()), line_of(range.end()));

    let out = std::process::Command::new("git")
        .args(["-C", &layer.to_string_lossy(), "log", "-L",
               &format!("{a},{b}:{}", cap.file), "--format=%H", "-s", "-n", "1"])
        .output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout).lines().next().map(str::to_string)
}

/// El hash de la s-expression, **sólo donde el AST discrimina el contenido**.
fn ast_hash_of(layer: &Path, cap: &Capture, source: &str) -> Result<Option<String>> {
    let _ = layer;
    let Some(q) = &cap.query else { return Ok(None) };
    let lang = grammar::language_for_file(&cap.file);
    if !grammar::ast_discriminates_content(lang) { return Ok(None); }
    let Ok(language) = grammar::for_language(lang) else { return Ok(None) };
    Ok(query::find_fragment(language, source, q)?
        .map(|f| hash::sha256(f.sexp.as_bytes())))
}

/// Los hashes de cada dimensión declarada, resueltas desde el nodo del capture.
///
/// Una que no resuelve hace fallar: no se puede aprobar una parte que no se pudo
/// localizar, y escribir las demás sin ella aprobaría menos de lo que el endpoint
/// declara.
fn dimensions_of(
    cap: &Capture,
    declared: &BTreeMap<String, DeclaredDimension>,
    source: &str,
    range: &bilink_format::Ranges,
) -> Result<BTreeMap<String, AcceptedDimension>> {
    if declared.is_empty() { return Ok(BTreeMap::new()); }
    let lang = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)
        .with_context(|| format!("{} no tiene gramática, y sus dimensiones no se pueden resolver", cap.file))?;
    let discriminates = grammar::ast_discriminates_content(lang);

    declared.iter().map(|(name, d)| {
        let part = query::dimension(language.clone(), source, &d.query, (range.start(), range.end()))
            .with_context(|| format!("la dimensión {name}"))?
            .with_context(|| format!("la dimensión {name} no resuelve en {}: no se puede aprobar \
                                      una parte que no se pudo localizar", cap.file))?;
        Ok((name.clone(), AcceptedDimension {
            hash: hash::sha256(part.ranges.text(source).as_bytes()),
            hash_ast: discriminates.then(|| hash::sha256(part.sexp.as_bytes())),
        }))
    }).collect()
}

/// El bilink cuyo uuid empieza con el prefijo dado.
pub fn find_bilink_path(layer: &Path, prefix: &str) -> Result<std::path::PathBuf> {
    let hits: Vec<_> = bilink_files(&layer.join(".bilink")).into_iter()
        .filter(|p| p.file_stem().and_then(|s| s.to_str())
                     .map(|s| s.starts_with(prefix)).unwrap_or(false))
        .collect();
    match hits.len() {
        1 => Ok(hits.into_iter().next().expect("uno")),
        0 => bail!("no hay bilink que empiece con '{prefix}'"),
        n => bail!("'{prefix}' es ambiguo: {n} bilinks coinciden"),
    }
}

/// La tabla de `concepts/accept.md` § "El `n` previo tiene tres valores", y las dos
/// renuncias.
///
/// La regla que las gobierna: **no preguntar no puede reducir la cobertura de un
/// vínculo, ni afirmar una que nadie miró.**
#[cfg(test)]
mod n1_tests {
    use super::*;
    use bilink_format::Ranges;
    use tempfile::tempdir;

    use crate::neighbours::Neighbourhood;

    /// Un método Java: tiene firma resoluble, y por eso corresponde el aviso.
    const CON_FIRMA: &str = "class Svc {\n\tpublic Dto get(String t) { return null; }\n}\n";
    /// Un DTO: su declaración no menciona tipos como los menciona una firma.
    const SIN_FIRMA: &str = "class Dto {\n\tprivate String x;\n}\n";

    fn layer(body: &str) -> (tempfile::TempDir, Capture, crate::neighbours::Reach) {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Svc.java"), body).unwrap();
        // El rango del método, que es donde cae un `@target` de un capture de firma.
        let start = body.find("public").or_else(|| body.find("private")).unwrap();
        let cap = Capture { file: "Svc.java".into(), query: None };
        let r = Ranges::one(start, start + 10);
        let alcance = crate::neighbours::reach(d.path(), &cap.file, &r);
        (d, cap, alcance)
    }

    fn previo(hash: &str, n1: Option<&str>) -> Accepted {
        Accepted {
            agree: Default::default(),
            link: None,
            hash: hash.into(),
            hash_ast: None,
            n: n1.map(|h| N::of_level_1(Neighbourhood { link: Default::default(), hash: h.into(), hash_ast: None })),
            dimensions: Default::default(),
        }
    }

    fn declinado(hash: &str) -> Accepted {
        let mut p = previo(hash, None);
        p.n = Some(N::declined());
        p
    }

    fn folded() -> Option<Neighbourhood> {
        Some(Neighbourhood { link: Default::default(), hash: "nuevo".into(), hash_ast: Some("nuevo_ast".into()) })
    }

    /// El hash del vecindario adquirido, si lo hay.
    fn adquirido(o: &Option<N>) -> Option<&str> {
        o.as_ref().and_then(|n| n.level(1)).map(|n| n.hash.as_str())
    }

    fn decline() -> What { What { decline_n1: true, ..What::default() } }

    /// Preguntado, se escribe lo calculado, sea cual sea el previo: una renuncia
    /// anterior se levanta sola.
    #[test]
    fn asked_it_writes_what_was_computed_over_anything() {
        let (_d, cap, r) = layer(CON_FIRMA);
        for p in [None, Some(declinado("h")), Some(previo("h", Some("viejo")))] {
            let o = resolve_n1(&cap, &r, p.as_ref(), &folded(), What::default(), "h").unwrap();
            assert_eq!(adquirido(&o), Some("nuevo"));
        }
    }

    /// Sin preguntar y sin nada que conservar, **no se escribe nada**, y el error
    /// nombra las dos salidas.
    #[test]
    fn not_asked_with_nothing_to_keep_refuses() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let e = resolve_n1(&cap, &r, None, &None, What::default(), "h").unwrap_err().to_string();
        assert!(e.contains("lspd start --wait --lang java"), "{e}");
        assert!(e.contains("--decline-n1"), "{e}");
    }

    /// Sin preguntar, una renuncia escrita se conserva, aunque la firma haya cambiado.
    #[test]
    fn not_asked_a_written_decline_is_kept() {
        let (_d, cap, r) = layer(CON_FIRMA);
        for hash in ["h", "otro"] {
            let o = resolve_n1(&cap, &r, Some(&declinado(hash)), &None, What::default(), "h").unwrap();
            assert_eq!(o, Some(N::declined()));
        }
    }

    /// Sin preguntar, un adquirido con la firma intacta se conserva.
    #[test]
    fn not_asked_an_acquired_level_with_the_same_signature_is_kept() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let o = resolve_n1(&cap, &r, Some(&previo("h", Some("viejo"))), &None, What::default(), "h").unwrap();
        assert_eq!(adquirido(&o), Some("viejo"));
    }

    /// Sin preguntar, un adquirido con la firma cambiada no se conserva: el conjunto
    /// pudo cambiar con ella.
    #[test]
    fn not_asked_an_acquired_level_with_a_changed_signature_refuses() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let e = resolve_n1(&cap, &r, Some(&previo("viejo_hash", Some("viejo"))), &None, What::default(), "otro")
            .unwrap_err().to_string();
        assert!(e.contains("--decline-n1 --force"), "{e}");
    }

    /// `--decline-n1` escribe la renuncia, sin preguntar.
    #[test]
    fn declining_writes_it() {
        let (_d, cap, r) = layer(CON_FIRMA);
        for p in [None, Some(declinado("h"))] {
            let o = resolve_n1(&cap, &r, p.as_ref(), &None, decline(), "h").unwrap();
            assert_eq!(o, Some(N::declined()), "la renuncia se escribe, no se omite");
        }
    }

    /// Y sobre un adquirido pide `--force`, cambie o no la firma; con él, lo baja.
    #[test]
    fn declining_an_acquired_level_needs_force() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("h", Some("viejo"));
        let e = resolve_n1(&cap, &r, Some(&p), &None, decline(), "h").unwrap_err();
        assert!(e.to_string().contains("--force"), "{e}");
        let what = What { decline_n1: true, force: true, ..What::default() };
        assert_eq!(resolve_n1(&cap, &r, Some(&p), &None, what, "otro").unwrap(), Some(N::declined()));
    }

    /// **El aviso es preciso o es ruido.** Sobre algo sin firma resoluble no hay nada
    /// que avisar: ahí la ausencia de `n` ya era la correcta.
    #[test]
    fn without_a_resolvable_signature_there_is_nothing_to_warn_about() {
        let (_d, cap, r) = layer(SIN_FIRMA);
        let o = resolve_n1(&cap, &r, None, &None, What::default(), "h").unwrap();
        assert_eq!(o, None, "no tener firma no es haber renunciado");
    }

    /// Con `--place` el contenido no se toca, y el vecindario es del contenido.
    #[test]
    fn place_only_never_touches_the_neighbourhood() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("h", Some("viejo"));
        let o = resolve_n1(&cap, &r, Some(&p), &None, What::place_only(), "otro").unwrap();
        assert_eq!(adquirido(&o), Some("viejo"));
    }
}
