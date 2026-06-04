use std::path::{Path, PathBuf};
use std::time::SystemTime;
use anyhow::Result;

use crate::bilink::BiLinkFile;
use crate::link::LinkEndpoint;

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
        let Ok(bl) = BiLinkFile::load(&path) else { continue };
        for (n, link) in [(0u8, &bl.link0), (1u8, &bl.link1)] {
            if let LinkEndpoint::Structural(sref) = link {
                out.push_str(&sref.file);
                out.push('\t');
                out.push_str(&format!("{}.{n}", bl.uuid));
                out.push('\n');
                count += 1;
            }
        }
    }

    let index_dir = bilink_dir.join("index");
    std::fs::create_dir_all(&index_dir)?;
    std::fs::write(index_dir.join("index"), &out)?;
    ensure_gitignore(&bilink_dir)?;

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
    let Ok(entries) = std::fs::read_dir(bilink_dir) else { return vec![] };
    entries.flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("bilink")
                && !p.ancestors().any(|a| a.ends_with(".pending"))
        })
        .collect()
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
        let Some((indexed_file, ref_str)) = line.split_once('\t') else { continue };
        if indexed_file != file { continue; }
        let Some((uuid, n_str)) = ref_str.rsplit_once('.') else { continue };
        if let Ok(n) = n_str.parse::<u8>() {
            results.push((uuid.to_string(), n));
        }
    }
    Ok(results)
}

fn lookup_scan(bilink_dir: &Path, file: &str) -> Result<Vec<(String, u8)>> {
    let mut results = Vec::new();
    for path in bilink_files_in(bilink_dir) {
        let Ok(bl) = BiLinkFile::load(&path) else { continue };
        for (n, link) in [(0u8, &bl.link0), (1u8, &bl.link1)] {
            if let LinkEndpoint::Structural(sref) = link {
                if sref.file == file {
                    results.push((bl.uuid.clone(), n));
                }
            }
        }
    }
    Ok(results)
}

fn ensure_gitignore(bilink_dir: &Path) -> Result<()> {
    let path = bilink_dir.join(".gitignore");
    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    let mut out = existing.clone();
    for entry in ["index/", ".pending/"] {
        if !existing.lines().any(|l| l.trim() == entry) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(entry);
            out.push('\n');
        }
    }

    if out != existing {
        std::fs::write(&path, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilink::BiLinkFile;
    use crate::link::{LinkEndpoint, StructuralRef};
    use tempfile::tempdir;

    fn make_bilink(bilink_dir: &Path, uuid: &str, file0: &str, file1: &str) {
        let bl = BiLinkFile {
            uuid: uuid.into(),
            link0: LinkEndpoint::Structural(StructuralRef { file: file0.into(), query: None, range: None }),
            link1: LinkEndpoint::Structural(StructuralRef { file: file1.into(), query: None, range: None }),
            subgraph: None,
            hash0: None, commit0: None,
            hash1: None, commit1: None,
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        std::fs::create_dir_all(bilink_dir).unwrap();
        bl.write(&bilink_dir.join(format!("{uuid}.bilink"))).unwrap();
    }

    #[test]
    fn build_creates_index_and_gitignore() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");

        let count = build(root).unwrap();
        assert_eq!(count, 2);

        let index = std::fs::read_to_string(root.join(".bilink/index/index")).unwrap();
        assert!(index.contains("a.md\tuuid1.0"));
        assert!(index.contains("b.md\tuuid1.1"));

        let gi = std::fs::read_to_string(root.join(".bilink/.gitignore")).unwrap();
        assert!(gi.contains("index/"));
        assert!(gi.contains(".pending/"));
    }

    #[test]
    fn lookup_uses_index_when_valid() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");
        build(root).unwrap();

        let results = lookup(root, "a.md").unwrap();
        assert_eq!(results, vec![("uuid1".to_string(), 0u8)]);
    }

    #[test]
    fn lookup_falls_back_to_scan_when_no_index() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");

        let results = lookup(root, "b.md").unwrap();
        assert_eq!(results, vec![("uuid1".to_string(), 1u8)]);
    }

    #[test]
    fn status_missing_when_no_index() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");

        assert_eq!(status(root).unwrap(), IndexStatus::Missing);
    }

    #[test]
    fn status_ok_after_build() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");
        build(root).unwrap();

        assert_eq!(status(root).unwrap(), IndexStatus::Ok);
    }

    #[test]
    fn gitignore_is_idempotent() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        make_bilink(&root.join(".bilink"), "uuid1", "a.md", "b.md");
        build(root).unwrap();
        build(root).unwrap();

        let gi = std::fs::read_to_string(root.join(".bilink/.gitignore")).unwrap();
        assert_eq!(gi.matches("index/").count(), 1);
    }
}
