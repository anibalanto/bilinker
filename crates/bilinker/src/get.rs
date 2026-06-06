use std::path::Path;
use anyhow::{bail, Context, Result};

use crate::bilink::BiLinkFile;
use crate::grammar;
use crate::link::{ByteRange, LinkEndpoint, StructuralRef};
use crate::query;
use stratum::StratumPath;

pub struct GetResult {
    pub content: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub struct DiffResult {
    pub file: String,
    pub commit: String,
    pub start_line: usize,
    pub end_line: usize,
    /// None = no changes
    pub diff: Option<String>,
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

pub fn get_diff(
    root: &Path,
    bilink_name: &str,
    endpoint: u8,
) -> Result<DiffResult> {
    let bilinker_dir = root.join(".bilink");
    let (_, bl) = BiLinkFile::find_by_id(&bilinker_dir, bilink_name)?;

    let (link, commit, range) = match endpoint {
        0 => (&bl.link0, bl.commit0.as_deref(), bl.range0.as_ref()),
        1 => (&bl.link1, bl.commit1.as_deref(), bl.range1.as_ref()),
        _ => bail!("endpoint must be 0 or 1"),
    };

    let commit = commit.context("endpoint has no accepted commit — run bilinker accept first")?;

    match link {
        LinkEndpoint::Structural(sref) => diff_structural(root, sref, commit, range),
        LinkEndpoint::Layer(layer_path) => {
            let (adj_root, sref_owned, adj_commit, adj_range) =
                traverse_layer_for_diff(root, layer_path.clone(), &bl.uuid)?;
            diff_structural(&adj_root, &sref_owned, adj_commit.as_deref().unwrap_or(commit), adj_range.as_ref())
        }
        LinkEndpoint::Task(id) => bail!(
            "link.{endpoint} is a task reference ({id})"
        ),
    }
}

fn diff_structural(
    root: &Path,
    sref: &StructuralRef,
    commit: &str,
    stored_range: Option<&ByteRange>,
) -> Result<DiffResult> {
    // "after": current fragment via AST query
    let after_result = resolve(root, sref, None, None)?;
    let after_text = &after_result.content;

    // "before": fragment from accepted commit via git show
    let before_text = git_show_fragment(root, commit, &sref.file, stored_range)?;

    let diff = if before_text.trim_end() == after_text.trim_end() {
        None
    } else {
        Some(unified_diff(&before_text, after_text, commit))
    };

    Ok(DiffResult {
        file: sref.file.clone(),
        commit: commit.to_string(),
        start_line: after_result.start_line,
        end_line: after_result.end_line,
        diff,
    })
}

fn git_show_fragment(root: &Path, commit: &str, file: &str, range: Option<&ByteRange>) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &root.to_string_lossy(), "show", &format!("{commit}:{file}")])
        .output()
        .context("running git show")?;

    if !output.status.success() {
        bail!("git show {commit}:{file} failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let source = String::from_utf8_lossy(&output.stdout);

    let fragment = match range {
        Some(r) => {
            let start = r.start.min(source.len());
            let end   = r.end.min(source.len());
            source[start..end].to_string()
        }
        None => source.into_owned(),
    };

    Ok(fragment)
}

fn unified_diff(before: &str, after: &str, commit: &str) -> String {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let before_path = dir.join("bilinker_diff_before.tmp");
    let after_path  = dir.join("bilinker_diff_after.tmp");

    let _ = std::fs::write(&before_path, before);
    let _ = std::fs::write(&after_path, after);

    let output = std::process::Command::new("diff")
        .args([
            "-u",
            "--label", &format!("aceptado ({})", &commit[..8.min(commit.len())]),
            "--label", "actual",
            &before_path.to_string_lossy(),
            &after_path.to_string_lossy(),
        ])
        .output();

    let _ = std::fs::remove_file(&before_path);
    let _ = std::fs::remove_file(&after_path);

    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => format!("--- aceptado ({})\n+++ actual\n(diff no disponible)", &commit[..8.min(commit.len())]),
    }
}

fn traverse_layer_for_diff(
    root: &Path,
    layer_path: StratumPath,
    uuid: &str,
) -> Result<(std::path::PathBuf, StructuralRef, Option<String>, Option<ByteRange>)> {
    let adjacent_root = {
        let p = stratum::resolve(root, root, &layer_path)
            .map_err(|e| anyhow::anyhow!("resolving adjacent layer: {e}"))?;
        let (true_root, _) = crate::config::Config::load_from(&p)
            .with_context(|| format!("finding root of adjacent layer {}", p.display()))?;
        true_root
    };

    let adjacent_bilink_dir = adjacent_root.join(".bilink");
    let (_, adjacent_bl) = BiLinkFile::find_by_id(&adjacent_bilink_dir, uuid)
        .with_context(|| format!("bilink {uuid} not found in {}", adjacent_bilink_dir.display()))?;

    let (sref, commit, range) = match (&adjacent_bl.link0, &adjacent_bl.link1) {
        (LinkEndpoint::Structural(r), _) => (r.clone(), adjacent_bl.commit0.clone(), adjacent_bl.range0.clone()),
        (_, LinkEndpoint::Structural(r)) => (r.clone(), adjacent_bl.commit1.clone(), adjacent_bl.range1.clone()),
        _ => bail!("adjacent bilink {uuid} has no structural endpoint"),
    };

    Ok((adjacent_root, sref, commit, range))
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
