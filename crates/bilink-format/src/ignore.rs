//! `.bilink/.gitignore` — qué de `.bilink/` no se versiona.
//!
//! `cache/` e `index/` son derivados: se recalculan resolviendo, y versionarlos
//! produce conflictos de merge sobre archivos que nadie escribió a mano.
//!
//! La regla vive **adentro** de `.bilink/` y no en el `.gitignore` del repo, ni en
//! `.git/info/exclude`. Adentro viaja con el directorio que gobierna: una capa
//! nueva en un repo cualquiera trae su regla puesta, y un clon fresco la tiene sin
//! que nadie la configure. `info/exclude` es por clon —el segundo desarrollador no
//! la hereda— y el `.gitignore` del repo obligaría a una entrada por capa.
//!
//! La escriben los mismos comandos que crean `cache/` e `index/`: el directorio no
//! puede existir sin su regla. Y va de la mano de `ensure_version`, por lo mismo:
//! un `.bilink/` recién creado tiene que declarar qué formato son sus archivos, o
//! del otro lado de la frontera es indistinguible de uno anterior al campo.

use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

pub const IGNORE_FILE: &str = ".gitignore";

/// Lo derivable, en el orden en que aparece en el archivo.
pub const IGNORED: &[&str] = &["cache/", "index/"];

pub fn ignore_path(layer: &Path) -> PathBuf {
    layer.join(".bilink").join(IGNORE_FILE)
}

/// Escribe la regla si falta. No toca un archivo que ya la tenga entera: puede
/// llevar agregados de quien use la capa, y no son nuestros para borrar.
pub fn write_ignore(layer: &Path) -> Result<()> {
    let path = ignore_path(layer);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if IGNORED.iter().all(|p| current.lines().any(|l| l.trim() == *p)) {
        return Ok(());
    }

    let mut out = current;
    for pat in IGNORED {
        if !out.lines().any(|l| l.trim() == *pat) {
            if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
            out.push_str(pat);
            out.push('\n');
        }
    }
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(&path, out).with_context(|| format!("escribiendo {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writes_both_rules() {
        let d = tempdir().unwrap();
        write_ignore(d.path()).unwrap();
        let s = std::fs::read_to_string(ignore_path(d.path())).unwrap();
        assert_eq!(s, "cache/\nindex/\n");
    }

    #[test]
    fn is_idempotent() {
        let d = tempdir().unwrap();
        write_ignore(d.path()).unwrap();
        write_ignore(d.path()).unwrap();
        let s = std::fs::read_to_string(ignore_path(d.path())).unwrap();
        assert_eq!(s, "cache/\nindex/\n");
    }

    /// Lo que alguien haya agregado a mano se conserva.
    #[test]
    fn preserves_foreign_entries() {
        let d = tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".bilink")).unwrap();
        std::fs::write(ignore_path(d.path()), "scratch/\n").unwrap();
        write_ignore(d.path()).unwrap();
        let s = std::fs::read_to_string(ignore_path(d.path())).unwrap();
        assert_eq!(s, "scratch/\ncache/\nindex/\n");
    }
}
