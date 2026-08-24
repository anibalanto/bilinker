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
    pub layer_root: std::path::PathBuf,
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
        LinkEndpoint::Capture(_) | LinkEndpoint::LegacyStructural(_) => {
            let sref = crate::capture::sref_of(root, link)?
                .context("endpoint estructural sin capture resoluble")?;
            resolve(root, &sref, before, after)
        }
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

    if endpoint > 1 { bail!("endpoint must be 0 or 1"); }
    let link   = bl.link(endpoint);
    let commit = bl.commit(endpoint);
    let hash   = bl.hash(endpoint);
    // El range del capture, no el del bilink: ahí ya no vive.
    let cap    = bl.capture_for(root, endpoint).ok().flatten();
    let range  = cap.as_ref().and_then(|c| c.range.clone());

    let commit = commit.context("endpoint has no accepted commit — run bilinker accept first")?;

    match link {
        LinkEndpoint::Capture(_) | LinkEndpoint::LegacyStructural(_) => {
            let sref = crate::capture::sref_of(root, link)?
                .context("endpoint estructural sin capture resoluble")?;
            diff_structural(root, &sref, commit, range.as_ref(), hash)
        }
        LinkEndpoint::Layer(layer_path) => {
            let (adj_root, sref_owned, adj_commit, adj_range, adj_hash) =
                traverse_layer_for_diff(root, layer_path.clone(), &bl.uuid)?;
            diff_structural(&adj_root, &sref_owned,
                            adj_commit.as_deref().unwrap_or(commit),
                            adj_range.as_ref(), adj_hash.as_deref())
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
    hash: Option<&str>,
) -> Result<DiffResult> {
    // "after": current fragment via AST query
    let after_result = resolve(root, sref, None, None)?;
    let after_text = &after_result.content;

    // "before": el fragmento aceptado, resolviendo la query contra el contenido
    // de `commit`. Recortarlo por `stored_range` daría bytes arbitrarios: ese
    // range es la posición *actual*, que check reescribe en cada corrida.
    //
    // Si la verificación por hash falla, se cae al recorte por range: para un
    // diff informativo es mejor mostrar algo aproximado que no mostrar nada.
    let before_text = match crate::capture::accepted_text(root, sref, commit, hash) {
        Some(t) => t,
        None    => git_show_fragment(root, commit, &sref.file, stored_range)?,
    };

    let diff = if before_text.trim_end() == after_text.trim_end() {
        None
    } else {
        Some(unified_diff(&before_text, after_text, commit))
    };

    Ok(DiffResult {
        file: sref.file.clone(),
        layer_root: root.to_path_buf(),
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

/// Find a function/method named `callee_name` in `source` using tree-sitter.
/// Returns byte range (start, end) of the matching declaration.
fn find_function_body(source: &str, lang: &str, callee_name: &str) -> Option<(usize, usize)> {
    let language = grammar::for_language(lang).ok()?;
    let escaped = callee_name.replace('"', "\\\"");
    for &anchor_kind in grammar::stable_anchor_kinds(lang) {
        let Some(field) = grammar::name_field(lang, anchor_kind) else { continue };
        let name_node_type = grammar::name_node_type(lang, anchor_kind);
        let query_str = format!(
            "({anchor_kind} {field}: ({name_node_type}) @name (#eq? @name \"{escaped}\")) @target"
        );
        if let Ok(Some((s, e))) = query::find_target(language.clone(), source, &query_str) {
            return Some((s, e));
        }
    }
    None
}

pub fn get_callee_diff(
    root: &Path,
    callee_file_abs: &str,
    callee_name: &str,
    commit: &str,
) -> Result<DiffResult> {
    let callee_path = std::path::Path::new(callee_file_abs);
    let rel_file = callee_path
        .strip_prefix(root)
        .unwrap_or(callee_path)
        .to_string_lossy()
        .to_string();

    let lang = grammar::language_for_file(&rel_file);

    let current_source = std::fs::read_to_string(callee_file_abs)
        .with_context(|| format!("reading {callee_file_abs}"))?;

    let (start_byte, end_byte) = find_function_body(&current_source, lang, callee_name)
        .ok_or_else(|| anyhow::anyhow!("function '{callee_name}' not found in {rel_file}"))?;

    let after_text = current_source[start_byte..end_byte].to_string();
    let start_line = current_source[..start_byte].chars().filter(|&c| c == '\n').count() + 1;
    let end_line   = current_source[..end_byte].chars().filter(|&c| c == '\n').count() + 1;

    let before_text = match git_show_fragment(root, commit, &rel_file, None) {
        Ok(old_source) => {
            find_function_body(&old_source, lang, callee_name)
                .map(|(s, e)| old_source[s..e].to_string())
                .unwrap_or_default()
        }
        Err(_) => String::new(),
    };

    let diff = if before_text.trim_end() == after_text.trim_end() {
        None
    } else {
        Some(unified_diff(&before_text, &after_text, commit))
    };

    Ok(DiffResult {
        file: rel_file,
        layer_root: root.to_path_buf(),
        commit: commit.to_string(),
        start_line,
        end_line,
        diff,
    })
}

fn traverse_layer_for_diff(
    root: &Path,
    layer_path: StratumPath,
    uuid: &str,
) -> Result<(std::path::PathBuf, StructuralRef, Option<String>, Option<ByteRange>, Option<String>)> {
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

    let n = adjacent_bl.structural_n()
        .with_context(|| format!("adjacent bilink {uuid} has no structural endpoint"))?;
    let cap = adjacent_bl.capture_for(&adjacent_root, n)?
        .with_context(|| format!("adjacent bilink {uuid}: capture no resoluble"))?;
    let (sref, commit, range, hash) = (
        cap.sref.clone(),
        adjacent_bl.commit(n).map(String::from),
        cap.range.clone(),
        adjacent_bl.hash(n).map(String::from),
    );

    Ok((adjacent_root, sref, commit, range, hash))
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

    let n = adjacent_bl.structural_n()
        .with_context(|| format!("adjacent bilink {uuid} has no structural endpoint"))?;
    let cap = adjacent_bl.capture_for(&adjacent_root, n)?
        .with_context(|| format!("adjacent bilink {uuid}: capture no resoluble"))?;

    resolve(&adjacent_root, &cap.sref, before, after)
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
