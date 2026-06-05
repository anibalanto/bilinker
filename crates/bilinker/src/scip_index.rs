use std::collections::{HashMap, HashSet};
use std::path::Path;
use anyhow::{Context, Result};
use protobuf::Message;
use scip::types::Index;

use crate::hash;
use crate::link::ByteRange;
use crate::sciplink::{ScipLink, ScipLinkState};

// Symbol role bitmask from SCIP spec
const ROLE_DEFINITION: i32 = 1;

/// In Rust SCIP, function and method symbols end with `().`
fn is_callable(_kind: i32, symbol: &str) -> bool {
    symbol.ends_with("().")
}

pub struct ScipIndex {
    /// symbol → (relative_path, name_range, body_range)
    /// body_range comes from enclosing_range when available, else falls back to name_range
    definitions: HashMap<String, (String, ByteRange, ByteRange)>,
    /// symbol → kind
    kinds: HashMap<String, i32>,
    /// document relative_path → occurrences (symbol, roles, range)
    doc_occurrences: HashMap<String, Vec<(String, i32, ByteRange)>>,
}

impl ScipIndex {
    pub fn load(scip_path: &Path, layer_root: &Path) -> Result<Self> {
        let bytes = std::fs::read(scip_path)
            .with_context(|| format!("reading {}", scip_path.display()))?;
        let index = Index::parse_from_bytes(&bytes)
            .with_context(|| format!("decoding {}", scip_path.display()))?;

        let mut definitions: HashMap<String, (String, ByteRange, ByteRange)> = HashMap::new();
        let mut kinds: HashMap<String, i32> = HashMap::new();
        let mut doc_occurrences: HashMap<String, Vec<(String, i32, ByteRange)>> = HashMap::new();

        // Collect kinds from symbol information in all documents
        for doc in &index.documents {
            for si in &doc.symbols {
                let k = si.kind.value();
                if k != 0 {
                    kinds.insert(si.symbol.clone(), k);
                }
            }
        }
        for si in &index.external_symbols {
            let k = si.kind.value();
            if k != 0 {
                kinds.insert(si.symbol.clone(), k);
            }
        }

        // Collect definitions and build per-document occurrence index
        for doc in &index.documents {
            let mut occs = Vec::new();
            for occ in &doc.occurrences {
                if occ.symbol.is_empty() { continue; }
                match scip_range_to_byte_range(&occ.range, &doc.relative_path, layer_root) {
                    Ok(name_range) => {
                        if occ.symbol_roles & ROLE_DEFINITION != 0 {
                            // Try enclosing_range first, then tree-sitter, then fall back to name_range
                            let body_range = if !occ.enclosing_range.is_empty() {
                                scip_range_to_byte_range(&occ.enclosing_range, &doc.relative_path, layer_root)
                                    .unwrap_or_else(|_| name_range.clone())
                            } else {
                                expand_to_function_body(&name_range, &doc.relative_path, layer_root)
                                    .unwrap_or_else(|| name_range.clone())
                            };
                            definitions.insert(occ.symbol.clone(), (doc.relative_path.clone(), name_range.clone(), body_range));
                        }
                        occs.push((occ.symbol.clone(), occ.symbol_roles, name_range));
                    }
                    Err(_) => {} // external file not in layer — skip
                }
            }
            if !occs.is_empty() {
                doc_occurrences.insert(doc.relative_path.clone(), occs);
            }
        }

        Ok(Self { definitions, kinds, doc_occurrences })
    }

    /// Returns the definition name range of a symbol, if known.
    pub fn definition(&self, symbol: &str) -> Option<(&str, &ByteRange)> {
        self.definitions.get(symbol).map(|(f, name_r, _)| (f.as_str(), name_r))
    }

    /// Finds direct callees of `symbol` by scanning reference occurrences within its body.
    pub fn direct_callees(&self, symbol: &str) -> Vec<(String, String, ByteRange)> {
        let Some((def_file, _name_range, body_range)) = self.definitions.get(symbol) else {
            return vec![];
        };
        let Some(occs) = self.doc_occurrences.get(def_file) else {
            return vec![];
        };

        let mut seen = HashSet::new();
        let mut callees = Vec::new();

        for (sym, roles, occ_range) in occs {
            if sym == symbol { continue; }
            if roles & ROLE_DEFINITION != 0 { continue; }
            // Must fall within the body range
            if occ_range.start < body_range.start || occ_range.end > body_range.end { continue; }

            let kind = self.kinds.get(sym).copied().unwrap_or(0);
            if !is_callable(kind, sym) { continue; }

            if seen.insert(sym.clone()) {
                if let Some((callee_file, callee_name_range, _)) = self.definitions.get(sym) {
                    callees.push((sym.clone(), callee_file.clone(), callee_name_range.clone()));
                }
            }
        }

        callees
    }

    /// Returns the body range (enclosing range) of a symbol for source extraction.
    pub fn body_range(&self, symbol: &str) -> Option<(&str, &ByteRange)> {
        self.definitions.get(symbol).map(|(f, _, body_r)| (f.as_str(), body_r))
    }

    /// Finds the innermost callable symbol whose body_range contains `range` in `file`.
    /// Used to auto-detect which symbol an endpoint belongs to.
    pub fn find_callable_at(&self, file: &str, range: &ByteRange) -> Option<String> {
        let mut best: Option<(String, usize)> = None; // (symbol, body_range size)
        for (sym, (f, _name_r, body_r)) in &self.definitions {
            if f != file { continue; }
            if !is_callable(self.kinds.get(sym).copied().unwrap_or(0), sym) { continue; }
            // body_range must contain the endpoint range
            if body_r.start > range.start || body_r.end < range.end { continue; }
            let size = body_r.end - body_r.start;
            // Prefer the smallest body that still contains the range (innermost function)
            if best.as_ref().map_or(true, |(_, s)| size < *s) {
                best = Some((sym.clone(), size));
            }
        }
        best.map(|(sym, _)| sym)
    }

    /// Returns the first callable symbol defined in `file` (used as fallback when no range is available).
    pub fn find_callable_in_file(&self, file: &str) -> Option<String> {
        self.definitions.iter()
            .find(|(sym, (f, _, _))| f == file && is_callable(self.kinds.get(*sym).copied().unwrap_or(0), sym))
            .map(|(sym, _)| sym.clone())
    }

    pub fn occurrences_in(&self, file: &str, body: &ByteRange) -> Vec<(String, i32, ByteRange)> {
        self.doc_occurrences.get(file).map(|occs| {
            occs.iter()
                .filter(|(_, _, r)| r.start >= body.start && r.end <= body.end)
                .cloned()
                .collect()
        }).unwrap_or_default()
    }

    pub fn kind(&self, symbol: &str) -> i32 {
        self.kinds.get(symbol).copied().unwrap_or(0)
    }

    /// Looks for a symbol by file+range similarity (for RENAMED detection).
    pub fn find_by_location(&self, file: &str, range: &ByteRange) -> Option<String> {
        for (sym, (f, name_r, _)) in &self.definitions {
            if f == file && overlaps(name_r, range) {
                return Some(sym.clone());
            }
        }
        None
    }
}

/// Checks or creates a `.sciplink` for a single symbol, returning its updated state.
pub fn check_or_create_sciplink(
    path: &Path,
    symbol: &str,
    file: &str,
    range: &ByteRange,
    layer_root: &Path,
) -> Result<ScipLink> {
    let source_path = layer_root.join(file);
    let content = std::fs::read(&source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;
    let fragment = &content[range.start.min(content.len())..range.end.min(content.len())];
    let current_hash = hash::sha256(fragment);
    let current_commit = crate::git::try_head_commit_for_file(layer_root, file).unwrap_or_default();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    if !path.exists() {
        let sl = ScipLink {
            symbol: symbol.to_string(),
            file: file.to_string(),
            range: range.clone(),
            hash: Some(current_hash),
            commit: Some(current_commit),
            state: Some(ScipLinkState::Ok),
            resolved_at: Some(now),
        };
        sl.write(path)?;
        return Ok(sl);
    }

    let mut sl = ScipLink::load(path)?;
    let stored_hash = sl.hash.as_deref().unwrap_or("");
    let new_state = if current_hash == stored_hash {
        ScipLinkState::Ok
    } else {
        ScipLinkState::Altered
    };

    sl.range = range.clone();
    sl.state = Some(new_state);
    sl.resolved_at = Some(now);
    sl.write(path)?;
    Ok(sl)
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Converts a SCIP occurrence range (line/char, 0-indexed) to a byte range.
/// SCIP range: [start_line, start_char, end_char] or [start_line, start_char, end_line, end_char]
fn scip_range_to_byte_range(range: &[i32], file: &str, layer_root: &Path) -> Result<ByteRange> {
    if range.len() < 3 {
        anyhow::bail!("malformed SCIP range ({} elements)", range.len());
    }
    let start_line = range[0] as usize;
    let start_char = range[1] as usize;
    let (end_line, end_char) = if range.len() == 3 {
        (start_line, range[2] as usize)
    } else {
        (range[2] as usize, range[3] as usize)
    };

    let source_path = layer_root.join(file);
    let source = std::fs::read_to_string(&source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;

    let start = line_char_to_byte(&source, start_line, start_char);
    let end   = line_char_to_byte(&source, end_line, end_char);
    Ok(ByteRange { start, end })
}

fn line_char_to_byte(source: &str, target_line: usize, char_offset: usize) -> usize {
    let mut line = 0;
    let mut line_start = 0;

    for (i, ch) in source.char_indices() {
        if line == target_line {
            // Count chars from line_start
            let from_line_start = &source[line_start..];
            let mut chars = 0;
            for (j, _) in from_line_start.char_indices() {
                if chars == char_offset {
                    return line_start + j;
                }
                chars += 1;
            }
            return source.len();
        }
        if ch == '\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    source.len()
}

fn overlaps(a: &ByteRange, b: &ByteRange) -> bool {
    a.start < b.end && b.start < a.end
}

/// Uses tree-sitter to find the full function/method body that contains `name_range`.
/// Returns the byte range of the enclosing callable node, or None if not found.
fn expand_to_function_body(name_range: &ByteRange, file: &str, layer_root: &Path) -> Option<ByteRange> {
    let source_path = layer_root.join(file);
    let source = std::fs::read_to_string(&source_path).ok()?;

    let lang = crate::grammar::language_for_file(file);
    let language = crate::grammar::for_language(lang).ok()?;

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source, None)?;
    let root = tree.root_node();

    // Find the node at the name position
    let point = tree_sitter::Point {
        row:    byte_to_line(&source, name_range.start),
        column: name_range.start - line_start_byte(&source, byte_to_line(&source, name_range.start)),
    };
    let node = root.named_descendant_for_point_range(point, point)?;

    // Walk up to find an enclosing callable node
    const CALLABLE_NODES: &[&str] = &[
        "function_item", "method_declaration", "function_declaration",
        "method_definition", "function_definition", "impl_item",
        "function", "arrow_function",
    ];

    let mut cursor = node;
    loop {
        if CALLABLE_NODES.contains(&cursor.kind()) {
            return Some(ByteRange { start: cursor.start_byte(), end: cursor.end_byte() });
        }
        match cursor.parent() {
            Some(p) => cursor = p,
            None    => break,
        }
    }
    None
}

fn byte_to_line(source: &str, byte: usize) -> usize {
    source[..byte.min(source.len())].bytes().filter(|&b| b == b'\n').count()
}

fn line_start_byte(source: &str, line: usize) -> usize {
    source.bytes().enumerate()
        .filter(|(_, b)| *b == b'\n')
        .nth(line.saturating_sub(1))
        .map(|(i, _)| i + 1)
        .unwrap_or(0)
}
