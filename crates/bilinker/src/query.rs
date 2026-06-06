use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// Run a tree-sitter query against `source` and return the byte range of the `@target` capture.
pub fn find_target(language: Language, source: &str, query_str: &str) -> Result<Option<(usize, usize)>> {
    Ok(find_target_with_sexp(language, source, query_str)?.map(|(s, e, _)| (s, e)))
}

/// Like `find_target` but also returns the S-expression of the matched node.
/// The sexp is stable across whitespace/formatting changes — suitable for AST hashing.
pub fn find_target_with_sexp(language: Language, source: &str, query_str: &str) -> Result<Option<(usize, usize, String)>> {
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(source, None).context("parse failed")?;

    let query = Query::new(&language, query_str)
        .with_context(|| format!("invalid query:\n{query_str}"))?;

    let target_idx = query.capture_index_for_name("target")
        .context("query has no @target capture")?;

    let mut cursor = QueryCursor::new();
    let root = tree.root_node();
    let mut matches = cursor.matches(&query, root, source.as_bytes());

    while let Some(m) = matches.next() {
        for cap in m.captures {
            if cap.index == target_idx {
                let sexp = cap.node.to_sexp();
                return Ok(Some((cap.node.start_byte(), cap.node.end_byte(), sexp)));
            }
        }
    }
    Ok(None)
}
