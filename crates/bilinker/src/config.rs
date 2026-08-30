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
/// misma cosa, y como la ref es append-only el fetch es siempre fast-forward.
pub const REFSPEC: &str = "+refs/bilink/*:refs/bilink/*";

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
/// **La detección es por el refspec**, no por el exclude ni por el `.bilink/` del
/// árbol: es la única pieza que no puede estar por accidente. Un `.bilink/` puede
/// venir de antes del corte, y el exclude lo pudo escribir alguien a mano.
pub fn is_initialized(repo: &Path) -> bool {
    remotes(repo)
        .map(|rs| !rs.is_empty() && rs.iter().all(|r| has_refspec(repo, r)))
        .unwrap_or(false)
}

/// El error que ve quien no corrió `init`.
///
/// Que un comando de lectura configurara el repo de callado sería peor que fallar.
pub fn require_initialized(repo: &Path) -> Result<()> {
    if is_initialized(repo) {
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
    let missing: Vec<String> = remotes(repo)?
        .into_iter()
        .filter(|r| !has_refspec(repo, r))
        .collect();

    if dry_run {
        return Ok(missing);
    }
    for remote in &missing {
        git(repo, &["config", "--add", &format!("remote.{remote}.fetch"), REFSPEC])?;
    }
    Ok(missing)
}

pub fn remotes(repo: &Path) -> Result<Vec<String>> {
    let out = git(repo, &["remote"])?;
    Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

fn has_refspec(repo: &Path, remote: &str) -> bool {
    git(repo, &["config", "--get-all", &format!("remote.{remote}.fetch")])
        .map(|out| out.lines().any(|l| l.trim() == REFSPEC))
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
