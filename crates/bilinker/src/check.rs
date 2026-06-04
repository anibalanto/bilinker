use std::collections::HashSet;
use std::path::{Path, PathBuf};
use anyhow::Result;
use chrono::Utc;

use crate::bilink::{walkdir, BiLinkFile};
use crate::chain::resolve_layer_link;
use crate::grammar;
use crate::hash;
use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
use crate::query;
use crate::scip_index::{check_or_create_sciplink, ScipIndex};
use crate::sciplink::{sciplink_path, ScipLinkState};
use crate::task::resolve_task_path;

#[derive(Debug)]
pub struct CheckResult {
    pub uuid: String,
    pub state0: EndpointState,
    pub state1: EndpointState,
    pub updated: bool,
}

impl CheckResult {
    pub fn is_clean(&self) -> bool {
        use EndpointState::*;
        matches!(self.state0, Ok | Moved | Displaced | Reanchored | Expanded | Todo)
            && matches!(self.state1, Ok | Moved | Displaced | Reanchored | Expanded | Todo)
    }
}

pub fn check(root: &Path, path: &Path) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();

    if path.is_file() {
        results.push(check_file(root, path)?);
        return Ok(results);
    }

    let bilink_dir = if path.ends_with(".bilink") { path.to_path_buf() }
                     else { path.join(".bilink") };

    for entry in walkdir(&bilink_dir)? {
        if entry.extension().and_then(|e| e.to_str()) == Some("bilink")
            && !entry.ancestors().any(|a| a.ends_with(".pending"))
        {
            results.push(check_file(root, &entry)?);
        }
    }
    Ok(results)
}

fn check_file(root: &Path, bilink_path: &Path) -> Result<CheckResult> {
    let mut bl = BiLinkFile::load(bilink_path)?;

    let layer_root = bilink_path
        .parent().and_then(|p| p.parent())
        .unwrap_or(root);

    let uuid = bl.uuid.clone();

    let (state0, range0) =
        check_endpoint(root, layer_root, &bl.link0, &uuid, bl.hash0.as_deref(), bl.range0.as_ref())?;

    let (state1, range1) =
        check_endpoint(root, layer_root, &bl.link1, &uuid, bl.hash1.as_deref(), bl.range1.as_ref())?;

    let updated = bl.state0.as_ref() != Some(&state0)
        || bl.state1.as_ref() != Some(&state1)
        || bl.range0 != range0
        || bl.range1 != range1;

    bl.range0      = range0;
    bl.range1      = range1;
    bl.state0      = Some(state0.clone());
    bl.state1      = Some(state1.clone());
    bl.resolved_at = Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

    bl.write(bilink_path)?;

    // Check subgraph if declared
    if let Some(subgraph_symbol) = &bl.subgraph.clone() {
        let bilink_dir = layer_root.join(".bilink");
        let scip_path  = bilink_dir.join("index/index.scip");
        if scip_path.exists() {
            if let Ok(index) = ScipIndex::load(&scip_path, layer_root) {
                let _ = check_subgraph(&index, layer_root, &bilink_dir, subgraph_symbol, false);
            }
        }
    }

    Ok(CheckResult { uuid, state0, state1, updated })
}

pub fn check_subgraph(
    index: &ScipIndex,
    layer_root: &Path,
    bilink_dir: &Path,
    root_symbol: &str,
    prune: bool,
) -> Result<Vec<(String, ScipLinkState)>> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    check_subgraph_recursive(index, layer_root, bilink_dir, root_symbol, &mut visited, &mut results, prune)?;
    Ok(results)
}

fn check_subgraph_recursive(
    index: &ScipIndex,
    layer_root: &Path,
    bilink_dir: &Path,
    symbol: &str,
    visited: &mut HashSet<String>,
    results: &mut Vec<(String, ScipLinkState)>,
    prune: bool,
) -> Result<()> {
    if !visited.insert(symbol.to_string()) { return Ok(()); }

    // Check or create the sciplink for this symbol
    if let Some((file, range)) = index.definition(symbol) {
        let path = sciplink_path(bilink_dir, symbol);

        // RENAMED: sciplink exists but symbol moved
        if path.exists() {
            if let Ok(sl) = crate::sciplink::ScipLink::load(&path) {
                if sl.symbol != symbol {
                    // update symbol in place
                    let updated = crate::sciplink::ScipLink {
                        symbol: symbol.to_string(),
                        file: file.to_string(),
                        range: range.clone(),
                        ..sl
                    }.with_state(ScipLinkState::Renamed);
                    updated.write(&path)?;
                    results.push((symbol.to_string(), ScipLinkState::Renamed));
                    return Ok(());
                }
            }
        }

        let sl = check_or_create_sciplink(&path, symbol, file, range, layer_root)?;
        let state = sl.state.clone().unwrap_or(ScipLinkState::Ok);
        results.push((symbol.to_string(), state));
    } else {
        // Symbol not in index — DELETED or RENAMED
        let path = sciplink_path(bilink_dir, symbol);
        if path.exists() {
            if let Ok(sl) = crate::sciplink::ScipLink::load(&path) {
                // Try to find by location
                if let Some(new_sym) = index.find_by_location(&sl.file, &sl.range) {
                    let updated = crate::sciplink::ScipLink {
                        symbol: new_sym.clone(),
                        ..sl
                    }.with_state(ScipLinkState::Renamed);
                    updated.write(&path)?;
                    results.push((symbol.to_string(), ScipLinkState::Renamed));
                } else {
                    let deleted = sl.with_state(ScipLinkState::Deleted);
                    if prune {
                        std::fs::remove_file(&path)?;
                    } else {
                        deleted.write(&path)?;
                    }
                    results.push((symbol.to_string(), ScipLinkState::Deleted));
                }
            }
        }
        return Ok(());
    }

    // Recurse into direct callees
    for (callee, callee_file, callee_range) in index.direct_callees(symbol) {
        check_subgraph_recursive(index, layer_root, bilink_dir, &callee, visited, results, prune)?;
    }

    Ok(())
}

fn check_endpoint(
    root: &Path,
    layer_root: &Path,
    endpoint: &LinkEndpoint,
    uuid: &str,
    hash: Option<&str>,
    stored_range: Option<&ByteRange>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    match endpoint {
        LinkEndpoint::Structural(sref) => check_structural(root, sref, hash, stored_range),
        LinkEndpoint::Layer(tokens)    => check_layer(layer_root, tokens, uuid, hash),
        LinkEndpoint::Task(id)         => check_task(layer_root, id, hash),
    }
}

fn check_structural(
    root: &Path,
    sref: &StructuralRef,
    hash: Option<&str>,
    stored_range: Option<&ByteRange>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let file_path = root.join(&sref.file);

    if !file_path.exists() {
        return Ok((EndpointState::Broken, None));
    }

    let source = std::fs::read_to_string(&file_path)?;

    let Some(query_str) = &sref.query else {
        let new_hash = hash::sha256(source.as_bytes());
        let range    = ByteRange { start: 0, end: source.len() };
        let state = if hash.is_none() {
            EndpointState::Pending
        } else if hash == Some(new_hash.as_str()) {
            EndpointState::Ok
        } else {
            EndpointState::Altered
        };
        return Ok((state, Some(range)));
    };

    let lang     = grammar::language_for_file(&sref.file);
    let language = grammar::for_language(lang)?;
    let node_range = query::find_target(language, &source, query_str)?;

    let Some((node_start, node_end)) = node_range else {
        return Ok((EndpointState::Unanchored, None));
    };

    let (frag_start, frag_end) = match &sref.range {
        Some(r) => (node_start + r.start, (node_start + r.end).min(source.len())),
        None    => (node_start, node_end),
    };
    let fragment  = &source[frag_start..frag_end];
    let new_hash  = hash::sha256(fragment.as_bytes());
    let new_range = ByteRange { start: frag_start, end: frag_end };

    if hash.is_none() {
        return Ok((EndpointState::Pending, Some(new_range)));
    }

    if hash == Some(new_hash.as_str()) {
        return Ok((EndpointState::Ok, Some(new_range)));
    }

    if let (Some(stored_hash), Some(sr)) = (hash, stored_range) {
        let frag_len = sr.end - sr.start;
        if let Some(displaced) = find_in_node(&source, node_start, node_end, stored_hash, frag_len) {
            return Ok((EndpointState::Displaced, Some(displaced)));
        }
    }

    Ok((EndpointState::Altered, Some(new_range)))
}

fn check_layer(
    layer_root: &Path,
    tokens: &stratum::StratumPath,
    uuid: &str,
    stored_hash: Option<&str>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let absent = if stored_hash.is_none() { EndpointState::Todo } else { EndpointState::Broken };

    let target_layer = match stratum::resolve(layer_root, layer_root, tokens) {
        Ok(p)  => p,
        Err(_) => return Ok((absent, None)),
    };

    let target_bilink = resolve_layer_link(
        &layer_root.join(".bilink").join(format!("{uuid}.bilink")),
        layer_root,
        &target_layer,
        uuid,
    );

    if !target_bilink.exists() {
        return Ok((absent, None));
    }

    // Hash = structural endpoint's accepted hash in the adjacent bilink.
    // This avoids circular dependency: accepting a layer endpoint never modifies
    // the adjacent bilink file, so the hash never cascades back.
    let adj_bl = crate::bilink::BiLinkFile::load(&target_bilink)?;
    let Some(adj_struct_hash) = adj_bl.structural_hash() else {
        return Ok((EndpointState::Pending, None));
    };

    let state = if stored_hash.is_none() {
        EndpointState::Pending
    } else if stored_hash == Some(adj_struct_hash) {
        EndpointState::Ok
    } else {
        EndpointState::ChainDirty
    };

    Ok((state, None))
}

fn check_task(
    layer_root: &Path,
    task_id: &str,
    stored_hash: Option<&str>,
) -> Result<(EndpointState, Option<ByteRange>)> {
    let (task_path, _) = resolve_task_path(layer_root, task_id);
    let task_dir = match task_path.parent() {
        Some(d) => d.to_path_buf(),
        None => return Ok((EndpointState::Broken, None)),
    };
    let filename = match task_path.file_name().and_then(|n| n.to_str()) {
        Some(f) => f.to_string(),
        None => return Ok((EndpointState::Broken, None)),
    };
    let sref = StructuralRef { file: filename, query: None, range: None };
    check_structural(&task_dir, &sref, stored_hash, None)
}

fn find_in_node(
    source: &str,
    node_start: usize,
    node_end: usize,
    target_hash: &str,
    frag_len: usize,
) -> Option<ByteRange> {
    if frag_len == 0 || frag_len > node_end.saturating_sub(node_start) {
        return None;
    }
    let node = &source[node_start..node_end];
    let mut start = 0;
    while start + frag_len <= node.len() {
        if source.is_char_boundary(node_start + start) {
            let end = start + frag_len;
            if end <= node.len() && source.is_char_boundary(node_start + end) {
                if hash::sha256(node[start..end].as_bytes()) == target_hash {
                    return Some(ByteRange {
                        start: node_start + start,
                        end: node_start + end,
                    });
                }
            }
        }
        start += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bilink::BiLinkFile;
    use crate::hash;
    use crate::link::{ByteRange, EndpointState, LinkEndpoint, StructuralRef};
    use tempfile::tempdir;

    fn whole_file_endpoint(file: &str) -> LinkEndpoint {
        LinkEndpoint::Structural(StructuralRef {
            file: file.into(),
            query: None,
            range: None,
        })
    }

    fn layer_endpoint(path: &str) -> LinkEndpoint {
        LinkEndpoint::Layer(stratum::parse_path(path).unwrap())
    }

    fn make_bilink(dir: &Path, uuid: &str, link0: LinkEndpoint, link1: LinkEndpoint) -> std::path::PathBuf {
        let bl = BiLinkFile {
            uuid: uuid.into(),
            link0, link1,
            subgraph: None,
            hash0: None, commit0: None,
            hash1: None, commit1: None,
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        path
    }

    #[test]
    fn check_whole_file_first_time_is_pending() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "hello world").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("a.md"),
            whole_file_endpoint("a.md"),
        );

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Pending);
        assert_eq!(result.state1, EndpointState::Pending);
    }

    #[test]
    fn check_whole_file_ok_when_hash_matches() {
        let dir = tempdir().unwrap();
        let content = b"stable content";
        std::fs::write(dir.path().join("a.md"), content).unwrap();
        let stored_hash = hash::sha256(content);

        let bilink_dir = dir.path().join(".bilink");
        let bl = BiLinkFile {
            uuid:    "uuid1".into(),
            link0:   whole_file_endpoint("a.md"),
            link1:   whole_file_endpoint("a.md"),
            subgraph: None,
            hash0:   Some(stored_hash.clone()),
            commit0: Some("abc1234".into()),
            hash1:   Some(stored_hash),
            commit1: Some("abc1234".into()),
            range0:  Some(ByteRange { start: 0, end: content.len() }),
            range1:  Some(ByteRange { start: 0, end: content.len() }),
            state0:  Some(EndpointState::Ok),
            state1:  Some(EndpointState::Ok),
            resolved_at: Some("2026-01-01T00:00:00Z".into()),
        };
        let path = bilink_dir.join("uuid1.bilink");
        bl.write(&path).unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Ok);
    }

    #[test]
    fn check_whole_file_altered_when_hash_differs() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "new content").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let bl = BiLinkFile {
            uuid:    "uuid1".into(),
            link0:   whole_file_endpoint("a.md"),
            link1:   whole_file_endpoint("a.md"),
            subgraph: None,
            hash0:   Some("old-hash-that-wont-match".into()),
            commit0: Some("abc1234".into()),
            hash1:   Some("old-hash-that-wont-match".into()),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = bilink_dir.join("uuid1.bilink");
        bl.write(&path).unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Altered);
    }

    #[test]
    fn check_structural_broken_when_file_missing() {
        let dir = tempdir().unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("missing.md"),
            whole_file_endpoint("missing.md"),
        );

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state0, EndpointState::Broken);
    }

    #[test]
    fn check_layer_first_time_is_pending() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        std::fs::write(adj_dir.join(format!("{uuid}.bilink")), "link.0: a.md\nlink.1: b.md\n").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, uuid,
            whole_file_endpoint("a.md"),
            layer_endpoint(".stratum/impl"),
        );
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Pending);
    }

    #[test]
    fn check_layer_ok_when_hash_matches() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        // Adjacent bilink has an accepted structural endpoint (link.1 = b.md, hash.1 set)
        let adj_struct_hash = "deadbeefdeadbeef".to_string();
        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        let adj_bl = BiLinkFile {
            uuid:    uuid.into(),
            link0:   layer_endpoint("../.."),
            link1:   whole_file_endpoint("b.md"),
            subgraph: None,
            hash0:   None, commit0: None,
            hash1:   Some(adj_struct_hash.clone()),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        adj_bl.write(&adj_dir.join(format!("{uuid}.bilink"))).unwrap();

        // Spec bilink stores adj structural hash as its layer endpoint hash
        let bilink_dir = dir.path().join(".bilink");
        let bl = BiLinkFile {
            uuid:    uuid.into(),
            link0:   whole_file_endpoint("a.md"),
            link1:   layer_endpoint(".stratum/impl"),
            subgraph: None,
            hash0:   None, commit0: None,
            hash1:   Some(adj_struct_hash),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Ok);
    }

    #[test]
    fn check_layer_chain_dirty_when_hash_differs() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        // Adjacent bilink has structural hash "current-hash"
        let adj_dir = dir.path().join(".stratum/impl/.bilink");
        std::fs::create_dir_all(&adj_dir).unwrap();
        let adj_bl = BiLinkFile {
            uuid:    uuid.into(),
            link0:   layer_endpoint("../.."),
            link1:   whole_file_endpoint("b.md"),
            subgraph: None,
            hash0:   None, commit0: None,
            hash1:   Some("current-hash".into()),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        adj_bl.write(&adj_dir.join(format!("{uuid}.bilink"))).unwrap();

        // Spec bilink stores a different (stale) hash
        let bilink_dir = dir.path().join(".bilink");
        let bl = BiLinkFile {
            uuid:    uuid.into(),
            link0:   whole_file_endpoint("a.md"),
            link1:   layer_endpoint(".stratum/impl"),
            subgraph: None,
            hash0:   None, commit0: None,
            hash1:   Some("stale-hash-000".into()),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();
        std::fs::write(dir.path().join("a.md"), "content").unwrap();

        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::ChainDirty);
    }

    #[test]
    fn check_layer_todo_when_adjacent_missing_and_no_hash() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let bilink_dir = dir.path().join(".bilink");
        std::fs::write(dir.path().join("a.md"), "content").unwrap();
        let path = make_bilink(&bilink_dir, uuid,
            whole_file_endpoint("a.md"),
            layer_endpoint(".stratum/impl"),
        );

        // No hash stored, target layer doesn't exist → TODO (intentional absence)
        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Todo);
    }

    #[test]
    fn check_layer_broken_when_adjacent_missing_but_had_hash() {
        let dir = tempdir().unwrap();
        let uuid = "aaaabbbb-cccc-dddd-eeee-ffffaaaabbbb";

        let bilink_dir = dir.path().join(".bilink");
        std::fs::write(dir.path().join("a.md"), "content").unwrap();
        let bl = BiLinkFile {
            uuid:    uuid.into(),
            link0:   whole_file_endpoint("a.md"),
            link1:   layer_endpoint(".stratum/impl"),
            subgraph: None,
            hash0:   None, commit0: None,
            hash1:   Some("previously-accepted-hash".into()),
            commit1: Some("abc1234".into()),
            range0: None, range1: None,
            state0: None, state1: None,
            resolved_at: None,
        };
        let path = bilink_dir.join(format!("{uuid}.bilink"));
        bl.write(&path).unwrap();

        // Hash present but target gone → BROKEN (regression)
        let result = check_file(dir.path(), &path).unwrap();
        assert_eq!(result.state1, EndpointState::Broken);
    }

    #[test]
    fn check_writes_state_and_timestamp() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("doc.md"), "# Title\nContent here.").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        let path = make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("doc.md"),
            whole_file_endpoint("doc.md"),
        );

        check_file(dir.path(), &path).unwrap();

        let updated = BiLinkFile::load(&path).unwrap();
        assert!(updated.state0.is_some(),      "state.0 should be written");
        assert!(updated.resolved_at.is_some(), "resolved_at should be written");
        assert!(updated.hash0.is_none(),        "check must not modify hash.0");
    }

    #[test]
    fn check_dir_processes_all_bilinks() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "content a").unwrap();
        std::fs::write(dir.path().join("b.md"), "content b").unwrap();

        let bilink_dir = dir.path().join(".bilink");
        make_bilink(&bilink_dir, "uuid1",
            whole_file_endpoint("a.md"),
            whole_file_endpoint("a.md"),
        );
        make_bilink(&bilink_dir, "uuid2",
            whole_file_endpoint("b.md"),
            whole_file_endpoint("b.md"),
        );

        let results = check(dir.path(), dir.path()).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.state0 == EndpointState::Pending));
    }
}

/// Finds all bilinks referencing `file_path` across all layers under `root`.
/// Returns `(bilink_path, endpoint_index, absolute_range)`.
/// Uses `.bilink/.index` per layer when valid; falls back to O(N) scan.
pub fn find_by_file(root: &Path, file_path: &Path) -> Result<Vec<(PathBuf, u8, ByteRange)>> {
    let mut results = Vec::new();

    for layer_root in crate::index::layer_roots(root) {
        let Ok(rel) = file_path.strip_prefix(&layer_root) else { continue };
        let Some(rel_str) = rel.to_str() else { continue };

        let bilink_dir = layer_root.join(".bilink");
        for (uuid, n) in crate::index::lookup(&layer_root, rel_str)? {
            let bilink_path = bilink_dir.join(format!("{uuid}.bilink"));
            let Ok(bl) = BiLinkFile::load(&bilink_path) else { continue };
            let range = if n == 0 { &bl.range0 } else { &bl.range1 };
            if let Some(r) = range {
                results.push((bilink_path, n, r.clone()));
            }
        }
    }

    Ok(results)
}
