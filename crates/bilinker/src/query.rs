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

/// Un match de la query: rango del `@target`, su S-expression, y el texto del
/// primer predicado de nombre (`@n0`) si la query lo tiene.
pub struct TargetMatch {
    pub start: usize,
    pub end:   usize,
    pub sexp:  String,
    pub name:  Option<String>,
}

/// Todos los matches de `@target`, no solo el primero.
///
/// `find_target_with_sexp` corta en el primero porque la query lleva predicados
/// que la hacen única. Esta versión existe para las queries relajadas: al quitar
/// los `#eq?` hay varios candidatos y hay que recorrerlos.
pub fn find_all_targets(language: Language, source: &str, query_str: &str) -> Result<Vec<TargetMatch>> {
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(source, None).context("parse failed")?;

    let query = Query::new(&language, query_str)
        .with_context(|| format!("invalid query:\n{query_str}"))?;

    let target_idx = query.capture_index_for_name("target")
        .context("query has no @target capture")?;
    let name_idx = query.capture_index_for_name("n0");

    let mut cursor  = QueryCursor::new();
    let root        = tree.root_node();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    let mut out     = Vec::new();

    while let Some(m) = matches.next() {
        let mut target: Option<&tree_sitter::QueryCapture> = None;
        let mut name:   Option<String> = None;
        for cap in m.captures {
            if cap.index == target_idx {
                target = Some(cap);
            } else if Some(cap.index) == name_idx {
                name = Some(source[cap.node.byte_range()].to_string());
            }
        }
        if let Some(t) = target {
            out.push(TargetMatch {
                start: t.node.start_byte(),
                end:   t.node.end_byte(),
                sexp:  t.node.to_sexp(),
                name,
            });
        }
    }
    Ok(out)
}

/// Quita los predicados `(#eq? @nK "...")` de una query.
///
/// Deja la estructura y las capturas intactas, así que la query relajada matchea
/// todos los nodos de la misma forma sin importar cómo se llamen. Es lo que
/// permite buscar un anchor renombrado.
pub fn relax_name_predicates(query_str: &str) -> String {
    let mut out   = String::with_capacity(query_str.len());
    let mut rest  = query_str;

    while let Some(pos) = rest.find("(#eq?") {
        out.push_str(&rest[..pos]);
        // Saltar hasta el paréntesis que cierra el predicado, contando anidados.
        let mut depth = 0usize;
        let mut end   = pos;
        for (i, c) in rest[pos..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 { end = pos + i + c.len_utf8(); break; }
                }
                _ => {}
            }
        }
        if end == pos { break; } // predicado sin cerrar: dejar el resto tal cual
        rest = &rest[end..];
    }
    out.push_str(rest);

    // Limpiar espacios dobles que deja la remoción.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relax_removes_eq_predicate() {
        let q = r#"(function_item name: (identifier) @n0 (#eq? @n0 "foo")) @target"#;
        assert_eq!(relax_name_predicates(q), "(function_item name: (identifier) @n0 ) @target");
    }

    #[test]
    fn relax_removes_several_predicates() {
        let q = r#"(class_declaration name: (identifier) @n0 (#eq? @n0 "A") body: (block (method name: (identifier) @n1 (#eq? @n1 "b")) @target))"#;
        let r = relax_name_predicates(q);
        assert!(!r.contains("#eq?"), "quedaron predicados: {r}");
        assert!(r.contains("@target"));
    }

    #[test]
    fn relax_leaves_query_without_predicates_intact() {
        let q = "(source_file) @target";
        assert_eq!(relax_name_predicates(q), q);
    }
}
