use std::path::{Path, PathBuf};
use anyhow::{bail, Result};

use crate::bilink::{walkdir, BiLinkFile};
use crate::grammar;
use crate::hash;
use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
use crate::query;

// ─── tipos públicos ────────────────────────────────────────────────────────────

pub struct PendingFix {
    pub bilink_path: PathBuf,
    pub uuid_short:  String,
    pub n:           u8,
    pub sref_file:   String,
    pub fix:         Fix,
}

pub enum Fix {
    Moved      { new_file:  String    },
    Displaced  { new_range: ByteRange },
    Expanded   { new_range: ByteRange },
    Reanchored { new_query: String    },
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

    pub fn description(&self, sref_file: &str) -> String {
        match self {
            Fix::Moved      { new_file }  => format!("{sref_file} → {new_file}"),
            Fix::Displaced  { new_range } => format!("offset {}~{}", new_range.start, new_range.end),
            Fix::Expanded   { new_range } => format!("range {}~{} ampliado", new_range.start, new_range.end),
            Fix::Reanchored { new_query } => format!("query → {new_query}"),
        }
    }
}

// ─── scan ─────────────────────────────────────────────────────────────────────

/// Scan the current layer for bilinks with auto-fixeable states and compute their fixes.
/// Returns an error only if a fix calculation fails irrecoverably; individual bilinks
/// that can't be fixed are skipped with a warning.
pub fn scan_fixeable(layer: &Path) -> Result<Vec<PendingFix>> {
    let bilink_dir = layer.join(".bilink");
    let mut fixes  = Vec::new();

    for entry in walkdir(&bilink_dir)? {
        if entry.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        if entry.file_name().and_then(|n| n.to_str())
            .map(|n| n.starts_with('.')).unwrap_or(false) { continue; }

        let Ok(bl) = BiLinkFile::load(&entry) else { continue };

        for n in [0u8, 1u8] {
            let state       = if n == 0 { &bl.state0     } else { &bl.state1     };
            let link        = if n == 0 { &bl.link0      } else { &bl.link1      };
            let range       = if n == 0 { bl.range0.as_ref() } else { bl.range1.as_ref() };
            let stored_hash = if n == 0 { bl.hash0.as_deref() } else { bl.hash1.as_deref() };

            let LinkEndpoint::Structural(sref) = link else { continue };

            let result = match state {
                Some(EndpointState::Moved) => {
                    compute_moved(layer, &sref.file, stored_hash)
                }
                Some(EndpointState::Displaced) => {
                    let Some(abs) = range else { continue };
                    compute_relative(layer, sref, abs, false).map(Some)
                }
                Some(EndpointState::Expanded) => {
                    let Some(abs) = range else { continue };
                    compute_relative(layer, sref, abs, true).map(Some)
                }
                Some(EndpointState::Reanchored) => {
                    // check no detecta REANCHORED aún — skip silencioso
                    continue;
                }
                _ => continue,
            };

            match result {
                Ok(Some(fix)) => fixes.push(PendingFix {
                    bilink_path: entry.clone(),
                    uuid_short:  bl.uuid[..8.min(bl.uuid.len())].to_string(),
                    n,
                    sref_file:   sref.file.clone(),
                    fix,
                }),
                Ok(None) => {}
                Err(e) => eprintln!("warn  {}.{n}: {e}", &bl.uuid[..8.min(bl.uuid.len())]),
            }
        }
    }

    Ok(fixes)
}

// ─── cálculo por estado ────────────────────────────────────────────────────────

/// MOVED: encuentra la nueva ruta vía `git diff -M --name-status`.
fn compute_moved(
    root:        &Path,
    old_file:    &str,
    stored_hash: Option<&str>,
) -> Result<Option<Fix>> {
    for git_args in [
        &["diff", "-M", "--name-status", "HEAD"][..],
        &["diff", "-M", "--name-status", "--cached"][..],
    ] {
        let out = std::process::Command::new("git")
            .args(git_args)
            .current_dir(root)
            .output()?;

        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if !line.starts_with('R') { continue; }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 || parts[1] != old_file { continue; }

            let new_file = parts[2];
            let new_abs  = root.join(new_file);
            if !new_abs.exists() { continue; }

            // Verify hash still matches at new path
            if let Some(expected) = stored_hash {
                let content = std::fs::read(&new_abs).unwrap_or_default();
                if hash::sha256(&content) != expected { continue; }
            }

            return Ok(Some(Fix::Moved { new_file: new_file.to_string() }));
        }
    }

    bail!("MOVED: no se encontró la nueva ruta para '{old_file}' en git diff -M")
}

/// DISPLACED / EXPANDED: convierte el range.N absoluto al relativo dentro del nodo AST.
fn compute_relative(
    root:      &Path,
    sref:      &StructuralRef,
    abs_range: &ByteRange,
    expanded:  bool,
) -> Result<Fix> {
    // Archivos sin query usan el archivo completo — el range ya es desde byte 0.
    let Some(query_str) = &sref.query else {
        let rel = ByteRange { start: abs_range.start, end: abs_range.end };
        return Ok(if expanded { Fix::Expanded { new_range: rel } } else { Fix::Displaced { new_range: rel } });
    };

    let file_path = root.join(&sref.file);
    let source    = std::fs::read_to_string(&file_path)?;
    let lang      = grammar::language_for_file(&sref.file);
    let language  = grammar::for_language(lang)?;

    let state_name = if expanded { "EXPANDED" } else { "DISPLACED" };
    let Some((node_start, _, _)) = query::find_target_with_sexp(language, &source, query_str)? else {
        bail!("{state_name}: la query ya no matchea en '{}'", sref.file);
    };

    let rel = ByteRange {
        start: abs_range.start.saturating_sub(node_start),
        end:   abs_range.end.saturating_sub(node_start),
    };
    Ok(if expanded { Fix::Expanded { new_range: rel } } else { Fix::Displaced { new_range: rel } })
}

// ─── aplicación ───────────────────────────────────────────────────────────────

/// Aplica un fix al archivo .bilink (in place).
pub fn apply_fix(bilink_path: &Path, n: u8, fix: &Fix) -> Result<()> {
    let mut bl = BiLinkFile::load(bilink_path)?;

    let link = if n == 0 { &mut bl.link0 } else { &mut bl.link1 };
    let LinkEndpoint::Structural(sref) = link else {
        bail!("endpoint {n} no es estructural");
    };

    match fix {
        Fix::Moved { new_file } => {
            sref.file = new_file.clone();
        }
        Fix::Displaced { new_range } | Fix::Expanded { new_range } => {
            sref.range = Some(new_range.clone());
        }
        Fix::Reanchored { new_query } => {
            sref.query = Some(new_query.clone());
        }
    }

    bl.write(bilink_path)
}

// ─── git commit ───────────────────────────────────────────────────────────────

/// Stagea y commitea todos los .bilink modificados.
/// Retorna el hash corto del commit resultante.
pub fn git_commit(root: &Path, paths: &[&Path], message: &str) -> Result<String> {
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

    // Primera línea de git commit: "[branch abc1234] message"
    let stdout   = String::from_utf8_lossy(&out.stdout);
    let hash_str = stdout.lines().next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("?")
        .to_string();

    Ok(hash_str)
}
