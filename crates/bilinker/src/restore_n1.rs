//! `bilinker restore-n1` — devuelve el vecindario que la `003` descartó.
//!
//! La migración tuvo los dos hashes de cada nivel en la mano y los reemplazó por
//! `declined`, porque no había cómo escribir *"tengo el contenido y no la ubicación"*.
//! Ahora hay —`link: unknown`— así que el contrato se puede devolver.
//!
//! **No es una migración**, y la razón que decide es una: una migración tiene que ser
//! reproducible desde lo que el repo contiene, y esto lee un directorio que no está en
//! git y que puede no existir. Ver `commands/restore-n1.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use bilink_format::{BiLink, LevelLink, Neighbourhood, N};

/// El directorio que dejó el corte de la `003`, al lado del `.bilink/` de la capa.
pub const DEFAULT_BACKUP_DIR: &str = ".bilink-formato-3";

/// La forma **3.8** del backup, con lo justo para leer el vecindario.
///
/// Structs locales y no un crate congelado, por lo mismo que la `003`: 3.8 y 4.x son
/// el mismo YAML y difieren en dos lugares. Lo que no se lee no se modela — de este
/// lado sólo hacen falta el `hash` del fragmento, que es el discriminador, y los
/// niveles.
mod v38 {
    use super::*;

    #[derive(Deserialize)]
    pub struct BiLink {
        pub endpoint: BTreeMap<String, Endpoint>,
    }

    #[derive(Deserialize)]
    pub struct Endpoint {
        /// En 3.8 `accepted` es **un objeto**, no una lista: ahí está el cambio que la
        /// `003` hizo además de tirar el `n`.
        #[serde(default)]
        pub accepted: Option<Accepted>,
    }

    #[derive(Deserialize)]
    pub struct Accepted {
        pub hash: String,
        /// `declined` o un mapa de niveles. Se lee crudo y se interpreta después: un
        /// enum acá haría fallar la lectura de un backup renunciado, que es legítimo.
        #[serde(default)]
        pub n: Option<serde_yaml_ng::Value>,
    }

    /// Un nivel del backup: los dos folds, **sin `link`**. Que no lo tenga es el hecho
    /// que hace falta `unknown`: la ubicación no está para traer.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Level {
        pub hash: String,
        #[serde(default)]
        pub hash_ast: Option<String>,
    }
}

/// Por qué un hueco no se pudo llenar.
///
/// **Las dos son sobre un endpoint que sí tenía contrato en el backup.** Que el `n`
/// vivo no sea `declined` no está acá: eso no es un hueco, es que no había nada que
/// devolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// El `hash` del fragmento se movió: el backup es de otra versión de la firma.
    ///
    /// **Se vence solo con el trabajo de todos los días**: cada `accept` sobre un
    /// endpoint degradado mueve ese hash.
    HashMoved,
    /// Más de un `accepted`: `CONSENSUS_DIVERGED`, y no hay un valor contra el cual
    /// comparar el hash.
    Diverged,
}

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HashMoved => "el hash del fragmento se movió: el backup es de otra versión",
            Self::Diverged  => "más de un accepted: no hay un valor contra el cual comparar",
        })
    }
}

/// Lo que pasó en una capa.
#[derive(Debug, Default)]
pub struct Report {
    pub layer: PathBuf,
    /// `<uuid-corto>.<N>` de cada nivel devuelto.
    pub restored: Vec<String>,
    /// Los huecos que quedaron, con su motivo. **Van nombrados y no sólo contados**:
    /// un endpoint que se queda en `declined` es limpio para `check`, así que esta
    /// lista es el único registro de que ahí había un contrato.
    pub skipped: Vec<(String, Skipped)>,
    /// La capa no tiene backup del corte. No es un error: la mayoría no lo necesita.
    pub no_backup: bool,
}

impl Report {
    pub fn touched(&self) -> bool { !self.restored.is_empty() }
}

/// Restituye lo que se pueda en una capa.
///
/// `from` es de dónde leer el backup; `None` usa [`DEFAULT_BACKUP_DIR`] al lado del
/// `.bilink/` de la capa.
pub fn restore(layer: &Path, from: Option<&Path>, dry_run: bool) -> Result<Report> {
    let mut r = Report { layer: layer.to_path_buf(), ..Default::default() };

    let backup = from.map(Path::to_path_buf).unwrap_or_else(|| layer.join(DEFAULT_BACKUP_DIR));
    if !backup.is_dir() { r.no_backup = true; return Ok(r); }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&backup)
        .with_context(|| format!("no se pudo leer el backup {}", backup.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    files.sort();

    for f in files {
        let uuid = f.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let live_path = BiLink::path_in(layer, &uuid);
        if !live_path.exists() { continue; }

        let raw = std::fs::read_to_string(&f)
            .with_context(|| format!("no se pudo leer {}", f.display()))?;
        let old: v38::BiLink = match serde_yaml_ng::from_str(&raw) {
            Ok(b) => b,
            // Un backup que no parsea **no aborta la corrida**: es un archivo de un
            // formato anterior que ya nadie escribe, y lo que importa es cuántos
            // contratos se devuelven. Los que no se pudieron leer no se cuentan como
            // salteados porque no se sabe si tenían contrato.
            Err(_) => continue,
        };

        // Los niveles que el backup conserva, por endpoint.
        let mut pendientes: Vec<(String, BTreeMap<u8, v38::Level>)> = Vec::new();
        for (k, ep) in &old.endpoint {
            let Some(acc) = ep.accepted.as_ref() else { continue };
            let Some(niveles) = levels_of(acc.n.as_ref()) else { continue };
            pendientes.push((k.clone(), niveles));
        }
        if pendientes.is_empty() { continue; }

        let mut live = BiLink::load(&live_path)
            .with_context(|| format!("no se pudo leer {}", live_path.display()))?;
        let mut cambio = false;

        for (k, niveles) in pendientes {
            let n_ep: u8 = match k.trim().parse() { Ok(v) if v < 2 => v, _ => continue };
            let at = format!("{}.{n_ep}", &uuid[..8.min(uuid.len())]);

            // El hash del backup, para el discriminador.
            let backup_hash = old.endpoint.get(&k)
                .and_then(|e| e.accepted.as_ref())
                .map(|a| a.hash.clone())
                .unwrap_or_default();

            let ep = live.endpoint.get_mut(n_ep);
            if ep.accepted.len() > 1 { r.skipped.push((at, Skipped::Diverged)); continue; }
            let Some(acc) = ep.accepted.first_mut() else { continue };

            // **Condición 1**: sólo sobre `declined`. Que no lo sea no es un salteo —
            // es que ahí no hay hueco: o alguien lo re-adquirió, o el fragmento no
            // tiene firma resoluble.
            if acc.n.as_ref().map(N::is_acquired) != Some(false) { continue; }

            // **Condición 2**: el fragmento tiene que ser el mismo.
            if acc.hash != backup_hash { r.skipped.push((at, Skipped::HashMoved)); continue; }

            acc.n = Some(N::Levels(niveles.into_iter().map(|(lvl, l)| (lvl, Neighbourhood {
                // La ubicación no está en el backup, y no se inventa.
                link: LevelLink::Unknown,
                hash: l.hash,
                hash_ast: l.hash_ast,
            })).collect()));
            r.restored.push(at);
            cambio = true;
        }

        if cambio && !dry_run {
            live.write(&live_path)
                .with_context(|| format!("no se pudo escribir {}", live_path.display()))?;
        }
    }

    Ok(r)
}

/// Los niveles adquiridos de un `n` de 3.8, o `None` si es una renuncia o no está.
fn levels_of(n: Option<&serde_yaml_ng::Value>) -> Option<BTreeMap<u8, v38::Level>> {
    let v = n?;
    if v.as_str().is_some() { return None; }   // `declined`
    let m: BTreeMap<u8, v38::Level> = serde_yaml_ng::from_value(v.clone()).ok()?;
    if m.is_empty() { return None; }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bilink_format::{Accepted, LinkEndpoint};
    use tempfile::tempdir;

    /// Escribe un backup 3.8 y su bilink vivo, y devuelve la capa.
    fn layer_with(backup_n: &str, live_hash: &str, live_n: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        let uuid = "7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a";
        std::fs::create_dir_all(d.path().join(DEFAULT_BACKUP_DIR)).unwrap();
        std::fs::create_dir_all(d.path().join(".bilink")).unwrap();
        std::fs::write(d.path().join(DEFAULT_BACKUP_DIR).join(format!("{uuid}.yaml")),
            format!("endpoint:\n  0:\n    link: capture {a}\n    accepted:\n      link: capture {a}\n      hash: elhash\n{backup_n}  1:\n    link: abstract\n", a = "a".repeat(32))).unwrap();

        let mut bl = BiLink::new(
            format!("capture {}", "a".repeat(32)).parse::<LinkEndpoint>().unwrap(),
            LinkEndpoint::Abstract);
        bl.endpoint.get_mut(0).accepted = vec![Accepted {
            agree: Default::default(),
            link: Some(format!("capture {}", "a".repeat(32)).parse().unwrap()),
            hash: live_hash.into(), hash_ast: None,
            n: if live_n.is_empty() { None } else { serde_yaml_ng::from_str(live_n).unwrap() },
        }];
        bl.write(&BiLink::path_in(d.path(), uuid)).unwrap();
        d
    }

    const NIVEL: &str = "      n:\n        1:\n          hash: elcontrato\n          hash_ast: elast\n";

    /// El caso central: `declined` vivo, contrato en el backup, mismo fragmento.
    #[test]
    fn a_declined_level_gets_its_contract_back_with_an_unknown_location() {
        let d = layer_with(NIVEL, "elhash", "declined");
        let r = restore(d.path(), None, false).unwrap();
        assert_eq!(r.restored.len(), 1, "{r:?}");
        assert!(r.skipped.is_empty());

        let bl = BiLink::load(&BiLink::path_in(d.path(), "7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a")).unwrap();
        let nb = bl.endpoint.get(0).accepted[0].n.as_ref().unwrap().level(1).unwrap();
        assert_eq!(nb.hash, "elcontrato");
        assert_eq!(nb.hash_ast.as_deref(), Some("elast"));
        assert_eq!(nb.link, LevelLink::Unknown, "la ubicación no está en el backup y no se inventa");
    }

    /// **El discriminador.** El fragmento cambió desde el corte, así que ese contrato
    /// era de otra versión de la firma: no se restituye, y se dice cuál.
    #[test]
    fn a_moved_fragment_hash_is_skipped_and_named() {
        let d = layer_with(NIVEL, "otrohash", "declined");
        let r = restore(d.path(), None, false).unwrap();
        assert!(r.restored.is_empty());
        assert_eq!(r.skipped, vec![("7f3d8e9a.0".to_string(), Skipped::HashMoved)]);
    }

    /// Un `n` adquirido después del corte es más nuevo que el backup, y **no se cuenta
    /// como salteado**: no hay hueco ahí.
    #[test]
    fn a_reacquired_level_is_left_alone_and_is_not_a_skip() {
        let d = layer_with(NIVEL, "elhash", "{1: {link: capture aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, hash: nuevo}}");
        let r = restore(d.path(), None, false).unwrap();
        assert!(r.restored.is_empty() && r.skipped.is_empty(), "{r:?}");
    }

    /// Idempotente, y **por las condiciones y no por un registro**: después de la
    /// primera corrida el `n` ya no es `declined`.
    #[test]
    fn the_second_run_is_a_no_op() {
        let d = layer_with(NIVEL, "elhash", "declined");
        assert_eq!(restore(d.path(), None, false).unwrap().restored.len(), 1);
        let r = restore(d.path(), None, false).unwrap();
        assert!(r.restored.is_empty() && r.skipped.is_empty(), "{r:?}");
    }

    /// `--dry-run` no escribe: cuenta lo mismo y el archivo queda igual.
    #[test]
    fn dry_run_counts_without_writing() {
        let d = layer_with(NIVEL, "elhash", "declined");
        assert_eq!(restore(d.path(), None, true).unwrap().restored.len(), 1);
        let bl = BiLink::load(&BiLink::path_in(d.path(), "7f3d8e9a-1b2c-4d5e-8f6a-7b8c9d0e1f2a")).unwrap();
        assert!(!bl.endpoint.get(0).accepted[0].n.as_ref().unwrap().is_acquired(),
                "sigue renunciado");
    }

    /// Un backup renunciado no tiene nada que devolver, y tampoco es un salteo.
    #[test]
    fn a_declined_backup_has_nothing_to_give() {
        let d = layer_with("      n: declined\n", "elhash", "declined");
        let r = restore(d.path(), None, false).unwrap();
        assert!(r.restored.is_empty() && r.skipped.is_empty(), "{r:?}");
    }

    /// Una capa sin backup no es un error: la mayoría no lo necesita.
    #[test]
    fn a_layer_without_a_backup_is_not_an_error() {
        let d = tempdir().unwrap();
        let r = restore(d.path(), None, false).unwrap();
        assert!(r.no_backup && !r.touched());
    }
}
