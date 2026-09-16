use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

use bilink_format::{ByteRange, Ranges, FRAGMENT_SEPARATOR};

/// Lo que la query nombra: uno o más nodos, con su huella.
///
/// Una query puede llevar **más de una** captura `@target`, y entonces el fragmento
/// es la concatenación de sus rangos en orden de archivo. Es lo que permite decir
/// menos que un nodo —la firma de un método sin su cuerpo— y más que un nodo —una
/// ruta que sale de dos anotaciones distintas—, sin dejar de ser estructural: son
/// nodos, no rangos de bytes, así que la referencia sobrevive a que el código se
/// mueva. Ver `concepts/capture.md` § "El fragmento son los `@target`".
#[derive(Debug, Clone)]
pub struct Fragment {
    /// Los rangos, recortados y en orden de archivo.
    pub ranges: Ranges,
    /// Las s-expressions de los nodos, en el mismo orden, unidas por el separador
    /// del fragmento. Ver [`shape_and_tokens`].
    pub sexp: String,
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
///
/// Con varios `@target` se llama **una vez por parte**, antes de concatenar:
/// recortar la concatenación dejaría los bordes internos a merced de dónde termina
/// un nodo y empieza el otro, que es el contexto del que esto existe para
/// independizar.
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


/// El fragmento que la query nombra: **todos** sus `@target`, no sólo el primero.
///
/// Los rangos salen ordenados por posición en el archivo, no por el orden en que la
/// query los nombra: ese orden es un detalle de cómo se escribió el patrón, y el
/// fragmento —que es lo que se hashea— no puede depender de él.
///
/// Cada rango se recorta por separado antes de concatenar. Recortar la
/// concatenación dejaría los bordes internos a merced de dónde termina un nodo y
/// empieza el otro, que es justo el contexto del que [`trim_edges`] existe para
/// independizar.
///
/// Se queda con el **primer match** del patrón, igual que antes: la query lleva
/// predicados que la hacen única, y varios matches significan que no los tiene.
pub fn find_fragment(language: Language, source: &str, query_str: &str) -> Result<Option<Fragment>> {
    Ok(fragments(language, source, query_str, true)?.into_iter().next().map(|m| m.fragment))
}

/// Todos los matches del patrón, cada uno con **todas** sus partes.
///
/// Es lo que usa la verificación al generar una query: un patrón que matchea dos
/// veces no identifica nada, y hay que poder decirlo antes de escribir el capture.
pub fn find_all_fragments(language: Language, source: &str, query_str: &str) -> Result<Vec<Fragment>> {
    Ok(fragments(language, source, query_str, false)?.into_iter().map(|m| m.fragment).collect())
}

/// Los rangos del primer match y la declaración de su ancla: el padre del nodo que
/// nombra el último predicado `(#eq? @nK "...")`.
///
/// Un fragmento de varias partes deja afuera lo que hay entre ellas —en un endpoint
/// de Spring, el nombre y el cuerpo del método—, y la declaración es el nodo que
/// las envuelve. `None` si la query no matchea o no tiene ancla.
pub fn declaration(language: Language, source: &str, query_str: &str) -> Result<Option<(Ranges, ByteRange)>> {
    let Some(anchor) = anchor_capture(query_str) else { return Ok(None) };

    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(source, None).context("parse failed")?;
    let query = Query::new(&language, query_str)
        .with_context(|| format!("invalid query:\n{query_str}"))?;
    let Some(anchor_idx) = query.capture_index_for_name(&anchor) else { return Ok(None) };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let Some(m) = matches.next() else { return Ok(None) };
    let Some(parent) = m.captures.iter()
        .find(|cap| cap.index == anchor_idx)
        .and_then(|cap| cap.node.parent()) else { return Ok(None) };

    let Some(fragment) = find_fragment(language, source, query_str)? else { return Ok(None) };
    let (start, end) = trim_edges(source, parent.start_byte(), parent.end_byte());
    Ok(Some((fragment.ranges, ByteRange { start, end })))
}

/// El nombre de la captura del último predicado `(#eq? @nK "...")`: `nK`.
fn anchor_capture(query_str: &str) -> Option<String> {
    let at = query_str.rfind("(#eq? @")? + "(#eq? @".len();
    let name: String = query_str[at..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    (!name.is_empty()).then_some(name)
}

/// Un match de la query: el fragmento entero, y el texto del último predicado de
/// nombre si la query lo tiene.
pub struct TargetMatch {
    pub fragment: Fragment,
    pub name:     Option<String>,
}

/// Todos los matches de la query, no solo el primero.
///
/// [`find_fragment`] corta en el primero porque la query lleva predicados que la
/// hacen única. Esta versión existe para las queries relajadas: al quitar los
/// `#eq?` hay varios candidatos y hay que recorrerlos. Cada candidato es el
/// fragmento con todas sus partes, que es contra lo que se compara el texto
/// aceptado.
pub fn find_all_targets(language: Language, source: &str, query_str: &str) -> Result<Vec<TargetMatch>> {
    fragments(language, source, query_str, false)
}

fn fragments(language: Language, source: &str, query_str: &str, first_only: bool) -> Result<Vec<TargetMatch>> {
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

    let mut cursor = QueryCursor::new();
    let root = tree.root_node();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    let mut out = Vec::new();

    while let Some(m) = matches.next() {
        let mut nodes: Vec<Node> = m.captures.iter()
            .filter(|cap| cap.index == target_idx)
            .map(|cap| cap.node)
            .collect();
        if nodes.is_empty() { continue; }

        let name = m.captures.iter()
            .find(|cap| Some(cap.index) == name_idx)
            .map(|cap| source[cap.node.byte_range()].to_string());

        nodes.sort_by_key(|n| (n.start_byte(), n.end_byte()));
        nodes.dedup_by_key(|n| (n.start_byte(), n.end_byte()));

        let sexp = nodes.iter()
            .map(|n| shape_and_tokens(*n, source))
            .collect::<Vec<_>>()
            .join(FRAGMENT_SEPARATOR);

        let parts = nodes.iter()
            .map(|n| {
                let (start, end) = trim_edges(source, n.start_byte(), n.end_byte());
                ByteRange { start, end }
            })
            .collect();

        out.push(TargetMatch {
            fragment: Fragment {
                ranges: Ranges::new(parts).expect("hay al menos un @target"),
                sexp,
            },
            name,
        });
        if first_only { break; }
    }
    Ok(out)
}

/// El texto de un nombre, listo para entrar en un predicado `(#eq? @nK "...")`.
///
/// Un predicado es un string **adentro** de una query, así que dos caracteres no se
/// pueden escribir tal cual: un `\` cambia lo que sigue —`\n` es un salto de línea y
/// no dos caracteres, así que un heading llamado `` El separador es `\n` `` produce
/// una query que no matchea nada— y un `"` cierra el string y hace **inválida** la
/// query entera.
///
/// **Está acá y no en cada generador de predicados** porque hay seis, y hasta que
/// esto existió tenían seis políticas distintas: uno no escapaba nada, dos
/// descartaban el ancla si llevaba comillas, y tres escapaban la comilla y no la
/// barra. El mismo nombre producía dos queries según por qué camino se hubiera
/// generado — y [`rewrite_name_predicate`], que reescribe el mismo campo, escapaba
/// de una tercera forma.
pub fn escape_query_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// La inversa de [`escape_query_string`]: el nombre tal como está en el archivo.
pub fn unescape_query_string(text: &str) -> String {
    text.replace("\\\"", "\"").replace("\\\\", "\\")
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
mod fragment_tests {
    use super::*;

    fn rust(src: &str, query: &str) -> Fragment {
        let language = crate::grammar::for_language("rust").unwrap();
        find_fragment(language, src, query).unwrap().expect("la query debería resolver")
    }

    const SRC: &str = "fn f(a: u8) -> u8 {\n    let x = a;\n    x\n}\n";

    /// El caso de siempre: un `@target`, un rango, y el fragmento es el nodo.
    #[test]
    fn one_target_is_the_node_and_nothing_else() {
        let f = rust(SRC, r#"(function_item name: (identifier) @n0 (#eq? @n0 "f")) @target"#);
        assert_eq!(f.ranges.parts().len(), 1);
        assert_eq!(f.ranges.text(SRC), SRC.trim_end());
    }

    /// El fragmento es de N partes, en orden de archivo. Que el orden sea el del
    /// archivo y no el de la query lo fija [`bilink_format::Ranges::new`]; acá lo
    /// que se comprueba es que las N partes lleguen.
    #[test]
    fn several_targets_make_a_fragment_of_several_parts() {
        let q = r#"(function_item
                      name: (identifier) @target
                      return_type: (primitive_type) @target) @fn"#;
        let f = rust(SRC, q);
        assert_eq!(f.ranges.parts().len(), 2);
        assert_eq!(f.ranges.text(SRC), "f\nu8");
    }

    /// Un candidato de la búsqueda de un ancla renombrada es el fragmento entero, y
    /// no la última de sus partes: la similitud se mide contra el texto aceptado, que
    /// es la concatenación de todas.
    #[test]
    fn a_relaxed_candidate_carries_every_part() {
        let language = crate::grammar::for_language("rust").unwrap();
        let q = r#"(function_item
                      name: (identifier) @n0
                      parameters: (parameters) @target
                      return_type: (primitive_type) @target)"#;
        let all = find_all_targets(language, SRC, q).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].fragment.ranges.text(SRC), "(a: u8)\nu8");
        assert_eq!(all[0].name.as_deref(), Some("f"));
    }

    /// El separador es `\n`, y entra en el hash. Cambiarlo movería el de todos los
    /// captures multi-fragmento a la vez, así que queda fijado acá.
    #[test]
    fn the_separator_is_a_newline() {
        assert_eq!(bilink_format::FRAGMENT_SEPARATOR, "\n");
        let q = r#"(function_item
                      name: (identifier) @target
                      return_type: (primitive_type) @target) @fn"#;
        let f = rust(SRC, q);
        assert_eq!(f.ranges.text(SRC), ["f", "u8"].join(bilink_format::FRAGMENT_SEPARATOR));
    }

    /// La firma sin el cuerpo: el cuerpo cambia y el fragmento no.
    #[test]
    fn the_signature_survives_a_change_in_the_body() {
        let q = r#"(function_item
                      name: (identifier) @target
                      parameters: (parameters) @target
                      return_type: (primitive_type) @target) @fn"#;
        let otro = "fn f(a: u8) -> u8 {\n    a + 1\n}\n";
        assert_eq!(rust(SRC, q).ranges.text(SRC), rust(otro, q).ranges.text(otro));
    }

    /// Y sí cambia cuando cambia el tipo de retorno — el caso que rompió a
    /// `retinar`: la ruta seguía igual y lo que devolvía era otra cosa.
    #[test]
    fn the_signature_changes_when_the_return_type_does() {
        let q = r#"(function_item
                      name: (identifier) @target
                      parameters: (parameters) @target
                      return_type: (primitive_type) @target) @fn"#;
        let otro = "fn f(a: u8) -> u16 {\n    let x = a;\n    x\n}\n";
        assert_ne!(rust(SRC, q).ranges.text(SRC), rust(otro, q).ranges.text(otro));
    }

    /// Cada parte se recorta por su cuenta. Recortar la concatenación dejaría los
    /// bordes internos a merced de dónde termina un nodo y empieza el otro.
    #[test]
    fn each_part_is_trimmed_on_its_own() {
        let f = rust(SRC, r#"(function_item
                                name: (identifier) @target
                                body: (block) @target) @fn"#);
        for r in f.ranges.parts() {
            assert!(!SRC[r.start..r.end].starts_with(char::is_whitespace));
            assert!(!SRC[r.start..r.end].ends_with(char::is_whitespace));
        }
    }

    /// La huella sigue al fragmento: una s-expression por nodo, en el mismo orden
    /// y con el mismo separador.
    #[test]
    fn the_fingerprint_is_one_sexp_per_node() {
        let f = rust(SRC, r#"(function_item
                                name: (identifier) @target
                                return_type: (primitive_type) @target) @fn"#);
        assert_eq!(f.sexp.split(bilink_format::FRAGMENT_SEPARATOR).count(), 2);
    }
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

    /// Una barra adentro de un predicado cambia lo que sigue: `\\n` es un salto de
    /// línea, así que sin escapar el predicado busca un nombre que no existe. Es el
    /// caso que lo destapó — un heading de este mismo proyecto.
    #[test]
    fn a_backslash_in_the_name_survives_the_predicate() {
        let esc = escape_query_string("El separador es `\\n`");
        assert_eq!(esc, "El separador es `\\\\n`");
        assert_eq!(unescape_query_string(&esc), "El separador es `\\n`");
    }

    /// Una comilla es peor: no da una query que no matchea, da una inválida.
    #[test]
    fn a_quote_in_the_name_does_not_end_the_string() {
        let esc = escape_query_string("dice \"hola\"");
        assert_eq!(esc, "dice \\\"hola\\\"");
        assert_eq!(unescape_query_string(&esc), "dice \"hola\"");
    }

    /// Escribir y reescribir el mismo campo tienen que usar el mismo escape: si no,
    /// el mismo nombre produce dos queries según por qué camino se generó.
    #[test]
    fn rewriting_escapes_the_same_way_as_writing() {
        let q = r#"(function_item name: (identifier) @n0 (#eq? @n0 "foo")) @target"#;
        let r = rewrite_name_predicate(q, r"a\b").unwrap();
        assert!(r.contains(r#"(#eq? @n0 "a\\b")"#), "{r}");
        assert_eq!(anchor_name(&r).as_deref(), Some(r"a\b"));
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
    let (start, end) = last_predicate_value(query_str)?;
    Some(unescape_query_string(&query_str[start..end]))
}

/// El rango, sin las comillas, del valor del último predicado `(#eq? @nK "...")`.
///
/// El valor está escapado con las reglas de [`escape_query_string`], así que una `"`
/// precedida de `\` no cierra el string: el literal de una anotación de ruta,
/// `("/report")`, lleva dos adentro.
fn last_predicate_value(query_str: &str) -> Option<(usize, usize)> {
    let at = query_str.rfind("(#eq? @n")?;
    let start = at + query_str[at..].find('"')? + 1;
    let mut escaped = false;
    for (i, c) in query_str[start..].char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped  => return Some((start, start + i)),
            _                => escaped = false,
        }
    }
    None
}

/// Reemplaza el valor del predicado de nombre del anchor por `new_name`.
///
/// Reescribe el **último** predicado `(#eq? @nK "...")`, que es el que nombra al
/// fragmento. `capture` numera las capturas de afuera hacia adentro: en
/// `(section (atx_heading (inline) @n0 (#eq? @n0 "Doc")) (section (atx_heading
/// (inline) @n1 (#eq? @n1 "Sección"))) @target)` el anchor es `@n1`, y tocar
/// `@n0` reescribiría el título del documento — que no cambió.
pub fn rewrite_name_predicate(query_str: &str, new_name: &str) -> Option<String> {
    let (start, end) = last_predicate_value(query_str)?;
    Some(format!("{}{}{}",
        &query_str[..start], escape_query_string(new_name), &query_str[end..]))
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

    /// El valor de un predicado puede llevar comillas escapadas —el literal de una
    /// anotación de ruta es `("/report")`—, y la primera `"` que aparece después de
    /// la de apertura no es la de cierre. Cortar ahí devolvía `(\`, que es lo que
    /// `check` mostraba como anchor de un endpoint sin resolver.
    #[test]
    fn reads_an_anchor_whose_value_has_escaped_quotes() {
        let q = "(annotation arguments: (annotation_argument_list) @n0 (#eq? @n0 \"(\\\"/report\\\")\")) @target";
        assert_eq!(anchor_name(q).as_deref(), Some("(\"/report\")"));
    }

    #[test]
    fn rewrites_an_anchor_whose_value_has_escaped_quotes() {
        let q = "(annotation arguments: (annotation_argument_list) @n0 (#eq? @n0 \"(\\\"/report\\\")\")) @target";
        let r = rewrite_name_predicate(q, "(\"/informe\")").unwrap();
        assert_eq!(r, "(annotation arguments: (annotation_argument_list) @n0 (#eq? @n0 \"(\\\"/informe\\\")\")) @target");
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



