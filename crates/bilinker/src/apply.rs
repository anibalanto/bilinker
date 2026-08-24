//! `bilinker apply` — corrige **dónde está** un fragmento, escribiendo en el capture.
//!
//! Nunca escribe `hash.N`, `hash_ast.N` ni `commit.N`: eso es exclusivo de `accept`.
//! Su único efecto sobre un bilink es `state.N` y repuntar `link.N` al forkear.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{bail, Result};

use crate::bilink::{walkdir, BiLinkFile};
use crate::capture::{CaptureFile, CaptureState};
use crate::check;
use crate::grammar;
use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
use crate::query;

// ─── tipos públicos ────────────────────────────────────────────────────────────

pub struct PendingFix {
    pub bilink_path:  PathBuf,
    pub uuid_short:   String,
    pub n:            u8,
    pub capture_uuid: String,
    pub sref_file:    String,
    pub fix:          Fix,
    /// El capture está compartido y este fix depende de datos propios del bilink,
    /// así que se crea un capture nuevo en vez de corregir el existente.
    pub fork:         bool,
    /// Estado del endpoint después de aplicar el fix.
    pub post_state:   EndpointState,
}

pub enum Fix {
    Moved      { new_file:   String    },
    Displaced  { new_offset: ByteRange },
    Expanded   { new_offset: ByteRange },
    Reanchored { new_query:  String    },
}

impl Fix {
    pub fn state_name(&self) -> &'static str {
        match self {
            Fix::Moved      { .. } => "MOVED",
            Fix::Displaced  { .. } => "DISPLACED",
            Fix::Expanded   { .. } => "EXPANDED",
            Fix::Reanchored { .. } => "REANCHORED",
        }
    }

    /// Un fix cuya resolución depende de `hash.N` o de una inferencia ambigua no
    /// puede imponerse a los demás referentes de un capture compartido.
    fn needs_fork(&self) -> bool {
        matches!(self, Fix::Displaced { .. } | Fix::Reanchored { .. })
    }

    pub fn description(&self, sref_file: &str) -> String {
        match self {
            Fix::Moved      { new_file }   => format!("{sref_file} → {new_file}"),
            Fix::Displaced  { new_offset } => format!("offset → {new_offset}"),
            Fix::Expanded   { new_offset } => format!("offset → {new_offset} ampliado"),
            Fix::Reanchored { new_query }  => format!("query → {new_query}"),
        }
    }
}

// ─── scan ─────────────────────────────────────────────────────────────────────

/// Recorre la capa, re-resuelve cada endpoint auto-fixeable y calcula su fix.
///
/// Nunca deriva el fix de la cache: re-resuelve contra git y el AST actuales, y
/// descarta el fix si el estado re-derivado no coincide con `state.N`.
pub fn scan_fixeable(layer: &Path) -> Result<Vec<PendingFix>> {
    let bilink_dir = layer.join(".bilink");
    let referents  = count_referents(&bilink_dir)?;
    let mut fixes  = Vec::new();

    for entry in bilink_files(&bilink_dir) {
        let Ok(bl) = BiLinkFile::load(&entry) else { continue };
        let short = bl.uuid[..8.min(bl.uuid.len())].to_string();

        for n in [0u8, 1u8] {
            let Ok(Some(cap)) = bl.capture_for(layer, n) else { continue };

            // Los estados auto-fixeables viven en dos lados: MOVED y REANCHORED
            // son de resolución y los reporta el capture; DISPLACED y EXPANDED
            // dependen de `hash.N` y los reporta el bilink.
            let Some(state) = autofixeable_state(&bl, n, &cap) else { continue };

            match compute_fix(layer, &bl, n, &cap, &state) {
                Ok(Some(fix)) => {
                    let shared = referents.get(&cap.uuid).copied().unwrap_or(1) > 1;
                    fixes.push(PendingFix {
                        bilink_path:  entry.clone(),
                        uuid_short:   short.clone(),
                        n,
                        capture_uuid: cap.uuid.clone(),
                        sref_file:    cap.sref.file.clone(),
                        post_state:   post_state_for(&fix),
                        fork:         fix.needs_fork() && shared,
                        fix,
                    });
                }
                Ok(None)  => {}
                Err(e)    => eprintln!("warn  {short}.{n}: {e}"),
            }
        }
    }

    Ok(fixes)
}

/// El estado auto-fixeable de un endpoint, mirando capture y bilink.
///
/// Devuelve `None` si no hay nada que arreglar. Se consulta primero el capture:
/// si la ubicación no resolvió, el estado de aceptación del bilink es
/// `UNRESOLVED` y no dice nada útil.
fn autofixeable_state(bl: &BiLinkFile, n: u8, cap: &CaptureFile) -> Option<EndpointState> {
    match cap.state {
        Some(CaptureState::Moved)      => return Some(EndpointState::Moved),
        Some(CaptureState::Reanchored) => return Some(EndpointState::Reanchored),
        _ => {}
    }
    match bl.state(n) {
        Some(s @ EndpointState::Displaced) | Some(s @ EndpointState::Expanded) => Some(s.clone()),
        _ => None,
    }
}

/// Tras el fix: MOVED y DISPLACED no cambiaron el contenido, así que cierran en OK.
/// EXPANDED y REANCHORED sí — siguen no-OK hasta que un humano corra `accept`.
fn post_state_for(fix: &Fix) -> EndpointState {
    match fix {
        Fix::Moved { .. } | Fix::Displaced { .. } => EndpointState::Ok,
        Fix::Expanded { .. }   => EndpointState::Expanded,
        Fix::Reanchored { .. } => EndpointState::Reanchored,
    }
}

/// Cuántos bilinks de la capa referencian cada capture.
fn count_referents(bilink_dir: &Path) -> Result<HashMap<String, usize>> {
    let mut counts = HashMap::new();
    for entry in bilink_files(bilink_dir) {
        let Ok(bl) = BiLinkFile::load(&entry) else { continue };
        for n in [0u8, 1u8] {
            if let Some(uuid) = bl.link(n).capture_uuid() {
                *counts.entry(uuid.to_string()).or_insert(0usize) += 1;
            }
        }
    }
    Ok(counts)
}

fn bilink_files(bilink_dir: &Path) -> Vec<PathBuf> {
    walkdir(bilink_dir).unwrap_or_default().into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("bilink"))
        .filter(|p| !p.file_name().and_then(|n| n.to_str())
                      .map(|n| n.starts_with('.')).unwrap_or(false))
        .collect()
}

// ─── cálculo del fix ──────────────────────────────────────────────────────────

fn compute_fix(
    layer: &Path,
    bl:    &BiLinkFile,
    n:     u8,
    cap:   &CaptureFile,
    state: &EndpointState,
) -> Result<Option<Fix>> {
    // MOVED no pasa por check_structural: el archivo no está en su path conocido,
    // así que la re-resolución es contra el índice de renames de git.
    if *state == EndpointState::Moved {
        return compute_moved(layer, &cap.sref);
    }

    // Para el resto, re-resolver con el mismo algoritmo que usa `check`.
    // `cached_state: None` fuerza la evaluación completa.
    let (derived, new_range) = check::check_structural(
        layer,
        &cap.sref,
        bl.hash(n),
        bl.hash_ast(n),
        cap.range.as_ref(),
        bl.commit(n),
        None,
    )?;

    // Validación de frescura.
    if derived == EndpointState::Ok {
        return Ok(None); // el fix ya no hace falta
    }
    if derived != *state {
        bail!(
            "state.{n} dice {state} pero la resolución actual da {derived} \
             — cache desactualizada, fix descartado. Correr `bilinker check`."
        );
    }

    let Some(abs) = new_range else { return Ok(None) };
    let Some(node_start) = node_start_of(layer, &cap.sref)? else { return Ok(None) };

    let new_offset = ByteRange {
        start: abs.start.saturating_sub(node_start),
        end:   abs.end.saturating_sub(node_start),
    };

    // No-op: el capture ya apunta al lugar correcto.
    if cap.sref.range.as_ref() == Some(&new_offset) {
        return Ok(None);
    }

    Ok(Some(match derived {
        EndpointState::Displaced => Fix::Displaced { new_offset },
        EndpointState::Expanded  => Fix::Expanded  { new_offset },
        other => bail!("estado {other} sin fix definido"),
    }))
}

/// MOVED: nueva ruta vía `git diff -M --name-status`, sin pathspec — filtrar por el
/// path viejo puede impedir que git detecte el rename.
fn compute_moved(layer: &Path, sref: &StructuralRef) -> Result<Option<Fix>> {
    for git_args in [
        &["diff", "-M", "--name-status", "HEAD"][..],
        &["diff", "-M", "--name-status", "--cached"][..],
    ] {
        let out = std::process::Command::new("git")
            .args(git_args)
            .current_dir(layer)
            .output()?;

        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if !line.starts_with('R') { continue; }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 || parts[1] != sref.file { continue; }

            let new_file = parts[2];
            if !layer.join(new_file).exists() { continue; }

            // Verificar que la referencia siga resolviendo en el path nuevo.
            let probe = StructuralRef {
                file:  new_file.to_string(),
                query: sref.query.clone(),
                range: sref.range.clone(),
            };
            if node_start_of(layer, &probe)?.is_none() && sref.query.is_some() {
                continue;
            }

            return Ok(Some(Fix::Moved { new_file: new_file.to_string() }));
        }
    }

    bail!("MOVED: no se encontró la nueva ruta de '{}' en git diff -M", sref.file)
}

/// Byte de inicio del nodo que matchea la query. `None` para endpoints de archivo
/// completo (el nodo es el archivo, arranca en 0) o si la query no matchea.
fn node_start_of(layer: &Path, sref: &StructuralRef) -> Result<Option<usize>> {
    let Some(query_str) = &sref.query else { return Ok(Some(0)) };
    let path = layer.join(&sref.file);
    if !path.exists() { return Ok(None); }
    let source   = std::fs::read_to_string(&path)?;
    let lang     = grammar::language_for_file(&sref.file);
    let language = grammar::for_language(lang)?;
    Ok(query::find_target_with_sexp(language, &source, query_str)?.map(|(s, _, _)| s))
}

// ─── aplicación ───────────────────────────────────────────────────────────────

/// Escribe el fix. Devuelve el path del capture escrito (nuevo si forkeó) y, si
/// forkeó, el path del bilink repuntado.
pub fn apply_fix(layer: &Path, pf: &PendingFix, now: &str) -> Result<Vec<PathBuf>> {
    let mut bl  = BiLinkFile::load(&pf.bilink_path)?;
    let mut cap = bl.capture_for(layer, pf.n)?
        .ok_or_else(|| anyhow::anyhow!("endpoint {} sin capture", pf.n))?;
    let mut written = Vec::new();

    match &pf.fix {
        Fix::Moved { new_file }        => cap.sref.file  = new_file.clone(),
        Fix::Reanchored { new_query }  => cap.sref.query = Some(new_query.clone()),
        Fix::Displaced { new_offset } | Fix::Expanded { new_offset } => {
            cap.sref.range = Some(new_offset.clone());
        }
    }

    // Re-resolver el range absoluto tras el fix.
    cap.range = node_start_of(layer, &cap.sref)?.map(|start| {
        let off = cap.sref.range.clone().unwrap_or(ByteRange { start: 0, end: 0 });
        ByteRange { start: start + off.start, end: start + off.end }
    });
    cap.state       = Some(CaptureState::Resolved);
    cap.resolved_at = Some(now.to_string());

    if pf.fork {
        cap.uuid = uuid::Uuid::new_v4().to_string();
        *bl.link_mut(pf.n) = LinkEndpoint::Capture(cap.uuid.clone());
    }

    written.push(cap.write_in(layer)?);

    bl.set_state(pf.n, Some(pf.post_state.clone()));
    bl.resolved_at = Some(now.to_string());
    bl.write(&pf.bilink_path)?;
    written.push(pf.bilink_path.clone());

    Ok(written)
}

// ─── git commit ───────────────────────────────────────────────────────────────

/// Stagea y commitea los archivos modificados. Retorna el hash corto del commit.
pub fn git_commit(root: &Path, paths: &[PathBuf], message: &str) -> Result<String> {
    for path in paths {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let st = std::process::Command::new("git")
            .args(["add", &rel.display().to_string()])
            .current_dir(root)
            .status()?;
        if !st.success() {
            bail!("git add falló para {}", path.display());
        }
    }

    let out = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .output()?;

    if !out.status.success() {
        bail!("git commit falló:\n{}", String::from_utf8_lossy(&out.stderr));
    }

    let stdout   = String::from_utf8_lossy(&out.stdout);
    let hash_str = stdout.lines().next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("?")
        .to_string();

    Ok(hash_str)
}
