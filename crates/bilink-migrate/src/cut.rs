//! El corte: cambiar `.bilink/` por la carpeta migrada.
//!
//! Es el único paso irreversible de la migración, y por eso es el único que
//! **verifica antes de escribir**. Todo lo anterior es un derivado que se puede
//! borrar y regenerar; esto no.
//!
//! El orden importa y es el mismo que el ADR fija: regenerar, verificar, mover, y
//! recién entonces registrar en el ledger. Registrar antes dejaría el repo marcado
//! como migrado mientras todavía corre el formato viejo.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::partition;

/// Qué necesita el corte de una migración, para no estar acoplado a ninguna.
///
/// **El corte es el mismo para todas** —regenerar, verificar, mover, registrar— y lo
/// único que cambia es cuál migración se está cortando. Cablearlo a una fue barato
/// mientras hubo una sola: al escribir la segunda, `--cut` no sabía cortarla y falló
/// verificando la primera.
pub struct Cuttable {
    /// La carpeta donde esta migración escribe.
    pub out_dir: &'static str,
    /// Dónde va lo anterior. Lleva el nombre del formato que se deja atrás, así que
    /// dos cortes no se pisan el backup.
    pub backup_dir: &'static str,
    /// Qué habría perdido, antes de escribir nada. Vacío es *"no pierde nada"*.
    pub verify: fn(&Path) -> Result<Vec<String>>,
    /// Regenerar la salida.
    pub regenerate: fn(&Path, bool) -> Result<accreta_migrate::Outcome>,
    /// Cuántos bilinks y captures produjo, y los `commit` rescatados si hay.
    pub counts: fn(&Path) -> Result<(usize, usize, Vec<(String, u8, String)>)>,
}

/// El corte de `bilinker-002-file-partition`.
pub fn partition_cut() -> Cuttable {
    Cuttable {
        out_dir:    partition::OUT_DIR,
        backup_dir: ".bilink-formato-1",
        verify:     partition::verify,
        regenerate: partition::run,
        counts:     |l| {
            let p = partition::plan(l)?;
            Ok((p.bilinks.len(), p.captures.len(), p.commits))
        },
    }
}

/// El corte de `bilinker-003-accepted-list`.
///
/// **No verifica nada, y es correcto.** La `002` puenteaba dos serializaciones y podía
/// perder un hash por el camino; ésta reescribe el mismo YAML cambiando dos campos, y
/// lo que podría perderse —los captures de un vecindario que no existen— no se pierde:
/// se declara con `declined`, que es la decisión del ítem y no una falla.
pub fn accepted_list_cut() -> Cuttable {
    use crate::accepted_list;
    Cuttable {
        out_dir:    accepted_list::OUT_DIR,
        backup_dir: ".bilink-formato-3",
        verify:     |_| Ok(Vec::new()),
        regenerate: accepted_list::run,
        counts:     |l| Ok((accepted_list::plan(l)?.files.len(), 0, Vec::new())),
    }
}

pub struct CutPlan {
    pub layer: PathBuf,
    /// Lo que la migración produjo, y que va a reemplazar a `.bilink/`.
    pub from: PathBuf,
    /// Dónde queda lo anterior, por si hay que volver.
    pub backup: PathBuf,
    pub bilinks: usize,
    pub captures: usize,
    /// Los `commit` rescatados del formato 1, para sembrar la cache tras el corte.
    pub commits: Vec<(String, u8, String)>,
}

/// Prepara el corte de una capa: regenera, verifica, y devuelve qué haría.
///
/// **Regenera siempre.** La carpeta es un derivado, y regenerar es exactamente lo
/// que recupera un `accept` hecho con el binario viejo entre la generación y el
/// corte. La regla operativa es regenerar justo antes de cortar, y acá está
/// incorporada para que no dependa de que alguien se acuerde.
pub fn plan_cut(layer: &Path) -> Result<CutPlan> {
    plan_cut_of(layer, &partition_cut())
}

pub fn plan_cut_of(layer: &Path, m: &Cuttable) -> Result<CutPlan> {
    let src = layer.join(".bilink");
    if !src.exists() {
        bail!("no hay .bilink/ en {}", layer.display());
    }

    let problems = (m.verify)(layer)
        .with_context(|| format!("verificando la migración de {}", layer.display()))?;
    if !problems.is_empty() {
        bail!("la migración de {} pierde información:\n  {}",
              layer.display(), problems.join("\n  "));
    }

    let (bilinks, captures, commits) = (m.counts)(layer)?;
    (m.regenerate)(layer, false)?;

    Ok(CutPlan {
        layer:    layer.to_path_buf(),
        from:     layer.join(m.out_dir),
        backup:   layer.join(m.backup_dir),
        bilinks, captures, commits,
    })
}

/// Ejecuta el corte: `.bilink/` pasa a backup y la carpeta migrada toma su lugar.
///
/// El backup se conserva. Borrar lo anterior en el mismo paso que se lo reemplaza
/// deja sin red justo donde más hace falta; limpiarlo después es una decisión
/// aparte, que se toma con el resultado a la vista.
pub fn execute(cut: &CutPlan) -> Result<()> {
    if !cut.from.exists() {
        bail!("no está la carpeta migrada {}", cut.from.display());
    }
    if cut.backup.exists() {
        std::fs::remove_dir_all(&cut.backup)
            .with_context(|| format!("limpiando el backup anterior {}", cut.backup.display()))?;
    }
    let live = cut.layer.join(".bilink");
    std::fs::rename(&live, &cut.backup)
        .with_context(|| format!("moviendo {} a {}", live.display(), cut.backup.display()))?;
    std::fs::rename(&cut.from, &live)
        .with_context(|| format!("moviendo {} a {}", cut.from.display(), live.display()))?;
    Ok(())
}

/// Deshace un corte: el backup vuelve a `.bilink/`.
///
/// Existe porque el corte es el único paso irreversible, y un paso irreversible sin
/// camino de vuelta obliga a deshacerlo a mano — que es justo lo que la migración
/// evita en todos los demás pasos.
///
/// No toca el ledger: quitarlo es del comando que llama, que sabe qué migraciones
/// estaba deshaciendo.
pub fn rollback(layer: &Path) -> Result<()> {
    rollback_of(layer, ".bilink-formato-1")
}

pub fn rollback_of(layer: &Path, backup_dir: &str) -> Result<()> {
    let live   = layer.join(".bilink");
    let backup = layer.join(backup_dir);
    if !backup.exists() {
        bail!("no hay backup en {} — el corte no se hizo, o ya se deshizo", backup.display());
    }
    if live.exists() {
        std::fs::remove_dir_all(&live)
            .with_context(|| format!("quitando {}", live.display()))?;
    }
    std::fs::rename(&backup, &live)
        .with_context(|| format!("restaurando {} desde el backup", live.display()))?;
    Ok(())
}

/// Deja `.bilink-migrate-*` y el backup fuera de git.
///
/// Va al empezar la migración y no en el corte: son temporales, se borran al
/// terminar, y nunca se commitean.
pub fn exclude_in(repo_root: &Path) -> Result<()> {
    let path = repo_root.join(".git").join("info").join("exclude");
    let Some(parent) = path.parent() else { return Ok(()) };
    if !parent.exists() { return Ok(()); }

    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = current.clone();
    for pat in [".bilink-migrate-*", ".bilink-formato-1"] {
        if !current.lines().any(|l| l.trim() == pat) {
            if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
            out.push_str(pat);
            out.push('\n');
        }
    }
    if out != current {
        std::fs::write(&path, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn layer_v1() -> tempfile::TempDir {
        let d = tempdir().unwrap();
        let b = d.path().join(".bilink");
        std::fs::create_dir_all(b.join("capture")).unwrap();
        std::fs::write(b.join("capture/c0.capture"), "file:   a.rs\n").unwrap();
        std::fs::write(b.join("aaaa1111-0000-4000-8000-000000000001.bilink"),
            "link.0: capture c0\nlink.1: issue 3a\n\n# Cache\nhash.0: deadbeef\ncommit.0: cafe\n").unwrap();
        d
    }

    /// Tras el corte, `.bilink/` está en formato 2 y lo anterior sigue disponible.
    #[test]
    fn the_cut_swaps_and_keeps_a_backup() {
        let d = layer_v1();
        let cut = plan_cut(d.path()).unwrap();
        execute(&cut).unwrap();

        let live = d.path().join(".bilink");
        assert!(live.join("aaaa1111-0000-4000-8000-000000000001.yaml").exists(),
            "el bilink migrado tiene que estar vivo");
        assert!(!live.join("aaaa1111-0000-4000-8000-000000000001.bilink").exists(),
            "el del formato 1 no");
        assert!(cut.backup.join("aaaa1111-0000-4000-8000-000000000001.bilink").exists(),
            "y tiene que quedar en el backup");
        assert!(!cut.from.exists(), "la carpeta transitoria se consumió");
    }

    /// El corte regenera antes de mover: recoge lo que pasó después de generar.
    #[test]
    fn the_cut_regenerates_first() {
        let d = layer_v1();
        partition::run(d.path(), false).unwrap();

        // Alguien acepta con el binario viejo después de generar.
        let p = d.path().join(".bilink/aaaa1111-0000-4000-8000-000000000001.bilink");
        std::fs::write(&p, "link.0: capture c0\nlink.1: issue 3a\n\n# Cache\nhash.0: 99999999\ncommit.0: cafe\n").unwrap();

        let cut = plan_cut(d.path()).unwrap();
        execute(&cut).unwrap();

        let text = std::fs::read_to_string(
            d.path().join(".bilink/aaaa1111-0000-4000-8000-000000000001.yaml")).unwrap();
        assert!(text.contains("99999999"), "el corte se comió la aceptación:\n{text}");
    }

    /// El corte se puede repetir sin romper nada: la segunda vez no hay qué migrar.
    #[test]
    fn cutting_an_already_cut_layer_is_refused_not_destructive() {
        let d = layer_v1();
        let cut = plan_cut(d.path()).unwrap();
        execute(&cut).unwrap();
        let after = std::fs::read_to_string(
            d.path().join(".bilink/aaaa1111-0000-4000-8000-000000000001.yaml")).unwrap();

        // Ya no hay nada en formato 1 que migrar: se rechaza en vez de vaciar.
        assert!(plan_cut(d.path()).is_err(), "cortar dos veces tiene que fallar");
        assert_eq!(std::fs::read_to_string(
            d.path().join(".bilink/aaaa1111-0000-4000-8000-000000000001.yaml")).unwrap(), after,
            "y no tocar lo que ya estaba");
    }
}

/// Qué corte le corresponde a cada capa, **según el formato que declara**.
///
/// La primera versión de esto elegía *"la primera migración que el ledger no tiene"*,
/// y está mal por un caso que aparece enseguida: **una capa puede haber nacido en un
/// formato**. Los repos de `hsi`, `retinar` y `filasvirtuales` los creó `init` en 3.8
/// y nunca corrieron la `002` — con la regla del ledger, `--cut` intentaba cortar una
/// migración de formato 1 sobre archivos de formato 3 y fallaba verificando.
///
/// Lo que decide es `.bilink/version`, que **es el dato que existe para esto** y que
/// hasta acá no tenía lector: se escribía y nadie lo comparaba nunca. Ver la task
/// `3x`, que es el mismo campo sin leer en el otro comando.
pub fn cuts_for(layers: &[PathBuf]) -> Vec<(PathBuf, Cuttable)> {
    layers.iter().filter_map(|l| {
        let major = bilink_format::read_version(l)
            .and_then(|v| v.split('.').next()?.parse::<u32>().ok());
        let cut = match major {
            // Sin versión declarada es una capa anterior a que el campo existiera:
            // formato 1, y le toca la `002`. Es la misma lectura que hace
            // `read_version` cuando el archivo no está.
            None    => partition_cut(),
            Some(3) => accepted_list_cut(),
            // Al día, o de un formato que este binario no puentea. Lo segundo no es
            // un corte que haya que elegir: es una versión que no se entiende, y
            // decirlo es de quien lee, no de acá.
            _ => return None,
        };
        Some((l.clone(), cut))
    }).collect()
}
