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

/// La declara si falta, con la versión de este binario.
///
/// **La escribe quien crea el `.bilink/`**, en la misma operación — igual que la
/// regla de `.gitignore`. Que sea un paso aparte es lo que la vuelve olvidable, y
/// una capa sin versión es indistinguible de una anterior a que el campo existiera:
/// del otro lado de la frontera, eso significa *"no puedo interpretar lo que
/// publica"*, y sería una capa nacida hoy.
///
/// No pisa una versión ya declarada: puede ser más vieja que este binario a
/// propósito, y decidir eso es de `migrate`, no de quien crea un bilink.
pub fn ensure_version(layer: &Path) -> Result<()> {
    if read_version(layer).is_some() {
        return Ok(());
    }
    write_version(layer, crate::VERSION)
}

/// Una capa que dice ser de un formato que este binario no lee.
///
/// `declared: None` es una capa con `.bilink/` y sin `version`: **anterior a que el
/// campo existiera, o sea formato 1**. No es lo mismo que no haber capa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub declared: Option<String>,
    pub ours: &'static str,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.declared {
            Some(v) => write!(f, "esta capa declara formato {v} y este binario lee {}", self.ours)?,
            None    => write!(f, "esta capa no declara versión de formato: es formato 1, \
                                  anterior a que el campo existiera")?,
        }
        write!(f, "\n  No se interpreta lo que no se entiende: bilinker migrate --recursive")
    }
}

impl std::error::Error for Mismatch {}

/// Se niega si el formato declarado no es de este major.
///
/// **Se pregunta antes de abrir un bilink**, porque un archivo de formato viejo puede
/// parsear bien y significar otra cosa: en `3.3.0` la `query` pasó a poder llevar
/// varios `@target` sin que el tipo ni el archivo cambiaran, así que el parseo no
/// delata nada y la versión es el único dato que discrimina en esa dirección.
///
/// **No pregunta si hay capa.** Que la ausencia de `.bilink/` sea "acá no hay nada
/// que leer" o "no puedo interpretar lo que publicás" depende de quién pregunte, y
/// las dos respuestas son legítimas: adentro es un directorio cualquiera, cruzando
/// la frontera es un proveedor que no se entiende.
pub fn ensure_readable(layer: &Path) -> std::result::Result<(), Mismatch> {
    let declared = read_version(layer);
    let ours = crate::VERSION;
    match &declared {
        Some(v) if major(v) == major(ours) => Ok(()),
        _ => Err(Mismatch { declared, ours }),
    }
}

/// El major de una versión semver, o la cadena entera si no tiene puntos.
///
/// **Es la unidad de compatibilidad**: adentro de un major los cambios son aditivos
/// y con default, así que un parser nuevo lee archivos viejos. Cruzar un major no.
pub fn major(v: &str) -> &str {
    v.split('.').next().unwrap_or(v)
}

/// Sella un `.bilink/` **recién creado**: su `.gitignore` y su `version`.
///
/// Va acá y no en cada comando porque un paso aparte es un paso olvidable, y se
/// olvidó: hasta que alguien comparó la versión, sólo `chain new` la declaraba y
/// `capture` creaba la capa sin ella.
///
/// **Sólo si no hay nada que malinterpretar.** Un `.bilink/` que ya tiene bilinks o
/// captures y no declara versión *es* formato 1: estamparle la versión de hoy sería
/// escribir una respuesta falsa encima de una verdadera, y decidir eso es de
/// `migrate`. Una capa vacía no tiene ese problema — no hay archivo viejo que se
/// pueda leer con el parser nuevo — así que declarar es lo honesto.
///
/// Y la condición **no** puede ser que el directorio no exista: cruzando la frontera
/// el consumidor crea `.bilink/` sólo para poner el `.{alias}.toml`, antes de que
/// exista un solo bilink. Esa capa es nueva, no vieja.
pub fn ensure_layer(layer: &Path) -> Result<()> {
    if read_version(layer).is_some() || has_content(layer) { return Ok(()); }
    crate::write_ignore(layer)?;
    write_version(layer, crate::VERSION)
}

/// Si la capa tiene algún archivo de bilinker escrito en un formato desconocido.
///
/// Los dot-files no cuentan: el `.{alias}.toml` de la frontera y el `.gitignore` no
/// son datos del formato, y ninguno se malinterpreta.
fn has_content(layer: &Path) -> bool {
    let dir = layer.join(".bilink");
    if !crate::bilink::bilink_files(&dir).is_empty() { return true; }
    std::fs::read_dir(dir.join("capture"))
        .map(|mut rd| rd.any(|e| e.is_ok()))
        .unwrap_or(false)
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
    fn creating_a_layer_declares_its_version() {
        let dir = tempdir().unwrap();
        ensure_layer(dir.path()).unwrap();
        assert_eq!(read_version(dir.path()).as_deref(), Some(crate::VERSION));
        assert!(ensure_readable(dir.path()).is_ok());
    }

    #[test]
    fn a_layer_with_content_and_no_version_is_left_alone() {
        let dir = tempdir().unwrap();
        // Tiene un bilink y no declara nada: es formato 1. Sellarla acá sería
        // decidir una migración que nadie corrió.
        std::fs::create_dir_all(dir.path().join(".bilink")).unwrap();
        std::fs::write(dir.path().join(".bilink/aaaa.yaml"), "endpoint: {}\n").unwrap();
        ensure_layer(dir.path()).unwrap();
        assert_eq!(read_version(dir.path()), None);
    }

    #[test]
    fn an_empty_layer_directory_is_still_new() {
        let dir = tempdir().unwrap();
        // Cruzando la frontera el `.bilink/` se crea para el `.toml` del alias,
        // antes del primer bilink. Un dot-file no es contenido del formato.
        std::fs::create_dir_all(dir.path().join(".bilink")).unwrap();
        std::fs::write(dir.path().join(".bilink/.hsi.toml"), "remote = \"x\"\n").unwrap();
        ensure_layer(dir.path()).unwrap();
        assert_eq!(read_version(dir.path()).as_deref(), Some(crate::VERSION));
    }

    #[test]
    fn a_layer_of_this_major_is_readable() {
        let dir = tempdir().unwrap();
        // El minor viejo se lee: adentro de un major los cambios son aditivos.
        let viejo = format!("{}.0.0", major(crate::VERSION));
        write_version(dir.path(), &viejo).unwrap();
        assert!(ensure_readable(dir.path()).is_ok());
    }

    #[test]
    fn another_major_is_refused_and_the_message_says_both() {
        let dir = tempdir().unwrap();
        write_version(dir.path(), "0.0.1").unwrap();
        let e = ensure_readable(dir.path()).unwrap_err();
        assert_eq!(e.declared.as_deref(), Some("0.0.1"));
        let msg = e.to_string();
        assert!(msg.contains("0.0.1") && msg.contains(crate::VERSION), "{msg}");
        assert!(msg.contains("migrate"), "{msg}");
    }

    #[test]
    fn no_declared_version_is_format_1_and_says_so() {
        let dir = tempdir().unwrap();
        let e = ensure_readable(dir.path()).unwrap_err();
        assert_eq!(e.declared, None);
        assert!(e.to_string().contains("formato 1"), "{e}");
    }

    #[test]
    fn the_version_round_trips() {
        let dir = tempdir().unwrap();
        write_version(dir.path(), "2.0.0").unwrap();
        assert_eq!(read_version(dir.path()).as_deref(), Some("2.0.0"));
    }
}
