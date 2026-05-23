use anyhow::{Context, Result};
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// Run a tree-sitter query against `source` and return the byte range of the `@target` capture.
pub fn find_target(language: Language, source: &str, query_str: &str) -> Result<Option<(usize, usize)>> {
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(source, None).context("parse failed")?;

    let query = Query::new(&language, query_str)
        .with_context(|| format!("invalid query:\n{query_str}"))?;

    let target_idx = query.capture_index_for_name("target")
        .context("query has no @target capture")?;

    let mut cursor = QueryCursor::new();
    let root = tree.root_node();

    for m in cursor.matches(&query, root, source.as_bytes()) {
        for cap in m.captures {
            if cap.index == target_idx {
                return Ok(Some((cap.node.start_byte(), cap.node.end_byte())));
            }
        }
    }
    Ok(None)
}
