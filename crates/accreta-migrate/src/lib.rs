//! Runner de migraciones de metadatos para el ecosistema Accreta.
//!
//! Las herramientas de Accreta guardan su estado en archivos —`.bilink`,
//! `.capture`, `.task`— cuyo formato evoluciona con las specs. Este crate aplica
//! esas transformaciones una sola vez por repo y lleva el registro de cuáles ya
//! corrieron.
//!
//! # Por qué no es Liquibase
//!
//! Buena parte de la maquinaria de Liquibase existe para compensar que una base
//! de datos no está versionada. Acá los metadatos sí lo están, así que sobran:
//!
//! | Liquibase | Acá |
//! |---|---|
//! | bloques de rollback | `git revert` |
//! | checksums de changeset | el changeset está en git |
//! | coordinación de equipo | el commit de migración se propaga solo |
//!
//! Lo que queda es lo que este crate hace: ids ordenados, un ledger de
//! aplicadas, y un runner idempotente.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

// ─── migración ────────────────────────────────────────────────────────────────

/// Una transformación de formato, identificada y ordenada.
///
/// El id es `<herramienta>-<NNN>-<slug>`: el prefijo evita colisiones entre
/// subsistemas, el número fija el orden, el slug lo hace legible en el ledger.
pub struct Migration {
    pub id:          &'static str,
    pub description: &'static str,
    /// Se ejecuta sobre la raíz de una capa. Recibe `dry_run`: con `true` debe
    /// calcular y reportar exactamente lo mismo, pero **sin escribir un solo
    /// archivo**. Debe ser idempotente: una capa ya migrada devuelve un
    /// `Outcome` vacío en vez de fallar.
    pub run: fn(&Path, bool) -> Result<Outcome>,
}

/// Qué hizo una migración en una capa.
#[derive(Default)]
pub struct Outcome {
    pub changed: Vec<PathBuf>,
    /// Una línea por capa para el reporte. Vacío si no hubo nada que hacer.
    pub notes: Vec<String>,
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.notes.is_empty()
    }
}

// ─── ledger ───────────────────────────────────────────────────────────────────

/// Registro de migraciones aplicadas, versionado junto al repo.
///
/// Es un **conjunto de ids**, no un número de versión: si dos ramas agregan
/// migraciones distintas, un entero daría conflicto y perdería una de las dos,
/// mientras que la unión de dos conjuntos siempre es la respuesta correcta.
///
/// Vive en un archivo y no en los mensajes de commit a propósito: el historial
/// de git es reescribible —un rebase borra trailers, un cherry-pick los duplica,
/// un clone shallow no los ve— y un registro tiene que ser contenido.
pub struct Ledger {
    path:    PathBuf,
    applied: BTreeSet<String>,
}

impl Ledger {
    /// `<repo-root>/.accreta/migrations`
    pub fn path_for(repo_root: &Path) -> PathBuf {
        repo_root.join(".accreta").join("migrations")
    }

    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = Self::path_for(repo_root);
        let applied = match std::fs::read_to_string(&path) {
            Ok(text) => text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e).with_context(|| format!("leyendo {}", path.display())),
        };
        Ok(Self { path, applied })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.applied.contains(id)
    }

    pub fn record(&mut self, id: &str) {
        self.applied.insert(id.to_string());
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::from("# Migraciones aplicadas en este repo.\n\
                                    # Una por línea, ordenadas. Los merges se resuelven por unión.\n");
        for id in &self.applied {
            out.push_str(id);
            out.push('\n');
        }
        std::fs::write(&self.path, out)
            .with_context(|| format!("escribiendo {}", self.path.display()))
    }
}

/// Raíz del repo git que contiene `start`, o `start` si no hay ninguno.
///
/// El ledger es por repo porque el repo es la unidad que git versiona; las
/// migraciones, en cambio, corren por capa. En un proyecto Stratum una capa
/// suele ser su propio repo, así que una corrida recursiva toca varios ledgers.
pub fn repo_root_of(start: &Path) -> PathBuf {
    let mut cur = start;
    loop {
        if cur.join(".git").exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None    => return start.to_path_buf(),
        }
    }
}

// ─── runner ───────────────────────────────────────────────────────────────────

pub struct Applied {
    pub id:      String,
    /// Repo cuyo ledger registra esta aplicación. Una misma migración aparece
    /// una vez por repo alcanzado, no una sola vez en total.
    pub repo:    PathBuf,
    pub notes:   Vec<String>,
    pub changed: Vec<PathBuf>,
}

#[derive(Default)]
pub struct Report {
    pub applied:   Vec<Applied>,
    /// Migraciones que el ledger ya tenía registradas.
    pub skipped:   Vec<String>,
    /// Ledgers tocados, uno por repo alcanzado.
    pub ledgers:   Vec<PathBuf>,
    pub dry_run:   bool,
}

impl Report {
    pub fn is_noop(&self) -> bool {
        self.applied.is_empty()
    }
}

/// Aplica las migraciones pendientes sobre `layers`.
///
/// Todas las capas comparten un ledger si viven en el mismo repo. Una migración
/// se marca como aplicada cuando corrió sobre **todas** las capas del repo, no
/// sobre la primera: si no, una corrida parcial dejaría el repo marcado como
/// migrado con capas sin tocar.
/// **Genera** la salida de las migraciones pendientes. No toca el ledger.
///
/// El ledger se escribe en el corte, no acá: si se escribiera al generar, el repo
/// quedaría marcado como migrado mientras sigue corriendo el formato viejo. Es el
/// mismo principio por el que una migración se marca recién cuando corrió sobre
/// todas las capas del repo — se registra cuando el estado es verdadero, no cuando
/// el trabajo empezó.
///
/// Y **siempre regenera**: la salida es un derivado, y regenerar es lo que recupera
/// un cambio hecho con el binario viejo entre la generación y el corte.
pub fn generate(
    layers:     &[PathBuf],
    migrations: &[Migration],
    dry_run:    bool,
) -> Result<Report> {
    let mut report = Report { dry_run, ..Default::default() };
    if layers.is_empty() {
        return Ok(report);
    }

    for (repo_root, repo_layers) in group_by_repo(layers) {
        let ledger = Ledger::load(&repo_root)?;

        for m in migrations {
            if ledger.contains(m.id) {
                if !report.skipped.iter().any(|s| s == m.id) {
                    report.skipped.push(m.id.to_string());
                }
                continue;
            }

            let mut notes   = Vec::new();
            let mut changed = Vec::new();
            for layer in &repo_layers {
                let outcome = (m.run)(layer, dry_run)
                    .with_context(|| format!("migración {} en {}", m.id, layer.display()))?;
                notes.extend(outcome.notes);
                changed.extend(outcome.changed);
            }
            report.applied.push(Applied {
                id: m.id.to_string(), repo: repo_root.clone(), notes, changed,
            });
        }
    }
    Ok(report)
}

/// **Registra** las migraciones en el ledger del repo. Es la mitad del corte.
///
/// Se llama cuando el estado ya es verdadero: los archivos nuevos están en su lugar
/// y el binario que los entiende es el que corre.
pub fn record(layers: &[PathBuf], migrations: &[Migration]) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    for (repo_root, _) in group_by_repo(layers) {
        let mut ledger = Ledger::load(&repo_root)?;
        let mut changed = false;
        for m in migrations {
            if !ledger.contains(m.id) {
                ledger.record(m.id);
                changed = true;
            }
        }
        if changed {
            ledger.save()?;
            written.push(Ledger::path_for(&repo_root));
        }
    }
    Ok(written)
}

/// Las capas agrupadas por el repo que las contiene: un ledger por repo.
fn group_by_repo(layers: &[PathBuf]) -> Vec<(PathBuf, Vec<PathBuf>)> {
    let mut by_repo: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for layer in layers {
        let root = repo_root_of(layer);
        match by_repo.iter_mut().find(|(r, _)| *r == root) {
            Some((_, ls)) => ls.push(layer.clone()),
            None          => by_repo.push((root, vec![layer.clone()])),
        }
    }
    by_repo
}
