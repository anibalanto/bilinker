//! `bilinker accept` — el único que escribe una decisión.
//!
//! Aceptar es decir *"revisé esto y lo apruebo"*, y hay **dos cosas que aprobar**:
//! dónde está el fragmento y qué dice. Se pueden aprobar juntas o por separado.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{Accepted, BiLink, Capture, LinkEndpoint, N};

use crate::cache::Cache;
use crate::state::EndpointState;
use crate::{grammar, hash, query};

/// Qué dimensiones aprueba esta aceptación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct What {
    pub place:   bool,
    pub content: bool,
    /// Aceptar renunciando al [vecindario](crate::neighbours): escribe
    /// `n1: declined` en vez de los folds.
    ///
    /// Está acá y no en un parámetro aparte porque **el vecindario es una tercera
    /// dimensión** y se comporta como las otras dos: se aprueba, o no se toca.
    ///
    /// **Renuncia del nivel 1 para arriba, no al nivel 1.** El día que exista un
    /// nivel 2 —los campos de los tipos que el 1 resuelve— queda adentro de esta
    /// misma renuncia, porque está definido a través del 1. No va a haber un
    /// `no_n2`: el nombre marca dónde empieza lo que pide un language server.
    pub no_n1:   bool,
    /// Sólo junto a `no_n1`, y sólo donde éste **baja** una cobertura que ya estaba.
    ///
    /// Escalonado a propósito: `--no-n1` en una persona se tipea una vez, en un CI
    /// se escribe una vez y queda para siempre. Sin el escalón, esa línea de
    /// configuración sería una autorización permanente a bajar cobertura.
    pub force:   bool,
}

impl Default for What {
    /// Por defecto se aprueban las dos.
    fn default() -> Self { Self { place: true, content: true, no_n1: false, force: false } }
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

    // **Quiénes aprobaron *estos* valores.**
    //
    // El set anterior sobrevive sólo si los valores no se movieron: quien aprobó el
    // hash de antes no aprobó el de ahora, y arrastrar su nombre sería atribuirle
    // una decisión que no tomó. Y arranca vacío también cuando `compute` trajo el
    // `accepted` de un vecino —un endpoint `path` o `repo`— porque **`agree` no se
    // copia**: los que aprobaron allá aprobaron ese fragmento, no esta copia.
    // **La primera entrada, provisoriamente.** Escribir en la lista —colapsar las
    // que aprobaban otros valores, unirse al `agree` de la que coincide— es la task
    // `3u`. Acá sólo se mantiene el comportamiento de una sola decisión, que es lo
    // que había, para que el cambio de tipo no arrastre semántica.
    let previous = bl.endpoint.get(n).accepted.first();
    let iguales = previous.map(|p| p.same_values(&accepted)).unwrap_or(false);
    accepted.agree = match previous {
        Some(p) if iguales => p.agree.clone(),
        _ => BTreeSet::new(),
    };
    let sumado = accepted.agree.insert(signer(layer)?);
    let wrote = sumado || !iguales;

    let e = bl.endpoint.get_mut(n);
    let hash = accepted.hash.clone();
    let agree = accepted.agree.clone();
    e.accepted = vec![accepted];
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

/// **Una falla de infraestructura no puede reducir la cobertura de un vínculo.**
///
/// Ver `concepts/accept.md` § "Cuándo se adquiere el vecindario" para las siete
/// combinaciones. Las dos que fallan son las únicas que `--no-n1` destraba, y la
/// única que *baja* algo pide además `--force`.
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

    // Se resolvió: se escribe el vecindario entero, y una renuncia anterior se
    // levanta. Los dos folds ya no se pueden mezclar — viven en el mismo objeto.
    if let Some(f) = folded {
        return Ok(Some(N::of_level_1(f.clone())));
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
            if what.no_n1 { return Ok(Some(N::declined())); }
            bail!(
                "el fragmento de {} {que}, y su vecindario no se puede alcanzar: el \
                 nivel 1 sale de una firma, y ahí no hay una que sea la suya.\n       \
                 Capturar el contrato con --as, o renunciar al vecindario con --no-n1.",
                cap.file);
        }

        Reach::At(_) => {}
    }

    // **Una renuncia escrita es una decisión, y se lee de vuelta.**
    //
    // Antes acá había un booleano —"había adquirido"— y un `declined` previo caía en
    // el mismo casillero que no tener nada, así que se volvía a pedir `--no-n1` en
    // cada `accept`. Eso convierte la renuncia en una tecla que se tipea siempre, y
    // un pedido que sale siempre no lo lee nadie: es la misma razón por la que el
    // aviso no sale sobre prosa ni sobre un DTO, un nivel más adentro.
    //
    // No encierra a nadie, y por eso es seguro: el `if let Some(f) = folded` de más
    // arriba levanta la renuncia sola en cuanto hay proveedor. Subir cobertura es
    // automático; bajarla sigue pidiendo que se declare.
    let had = match previous.and_then(|a| a.n.as_ref()) {
        None                       => Had::Nothing,
        Some(n) if n.is_acquired() => Had::Acquired,
        Some(_)                    => Had::Declined,
    };

    // **El conjunto de vecinos lo determina la firma, y la firma está en el
    // fragmento.** Con el capture de contrato el `hash` *es* el de la firma, así que
    // un `hash` quieto es el mismo conjunto. Sobre un capture que arrastra el cuerpo
    // esto sobre-dispara —un refactor adentro cuenta como cambio— y erra hacia
    // pedir que alguien mire, que es el lado correcto para errar.
    let signature_changed = previous.map(|p| p.hash != content_hash).unwrap_or(true);

    match (had, what.no_n1, what.force, signature_changed) {
        // No había vecindario que preservar: aceptar así lo deja sin vigilar, y el
        // baseline no lo diría.
        (Had::Nothing, true, _, _) => Ok(Some(N::declined())),
        (Had::Nothing, false, _, _) => bail!(
            "no hay proveedor de vecindario, y la firma de {} lo tiene.\n       \
             Aceptar así deja los tipos que la firma menciona sin vigilar, y el \
             baseline no lo diría.\n       \
             Levantar lspd, o aceptar sin el nivel 1 con --no-n1.", cap.file),

        // **Ya se decidió, y sin proveedor no hay con qué revisar la decisión.**
        // Preservar acá es lo mismo que preservar un adquirido cuya firma no se
        // movió, y por el mismo motivo. Que la firma haya cambiado no la vuelve
        // falsa: una renuncia no es sobre un conjunto de vecinos, es sobre si se
        // vigilan.
        //
        // Con `--no-n1` da idéntico —escribir la renuncia sobre una renuncia—, así
        // que el flag no cambia nada y no hace falta pedirlo.
        (Had::Declined, _, _, _) => Ok(preserve()),

        // Había, y la firma no se movió: es el mismo conjunto de vecinos. Preservar
        // es estrictamente más seguro que borrar — si alguno cambió con el proveedor
        // caído, el valor viejo sigue ahí y el próximo cierre lo reporta.
        (Had::Acquired, false, _, false) => Ok(preserve()),

        // Había y la firma cambió: preservar sería mentir, porque el conjunto pudo
        // cambiar con ella.
        (Had::Acquired, false, _, true) => bail!(
            "ya hay un vecindario aceptado, y la firma cambió.\n       \
             Preservarlo mentiría: el conjunto de vecinos pudo cambiar con la firma, \
             y sin proveedor no hay con qué reemplazarlo.\n       \
             Levantar lspd, o bajarlo a propósito con --no-n1 --force."),

        // Bajarlo es una decisión, y se pide entera.
        (Had::Acquired, true, true, _) => Ok(Some(N::declined())),
        (Had::Acquired, true, false, _) => bail!(
            "--no-n1 acá baja un vecindario que ya estaba aceptado.\n       \
             Levantar lspd para conservarlo, o bajarlo a propósito con --no-n1 --force."),
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

            // El vecindario, si hay quien lo resuelva y el fragmento tiene uno **al
            // que se llegue**. Las posiciones las pone la gramática: preguntar donde
            // arranca el fragmento devuelve `pub`, que no declara nada.
            let alcance = crate::neighbours::reach(layer, &cap.file, &range);
            let folded = match (what.content, nb, &alcance) {
                (true, Some(p), crate::neighbours::Reach::At(at)) => p.of(layer, &cap.file, at)?
                    .map(|locs| crate::neighbours::fold(layer, &locs))
                    .transpose()?,
                _ => None,
            };
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
                // no tiene firma resoluble. La regla que las gobierna es que una
                // falla del proveedor nunca baja la cobertura.
                n: neighbourhood,
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

            let needs = match cache.endpoint_state(uuid, n) {
                Some(s) => !s.is_ok(),
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

/// Las cinco filas de `concepts/accept.md` § "Cuándo se adquiere el vecindario",
/// más las dos que el corte por gramática deja afuera.
///
/// La regla que las gobierna a todas: **una falla de infraestructura no puede
/// reducir la cobertura de un vínculo.**
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
        }
    }

    fn folded() -> Option<Neighbourhood> {
        Some(Neighbourhood { link: Default::default(), hash: "nuevo".into(), hash_ast: Some("nuevo_ast".into()) })
    }

    /// El hash del vecindario adquirido, si lo hay.
    fn adquirido(o: &Option<N>) -> Option<&str> {
        o.as_ref().and_then(|n| n.level(1)).map(|n| n.hash.as_str())
    }

    /// Fila 1 — no había, se resolvió: se calcula y se escribe.
    #[test]
    fn row_1_resolved_without_a_previous_one_is_written() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let o = resolve_n1(&cap, &r, None, &folded(), What::default(), "h").unwrap();
        assert_eq!(adquirido(&o), Some("nuevo"));
        assert_eq!(o.as_ref().map(|n| n.is_acquired()).unwrap_or(false), true);
    }

    /// Fila 2 — no había y no se pudo mirar: **no se escribe nada**.
    ///
    /// Es la fila que vuelve imposible el baseline mudo: hoy esto salía `all clean`.
    #[test]
    fn row_2_nothing_to_lose_and_no_provider_refuses() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let e = resolve_n1(&cap, &r, None, &None, What::default(), "h").unwrap_err();
        assert!(e.to_string().contains("--no-n1"), "el error tiene que dar la salida: {e}");
    }

    /// Y `--no-n1` la destraba, dejándolo **escrito**.
    #[test]
    fn row_2_declines_explicitly_and_leaves_it_written() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let what = What { no_n1: true, ..What::default() };
        let o = resolve_n1(&cap, &r, None, &None, what, "h").unwrap();
        assert_eq!(o, Some(N::declined()), "la renuncia se escribe, no se omite");
    }

    /// Fila 3 — había y se resolvió: se recalcula, y una renuncia anterior se levanta.
    #[test]
    fn row_3_resolving_again_lifts_a_previous_decline() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let mut p = previo("h", None);
        p.n = Some(N::declined());
        let o = resolve_n1(&cap, &r, Some(&p), &folded(), What::default(), "h").unwrap();
        assert_eq!(adquirido(&o), Some("nuevo"), "volver a tener proveedor recupera la cobertura");
    }

    /// **La renuncia escrita se preserva sin volver a pedir `--no-n1`.**
    ///
    /// Es lo que la vuelve una decisión que se toma una vez. Antes caía en el mismo
    /// casillero que "no hay nada" y abortaba, así que el flag había que tipearlo en
    /// cada `accept` — y un pedido que sale siempre no lo lee nadie.
    #[test]
    fn a_written_decline_is_not_asked_for_again() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let mut p = previo("h", None);
        p.n = Some(N::declined());
        let o = resolve_n1(&cap, &r, Some(&p), &None, What::default(), "h").unwrap();
        assert_eq!(o, Some(N::declined()), "la decisión ya estaba tomada y escrita");
    }

    /// Y tampoco cuando la firma cambió.
    ///
    /// La fila que aborta sobre un vecindario **adquirido** no tiene gemela acá: una
    /// renuncia no es sobre un conjunto de vecinos, así que un conjunto que pudo
    /// cambiar no la vuelve falsa.
    #[test]
    fn a_written_decline_survives_the_signature_changing() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let mut p = previo("otro-hash", None);
        p.n = Some(N::declined());
        let o = resolve_n1(&cap, &r, Some(&p), &None, What::default(), "h").unwrap();
        assert_eq!(o, Some(N::declined()), "la firma no es lo que una renuncia afirma");
    }

    /// Y `--no-n1` sobre una renuncia da lo mismo: no hace falta, y no cambia nada.
    #[test]
    fn declining_over_a_decline_changes_nothing() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let mut p = previo("h", None);
        p.n = Some(N::declined());
        let what = What { no_n1: true, ..What::default() };
        let o = resolve_n1(&cap, &r, Some(&p), &None, what, "h").unwrap();
        assert_eq!(o, Some(N::declined()));
    }

    /// Fila 4 — había, no se pudo mirar, la firma no se movió: **se preserva**.
    ///
    /// Es el caso que contesta *"¿y si una caída de lspd borra lo que ya estaba?"*.
    /// Preservar es estrictamente más seguro que borrar: si un vecino cambió con el
    /// proveedor caído, el valor viejo sigue ahí y el próximo cierre lo reporta.
    #[test]
    fn row_4_a_provider_outage_never_erases_what_was_there() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("h", Some("viejo"));
        let o = resolve_n1(&cap, &r, Some(&p), &None, What::default(), "h").unwrap();
        assert_eq!(adquirido(&o), Some("viejo"), "una caída no puede bajar la cobertura");
    }

    /// Y `--no-n1` tampoco lo baja solo: bajar algo pide el escalón.
    #[test]
    fn row_4_declining_over_an_existing_one_needs_force() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("h", Some("viejo"));
        let what = What { no_n1: true, ..What::default() };
        let e = resolve_n1(&cap, &r, Some(&p), &None, what, "h").unwrap_err();
        assert!(e.to_string().contains("--force"), "{e}");
    }

    /// Fila 5 — había, no se pudo mirar, y la firma cambió: preservar mentiría,
    /// porque el conjunto de vecinos pudo cambiar con ella.
    #[test]
    fn row_5_a_changed_signature_cannot_keep_a_stale_neighbourhood() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("viejo_hash", Some("viejo"));
        let e = resolve_n1(&cap, &r, Some(&p), &None, What::default(), "otro").unwrap_err();
        assert!(e.to_string().contains("--no-n1 --force"), "{e}");
    }

    /// Y con el escalón entero, se baja.
    #[test]
    fn row_5_forced_is_a_decision_and_gets_written() {
        let (_d, cap, r) = layer(CON_FIRMA);
        let p = previo("viejo_hash", Some("viejo"));
        let what = What { no_n1: true, force: true, ..What::default() };
        let o = resolve_n1(&cap, &r, Some(&p), &None, what, "otro").unwrap();
        assert_eq!(o, Some(N::declined()));
    }

    /// **El aviso es preciso o es ruido.** Sobre algo sin firma resoluble no hay nada
    /// que avisar: ahí la ausencia de `n1` ya era la correcta, y un aviso que
    /// sale siempre no lo lee nadie.
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
