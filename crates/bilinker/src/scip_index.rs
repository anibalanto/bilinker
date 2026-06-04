use std::collections::{HashMap, HashSet};
use std::path::Path;
use anyhow::{Context, Result};
use prost::Message;

use crate::hash;
use crate::link::ByteRange;
use crate::sciplink::{ScipLink, ScipLinkState};

// ── SCIP protobuf structs ─────────────────────────────────────────────────────

#[derive(prost::Message)]
pub struct Index {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<Metadata>,
    #[prost(message, repeated, tag = "2")]
    pub documents: Vec<Document>,
    #[prost(message, repeated, tag = "3")]
    pub external_symbols: Vec<SymbolInformation>,
}

#[derive(prost::Message)]
pub struct Metadata {
    #[prost(string, tag = "3")]
    pub project_root: String,
}

#[derive(prost::Message)]
pub struct Document {
    #[prost(string, tag = "1")]
    pub relative_path: String,
    #[prost(message, repeated, tag = "2")]
    pub occurrences: Vec<Occurrence>,
    #[prost(message, repeated, tag = "3")]
    pub symbols: Vec<SymbolInformation>,
    #[prost(string, tag = "4")]
    pub language: String,
}

#[derive(prost::Message)]
pub struct Occurrence {
    /// [start_line, start_char, end_char] or [start_line, start_char, end_line, end_char]
    #[prost(int32, repeated, tag = "1")]
    pub range: Vec<i32>,
    #[prost(string, tag = "2")]
    pub symbol: String,
    /// 1 = Definition, others = Reference
    #[prost(int32, tag = "3")]
    pub symbol_roles: i32,
}

#[derive(prost::Message)]
pub struct SymbolInformation {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(message, repeated, tag = "3")]
    pub relationships: Vec<Relationship>,
    #[prost(int32, tag = "4")]
    pub kind: i32,
}

#[derive(prost::Message)]
pub struct Relationship {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(bool, tag = "2")]
    pub is_reference: bool,
}

// ── Symbol kind constants ─────────────────────────────────────────────────────

const KIND_FUNCTION: i32   = 14;
const KIND_METHOD: i32     = 23;
const KIND_CONSTRUCTOR: i32 = 7;
const KIND_STATIC_METHOD: i32 = 47;
const KIND_TRAIT_METHOD: i32  = 60;

fn is_callable(kind: i32) -> bool {
    matches!(kind, KIND_FUNCTION | KIND_METHOD | KIND_CONSTRUCTOR | KIND_STATIC_METHOD | KIND_TRAIT_METHOD)
}

// ── Public API ────────────────────────────────────────────────────────────────

pub struct ScipIndex {
    index: Index,
    /// symbol → (relative_path, byte_range)
    definitions: HashMap<String, (String, ByteRange)>,
    /// symbol → kind
    kinds: HashMap<String, i32>,
}

impl ScipIndex {
    pub fn load(scip_path: &Path, layer_root: &Path) -> Result<Self> {
        let bytes = std::fs::read(scip_path)
            .with_context(|| format!("reading {}", scip_path.display()))?;
        let index = Index::decode(bytes.as_slice())
            .context("decoding index.scip")?;

        let mut definitions: HashMap<String, (String, ByteRange)> = HashMap::new();
        let mut kinds: HashMap<String, i32> = HashMap::new();

        // Collect kinds from symbol information
        for doc in &index.documents {
            for si in &doc.symbols {
                if si.kind != 0 {
                    kinds.insert(si.symbol.clone(), si.kind);
                }
            }
        }
        for si in &index.external_symbols {
            if si.kind != 0 {
                kinds.insert(si.symbol.clone(), si.kind);
            }
        }

        // Collect definitions (symbol_roles & 1 == 1)
        for doc in &index.documents {
            for occ in &doc.occurrences {
                if occ.symbol_roles & 1 == 1 {
                    if let Ok(range) = occurrence_byte_range(&occ.range, &doc.relative_path, layer_root) {
                        definitions.insert(occ.symbol.clone(), (doc.relative_path.clone(), range));
                    }
                }
            }
        }

        Ok(Self { index, definitions, kinds })
    }

    /// Returns the definition location of a symbol, if known.
    pub fn definition(&self, symbol: &str) -> Option<(&str, &ByteRange)> {
        self.definitions.get(symbol).map(|(f, r)| (f.as_str(), r))
    }

    /// Finds direct callees of `symbol` (1 level deep).
    /// Returns a list of (callee_symbol, relative_file, byte_range).
    pub fn direct_callees(&self, symbol: &str) -> Vec<(String, String, ByteRange)> {
        let Some((def_file, def_range)) = self.definitions.get(symbol) else {
            return vec![];
        };

        // Find the document for this symbol's file
        let Some(doc) = self.index.documents.iter().find(|d| &d.relative_path == def_file) else {
            return vec![];
        };

        let mut seen = HashSet::new();
        let mut callees = Vec::new();

        for occ in &doc.occurrences {
            // Skip definitions and the root symbol itself
            if occ.symbol_roles & 1 == 1 { continue; }
            if occ.symbol == symbol       { continue; }
            if occ.symbol.is_empty()      { continue; }

            // Must be within the definition range of the root symbol
            if let Ok(occ_range) = occurrence_byte_range(&occ.range, def_file, &std::path::PathBuf::new()) {
                if occ_range.start < def_range.start || occ_range.end > def_range.end {
                    continue;
                }
            }

            // Only callable symbols
            let kind = self.kinds.get(&occ.symbol).copied().unwrap_or(0);
            if !is_callable(kind) { continue; }

            if seen.insert(occ.symbol.clone()) {
                if let Some((callee_file, callee_range)) = self.definitions.get(&occ.symbol) {
                    callees.push((occ.symbol.clone(), callee_file.clone(), callee_range.clone()));
                }
            }
        }

        callees
    }

    /// Looks for a symbol by file+range similarity (for RENAMED detection).
    pub fn find_by_location(&self, file: &str, range: &ByteRange) -> Option<String> {
        for (sym, (f, r)) in &self.definitions {
            if f == file && overlaps(r, range) {
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
        // New callee — create with OK state directly
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

fn occurrence_byte_range(range: &[i32], file: &str, layer_root: &Path) -> Result<ByteRange> {
    if range.len() < 3 {
        anyhow::bail!("malformed SCIP range");
    }
    let start_line = range[0] as usize;
    let start_char = range[1] as usize;
    let (end_line, end_char) = if range.len() == 3 {
        (start_line, range[2] as usize)
    } else {
        (range[2] as usize, range[3] as usize)
    };

    // Convert line/char to byte offset by reading the source file
    let source_path = if layer_root.as_os_str().is_empty() {
        std::path::PathBuf::from(file)
    } else {
        layer_root.join(file)
    };

    let source = std::fs::read_to_string(&source_path)
        .with_context(|| format!("reading {} for range conversion", source_path.display()))?;

    let start = line_char_to_byte(&source, start_line, start_char);
    let end   = line_char_to_byte(&source, end_line, end_char);

    Ok(ByteRange { start, end })
}

fn line_char_to_byte(source: &str, line: usize, char_offset: usize) -> usize {
    let mut current_line = 0;
    let mut byte_offset = 0;

    for (i, ch) in source.char_indices() {
        if current_line == line {
            // Count chars within this line
            let line_start = byte_offset;
            let _ = line_start;
            // Find byte position of char_offset chars into this line
            let line_bytes = &source[i..];
            let mut chars_counted = 0;
            for (j, _) in line_bytes.char_indices() {
                if chars_counted == char_offset {
                    return i + j;
                }
                chars_counted += 1;
            }
            return source.len();
        }
        if ch == '\n' {
            current_line += 1;
            byte_offset = i + 1;
        }
    }
    source.len()
}

fn overlaps(a: &ByteRange, b: &ByteRange) -> bool {
    a.start < b.end && b.start < a.end
}
