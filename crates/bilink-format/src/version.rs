//! `.bilink/version` — qué formato son estos archivos.
//!
//! **Commiteado**, a diferencia de `cache/` e `index/`: describe archivos
//! versionados y viaja con ellos.
//!
//! No es lo mismo que el ledger de migraciones, y las dos cosas hacen falta. El
//! ledger dice qué pasos corrieron en este repo; la versión dice qué son estos
//! archivos. Coinciden casi siempre y divergen justo donde importa: un cambio
//! aditivo no lleva migración pero sí cambia el formato, y el ledger no puede
//! expresar eso porque no hubo migración que registrar.

use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

pub const VERSION_FILE: &str = "version";

pub fn version_path(layer: &Path) -> PathBuf {
    layer.join(".bilink").join(VERSION_FILE)
}

/// La versión declarada por la capa, o `None` si no la declara.
///
/// Una capa sin el archivo es una capa anterior a que el archivo existiera, o sea
/// formato 1. Quien lea eso decide qué hacer; acá no se adivina.
pub fn read_version(layer: &Path) -> Option<String> {
    std::fs::read_to_string(version_path(layer))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn write_version(layer: &Path, version: &str) -> Result<()> {
    let path = version_path(layer);
    if let Some(p) = path.parent() { std::fs::create_dir_all(p)?; }
    std::fs::write(&path, format!("{version}\n"))
        .with_context(|| format!("escribiendo {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_layer_without_the_file_declares_nothing() {
        let dir = tempdir().unwrap();
        assert_eq!(read_version(dir.path()), None);
    }

    #[test]
    fn the_version_round_trips() {
        let dir = tempdir().unwrap();
        write_version(dir.path(), "2.0.0").unwrap();
        assert_eq!(read_version(dir.path()).as_deref(), Some("2.0.0"));
    }
}
