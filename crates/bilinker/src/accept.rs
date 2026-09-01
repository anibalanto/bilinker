//! `bilinker accept` — el único que escribe una decisión.
//!
//! Aceptar es decir *"revisé esto y lo apruebo"*, y hay **dos cosas que aprobar**:
//! dónde está el fragmento y qué dice. Se pueden aprobar juntas o por separado.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use bilink_format::bilink::bilink_files;
use bilink_format::{Accepted, BiLink, Capture, LinkEndpoint};

use crate::cache::Cache;
use crate::state::EndpointState;
use crate::{grammar, hash, query};

/// Qué dimensiones aprueba esta aceptación.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct What {
    pub place:   bool,
    pub content: bool,
}

impl Default for What {
    /// Por defecto se aprueban las dos.
    fn default() -> Self { Self { place: true, content: true } }
}

impl What {
    pub fn place_only()   -> Self { Self { place: true,  content: false } }
    pub fn content_only() -> Self { Self { place: false, content: true } }
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
pub fn accept(layer: &Path, uuid: &str, n: u8, what: What) -> Result<AcceptResult> {
    let path = find_bilink_path(layer, uuid)?;
    let uuid = path.file_stem().and_then(|s| s.to_str())
        .context("el nombre del bilink no es un uuid")?.to_string();

    let mut bl = BiLink::load(&path)?;
    let mut cache = Cache::load(layer);

    let (mut accepted, commit) = compute(layer, &bl, &uuid, n, what, &cache)?;
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
    let previous = bl.endpoint.get(n).accepted.as_ref();
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
    e.accepted = Some(accepted);
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

/// Calcula el bloque `accepted` para un endpoint.
fn compute(
    layer: &Path,
    bl: &BiLink,
    uuid: &str,
    n: u8,
    what: What,
    cache: &Cache,
) -> Result<(Accepted, Option<String>)> {
    let e = bl.endpoint.get(n);
    let previous = e.accepted.as_ref();

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
                None    => bl.endpoint.get(n).accepted.is_none(),
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
