use std::path::{Path, PathBuf};
use std::time::SystemTime;
use anyhow::Result;

use bilink_format::BiLink;

#[derive(Debug, PartialEq)]
pub enum IndexStatus {
    Ok,
    Stale { stale_count: usize },
    Missing,
}

/// Builds `.bilink/.index` for the given layer root and writes `.bilink/.gitignore`.
/// Returns the number of entries written.
pub fn build(layer_root: &Path) -> Result<usize> {
    let bilink_dir = layer_root.join(".bilink");
    if !bilink_dir.exists() {
        return Ok(0);
    }

    let mut out = String::new();
    let mut count = 0;

    for path in bilink_files_in(&bilink_dir) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        for n in [0u8, 1u8] {
            let Some(id) = bl.endpoint.get(n).link.capture_id() else { continue };
            if let Ok(cap) = bilink_format::Capture::load_in(layer_root, id) {
                out.push_str(&cap.file);
                out.push('\t');
                out.push_str(&format!("{uuid}.{n}"));
                out.push('\t');
                out.push_str(id);
                out.push('\n');
                count += 1;
            }
        }
    }

    let index_dir = bilink_dir.join("index");
    std::fs::create_dir_all(&index_dir)?;
    std::fs::write(index_dir.join("index"), &out)?;
    bilink_format::write_ignore(layer_root)?;

    Ok(count)
}

/// Returns the status of `.bilink/.index` for the given layer root.
pub fn status(layer_root: &Path) -> Result<IndexStatus> {
    let bilink_dir = layer_root.join(".bilink");
    let index_path = bilink_dir.join("index/index");

    if !index_path.exists() {
        return Ok(IndexStatus::Missing);
    }

    let index_mtime = mtime(&index_path)?;
    let stale = bilink_files_in(&bilink_dir)
        .iter()
        .filter(|p| mtime(p).map(|m| m > index_mtime).unwrap_or(false))
        .count();

    if stale > 0 {
        Ok(IndexStatus::Stale { stale_count: stale })
    } else {
        Ok(IndexStatus::Ok)
    }
}

/// Looks up bilinks referencing `file` in the given layer.
/// `file` is relative to `layer_root`.
/// Uses `.bilink/.index` if valid; falls back to O(N) scan silently.
pub fn lookup(layer_root: &Path, file: &str) -> Result<Vec<(String, u8)>> {
    let bilink_dir = layer_root.join(".bilink");
    let index_path = bilink_dir.join("index/index");

    if index_path.exists() && index_is_valid(&bilink_dir, &index_path) {
        lookup_from_index(&index_path, file)
    } else {
        lookup_scan(&bilink_dir, file)
    }
}

/// Finds all layer roots (directories containing `.bilink/`) under `root`.
pub fn layer_roots(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    collect_layer_roots(root, &mut result);
    result
}

// ── private ──────────────────────────────────────────────────────────────────

fn collect_layer_roots(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.join(".bilink").is_dir() {
        out.push(dir.to_path_buf());
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(name, "target" | "node_modules" | ".git") { continue; }
        collect_layer_roots(&path, out);
    }
}

fn bilink_files_in(bilink_dir: &Path) -> Vec<PathBuf> {
    bilink_format::bilink::bilink_files(bilink_dir)
}

fn index_is_valid(bilink_dir: &Path, index_path: &Path) -> bool {
    let Ok(index_mtime) = mtime(index_path) else { return false };
    bilink_files_in(bilink_dir)
        .iter()
        .all(|p| mtime(p).map(|m| m <= index_mtime).unwrap_or(true))
}

fn mtime(path: &Path) -> Result<SystemTime> {
    Ok(std::fs::metadata(path)?.modified()?)
}

fn lookup_from_index(index_path: &Path, file: &str) -> Result<Vec<(String, u8)>> {
    let text = std::fs::read_to_string(index_path)?;
    let mut results = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() { continue; }
        let Some((indexed_file, rest)) = line.split_once('\t') else { continue };
        // `<archivo>\t<uuid>.<N>\t<capture-uuid>` — la 3ra columna es opcional
        let ref_str = rest.split('\t').next().unwrap_or(rest);
        if indexed_file != file { continue; }
        let Some((uuid, n_str)) = ref_str.rsplit_once('.') else { continue };
        if let Ok(n) = n_str.parse::<u8>() {
            results.push((uuid.to_string(), n));
        }
    }
    Ok(results)
}

fn lookup_scan(bilink_dir: &Path, file: &str) -> Result<Vec<(String, u8)>> {
    let layer_root = bilink_dir.parent().unwrap_or(bilink_dir);
    let mut results = Vec::new();
    for path in bilink_files_in(bilink_dir) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        for n in [0u8, 1u8] {
            let Some(id) = bl.endpoint.get(n).link.capture_id() else { continue };
            if let Ok(cap) = bilink_format::Capture::load_in(layer_root, id) {
                if cap.file == file {
                    results.push((uuid.to_string(), n));
                }
            }
        }
    }
    Ok(results)
}


#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_chain(layer: &std::path::Path, uuid: &str, file: &str) {
        let cap = bilink_format::Capture { file: file.into(), query: None, offset: None };
        let (id, _, _) = cap.write_in(layer).unwrap();
        BiLink::new(format!("capture {id}").parse().unwrap(), "issue 3a".parse().unwrap())
            .write(&BiLink::path_in(layer, uuid)).unwrap();
    }

    #[test]
    fn the_index_maps_a_file_to_its_endpoints() {
        let d = tempdir().unwrap();
        write_chain(d.path(), "uuid1", "src/lib.rs");
        build(d.path()).unwrap();

        assert_eq!(lookup(d.path(), "src/lib.rs").unwrap(), vec![("uuid1".to_string(), 0u8)]);
        assert!(lookup(d.path(), "otro.rs").unwrap().is_empty());
    }

    /// Sin índice, el lookup cae a scan y da lo mismo. Nunca es fuente de verdad.
    #[test]
    fn a_missing_index_falls_back_to_scanning() {
        let d = tempdir().unwrap();
        write_chain(d.path(), "uuid1", "src/lib.rs");
        assert_eq!(lookup(d.path(), "src/lib.rs").unwrap(), vec![("uuid1".to_string(), 0u8)]);
    }
}
