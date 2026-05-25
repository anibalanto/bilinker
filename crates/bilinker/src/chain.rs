use std::path::{Component, Path, PathBuf};
use anyhow::{bail, Result};
use uuid::Uuid;

use crate::bilink::BiLinkFile;
use crate::link::{LinkEndpoint, StructuralRef};

pub struct ChainNew {
    pub uuid: String,
    pub files: Vec<PathBuf>,
}

/// Creates a new chain or direct link.
///
/// `tips`: exactly 2 entries of (layer_path_relative_to_root, structural_endpoint).
/// `mids`: ordered layer paths between the two tips.
/// All paths are relative to `root`.
pub fn chain_new(
    root: &Path,
    tips: &[(PathBuf, LinkEndpoint)],
    mids: &[PathBuf],
) -> Result<ChainNew> {
    if tips.len() != 2 {
        bail!("chain new requires exactly 2 --tip arguments");
    }

    let uuid = Uuid::new_v4().to_string();

    let all_layers: Vec<PathBuf> = {
        let mut v = vec![tips[0].0.clone()];
        v.extend_from_slice(mids);
        v.push(tips[1].0.clone());
        v
    };

    let n = all_layers.len();
    let mut created = Vec::new();

    // Same-layer direct link: both tips in the same directory → one file.
    if n == 2 && normalize(&all_layers[0]) == normalize(&all_layers[1]) {
        let bl = BiLinkFile {
            uuid:        uuid.clone(),
            link0:       tips[0].1.clone(),
            link1:       tips[1].1.clone(),
            hash0:       None, hash1: None,
            range0:      None, range1: None,
            state0:      None, state1: None,
            resolved_at: None,
        };
        let path = bilink_path(root, &all_layers[0], &uuid);
        bl.write(&path)?;
        created.push(path);
        return Ok(ChainNew { uuid, files: created });
    }

    // Multi-layer chain
    for i in 0..n {
        let layer = &all_layers[i];

        let (link0, link1) = if i == 0 {
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            (tips[0].1.clone(), to_next)
        } else if i == n - 1 {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            (to_prev, tips[1].1.clone())
        } else {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            (to_prev, to_next)
        };

        let bl = BiLinkFile {
            uuid:        uuid.clone(),
            link0,
            link1,
            hash0:       None, hash1: None,
            range0:      None, range1: None,
            state0:      None, state1: None,
            resolved_at: None,
        };
        let path = bilink_path(root, layer, &uuid);
        bl.write(&path)?;
        created.push(path);
    }

    Ok(ChainNew { uuid, files: created })
}

/// Resolves the `.bilink/<uuid>.bilink` path for a layer endpoint at `target_layer`.
pub fn resolve_layer_link(
    bilink_file: &Path,
    layer_root: &Path,
    link_path: &Path,
    uuid: &str,
) -> PathBuf {
    let _ = bilink_file;
    layer_root.join(link_path).join(".bilink").join(format!("{uuid}.bilink"))
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn bilink_path(root: &Path, layer: &Path, uuid: &str) -> PathBuf {
    root.join(layer).join(".bilink").join(format!("{uuid}.bilink"))
}

fn layer_endpoint(from_layer: &Path, to_layer: &Path) -> Result<LinkEndpoint> {
    let rel = diff_paths(to_layer, from_layer);
    let rel_str = rel.to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 path: {}", rel.display()))?;
    let tokens = estrato::parse_path(rel_str)
        .map_err(|e| anyhow::anyhow!("invalid layer path '{rel_str}': {e}"))?;
    Ok(LinkEndpoint::Layer(tokens))
}

fn diff_paths(to: &Path, from: &Path) -> PathBuf {
    let to_norm   = normalize(to);
    let from_norm = normalize(from);
    let to_parts: Vec<Component>   = to_norm.components().collect();
    let from_parts: Vec<Component> = from_norm.components().collect();

    let common = to_parts.iter()
        .zip(from_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();
    for _ in &from_parts[common..] {
        result.push("..");
    }
    for c in &to_parts[common..] {
        result.push(c);
    }
    result
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilink::BiLinkFile;
    use crate::link::{LinkEndpoint, StructuralRef};
    use tempfile::tempdir;

    fn whole_file(file: &str) -> LinkEndpoint {
        LinkEndpoint::Structural(StructuralRef {
            file: file.into(),
            query: None,
            range: None,
        })
    }

    fn is_layer(ep: &LinkEndpoint) -> bool {
        matches!(ep, LinkEndpoint::Layer(_))
    }
    fn is_structural(ep: &LinkEndpoint) -> bool {
        matches!(ep, LinkEndpoint::Structural(_))
    }

    // ─── diff_paths ──────────────────────────────────────────────────────────

    #[test]
    fn diff_paths_root_to_child() {
        assert_eq!(
            diff_paths(Path::new(".estrato/tech-decisions"), Path::new(".")),
            PathBuf::from(".estrato/tech-decisions")
        );
    }

    #[test]
    fn diff_paths_child_to_root() {
        assert_eq!(
            diff_paths(Path::new("."), Path::new(".estrato/tech-decisions")),
            PathBuf::from("../..")
        );
    }

    #[test]
    fn diff_paths_sibling_layers() {
        assert_eq!(
            diff_paths(
                Path::new(".estrato/tech-decisions/.estrato/impl"),
                Path::new(".estrato/tech-decisions"),
            ),
            PathBuf::from(".estrato/impl")
        );
    }

    // ─── chain_new ───────────────────────────────────────────────────────────

    #[test]
    fn chain_new_direct_link_single_file() {
        let dir   = tempdir().unwrap();
        let root  = dir.path();
        let tips  = vec![
            (PathBuf::from("."), whole_file("a.md")),
            (PathBuf::from("."), whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[]).unwrap();

        assert_eq!(result.files.len(), 1);
        let bl = BiLinkFile::load(&result.files[0]).unwrap();
        assert!(is_structural(&bl.link0), "direct link: link0 must be structural");
        assert!(is_structural(&bl.link1), "direct link: link1 must be structural");
    }

    #[test]
    fn chain_new_adjacent_layers_two_files() {
        let dir  = tempdir().unwrap();
        let root = dir.path();
        let tips = vec![
            (PathBuf::from("."),             whole_file("a.md")),
            (PathBuf::from(".estrato/impl"), whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[]).unwrap();

        assert_eq!(result.files.len(), 2);

        let tip0 = BiLinkFile::load(&result.files[0]).unwrap();
        assert!(is_structural(&tip0.link0), "tip0.link0 must be structural");
        assert!(is_layer(&tip0.link1),      "tip0.link1 must be layer");

        let tip1 = BiLinkFile::load(&result.files[1]).unwrap();
        assert!(is_layer(&tip1.link0),      "tip1.link0 must be layer");
        assert!(is_structural(&tip1.link1), "tip1.link1 must be structural");
    }

    #[test]
    fn chain_new_three_layers_correct_endpoints() {
        let dir  = tempdir().unwrap();
        let root = dir.path();
        let tips = vec![
            (PathBuf::from("."),                            whole_file("a.md")),
            (PathBuf::from(".estrato/td/.estrato/impl"),   whole_file("b.md")),
        ];
        let mids = vec![PathBuf::from(".estrato/td")];

        let result = chain_new(root, &tips, &mids).unwrap();
        assert_eq!(result.files.len(), 3);

        let tip0 = BiLinkFile::load(&result.files[0]).unwrap();
        let mid  = BiLinkFile::load(&result.files[1]).unwrap();
        let tip1 = BiLinkFile::load(&result.files[2]).unwrap();

        assert!(is_structural(&tip0.link0));
        assert!(is_layer(&tip0.link1));

        assert!(is_layer(&mid.link0));
        assert!(is_layer(&mid.link1));

        assert!(is_layer(&tip1.link0));
        assert!(is_structural(&tip1.link1));
    }

    #[test]
    fn chain_new_uuid_shared_across_files() {
        let dir  = tempdir().unwrap();
        let root = dir.path();
        let tips = vec![
            (PathBuf::from("."),             whole_file("a.md")),
            (PathBuf::from(".estrato/impl"), whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[]).unwrap();

        let uuid0 = BiLinkFile::load(&result.files[0]).unwrap().uuid;
        let uuid1 = BiLinkFile::load(&result.files[1]).unwrap().uuid;
        assert_eq!(uuid0, uuid1);
        assert_eq!(uuid0, result.uuid);
    }

    #[test]
    fn chain_new_layer_paths_are_correct() {
        let dir  = tempdir().unwrap();
        let root = dir.path();
        let tips = vec![
            (PathBuf::from("."),               whole_file("a.md")),
            (PathBuf::from(".estrato/impl"),   whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[]).unwrap();

        let tip0 = BiLinkFile::load(&result.files[0]).unwrap();
        assert_eq!(tip0.link1.to_string(), ".estrato/impl");

        let tip1 = BiLinkFile::load(&result.files[1]).unwrap();
        assert_eq!(tip1.link0.to_string(), "../..");
    }
}
