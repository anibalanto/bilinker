use std::path::Path;
use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::bilink::BiLinkFile;
use crate::chain::resolve_layer_link;
use crate::git::try_head_commit_for_file;
use crate::grammar;
use crate::hash;
use crate::link::{EndpointState, LinkEndpoint};
use crate::query;

pub struct AcceptResult {
    pub uuid: String,
    pub n: u8,
    pub hash: String,
    pub commit: String,
}

/// Accept endpoint `n` of the bilink at `bilink_path`, establishing its hash baseline.
///
/// Sets `hash.N`, `commit.N`, `state.N = OK`, and `resolved_at` in the file.
pub fn accept(
    bilink_path: &Path,
    n: u8,
    hash_override: Option<&str>,
    commit_override: Option<&str>,
) -> Result<AcceptResult> {
    if n > 1 {
        bail!("endpoint index must be 0 or 1, got {n}");
    }

    let mut bl = BiLinkFile::load(bilink_path)?;

    let layer_root = bilink_path
        .parent().and_then(|p| p.parent())
        .unwrap_or(bilink_path);

    let endpoint = if n == 0 { &bl.link0 } else { &bl.link1 };

    let (h, c) = compute_hash_and_commit(
        layer_root, endpoint, &bl.uuid, hash_override, commit_override,
    )?;

    if n == 0 {
        bl.hash0   = Some(h.clone());
        bl.commit0 = Some(c.clone());
        bl.state0  = Some(EndpointState::Ok);
    } else {
        bl.hash1   = Some(h.clone());
        bl.commit1 = Some(c.clone());
        bl.state1  = Some(EndpointState::Ok);
    }
    bl.resolved_at = Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    bl.write(bilink_path)?;

    Ok(AcceptResult { uuid: bl.uuid, n, hash: h, commit: c })
}

fn compute_hash_and_commit(
    layer_root: &Path,
    endpoint: &LinkEndpoint,
    uuid: &str,
    hash_override: Option<&str>,
    commit_override: Option<&str>,
) -> Result<(String, String)> {
    match endpoint {
        LinkEndpoint::Structural(sref) => {
            let file_path = layer_root.join(&sref.file);
            let source = std::fs::read_to_string(&file_path)
                .with_context(|| format!("reading {}", file_path.display()))?;

            let frag_hash = if let Some(query_str) = &sref.query {
                let lang     = grammar::language_for_file(&sref.file);
                let language = grammar::for_language(lang)?;
                let node_range = query::find_target(language, &source, query_str)?;
                let (node_start, node_end) = node_range
                    .ok_or_else(|| anyhow::anyhow!(
                        "query matched nothing in '{}'; cannot accept", sref.file
                    ))?;
                let (frag_start, frag_end) = match &sref.range {
                    Some(r) => (node_start + r.start, (node_start + r.end).min(source.len())),
                    None    => (node_start, node_end),
                };
                hash::sha256(source[frag_start..frag_end].as_bytes())
            } else {
                hash::sha256(source.as_bytes())
            };

            let h = hash_override.map(String::from).unwrap_or(frag_hash);
            let c = commit_override.map(String::from)
                .unwrap_or_else(|| try_head_commit_for_file(layer_root, &sref.file)
                    .unwrap_or_default());
            Ok((h, c))
        }

        LinkEndpoint::Layer(tokens) => {
            let target_layer = stratum::resolve(layer_root, layer_root, tokens)
                .map_err(|e| anyhow::anyhow!("resolving layer endpoint: {e:?}"))?;

            let adj = resolve_layer_link(
                &layer_root.join(".bilink").join(format!("{uuid}.bilink")),
                layer_root,
                &target_layer,
                uuid,
            );

            let adj_bl = BiLinkFile::load(&adj)
                .with_context(|| format!("reading adjacent bilink {}", adj.display()))?;

            // Hash = structural endpoint's accepted hash in the adjacent bilink.
            // Avoids circular dependency: this value only changes when the adjacent
            // structural content is accepted, never from accepting a layer endpoint.
            let adj_hash = adj_bl.structural_hash()
                .ok_or_else(|| anyhow::anyhow!(
                    "adjacent bilink {} has no accepted structural endpoint yet; accept it first",
                    adj.display()
                ))?;
            let adj_commit = adj_bl.structural_commit().unwrap_or_default();

            let h = hash_override.map(String::from).unwrap_or_else(|| adj_hash.to_string());
            let c = commit_override.map(String::from).unwrap_or_else(|| adj_commit.to_string());
            Ok((h, c))
        }
    }
}

/// Accepts all PENDING endpoints in `layer_root/.bilink/`.
///
/// If `path_filter` is `None` or `"."`, accepts both structural and layer endpoints.
/// Otherwise only accepts structural endpoints whose file path starts with the filter.
pub fn accept_layer(
    layer_root: &Path,
    path_filter: Option<&str>,
) -> Result<Vec<AcceptResult>> {
    let bilink_dir = layer_root.join(".bilink");
    let mut results = Vec::new();

    if !bilink_dir.exists() {
        return Ok(results);
    }

    let all = path_filter.map(|f| f == "." || f.is_empty()).unwrap_or(true);

    for entry in std::fs::read_dir(&bilink_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        if path.file_name().and_then(|n| n.to_str())
            .map(|n| n.starts_with('.')).unwrap_or(false) { continue; }

        let bl = match BiLinkFile::load(&path) {
            Ok(b) => b,
            Err(e) => { eprintln!("warn: skipping {}: {e}", path.display()); continue; }
        };

        for n in [0u8, 1u8] {
            let endpoint = if n == 0 { &bl.link0 } else { &bl.link1 };
            let hash = if n == 0 { &bl.hash0 } else { &bl.hash1 };

            if hash.is_some() { continue; }

            let matches = if all {
                true
            } else if let (Some(filter), LinkEndpoint::Structural(sref)) = (path_filter, endpoint) {
                sref.file == filter
                    || sref.file.starts_with(&format!("{filter}/"))
                    || sref.file.starts_with(filter)
            } else {
                false
            };

            if !matches { continue; }

            match accept(&path, n, None, None) {
                Ok(r)  => results.push(r),
                Err(e) => eprintln!("warn: {}.{n}: {e}", &bl.uuid[..8.min(bl.uuid.len())]),
            }
        }
    }

    Ok(results)
}

/// Finds a `.bilink` file by UUID or 8-char prefix in `bilink_dir`.
pub fn find_bilink_path(bilink_dir: &Path, uuid_or_prefix: &str) -> Result<std::path::PathBuf> {
    if !bilink_dir.exists() {
        bail!("no .bilink/ directory at {}", bilink_dir.display());
    }
    let mut prefix_match: Option<std::path::PathBuf> = None;
    for entry in std::fs::read_dir(bilink_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if stem == uuid_or_prefix {
            return Ok(path);
        }
        if stem.starts_with(uuid_or_prefix) {
            prefix_match = Some(path);
        }
    }
    prefix_match.ok_or_else(|| anyhow::anyhow!(
        "no bilink '{}' in {}",
        uuid_or_prefix,
        bilink_dir.display(),
    ))
}
