use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::accept::find_bilink_path;
use crate::bilink::BiLinkFile;
use crate::check::find_by_file;
use crate::daemon;
use crate::link::{EndpointState, LinkEndpoint};

pub struct ImpactResult {
    pub bilink_uuid: String,
    pub endpoint: u8,
    pub state: Option<EndpointState>,
    pub other_endpoint: String,
    pub commit: String,
    /// Call chain from the starting function up to (not including) the bilink endpoint.
    /// Empty means the starting position IS the bilink endpoint.
    pub path: Vec<PathItem>,
}

#[derive(Clone)]
pub struct PathItem {
    pub file: String,
    pub line: u32,
}

pub enum Selector {
    /// All bilinks with state.N ≠ OK in this layer.
    All,
    /// A specific source position.
    FileLineCol { file: PathBuf, line: u32, col: u32 },
    /// A bilink UUID (optionally a single endpoint).
    Uuid { uuid: String, endpoint: Option<u8> },
}

pub fn impact(
    root: &Path,
    selector: &Selector,
    max_depth: Option<usize>,
) -> Result<Vec<ImpactResult>> {
    let starts = resolve_selector(root, selector)?;
    let depth = max_depth.unwrap_or(usize::MAX);

    let mut results: Vec<ImpactResult> = Vec::new();
    let mut seen_bilinks: HashSet<String> = HashSet::new();

    for (file_abs, line, col) in starts {
        let mut visited: HashSet<String> = HashSet::new();
        traverse(
            root,
            &file_abs,
            line,
            col,
            vec![],
            &mut visited,
            depth,
            &mut results,
            &mut seen_bilinks,
        )?;
    }

    Ok(results)
}

// ─── selector resolution ──────────────────────────────────────────────────────

fn resolve_selector(
    root: &Path,
    selector: &Selector,
) -> Result<Vec<(PathBuf, u32, u32)>> {
    match selector {
        Selector::All => {
            let bilink_dir = root.join(".bilink");
            let mut starts = Vec::new();

            let Ok(entries) = std::fs::read_dir(&bilink_dir) else {
                return Ok(vec![]);
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("bilink") {
                    continue;
                }
                if path.file_name().and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.')).unwrap_or(false) {
                    continue;
                }
                let Ok(bl) = BiLinkFile::load(&path) else { continue };

                for (_n, state, link, range) in [
                    (0u8, &bl.state0, &bl.link0, &bl.range0),
                    (1u8, &bl.state1, &bl.link1, &bl.range1),
                ] {
                    if matches!(state, Some(EndpointState::Ok)) || state.is_none() {
                        continue;
                    }
                    let Ok(Some(sref)) = crate::capture::sref_of(root, link) else { continue };
                    let Some(range) = range else { continue };

                    let file_abs = root.join(&sref.file);
                    if !file_abs.exists() { continue; }

                    let source = std::fs::read_to_string(&file_abs).unwrap_or_default();
                    let (line, _) = byte_to_line_col(&source, range.start);
                    let col = fn_col_from_source(&source, line as usize);
                    starts.push((file_abs, line, col));
                }
            }
            Ok(starts)
        }

        Selector::FileLineCol { file, line, col } => {
            let file_abs = if file.is_absolute() {
                file.clone()
            } else {
                root.join(file)
            };
            Ok(vec![(file_abs, *line, *col)])
        }

        Selector::Uuid { uuid, endpoint } => {
            let bilink_dir = root.join(".bilink");
            let bilink_path = find_bilink_path(&bilink_dir, uuid)?;
            let bl = BiLinkFile::load(&bilink_path)?;

            let ns: &[u8] = match endpoint {
                Some(n) => std::slice::from_ref(n),
                None    => &[0, 1],
            };

            let mut starts = Vec::new();
            for &n in ns {
                let (link, range) = if n == 0 {
                    (&bl.link0, &bl.range0)
                } else {
                    (&bl.link1, &bl.range1)
                };
                let Ok(Some(sref)) = crate::capture::sref_of(root, link) else { continue };
                let Some(range) = range else { continue };

                let file_abs = root.join(&sref.file);
                if !file_abs.exists() { continue; }

                let source = std::fs::read_to_string(&file_abs).unwrap_or_default();
                let (line, _) = byte_to_line_col(&source, range.start);
                let col = fn_col_from_source(&source, line as usize);
                starts.push((file_abs, line, col));
            }
            Ok(starts)
        }
    }
}

// ─── upward traversal ─────────────────────────────────────────────────────────

fn traverse(
    root: &Path,
    file_abs: &Path,
    line: u32,
    col: u32,
    path: Vec<PathItem>,
    visited: &mut HashSet<String>,
    depth_remaining: usize,
    results: &mut Vec<ImpactResult>,
    seen_bilinks: &mut HashSet<String>,
) -> Result<()> {
    let visit_key = format!("{}:{}", file_abs.display(), line);
    if !visited.insert(visit_key) {
        return Ok(());
    }

    let source = std::fs::read_to_string(file_abs).unwrap_or_default();
    let line_byte = line_to_byte_start(&source, line);

    let bilink_hits: Vec<_> = find_by_file(root, file_abs)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, range)| line_byte >= range.start && line_byte < range.end)
        .collect();

    for (bilink_path, n, _) in &bilink_hits {
        let uuid = bilink_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let bilink_key = format!("{uuid}:{n}");
        if seen_bilinks.insert(bilink_key) {
            if let Ok(bl) = BiLinkFile::load(bilink_path) {
                let state = if *n == 0 { bl.state0.clone() } else { bl.state1.clone() };
                let commit = if *n == 0 { bl.commit0.clone() } else { bl.commit1.clone() }
                    .unwrap_or_default();
                let other = if *n == 0 { bl.link1.to_string() } else { bl.link0.to_string() };
                results.push(ImpactResult {
                    bilink_uuid: uuid,
                    endpoint: *n,
                    state,
                    other_endpoint: other,
                    commit,
                    path: path.clone(),
                });
            }
        }
    }

    // Stop this branch once we hit a bilink, unless this is the starting position (path empty).
    // At the start, continue upward even if a bilink was found — there may be more bilinks above.
    if !bilink_hits.is_empty() && !path.is_empty() {
        return Ok(());
    }

    if depth_remaining == 0 {
        return Ok(());
    }

    let file_str = file_abs.to_string_lossy().to_string();
    let callers_val = daemon::rpc(
        "callers",
        serde_json::json!({ "file": file_str, "line": line, "col": col }),
    )
    .unwrap_or(serde_json::json!([]));

    #[derive(Deserialize)]
    struct CalInfo {
        file: String,
        line: u32,
        col: u32,
    }

    let callers: Vec<CalInfo> =
        serde_json::from_value(callers_val).unwrap_or_default();

    let current = PathItem { file: file_str, line };

    for caller in callers {
        if !Path::new(&caller.file).starts_with(root) {
            continue;
        }
        let mut new_path = path.clone();
        new_path.push(current.clone());
        traverse(
            root,
            Path::new(&caller.file),
            caller.line,
            caller.col,
            new_path,
            visited,
            depth_remaining - 1,
            results,
            seen_bilinks,
        )?;
    }

    Ok(())
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Byte offset of the start of `line` (0-based).
fn line_to_byte_start(source: &str, line: u32) -> usize {
    source
        .lines()
        .take(line as usize)
        .map(|l| l.len() + 1) // +1 for \n
        .sum()
}

/// Convert byte offset to (line_0based, col_0based).
pub fn byte_to_line_col(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.chars().filter(|&c| c == '\n').count() as u32;
    let col = prefix
        .rfind('\n')
        .map(|i| byte - i - 1)
        .unwrap_or(byte) as u32;
    (line, col)
}

/// Scan a source line and return the 0-based column of the first non-keyword identifier.
/// Handles `pub fn foo`, `def foo`, `public void foo`, etc.
pub fn fn_col_from_source(source: &str, line_0based: usize) -> u32 {
    const SKIP: &[&str] = &[
        "pub", "fn", "async", "unsafe", "extern",
        "def", "void", "public", "private", "protected", "static", "final",
        "abstract", "function", "class", "struct", "trait", "interface", "enum",
        "override", "virtual", "native", "synchronized",
    ];
    let Some(line) = source.lines().nth(line_0based) else { return 0 };
    let bytes = line.as_bytes();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let b = bytes[pos];
        if b == b' ' || b == b'\t' {
            pos += 1;
            continue;
        }
        if !b.is_ascii_alphanumeric() && b != b'_' {
            pos += 1;
            continue;
        }
        let start = pos;
        while pos < bytes.len()
            && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
        {
            pos += 1;
        }
        if !SKIP.contains(&&line[start..pos]) {
            return start as u32;
        }
    }
    0
}
