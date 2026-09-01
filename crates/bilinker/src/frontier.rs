//! La frontera entre proyectos: resolver un alias, y leer del proveedor lo mínimo.
//!
//! Un endpoint `repo <alias>` apunta a un bilink de **otro proyecto**. Lo que este
//! repo guarda de él son dos SHA-256 opacos y el alias; lo que el proveedor guarda
//! de este repo es nada.
//!
//! **Nada de acá hace red.** `check` es masivo y corre sobre todos los bilinks, así
//! que no puede clonar ni fetchear como efecto colateral: un clon ausente se reporta
//! `REMOTE_UNREACHABLE` y se sigue. Traer un repo ajeno es un acto explícito de
//! [`clone_provider`], que sí la hace y a la que sólo llegan comandos puntuales.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use bilink_format::{BiLink, LinkEndpoint};

/// Lo que un consumidor declara de un proveedor. **Un archivo por proveedor, y el
/// único lugar del consumidor que sabe algo del otro repo.**
///
/// No tiene campo `sparse`: para un repo ajeno el conjunto sale de los bilinks que
/// cruzan la frontera, y declararlo sería meter un derivado en un archivo de
/// declaración — además de quedar viejo con el primer vínculo nuevo.
#[derive(Debug, Clone, Deserialize)]
pub struct Provider {
    /// La URL del repo. Vive acá y **nunca en un `.bilink`**: si el proveedor cambia
    /// de host se edita un archivo, no N bilinks.
    pub remote: String,
    /// La rama **del proyecto**. La traducción a `refs/bilink/<branch>` la hace la
    /// herramienta: una sola fuente de verdad, y nadie tipeando namespaces de refs.
    pub branch: String,
}

impl Provider {
    /// `.bilink/.{alias}.toml` — un dotfile que describe a su hermano sin punto.
    ///
    /// **En `.bilink/` y no en `.stratum/`**: un proveedor externo no es una capa
    /// inferior del consumidor, y declararlo bajo `.stratum/` diría que sí.
    pub fn declaration_path(layer: &Path, alias: &str) -> PathBuf {
        layer.join(".bilink").join(format!(".{alias}.toml"))
    }

    /// Dónde vive el clon: al lado de su declaración, y gitignoreado.
    pub fn clone_path(layer: &Path, alias: &str) -> PathBuf {
        layer.join(".bilink").join(alias)
    }

    pub fn load(layer: &Path, alias: &str) -> Result<Self> {
        let path = Self::declaration_path(layer, alias);
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "el alias '{alias}' no está declarado: falta {}",
                path.display()
            )
        })?;
        toml::from_str(&text).with_context(|| format!("parseando {}", path.display()))
    }

    /// La ref de bilinks que le corresponde a la rama declarada.
    pub fn ref_name(&self) -> String {
        format!("refs/bilink/{}", self.branch)
    }
}

/// Lo que el consumidor necesita saber del proveedor para un bilink, y nada más.
pub struct RemoteView {
    /// El `accepted` del endpoint estructural del bilink remoto: los dos valores
    /// que este repo copia.
    pub accepted: Option<bilink_format::Accepted>,
    /// Si la otra punta del bilink remoto **sigue siendo `abstract`**.
    ///
    /// Es la segunda lectura, y es un hecho distinto de que el fragmento cambió:
    /// mezclarlos en el mismo token perdería cuál de los dos pasó.
    pub still_abstract: bool,
}

/// Qué se puede decir del proveedor sin hacer red.
pub enum Resolution {
    Found(RemoteView),
    /// El clon no está. **No se clona acá**: `check` no hace red.
    NotCloned,
    /// El clon está y el bilink del UUID no. Es una regresión, no una ausencia.
    BilinkGone,
}

/// Resuelve un endpoint repo contra el clon local del proveedor, **sin red**.
///
/// El clon lleva el árbol del proyecto más `.bilink/` —es un checkout de la ref—
/// así que las declaraciones del proveedor y el código al que apuntan vienen
/// coherentes por construcción, en un solo fetch.
pub fn resolve(layer: &Path, alias: &str, uuid: &str) -> Result<Resolution> {
    let clone = Provider::clone_path(layer, alias);
    if !clone.join(".bilink").is_dir() {
        return Ok(Resolution::NotCloned);
    }

    verify_format_version(&clone, alias)?;

    let remote_path = BiLink::path_in(&clone, uuid);
    let Ok(remote) = BiLink::load(&remote_path) else {
        return Ok(Resolution::BilinkGone);
    };

    // Dos lecturas, dos hechos, cero normalización.
    let still_abstract = [0u8, 1u8]
        .iter()
        .any(|n| matches!(remote.endpoint.get(*n).link, LinkEndpoint::Abstract));

    Ok(Resolution::Found(RemoteView {
        accepted: remote.structural_accepted().cloned(),
        still_abstract,
    }))
}

/// El consumidor se niega si no entiende el formato del proveedor.
///
/// **No devuelve un estado, corta.** Una versión que no se entiende no es drift: es
/// no poder leer los archivos, y reportar cualquier estado sobre eso sería inventar.
///
/// Es la razón de fondo de que `.bilink/version` exista además del ledger. Dentro de
/// un proyecto formato y binario se mueven juntos; entre proyectos son repos con
/// ciclos de release independientes, así que **la divergencia de versiones es lo
/// normal, no un accidente**.
pub fn verify_format_version(clone: &Path, alias: &str) -> Result<()> {
    let Some(theirs) = bilink_format::read_version(clone) else {
        // Sin archivo es formato 1 — anterior a que el archivo existiera. Un
        // proveedor así no publica abstracciones todavía.
        bail!(
            "el proveedor '{alias}' no declara versión de formato: es anterior a la \
             frontera.\n  No se puede interpretar lo que publica."
        );
    };
    let ours = bilink_format::VERSION;

    if major(&theirs) != major(ours) {
        bail!(
            "el proveedor '{alias}' publica formato {theirs} y este binario lee {ours}.\n  \
             No se interpreta lo que no se entiende: actualizar bilinker, o fijar el \
             `.toml` a una rama del proveedor que use un formato compatible."
        );
    }
    Ok(())
}

fn major(v: &str) -> &str {
    v.split('.').next().unwrap_or(v)
}

/// Los archivos del proveedor que este repo necesita: **derivado de los bilinks**.
///
/// No se persiste ni se declara. Git ya lo guarda en el clon —que además está
/// gitignoreado— y es incremental por naturaleza: sumar un vínculo de frontera
/// agrega un archivo, sacarlo lo quita. Un conjunto fijo en el `.toml` quedaría
/// desactualizado con el primer bilink nuevo.
///
/// Hacen falta los archivos y no sólo los `.bilink` porque **detectar el drift y
/// entenderlo son cosas distintas**: los valores aceptados dicen que algo cambió;
/// para mirar el fragmento y decidir si se acepta hay que tener el archivo.
pub fn sparse_set(layer: &Path, alias: &str) -> Result<Vec<String>> {
    let clone = Provider::clone_path(layer, alias);
    let mut files = std::collections::BTreeSet::new();

    for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let points_here = [0u8, 1u8]
            .iter()
            .any(|n| bl.endpoint.get(*n).link.repo_alias() == Some(alias));
        if !points_here {
            continue;
        }
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Del bilink remoto a su capture, y del capture a su archivo.
        let Ok(remote) = BiLink::load(&BiLink::path_in(&clone, uuid)) else { continue };
        for n in [0u8, 1u8] {
            if let Some(id) = remote.endpoint.get(n).link.capture_id() {
                if let Ok(cap) = bilink_format::Capture::load_in(&clone, id) {
                    files.insert(cap.file);
                }
            }
        }
    }
    Ok(files.into_iter().collect())
}

/// Recorre la ref del proveedor hacia atrás hasta la versión de su bilink cuyo
/// `accepted` coincide con el que este repo copió, **profundizando a demanda**.
///
/// El clon arranca superficial: el árbol actual y nada de historia. Alcanza para
/// `check`, que corre sobre todo. Acá se paga la profundidad, y sólo para un bilink
/// — que es el reparto correcto: **`check` es masivo y barato; ver el diff es
/// puntual y caro.**
///
/// Devuelve el commit del proveedor donde eso valía. Ningún commit suyo se guardó
/// nunca de este lado: se descubre.
pub fn deepen_until_accepted(
    clone: &Path,
    provider: &Provider,
    uuid: &str,
    accepted: &bilink_format::Accepted,
) -> Result<Option<String>> {
    /// Cuánta historia se trae por vuelta. Se profundiza en pasos y no de una,
    /// porque el caso común —el proveedor aceptó hace poco— termina en la primera.
    const PASO: usize = 50;
    const TECHO: usize = 500;

    let rel = format!(".bilink/{uuid}.yaml");
    let mut traidos = 0usize;

    loop {
        if let Some(c) = find_in_history(clone, &rel, accepted)? {
            return Ok(Some(c));
        }
        if traidos >= TECHO {
            return Ok(None);
        }
        traidos += PASO;
        let deepened = git(clone, &["fetch", "--deepen", &PASO.to_string(),
                                    "origin", &provider.ref_name()]).is_ok();
        if !deepened {
            // Sin más historia que traer —o sin red— se contesta "no lo encontré" y
            // quien preguntó degrada, en vez de fallar.
            return Ok(None);
        }
    }
}

fn find_in_history(
    clone: &Path, rel: &str, accepted: &bilink_format::Accepted,
) -> Result<Option<String>> {
    let log = git(clone, &["log", "--format=%H", "--", rel])?;
    for commit in log.lines() {
        let Ok(text) = git(clone, &["show", &format!("{commit}:{rel}")]) else { continue };
        let Ok(bl) = serde_yaml_ng::from_str::<BiLink>(&text) else { continue };
        if bl.structural_accepted() == Some(accepted) {
            return Ok(Some(commit.to_string()));
        }
    }
    Ok(None)
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("corriendo git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!("git {} falló: {}", args.join(" "),
              String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Trae el repo de un proveedor: **la única función de acá que hace red.**
///
/// Es un acto explícito, y por eso vive en un comando propio y no adentro de
/// `check`. El clon arranca **superficial** —el árbol de la rama declarada, sin
/// historia— porque alcanza para `check`, que corre sobre todo; la historia se paga
/// después y sólo donde alguien mira un diff.
///
/// Y se trae **una sola ref**: la de bilinks, que lleva el árbol del proyecto más
/// `.bilink/`. De ahí salen las declaraciones del proveedor y el código al que
/// apuntan, coherentes por construcción — no hay que traer dos refs y confiar en que
/// se correspondan.
pub fn fetch(layer: &Path, alias: &str) -> Result<FetchReport> {
    let provider = Provider::load(layer, alias)?;
    let clone = Provider::clone_path(layer, alias);
    let refname = provider.ref_name();

    if !clone.join(".git").exists() {
        std::fs::create_dir_all(&clone)?;
        git(&clone, &["init", "-q"])?;
        git(&clone, &["remote", "add", "origin", &provider.remote])?;
        // Sparse desde el principio: el paso siguiente lo amplía a lo que haga falta.
        git(&clone, &["config", "core.sparseCheckout", "true"])?;
        git(&clone, &["sparse-checkout", "set", "--no-cone", ".bilink/"])?;
    }

    // El `+` del refspec no es opcional con un clon superficial: git no tiene la
    // historia para probar que el tip nuevo desciende del viejo, así que sin él
    // rechaza como non-fast-forward un avance perfectamente legítimo.
    //
    // **Es seguro porque la ref es append-only por diseño**: nunca se rebasea ni se
    // cherry-pickea, así que un avance siempre lo es de verdad. Es el mismo `+` que
    // `bilinker init` deja en el refspec del propio repo.
    git(&clone, &["fetch", "--depth", "1", "origin", &format!("+{refname}:{refname}")])
        .with_context(|| format!(
            "no se pudo traer {refname} de '{alias}'.\n  \
             ¿El proveedor ya cortó a la ref? Sin `refs/bilink/<branch>` no hay nada que consumir."
        ))?;
    git(&clone, &["checkout", "-q", "--force", &refname])?;

    verify_format_version(&clone, alias)?;

    // El sparse **se calcula ahora**, con los `.bilink` ya en mano: es lo que hace
    // que sea derivado e incremental en vez de una lista que envejece.
    let files = sparse_set(layer, alias)?;
    if !files.is_empty() {
        let mut args = vec!["sparse-checkout".to_string(), "set".into(), "--no-cone".into(),
                            ".bilink/".into()];
        args.extend(files.iter().cloned());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git(&clone, &refs)?;
        git(&clone, &["checkout", "-q", "--force", &refname])?;
    }

    Ok(FetchReport { alias: alias.to_string(), branch: provider.branch, files: files.len() })
}

pub struct FetchReport {
    pub alias:  String,
    pub branch: String,
    pub files:  usize,
}

/// Los alias declarados en esta capa, por sus `.bilink/.{alias}.toml`.
pub fn declared_aliases(layer: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(layer.join(".bilink")) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix('.')?.strip_suffix(".toml").map(str::to_string)
        })
        .collect();
    out.sort();
    out
}

/// Una abstracción publicada, con su código.
pub struct Abstraction {
    pub uuid: String,
    /// El `name` de la punta `abstract`, si lo tiene. Hoy es una etiqueta libre.
    pub name: Option<String>,
    pub file: String,
    /// El fragmento. `None` si el capture no resuelve contra esa versión.
    pub text: Option<String>,
    /// Si esta capa ya se cuelga de ella. Sólo tiene sentido mirando a un
    /// proveedor: sirve para no volver a colgarse de lo mismo.
    pub consumed: bool,
}

/// Qué publica un proveedor, con el código de cada fragmento.
///
/// **Es el paso que falta antes de `chain new`**: para colgarse de algo hay que
/// poder ver de qué. Sin esto, la lista de abstracciones viaja por chat.
///
/// **No amplía el sparse-checkout ni trae nada.** El clon recorta el *árbol de
/// trabajo*, no el object store: los blobs del commit traído están todos, así que
/// el fragmento se lee con `git show` aunque el archivo no esté en disco. Mirar el
/// catálogo no cuesta ni un byte de red, y no ensucia el conjunto de archivos que
/// este repo declara necesitar — que es derivado de lo que consume, no de lo que
/// miró.
pub fn abstracts(layer: &Path, alias: &str) -> Result<Vec<Abstraction>> {
    let clone = Provider::clone_path(layer, alias);
    if !clone.join(".bilink").is_dir() {
        bail!("el repo '{alias}' no está clonado. Traerlo primero: `bilinker fetch {alias}`.");
    }
    verify_format_version(&clone, alias)?;

    let provider = Provider::load(layer, alias)?;
    let tip = git(&clone, &["rev-parse", &provider.ref_name()])?.trim().to_string();
    let mine = consumed_uuids(layer, alias);

    let mut out = Vec::new();
    for path in bilink_format::bilink::bilink_files(&clone.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Sólo las que están abiertas a que alguien las consuma.
        let open = [0u8, 1u8].iter().find(|n| bl.endpoint.get(**n).link.is_abstract());
        let Some(&open_n) = open else { continue };

        let Some(id) = [0u8, 1u8].iter().find_map(|n| bl.endpoint.get(*n).link.capture_id())
        else { continue };
        let Ok(cap) = bilink_format::Capture::load_in(&clone, id) else { continue };

        out.push(Abstraction {
            uuid: uuid.to_string(),
            name: bl.endpoint.get(open_n).name.clone(),
            file: cap.file.clone(),
            text: crate::capture::accepted_text(&clone, &cap, &tip, None),
            consumed: mine.contains(uuid),
        });
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then(a.uuid.cmp(&b.uuid)));
    Ok(out)
}

/// Lo que **esta** capa publica, leído del árbol de trabajo.
///
/// La misma pregunta desde el otro lado: *¿qué estoy publicando?* Y a diferencia del
/// catálogo de un proveedor, acá el fragmento sale del disco — es tu código.
pub fn published(layer: &Path) -> Result<Vec<Abstraction>> {
    let mut out = Vec::new();
    for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        let Some(&open_n) = [0u8, 1u8].iter().find(|n| bl.endpoint.get(**n).link.is_abstract())
        else { continue };
        let Some(id) = [0u8, 1u8].iter().find_map(|n| bl.endpoint.get(*n).link.capture_id())
        else { continue };
        let Ok(cap) = bilink_format::Capture::load_in(layer, id) else { continue };

        let text = crate::capture::absolute_range(layer, &cap).ok().flatten().and_then(|r| {
            let src = std::fs::read_to_string(layer.join(&cap.file)).ok()?;
            Some(r.text(&src))
        });

        out.push(Abstraction {
            uuid: uuid.to_string(),
            name: bl.endpoint.get(open_n).name.clone(),
            file: cap.file.clone(),
            text,
            consumed: false,
        });
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then(a.uuid.cmp(&b.uuid)));
    Ok(out)
}

/// Los uuids que esta capa ya consume de un alias.
fn consumed_uuids(layer: &Path, alias: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        if [0u8, 1u8].iter().any(|n| bl.endpoint.get(*n).link.repo_alias() == Some(alias)) {
            if let Some(u) = path.file_stem().and_then(|s| s.to_str()) {
                out.insert(u.to_string());
            }
        }
    }
    out
}
