use std::path::{Component, Path, PathBuf};
use anyhow::{bail, Result};
use uuid::Uuid;

use crate::bilink::BiLinkFile;
use crate::link::LinkEndpoint;

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
    no_subgraph: bool,
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
        let sg0 = if no_subgraph { None } else { detect_scip_symbol(root, &all_layers[0], &tips[0].1) };
        let sg1 = if no_subgraph { None } else { detect_scip_symbol(root, &all_layers[1], &tips[1].1) };
        let bl = BiLinkFile {
            uuid:      uuid.clone(),
            link0:     tips[0].1.clone(),
            link1:     tips[1].1.clone(),
            subgraph0: sg0,
            subgraph1: sg1,
            hash0: None, hash_ast0: None, commit0: None,
            hash1: None, hash_ast1: None, commit1: None,
            range0:    None, range1: None,
            state0:    None, state1: None,
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

        let (link0, link1, sg0, sg1) = if i == 0 {
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            let sg = if no_subgraph { None } else { detect_scip_symbol(root, layer, &tips[0].1) };
            // tip0: link0=structural, link1=layer → subgraph.0
            (tips[0].1.clone(), to_next, sg, None)
        } else if i == n - 1 {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            let sg = if no_subgraph { None } else { detect_scip_symbol(root, layer, &tips[1].1) };
            // tip1: link0=layer, link1=structural → subgraph.1
            (to_prev, tips[1].1.clone(), None, sg)
        } else {
            let to_prev = layer_endpoint(layer, &all_layers[i - 1])?;
            let to_next = layer_endpoint(layer, &all_layers[i + 1])?;
            (to_prev, to_next, None, None)
        };

        let bl = BiLinkFile {
            uuid:      uuid.clone(),
            link0,
            link1,
            subgraph0: sg0,
            subgraph1: sg1,
            hash0: None, hash_ast0: None, commit0: None,
            hash1: None, hash_ast1: None, commit1: None,
            range0:    None, range1: None,
            state0:    None, state1: None,
            resolved_at: None,
        };
        let path = bilink_path(root, layer, &uuid);
        bl.write(&path)?;
        created.push(path);
    }

    Ok(ChainNew { uuid, files: created })
}

/// Resolves the `.bilink/<uuid>.bilink` path for a layer endpoint at `target_layer`.
/// Retrofits existing bilinks in `layer_root` with `subgraph.N` where the
/// structural endpoint matches a callable symbol in the SCIP index.
/// Returns the number of bilinks updated.
pub fn scip_retrofit(layer_root: &Path) -> anyhow::Result<usize> {
    use crate::bilink::walkdir;
    let bilink_dir = layer_root.join(".bilink");
    if !bilink_dir.exists() { return Ok(0); }

    let scip_path = bilink_dir.join("index/index.scip");
    if !scip_path.exists() {
        anyhow::bail!("no index.scip found at {} — run `rust-analyzer scip .` first", scip_path.display());
    }

    let index = crate::scip_index::ScipIndex::load(&scip_path, layer_root)?;
    let mut count = 0;

    for path in walkdir(&bilink_dir)? {
        if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        if path.ancestors().any(|a| a.ends_with(".pending")) { continue; }

        let mut bl = crate::bilink::BiLinkFile::load(&path)?;
        let mut changed = false;

        if bl.subgraph0.is_none() {
            if let Some(sym) = detect_scip_symbol_from_bilink(&index, &bl.link0) {
                eprintln!("  {} → subgraph.0: {}", bl.uuid[..8].to_string(), &sym[sym.rfind('/').unwrap_or(0)..]);
                bl.subgraph0 = Some(sym);
                changed = true;
            }
        }
        if bl.subgraph1.is_none() {
            if let Some(sym) = detect_scip_symbol_from_bilink(&index, &bl.link1) {
                eprintln!("  {} → subgraph.1: {}", bl.uuid[..8].to_string(), &sym[sym.rfind('/').unwrap_or(0)..]);
                bl.subgraph1 = Some(sym);
                changed = true;
            }
        }

        if changed {
            bl.write(&path)?;
            count += 1;
        }
    }
    Ok(count)
}

fn detect_scip_symbol_from_bilink(
    index: &crate::scip_index::ScipIndex,
    endpoint: &crate::link::LinkEndpoint,
) -> Option<String> {
    use crate::link::LinkEndpoint;
    let LinkEndpoint::Structural(sref) = endpoint else { return None };

    let range = if let Some(r) = &sref.range {
        r.clone()
    } else if let Some(query_str) = &sref.query {
        // We don't have layer_root here — use a stub path; ScipIndex has the full paths
        // Try to find by file name match in definitions
        return index.find_callable_in_file(&sref.file);
    } else {
        return None;
    };

    index.find_callable_at(&sref.file, &range)
}

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
    let tokens = filesystem_to_stratum_tokens(&rel)?;
    Ok(LinkEndpoint::Layer(tokens))
}

/// Converts a filesystem relative path (as produced by `diff_paths`) into stratum tokens.
///
/// - Leading `../..` pairs → `PathToken::Up` (one stratum level = 2 fs components)
/// - Following `.stratum/<name>` pairs → `PathToken::Down`
/// - Any remaining components → `PathToken::Simple`
fn filesystem_to_stratum_tokens(rel: &Path) -> Result<stratum::StratumPath> {
    use stratum::PathToken;

    let components: Vec<Component> = rel.components().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i + 1 < components.len()
        && components[i] == Component::ParentDir
        && components[i + 1] == Component::ParentDir
    {
        tokens.push(PathToken::Up);
        i += 2;
    }
    if i < components.len() && components[i] == Component::ParentDir {
        anyhow::bail!("malformed stratum path: odd number of `..` in {}", rel.display());
    }

    while i + 1 < components.len() {
        if let (Component::Normal(a), Component::Normal(b)) = (&components[i], &components[i + 1]) {
            if *a == std::ffi::OsStr::new(".stratum") {
                let name = b.to_str().ok_or_else(|| anyhow::anyhow!("non-UTF8 layer name"))?;
                tokens.push(PathToken::Down(name.to_string()));
                i += 2;
                continue;
            }
        }
        break;
    }

    if i < components.len() {
        let remaining: std::path::PathBuf = components[i..].iter().collect();
        tokens.push(PathToken::Simple(remaining));
    }

    if tokens.is_empty() {
        anyhow::bail!("empty stratum path for {}", rel.display());
    }

    Ok(tokens)
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

/// Finds the SCIP symbol for a structural endpoint by looking up
/// the callable symbol whose body contains the endpoint's range.
fn detect_scip_symbol(root: &Path, layer: &Path, endpoint: &LinkEndpoint) -> Option<String> {
    let LinkEndpoint::Structural(sref) = endpoint else { return None };
    let layer_root = root.join(layer);
    let scip_path  = layer_root.join(".bilink/index/index.scip");
    if !scip_path.exists() { return None; }

    let index = crate::scip_index::ScipIndex::load(&scip_path, &layer_root).ok()?;

    // Need a byte range to look up the symbol.
    // Use stored range if available, otherwise run the tree-sitter query.
    let range = if let Some(r) = &sref.range {
        r.clone()
    } else if let Some(query_str) = &sref.query {
        let source = std::fs::read_to_string(layer_root.join(&sref.file)).ok()?;
        let lang = crate::grammar::language_for_file(&sref.file);
        let language = crate::grammar::for_language(lang).ok()?;
        let (start, end) = crate::query::find_target(language, &source, query_str).ok()??;
        crate::link::ByteRange { start, end }
    } else {
        return None;
    };

    index.find_callable_at(&sref.file, &range)
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

    // ─── filesystem_to_stratum_tokens ────────────────────────────────────────

    #[test]
    fn stratum_tokens_up_one() {
        let tokens = filesystem_to_stratum_tokens(Path::new("../..")).unwrap();
        let ep = LinkEndpoint::Layer(tokens);
        assert_eq!(ep.to_string(), "<");
    }

    #[test]
    fn stratum_tokens_up_two() {
        let tokens = filesystem_to_stratum_tokens(Path::new("../../../..")).unwrap();
        let ep = LinkEndpoint::Layer(tokens);
        assert_eq!(ep.to_string(), "<<");
    }

    #[test]
    fn stratum_tokens_down_one() {
        let tokens = filesystem_to_stratum_tokens(Path::new(".stratum/impl")).unwrap();
        let ep = LinkEndpoint::Layer(tokens);
        assert_eq!(ep.to_string(), ">impl");
    }

    #[test]
    fn stratum_tokens_down_two() {
        let tokens = filesystem_to_stratum_tokens(Path::new(".stratum/td/.stratum/impl")).unwrap();
        let ep = LinkEndpoint::Layer(tokens);
        assert_eq!(ep.to_string(), ">td>impl");
    }

    // ─── diff_paths ──────────────────────────────────────────────────────────

    #[test]
    fn diff_paths_root_to_child() {
        assert_eq!(
            diff_paths(Path::new(".stratum/tech-decisions"), Path::new(".")),
            PathBuf::from(".stratum/tech-decisions")
        );
    }

    #[test]
    fn diff_paths_child_to_root() {
        assert_eq!(
            diff_paths(Path::new("."), Path::new(".stratum/tech-decisions")),
            PathBuf::from("../..")
        );
    }

    #[test]
    fn diff_paths_sibling_layers() {
        assert_eq!(
            diff_paths(
                Path::new(".stratum/tech-decisions/.stratum/impl"),
                Path::new(".stratum/tech-decisions"),
            ),
            PathBuf::from(".stratum/impl")
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
        let result = chain_new(root, &tips, &[], true).unwrap();

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
            (PathBuf::from(".stratum/impl"), whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[], true).unwrap();

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
            (PathBuf::from(".stratum/td/.stratum/impl"),   whole_file("b.md")),
        ];
        let mids = vec![PathBuf::from(".stratum/td")];

        let result = chain_new(root, &tips, &mids, true).unwrap();
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
            (PathBuf::from(".stratum/impl"), whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[], true).unwrap();

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
            (PathBuf::from(".stratum/impl"),   whole_file("b.md")),
        ];
        let result = chain_new(root, &tips, &[], true).unwrap();

        let tip0 = BiLinkFile::load(&result.files[0]).unwrap();
        assert_eq!(tip0.link1.to_string(), ">impl");

        let tip1 = BiLinkFile::load(&result.files[1]).unwrap();
        assert_eq!(tip1.link0.to_string(), "<");
    }
}
