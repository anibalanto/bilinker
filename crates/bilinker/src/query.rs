use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

/// Run a tree-sitter query against `source` and return the byte range of the `@target` capture.
pub fn find_target(language: Language, source: &str, query_str: &str) -> Result<Option<(usize, usize)>> {
    Ok(find_target_with_sexp(language, source, query_str)?.map(|(s, e, _)| (s, e)))
}

/// El rango de un fragmento **sin el espacio que lo rodea**.
///
/// Dónde empieza un nodo depende de qué hay alrededor, y no debería: en YAML el
/// mismo item de secuencia empieza en el `-` cuando es el último y en la
/// indentación de su línea cuando lo sigue otro. Agregar un item más abajo le
/// cambiaba los bytes —y con ellos el hash— a un item que nadie tocó, que es
/// exactamente lo que una referencia tiene que sobrevivir.
///
/// Recortar los bordes lo vuelve independiente del contexto: el fragmento es su
/// contenido, y el espacio que lo separa de sus vecinos es de los dos. Va en el
/// único lugar donde un nodo se convierte en rango, así que no hay forma de
/// obtener uno sin recortar.
pub(crate) fn trim_edges(source: &str, start: usize, end: usize) -> (usize, usize) {
    let b = source.as_bytes();
    let (mut s, mut e) = (start.min(source.len()), end.min(source.len()));
    while s < e && b[s].is_ascii_whitespace() { s += 1; }
    while e > s && b[e - 1].is_ascii_whitespace() { e -= 1; }
    (s, e)
}

/// La forma del árbol **más el texto de cada token hoja**.
///
/// `Node::to_sexp` da sólo la forma: dice `(identifier)`, no *qué* identificador.
/// Hashear eso hace invisible todo renombre y todo literal, y el estado sale
/// RESTYLED —"sólo formato"— de un cambio de versión o de un parámetro renombrado.
///
/// Con los tokens adentro, dos fragmentos coinciden cuando tienen los mismos
/// tokens en el mismo orden y la misma estructura. Lo único que puede diferir es
/// el espacio entre ellos, que es lo que "sólo formato" quiere decir.
///
/// Un comentario es un token, así que cambiarlo no es sólo formato: un comentario
/// dice algo, y cambiar lo que dice es un cambio de contenido.
pub fn shape_and_tokens(node: Node, source: &str) -> String {
    let mut out = String::new();
    write_node(node, source, &mut out);
    out
}

fn write_node(node: Node, source: &str, out: &mut String) {
    out.push('(');
    out.push_str(node.kind());

    // El texto se toma de lo que **no** cubre ningún hijo. En una hoja es el token
    // entero; en un nodo interno son los huecos, que en código son espacios y se
    // descartan al recortar — salvo donde no lo son: el cuerpo de un comentario
    // cuelga así, con el `//` como único hijo, y sin esto sería invisible.
    let mut at = node.start_byte();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        write_gap(&source[at..child.start_byte()], out);
        out.push(' ');
        write_node(child, source, out);
        at = child.end_byte();
    }
    write_gap(&source[at..node.end_byte()], out);

    out.push(')');
}

fn write_gap(text: &str, out: &mut String) {
    let t = text.trim();
    if !t.is_empty() {
        out.push(' ');
        out.push_str(t);
    }
}


/// Como `find_target`, pero además devuelve la huella del nodo: forma del árbol y
/// tokens, estable ante cambios de espaciado. Ver [`shape_and_tokens`].
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
                let sexp = shape_and_tokens(cap.node, source);
                let (s, e) = trim_edges(source, cap.node.start_byte(), cap.node.end_byte());
                return Ok(Some((s, e, sexp)));
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
    // El anchor del fragmento es el **último** predicado de nombre, no el
    // primero: en una query anidada `@n0` identifica al ancestro más externo
    // —el título de un documento, la clase que contiene al método— y quien
    // nombra al fragmento es el más profundo.
    let name_idx = last_name_capture(&query);

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
                sexp:  shape_and_tokens(t.node, source),
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

/// Índice de la última captura `@nK` de la query.
///
/// `capture` numera de afuera hacia adentro, así que la de mayor K es la que
/// nombra al fragmento capturado.
fn last_name_capture(query: &Query) -> Option<u32> {
    (0..)
        .map_while(|k| query.capture_index_for_name(&format!("n{k}")))
        .last()
}

/// El nombre que la query busca: el valor del **último** predicado `(#eq? @nK "...")`.
///
/// Es el par de [`rewrite_name_predicate`] —el mismo predicado, leído en vez de
/// reescrito— y es lo que hay que ir a mirar cuando un capture no resuelve.
pub fn anchor_name(query_str: &str) -> Option<String> {
    let at = query_str.rfind("(#eq? @n")?;
    let rest = &query_str[at..];
    let open = rest.find('"')?;
    let close = rest[open + 1..].find('"')? + open + 1;
    Some(rest[open + 1..close].replace("\\\"", "\"").replace("\\\\", "\\"))
}

/// Reemplaza el valor del predicado de nombre del anchor por `new_name`.
///
/// Reescribe el **último** predicado `(#eq? @nK "...")`, que es el que nombra al
/// fragmento. `capture` numera las capturas de afuera hacia adentro: en
/// `(section (atx_heading (inline) @n0 (#eq? @n0 "Doc")) (section (atx_heading
/// (inline) @n1 (#eq? @n1 "Sección"))) @target)` el anchor es `@n1`, y tocar
/// `@n0` reescribiría el título del documento — que no cambió.
pub fn rewrite_name_predicate(query_str: &str, new_name: &str) -> Option<String> {
    let at = query_str.rfind("(#eq? @n")?;
    let rest = &query_str[at..];
    let open = rest.find('"')?;
    let close = rest[open + 1..].find('"')? + open + 1;
    let escaped = new_name.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("{}{}{}",
        &query_str[..at + open + 1], escaped, &query_str[at + close..]))
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;

    #[test]
    fn rewrites_the_anchor_name() {
        let q = r#"(function_item name: (identifier) @n0 (#eq? @n0 "foo")) @target"#;
        let r = rewrite_name_predicate(q, "bar").unwrap();
        assert!(r.contains(r#"(#eq? @n0 "bar")"#), "{r}");
        assert!(!r.contains("foo"));
    }

    #[test]
    fn rewrites_the_innermost_predicate() {
        // `capture` numera de afuera hacia adentro: @n0 es la clase, @n1 el
        // método. Reanclar un método renombrado tiene que tocar @n1.
        let q = r#"(class_declaration name: (identifier) @n0 (#eq? @n0 "A") body: (class_body (method_declaration name: (identifier) @n1 (#eq? @n1 "b")) @target))"#;
        let r = rewrite_name_predicate(q, "z").unwrap();
        assert!(r.contains(r#"(#eq? @n1 "z")"#), "{r}");
        assert!(r.contains(r#"(#eq? @n0 "A")"#), "el ancestro no debería cambiar: {r}");
    }

    #[test]
    fn rewrites_a_nested_markdown_section() {
        // El caso que motivó el arreglo: renombrar una sección tocaba el título
        // del documento en vez de la sección.
        let q = r#"(section (atx_heading (inline) @n0 (#eq? @n0 "Doc")) (section (atx_heading (inline) @n1 (#eq? @n1 "Auto-fix staging"))) @target)"#;
        let r = rewrite_name_predicate(q, "Auto-fix").unwrap();
        assert!(r.contains(r#"(#eq? @n1 "Auto-fix")"#), "{r}");
        assert!(r.contains(r#"(#eq? @n0 "Doc")"#), "el título del documento no cambió: {r}");
    }

    #[test]
    fn returns_none_without_a_name_predicate() {
        assert!(rewrite_name_predicate("(source_file) @target", "x").is_none());
    }

    fn fingerprint(src: &str) -> String {
        let language = crate::grammar::for_language("rust").unwrap();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        shape_and_tokens(tree.root_node(), src)
    }

    /// Reformatear no cambia la huella: es lo que RESTYLED significa.
    #[test]
    fn whitespace_does_not_change_the_fingerprint() {
        assert_eq!(
            fingerprint("fn f(a: u8) -> u8 { a }"),
            fingerprint("fn  f( a : u8 )  ->  u8\n{\n    a\n}\n"),
        );
    }

    /// Renombrar sí. Con la forma del árbol sola, `sref` y `cap` eran el mismo
    /// nodo `(identifier)` y el renombre salía como cambio de formato.
    #[test]
    fn renaming_an_identifier_changes_the_fingerprint() {
        assert_ne!(fingerprint("fn f(sref: u8) {}"), fingerprint("fn f(cap: u8) {}"));
    }

    /// Un literal también. `"0.1.0"` y `"2.0.0"` tienen el mismo árbol.
    #[test]
    fn changing_a_literal_changes_the_fingerprint() {
        assert_ne!(fingerprint(r#"const V: &str = "0.1.0";"#),
                   fingerprint(r#"const V: &str = "2.0.0";"#));
    }

    /// Y un comentario: dice algo, y cambiar lo que dice no es formato. Su cuerpo
    /// no es una hoja —cuelga como hueco, con el `//` de único hijo—, así que sin
    /// leer los huecos sería invisible.
    #[test]
    fn editing_a_comment_changes_the_fingerprint() {
        assert_ne!(fingerprint("// antes\nfn f() {}"), fingerprint("// después\nfn f() {}"));
    }
}



