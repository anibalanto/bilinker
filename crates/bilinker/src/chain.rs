use std::path::{Component, Path, PathBuf};
use anyhow::{bail, Result};
use uuid::Uuid;

use bilink_format::{BiLink, LinkEndpoint};

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
        let bl = BiLink::new(tips[0].1.clone(), tips[1].1.clone());
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

        let bl = BiLink::new(link0, link1);
        let path = bilink_path(root, layer, &uuid);
        bl.write(&path)?;
        created.push(path);
    }

    Ok(ChainNew { uuid, files: created })
}

pub fn resolve_layer_link(
    bilink_file: &Path,
    layer_root: &Path,
    link_path: &Path,
    uuid: &str,
) -> PathBuf {
    let _ = bilink_file;
    BiLink::path_in(&layer_root.join(link_path), uuid)
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn bilink_path(root: &Path, layer: &Path, uuid: &str) -> PathBuf {
    BiLink::path_in(&root.join(layer), uuid)
}

fn layer_endpoint(from_layer: &Path, to_layer: &Path) -> Result<LinkEndpoint> {
    let rel = diff_paths(to_layer, from_layer);
    let tokens = filesystem_to_stratum_tokens(&rel)?;
    format!("path {}", stratum::format_path(&tokens)).parse()
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
    use tempfile::tempdir;

    fn ep(raw: &str) -> LinkEndpoint { raw.parse().unwrap() }

    /// Una cadena entre dos capas: un bilink en cada una, con el mismo uuid.
    #[test]
    fn a_two_layer_chain_writes_one_file_per_layer() {
        let d = tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".stratum/impl")).unwrap();

        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from(".stratum/impl"), ep("capture bbb"))],
            &[]).unwrap();

        assert_eq!(r.files.len(), 2);
        for f in &r.files { assert!(f.exists(), "no se escribió {}", f.display()); }

        // El tip de la capa raíz apunta a su capture y a la capa vecina.
        let spec = BiLink::load(&r.files[0]).unwrap();
        assert_eq!(spec.endpoint.zero.link.to_string(), "capture aaa");
        assert_eq!(spec.endpoint.one.link.prefix(), "path");
    }

    /// Los dos endpoints en la misma capa: un solo archivo, sin traversal.
    #[test]
    fn a_direct_link_writes_a_single_file() {
        let d = tempdir().unwrap();
        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from("."), ep("capture bbb"))],
            &[]).unwrap();
        assert_eq!(r.files.len(), 1);
    }

    /// Una cadena nace sin nada aceptado: su ausencia *es* PENDING.
    #[test]
    fn a_fresh_chain_has_nothing_accepted() {
        let d = tempdir().unwrap();
        let r = chain_new(d.path(),
            &[(PathBuf::from("."), ep("capture aaa")),
              (PathBuf::from("."), ep("capture bbb"))],
            &[]).unwrap();
        let bl = BiLink::load(&r.files[0]).unwrap();
        assert!(bl.endpoint.zero.accepted.is_none());
        assert!(bl.endpoint.one.accepted.is_none());
    }
}
