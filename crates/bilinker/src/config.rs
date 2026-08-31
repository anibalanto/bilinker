//! Raíz del proyecto, y lo único que bilinker configura: dos líneas por clon.
//!
//! No existe ningún archivo de configuración de bilinker. Lo que sí es por clon
//! —y no viaja con un `git clone`— son la exclusión en `.git/info/exclude` y el
//! refspec en `.git/config`. Las pone [`init`], y por eso `init` es explícito:
//! **bilinker arregla solo lo que es suyo, y pide lo que es del repo del usuario.**
//!
//! [`init`]: crate::init

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Patrones que `init` deja en `.git/info/exclude`.
///
/// En `info/exclude` y no en `.gitignore`: `.gitignore` está versionado, y
/// agregarlo modificaría la rama del proyecto — justo lo que este diseño evita.
pub const EXCLUDE_PATTERNS: [&str; 2] = [".bilink/", ".bilink-migrate-*"];

/// El refspec que hace que `git fetch` traiga las refs de bilinks con las ramas.
///
/// Se mapea a sí mismo y no a `refs/remotes/`: la ref del remoto y la local son la
/// misma cosa.
///
/// **Y va sin `+`.** El `+` significa *"actualizá aunque no sea fast-forward"*, y
/// acá es exactamente lo que no se quiere: como la ref sólo crece, en operación
/// normal el fetch ya es fast-forward y el `+` no aporta nada. Lo único que agrega
/// es que, si el remoto divergió, el fetch **pisa la ref local en silencio** — y lo
/// que se pierde no es un valor sino **un padre**: el commit propio queda sin
/// referencia, y el commit de sincronización que lo uniría necesita dos.
pub const REFSPEC: &str = "refs/bilink/*:refs/bilink/*";

/// El que escribían las versiones anteriores, y que hay que sacar de un clon que ya
/// lo tiene. Poner el nuevo no alcanza: los dos refspecs conviven y el `+` gana.
pub const REFSPEC_FORZADO: &str = "+refs/bilink/*:refs/bilink/*";

#[derive(Debug, Default)]
pub struct Config {}

impl Config {
    /// Find the project root by walking up from `dir` looking for `.bilink/`,
    /// then falling back to the git root (`.git/`).
    pub fn load_from(dir: &Path) -> Result<(PathBuf, Config)> {
        let mut current = dir.to_path_buf();
        loop {
            if current.join(".bilink").is_dir() || current.join(".git").exists() {
                return Ok((current, Config {}));
            }
            if !current.pop() {
                return Ok((dir.to_path_buf(), Config {}));
            }
        }
    }
}

/// Si el clon está puesto a punto para bilinker.
///
/// Las dos piezas que `init` escribe: **el exclude, siempre**, y **el refspec en
/// cada remoto**. Se piden las dos porque cubren casos distintos.
///
/// El refspec es la que no puede estar por accidente —un `.bilink/` en el árbol
/// puede venir de antes del corte, y el exclude lo pudo escribir alguien a mano—
/// pero **no existe en un repo sin remoto**, y ahí pedirla dejaría al repo sin forma
/// de estar nunca inicializado: todo comando se negaría para siempre. Un repo local
/// sin origen usa la ref igual, sólo que nunca la empuja.
pub fn is_initialized(repo: &Path) -> bool {
    let excluded = std::fs::read_to_string(git_dir(repo).unwrap_or_default().join("info/exclude"))
        .map(|text| {
            EXCLUDE_PATTERNS.iter().all(|p| text.lines().any(|l| l.trim() == *p))
        })
        .unwrap_or(false);

    // **Cualquiera de los dos refspecs cuenta como inicializado**, incluido el
    // forzado que escribían las versiones anteriores. Un clon que corrió el `init`
    // viejo trae bilinks igual: lo que le falta es la protección, no la puesta a
    // punto. Bloquear todos sus comandos por eso sería cobrarle a quien no hizo
    // nada mal un cambio que `init` repara solo.
    excluded
        && remotes(repo)
            .map(|rs| rs.iter().all(|r| has_refspec(repo, r) || has_forced_refspec(repo, r)))
            .unwrap_or(false)
}

/// El id del corte a la ref en el ledger de migraciones.
pub const REF_CUTOVER: &str = "bilinker-005-ref-cutover";

/// Si este repo ya movió sus bilinks a la ref.
///
/// **Lo dice el ledger y no el filesystem**, y ésa es la diferencia que importa: el
/// ledger está commiteado, así que un clon fresco de un repo que cortó lo sabe antes
/// de tener una sola `refs/bilink/*` local — que es exactamente el caso en el que
/// hace falta exigir `init`. Mirar si hay refs daría la respuesta contraria justo
/// ahí, y el clon seguiría de largo sin bilinks y sin decir nada.
///
/// Se lee el archivo directo en vez de depender del runner de migraciones: es una
/// lista de ids, una por línea, y bilinker no tiene por qué depender de quien las
/// aplica.
pub fn has_cut_over(repo: &Path) -> bool {
    std::fs::read_to_string(repo.join(".accreta").join("migrations"))
        .map(|text| text.lines().any(|l| l.trim() == REF_CUTOVER))
        .unwrap_or(false)
}

/// El error que ve quien no corrió `init`.
///
/// Que un comando de lectura configurara el repo de callado sería peor que fallar:
/// **bilinker arregla solo lo que es suyo, y pide lo que es del repo del usuario.**
///
/// Sólo se exige en un repo que ya cortó. Antes del corte los bilinks viven en la
/// rama, no hace falta ni exclude ni refspec, y exigirlos rompería todos los repos
/// que todavía no cortaron — incluida la herramienta con la que se corta.
pub fn require_initialized(repo: &Path) -> Result<()> {
    if !has_cut_over(repo) || is_initialized(repo) {
        return Ok(());
    }
    anyhow::bail!("el repo no está inicializado para bilinker.\n  Correr `bilinker init`.")
}

/// Agrega a `.git/info/exclude` los patrones que falten. No toca lo demás.
///
/// Devuelve los que agregó — vacío si ya estaban, que es el caso de un `init`
/// repetido.
pub fn write_exclude(repo: &Path, dry_run: bool) -> Result<Vec<&'static str>> {
    let path = git_dir(repo)?.join("info").join("exclude");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let missing: Vec<&str> = EXCLUDE_PATTERNS
        .iter()
        .copied()
        .filter(|p| !existing.lines().any(|l| l.trim() == *p))
        .collect();

    if missing.is_empty() || dry_run {
        return Ok(missing);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.contains("# bilinker") {
        out.push_str("\n# bilinker — los bilinks viven en refs/bilink/*, no en la rama\n");
    }
    for p in &missing {
        out.push_str(p);
        out.push('\n');
    }
    std::fs::write(&path, out).with_context(|| format!("escribiendo {}", path.display()))?;
    Ok(missing)
}

/// Agrega el refspec de `refs/bilink/*` a todos los remotos que no lo tengan.
///
/// Devuelve los remotos que tocó. Sin remotos no hay nada que escribir y no es un
/// error: un repo local sin origen igual usa la ref, sólo que nunca la empuja.
pub fn write_refspec(repo: &Path, dry_run: bool) -> Result<Vec<String>> {
    let pendientes: Vec<String> = remotes(repo)?
        .into_iter()
        .filter(|r| !has_refspec(repo, r) || has_forced_refspec(repo, r))
        .collect();

    if dry_run {
        return Ok(pendientes);
    }
    for remote in &pendientes {
        // **Primero se saca el forzado.** Agregar el nuevo sin sacarlo dejaría los
        // dos, y git aplica los dos: el `+` seguiría pisando la ref local. Un clon
        // que ya corrió un `init` viejo se arregla corriendo `init` de nuevo.
        if has_forced_refspec(repo, remote) {
            git(repo, &["config", "--unset-all", &format!("remote.{remote}.fetch"),
                        &regex_of(REFSPEC_FORZADO)])?;
        }
        if !has_refspec(repo, remote) {
            git(repo, &["config", "--add", &format!("remote.{remote}.fetch"), REFSPEC])?;
        }
    }
    Ok(pendientes)
}

/// El refspec como regex de `git config --unset-all`, que matchea el **valor**.
fn regex_of(refspec: &str) -> String {
    let escapado: String = refspec.chars()
        .map(|c| if "+*.^$[]()|\\".contains(c) { format!("\\{c}") } else { c.to_string() })
        .collect();
    format!("^{escapado}$")
}

pub fn remotes(repo: &Path) -> Result<Vec<String>> {
    let out = git(repo, &["remote"])?;
    Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

fn has_refspec(repo: &Path, remote: &str) -> bool {
    tiene(repo, remote, REFSPEC)
}

/// Si el remoto todavía lleva el refspec con `+` que escribían las versiones
/// anteriores. Un clon así fetchea forzando aunque el nuevo también esté puesto.
pub fn has_forced_refspec(repo: &Path, remote: &str) -> bool {
    tiene(repo, remote, REFSPEC_FORZADO)
}

fn tiene(repo: &Path, remote: &str, refspec: &str) -> bool {
    git(repo, &["config", "--get-all", &format!("remote.{remote}.fetch")])
        .map(|out| out.lines().any(|l| l.trim() == refspec))
        .unwrap_or(false)
}

/// El `.git/` del repo — un directorio, o el archivo que apunta a uno en un worktree.
pub fn git_dir(repo: &Path) -> Result<PathBuf> {
    let out = git(repo, &["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(out.trim()))
}

/// La raíz del repo git. Distinta de la raíz de la capa: el exclude y el refspec
/// son **por repo**, y un solo patrón `.bilink/` cubre todas sus capas.
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(out.trim()))
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("corriendo git {}", args.join(" ")))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} falló: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
