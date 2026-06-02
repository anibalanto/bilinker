use std::path::Path;
use anyhow::{bail, Context, Result};

use crate::bilink::BiLinkFile;
use crate::grammar;
use crate::link::{LinkEndpoint, StructuralRef};
use crate::query;
use stratum::StratumPath;

pub struct GetResult {
    pub content: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn get(
    root: &Path,
    bilink_name: &str,
    endpoint: u8,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let bilinker_dir = root.join(".bilink");
    let (_, bl) = BiLinkFile::find_by_id(&bilinker_dir, bilink_name)?;

    let link = match endpoint {
        0 => &bl.link0,
        1 => &bl.link1,
        _ => bail!("endpoint must be 0 or 1"),
    };

    match link {
        LinkEndpoint::Structural(sref) => resolve(root, sref, before, after),
        LinkEndpoint::Layer(layer_path) => {
            traverse_layer(root, layer_path.clone(), &bl.uuid, before, after)
        }
        LinkEndpoint::Task(id) => bail!(
            "link.{endpoint} is a task reference ({id}) — use worklist to view it"
        ),
    }
}

fn traverse_layer(
    root: &Path,
    layer_path: StratumPath,
    uuid: &str,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let adjacent_root = {
        let p = stratum::resolve(root, root, &layer_path)
            .map_err(|e| anyhow::anyhow!("resolving adjacent layer: {e}"))?;
        // Walk up to the true root of the adjacent layer (.git or .bilink)
        let (true_root, _) = crate::config::Config::load_from(&p)
            .with_context(|| format!("finding root of adjacent layer {}", p.display()))?;
        true_root
    };

    let adjacent_bilink_dir = adjacent_root.join(".bilink");
    let (_, adjacent_bl) = BiLinkFile::find_by_id(&adjacent_bilink_dir, uuid)
        .with_context(|| format!("bilink {uuid} not found in {}", adjacent_bilink_dir.display()))?;

    let sref = match (&adjacent_bl.link0, &adjacent_bl.link1) {
        (LinkEndpoint::Structural(r), _) => r,
        (_, LinkEndpoint::Structural(r)) => r,
        _ => bail!("adjacent bilink {uuid} has no structural endpoint"),
    };

    resolve(&adjacent_root, sref, before, after)
}

fn resolve(
    root: &Path,
    sref: &StructuralRef,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let file_path = root.join(&sref.file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let Some(query_str) = &sref.query else {
        let total = count_lines(&source);
        return Ok(GetResult {
            content: source,
            file: sref.file.clone(),
            start_line: 1,
            end_line: total,
        });
    };

    let lang = grammar::language_for_file(&sref.file);
    let language = grammar::for_language(lang)?;

    let (node_start, node_end) = query::find_target(language, &source, query_str)?
        .with_context(|| format!("query matched nothing in {}", sref.file))?;

    let (frag_start, frag_end) = match &sref.range {
        Some(r) => (node_start + r.start, (node_start + r.end).min(source.len())),
        None    => (node_start, node_end),
    };

    let line_start = byte_to_line(&source, frag_start);
    let line_end   = byte_to_line(&source, frag_end.saturating_sub(1));

    let before_rows = before.map(|(r, _)| r).unwrap_or(0);
    let after_rows  = after.map(|(r, _)| r).unwrap_or(0);

    let ctx_start = line_start.saturating_sub(before_rows);
    let ctx_end   = (line_end + after_rows).min(count_lines(&source).saturating_sub(1));

    let content = extract_lines(&source, ctx_start, ctx_end);

    Ok(GetResult {
        content,
        file: sref.file.clone(),
        start_line: ctx_start + 1,
        end_line: ctx_end + 1,
    })
}

fn byte_to_line(source: &str, byte: usize) -> usize {
    source[..byte.min(source.len())].chars().filter(|&c| c == '\n').count()
}

fn count_lines(source: &str) -> usize {
    source.lines().count()
}

fn extract_lines(source: &str, from: usize, to: usize) -> String {
    source.lines()
        .enumerate()
        .filter(|(i, _)| *i >= from && *i <= to)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}
