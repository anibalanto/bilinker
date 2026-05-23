use std::path::Path;
use anyhow::{bail, Context, Result};

use crate::bilink::BiLinkFile;
use crate::config::Config;
use crate::grammar;
use crate::link::{LinkEndpoint, StructuralRef};
use crate::query;

pub struct GetResult {
    pub content: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn get(
    config: &Config,
    root: &Path,
    bilink_name: &str,
    endpoint: u8,            // 0 or 1
    before: Option<(usize, usize)>, // (rows, cols)
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let bilinker_dir = root.join(".bilinker");
    let (_, bl) = BiLinkFile::find_by_id(&bilinker_dir, bilink_name)?;

    let link = match endpoint {
        0 => &bl.link0,
        1 => &bl.link1,
        _ => bail!("endpoint must be 0 or 1"),
    };

    let sref = match link {
        LinkEndpoint::Structural(r) => r,
        LinkEndpoint::BiLinkRef(id) => bail!(
            "link.{endpoint} is a bilink reference to '{id}' — use 'get {id} <N>' instead"
        ),
    };

    resolve(config, root, sref, before, after)
}

fn resolve(
    config: &Config,
    root: &Path,
    sref: &StructuralRef,
    before: Option<(usize, usize)>,
    after: Option<(usize, usize)>,
) -> Result<GetResult> {
    let ws = config.workspaces.get(&sref.workspace)
        .with_context(|| format!("workspace '{}' not found", sref.workspace))?;

    let file_path = root.join(&ws.path).join(&sref.file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let language = grammar::for_language(&ws.language)?;

    let (node_start, node_end) = query::find_target(language, &source, &sref.query)?
        .with_context(|| format!("query matched nothing in {}", sref.file))?;

    // Apply start~end range within the node if present.
    let (frag_start, frag_end) = match &sref.range {
        Some(r) => (node_start + r.start, (node_start + r.end).min(source.len())),
        None    => (node_start, node_end),
    };

    // Extend to full lines for display.
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
