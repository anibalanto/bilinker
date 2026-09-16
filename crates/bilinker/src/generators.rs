//! Los generadores de query que este binario conoce.
//!
//! Uno es del núcleo —[`Interface`], que sólo sabe de gramática— y el otro sabe de
//! un framework —[`SpringController`]. Los dos se piden igual, `--as <nombre>`, y
//! los dos **desaparecen**: lo que queda escrito es una query normal.
//!
//! Agregar otro framework es agregar otra `impl` acá. Ver
//! [`CaptureGenerator`](crate::capture::CaptureGenerator).

use anyhow::{bail, Result};
use tree_sitter::Node;

use bilink_format::Ranges;

use crate::capture::{CaptureGenerator, GenCtx, Generated};
use crate::grammar;
use crate::query;

// ─── interface: la firma sin el cuerpo ────────────────────────────────────────

/// La firma: el nodo señalado **menos su cuerpo**.
///
/// Es lo único que se puede saber sin saber de ningún framework: la gramática nombra
/// el campo del cuerpo, y la firma es todo lo demás.
pub struct Interface;

impl CaptureGenerator for Interface {
    fn name(&self) -> &'static str { "interface" }

    fn describe(&self) -> &'static str {
        "la firma sin el cuerpo: el nodo menos su campo `body`"
    }

    /// Aplica donde haya un cuerpo que sacar. No aplica sobre un nodo que ya es sólo
    /// firma: no habría nada que el modo agregue.
    fn applies(&self, file: &str, _source: &str, node: Node) -> bool {
        grammar::body_field(grammar::language_for_file(file))
            .and_then(|f| node.child_by_field_name(f))
            .is_some()
    }

    fn query<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Generated<'t>> {
        let field = grammar::body_field(ctx.lang).ok_or_else(|| anyhow::anyhow!(
            "`--as interface` no sabe qué es el cuerpo en {}.\n       \
             Señalar las partes a mano, o agregar {} a la tabla.", ctx.lang, ctx.lang
        ))?;

        // Un nodo sin cuerpo se captura entero: si la gramática no le da campo
        // `body` —la firma de un método en una interface de TypeScript— la firma
        // *es* el nodo, y no hay nada que sacarle.
        let targets: Vec<Node> = match node.child_by_field_name(field) {
            None => vec![node],
            Some(body) => {
                let mut cursor = node.walk();
                let parts: Vec<Node> = node.named_children(&mut cursor)
                    .filter(|n| n.id() != body.id())
                    .collect();
                if parts.is_empty() {
                    bail!(
                        "el `{}` de la línea {} es todo cuerpo: no hay firma que capturar.",
                        node.kind(), node.start_position().row + 1
                    );
                }
                parts
            }
        };

        Ok(Generated {
            query: crate::capture::pattern_for(ctx, &[node], &targets),
            targets,
        })
    }

    /// Una firma se nombra por su método, que es lo que la distingue.
    ///
    /// **Y sale del fragmento**: `--as interface` captura el nombre —lo pone en los
    /// dos roles, capturado *y* anclado—, así que está adentro de lo referenciado y
    /// no hay que ir a buscarlo a la query.
    fn alias(&self, source: &str, ranges: &Ranges, _query: &str) -> Option<String> {
        nombre_entre_partes(source, ranges)
    }
}

// ─── spring-controller: el endpoint, no el método ─────────────────────────────

/// Las anotaciones con que Spring marca la ruta de un método.
const MAPPINGS: &[&str] = &[
    "GetMapping", "PostMapping", "PutMapping", "DeleteMapping", "PatchMapping",
    "RequestMapping",
];

/// La anotación con que Spring marca el prefijo de ruta de una clase.
const CLASS_MAPPING: &str = "RequestMapping";

/// El contrato de un endpoint de Spring: la ruta compuesta y la forma que devuelve.
///
/// Señalás **el método** y salen cuatro fragmentos: el `@RequestMapping` de la
/// clase, la anotación de ruta del método, el tipo de retorno y los parámetros.
///
/// **La ruta compuesta es el caso que no tenía salida.** Sale de dos anotaciones en
/// nodos distintos, y el literal completo no aparece en ningún lado del archivo.
///
/// **El nombre del método no se captura.** Renombrarlo no cambia el contrato del
/// endpoint; meterlo en el fragmento haría que un refactor interno disparara drift,
/// que es lo que capturar la firma existe para dejar de hacer.
pub struct SpringController;

impl CaptureGenerator for SpringController {
    fn name(&self) -> &'static str { "spring-controller" }

    fn describe(&self) -> &'static str {
        "el endpoint de Spring: la ruta compuesta, el tipo de retorno y los parámetros"
    }

    fn applies(&self, file: &str, source: &str, node: Node) -> bool {
        grammar::language_for_file(file) == "java"
            && node.kind() == "method_declaration"
            && route_annotation(node, source).is_some()
    }

    fn query<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Generated<'t>> {
        if ctx.lang != "java" {
            bail!("`--as spring-controller` es de Java, y esto es {}.", ctx.lang);
        }
        if node.kind() != "method_declaration" {
            bail!(
                "`--as spring-controller` va sobre un método, y la posición señala un `{}`.",
                node.kind()
            );
        }
        let Some(route) = route_annotation(node, ctx.source) else {
            bail!(
                "el método de la línea {} no tiene anotación de ruta ({}).\n       \
                 Sin ruta no hay endpoint que describir: probar `--as interface`.",
                node.start_position().row + 1, MAPPINGS.join(", ")
            );
        };

        let class = enclosing_class(node);
        let class_mapping = class.and_then(|c| class_annotation(c, ctx.source));

        // El tipo de retorno y los parámetros. El nombre queda afuera a propósito.
        let mut targets: Vec<Node> = Vec::new();
        if let Some(m) = class_mapping { targets.push(m); }
        targets.push(route);
        for field in ["type", "parameters"] {
            if let Some(child) = node.child_by_field_name(field) { targets.push(child); }
        }

        let query = spring_pattern(ctx, class, node, class_mapping);

        Ok(Generated { query, targets })
    }

    /// `GET /public-api/user/info/from-token`, compuesto de lo capturado.
    ///
    /// La ruta de clase y el literal del método son dos de los cuatro `@target`; el
    /// verbo sale del nombre de la anotación. Nada de esto se busca afuera del
    /// fragmento, que es lo que hace que no pueda mentir.
    fn alias(&self, source: &str, ranges: &Ranges, query: &str) -> Option<String> {
        let partes: Vec<&str> = ranges.parts().iter()
            .filter_map(|r| source.get(r.start..r.end))
            .collect();

        // La anotación de verbo es la primera parte que nombra un mapping; lo que
        // haya antes es la ruta de clase, que puede no estar.
        let (verbo_i, verbo) = partes.iter().enumerate()
            .find_map(|(i, p)| verbo_de(p).map(|v| (i, v)))?;

        let mut ruta = String::new();
        for p in &partes[..verbo_i] { ruta.push_str(&literal_de(p).unwrap_or_default()); }
        ruta.push_str(&literal_de(partes[verbo_i]).unwrap_or_default());
        if ruta.is_empty() { ruta.push('/'); }

        // **Sin literal propio, la ruta y el verbo los comparten los hermanos.** Lo
        // que distingue es el nombre del método, y está en la query porque `32` lo
        // puso ahí como ancla justo donde el literal falta: donde falta el literal
        // sobra el ancla, y viceversa.
        match (literal_de(partes[verbo_i]), nombre_entre_partes(source, ranges)) {
            (None, Some(m)) => Some(format!("{verbo} {ruta}  ·  {m}")),
            _               => Some(format!("{verbo} {ruta}")),
        }
    }
}

/// El verbo que declara una anotación de mapping, si es una.
fn verbo_de(texto: &str) -> Option<&'static str> {
    let nombre = texto.trim_start_matches('@');
    let nombre = nombre.split(['(', ' ', '\n']).next()?;
    Some(match nombre {
        "GetMapping"     => "GET",
        "PostMapping"    => "POST",
        "PutMapping"     => "PUT",
        "DeleteMapping"  => "DELETE",
        "PatchMapping"   => "PATCH",
        // `@RequestMapping` sin `method` no declara verbo: es la ruta de la clase.
        _ => return None,
    })
}

/// El literal de ruta de una anotación, si lo lleva.
///
/// **Sólo la ruta**: el primer argumento cuando es un string, o el valor de `value` o
/// de `path`. `params`, `produces`, `consumes` y `headers` también llevan strings, y
/// tomar el primero que aparece convertía `@GetMapping(params = "ids")` en la ruta
/// `ids`.
fn literal_de(texto: &str) -> Option<String> {
    let (_, args) = texto.split_once('(')?;
    let args = args.trim_start();
    let valor = if args.starts_with('"') || args.starts_with('{') {
        args
    } else {
        ["value", "path"].iter().find_map(|clave| {
            let mut resto = args;
            while let Some(i) = resto.find(clave) {
                let antes = resto[..i].chars().last();
                let despues = resto[i + clave.len()..].trim_start();
                if antes.map_or(true, |c| !c.is_alphanumeric() && c != '_') && despues.starts_with('=') {
                    return Some(despues[1..].trim_start());
                }
                resto = &resto[i + clave.len()..];
            }
            None
        })?
    };
    let valor = valor.trim_start_matches('{').trim_start();
    let resto = valor.strip_prefix('"')?;
    let (lit, _) = resto.split_once('"')?;
    (!lit.is_empty()).then(|| lit.to_string())
}

/// El nombre del método: lo que hay en el archivo **entre** el tipo de retorno y los
/// parámetros.
///
/// **No se lee de la query, y ése fue el error.** Parecía razonable —el nombre está
/// anclado ahí— pero `name: (identifier)` aparece también en las anotaciones y en la
/// clase, que van más arriba del árbol y por lo tanto antes en el patrón. Ni el
/// primero ni el último aciertan: sobre 98 endpoints reales salieron `GetMapping`,
/// `PutMapping` y hasta el nombre de una clase.
///
/// Entre los dos últimos `@target` de un capture de contrato **no hay nada más que el
/// nombre**: el tipo termina, viene el nombre, arrancan los parámetros. Eso no es una
/// heurística sobre texto, es la forma que el generador escribió.
fn nombre_entre_partes(source: &str, ranges: &Ranges) -> Option<String> {
    let partes = ranges.parts();
    let [.., tipo, params] = partes else { return None };
    let entre = source.get(tipo.end..params.start)?.trim();
    (!entre.is_empty() && entre.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then(|| entre.to_string())
}

/// La anotación de ruta de un método, si la tiene.
fn route_annotation<'t>(node: Node<'t>, source: &str) -> Option<Node<'t>> {
    annotation_named(node, MAPPINGS, source)
}

/// El `@RequestMapping` de la clase, si lo tiene.
fn class_annotation<'t>(class: Node<'t>, source: &str) -> Option<Node<'t>> {
    annotation_named(class, &[CLASS_MAPPING], source)
}

/// La primera anotación de `node` cuyo nombre esté en `names`.
///
/// Busca en el hijo `modifiers`, que es donde la gramática de Java cuelga las
/// anotaciones — antes del nombre y del tipo, que es también por qué el orden de las
/// partes de un patrón importa.
fn annotation_named<'t>(node: Node<'t>, names: &[&str], source: &str) -> Option<Node<'t>> {
    let mut c = node.walk();
    let modifiers: Vec<Node> = node.children(&mut c)
        .filter(|n| n.kind() == "modifiers")
        .collect();
    for m in modifiers {
        let mut c2 = m.walk();
        let found = m.children(&mut c2)
            .filter(|n| matches!(n.kind(), "annotation" | "marker_annotation"))
            .find(|n| n.child_by_field_name("name")
                .map(|id| names.contains(&&source[id.byte_range()]))
                .unwrap_or(false));
        if found.is_some() { return found; }
    }
    None
}

/// Si otro método de la clase se llama igual: una sobrecarga, que el nombre solo no
/// distingue.
fn is_overloaded(class: Option<Node>, method: Node, source: &str) -> bool {
    let (Some(class), Some(name)) = (class, method.child_by_field_name("name")) else { return false };
    let Some(body) = class.child_by_field_name("body") else { return false };
    let nombre = &source[name.byte_range()];
    let mut c = body.walk();
    let repetido = body.children(&mut c)
        .filter(|n| n.kind() == "method_declaration" && n.id() != method.id())
        .any(|n| n.child_by_field_name("name").is_some_and(|x| &source[x.byte_range()] == nombre));
    repetido
}

/// Los parámetros de una sobrecarga: el nodo es contenido, y el tipo de cada uno ancla.
///
/// **En orden y sin huecos**, con `.` entre hijos: `(Short)` no tiene que matchear
/// `(Short, Integer)`. **Con `#match?` y no `#eq?`**, para que el último `#eq?` de la
/// query siga siendo el nombre del método.
fn parameter_types_pattern(cap: &mut impl FnMut() -> String, params: Node, source: &str) -> String {
    let mut c = params.walk();
    let mut hijos = Vec::new();
    for p in params.named_children(&mut c) {
        match p.child_by_field_name("type") {
            Some(t) if p.kind() == "formal_parameter" => {
                let n = cap();
                hijos.push(format!(
                    "(formal_parameter type: (_) {n} (#match? {n} \"^{}$\"))",
                    query::escape_query_string(&regex_literal(&source[t.byte_range()]))));
            }
            _ => hijos.push(format!("({})", p.kind())),
        }
    }
    if hijos.is_empty() {
        return "parameters: (formal_parameters) @target".to_string();
    }
    format!("parameters: (formal_parameters\n        .\n        {}\n        .) @target",
            hijos.join("\n        .\n        "))
}

/// Un texto como regex que lo matchea literal.
fn regex_literal(texto: &str) -> String {
    let mut out = String::with_capacity(texto.len());
    for ch in texto.chars() {
        if "\\.^$|?*+()[]{}".contains(ch) { out.push('\\'); }
        out.push(ch);
    }
    out
}

fn enclosing_class<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "class_declaration" { return Some(n); }
        cur = n.parent();
    }
    None
}

/// El patrón de un endpoint, anclado por el nombre de su método.
///
/// Las capturas se numeran `@nK` de afuera hacia adentro, como las que escribe el
/// núcleo: así el **último** predicado es el del nombre, y es el que `recapture` y
/// la búsqueda de anclas renombradas van a mirar.
fn spring_pattern(
    ctx:           &GenCtx<'_>,
    class:         Option<Node>,
    method:        Node,
    class_mapping: Option<Node>,
) -> String {
    let source = ctx.source;
    let mut k = 0usize;
    let mut cap = || { let c = format!("@n{k}"); k += 1; c };

    // **Una anotación de ruta se reconoce por clase, no por nombre.** Con `#eq?`
    // cambiar `@GetMapping` por `@PostMapping` dejaba de matchear, y el verbo es
    // contenido. Y `#match?` no es `#eq?`: el último `#eq?` sigue siendo el ancla, y
    // la búsqueda de un ancla renombrada, que saca los `#eq?`, no los saca.
    //
    // **Y sin kind**: `annotation` y `marker_annotation` son la misma anotación con
    // y sin literal, y sacarle el literal es cambiar la ruta.
    let annotation_pat = |c: String, names: &[&str]| format!(
        "(_\n          name: (identifier) {c} (#match? {c} \"^({})$\")) @target",
        names.join("|"));

    let class_pat = class_mapping.map(|_| {
        format!("(modifiers\n    {})", annotation_pat(cap(), &[CLASS_MAPPING]))
    });

    let mut method_parts = vec![format!("(modifiers\n        {})", annotation_pat(cap(), MAPPINGS))];

    // **El ancla es el nombre del método, y sólo el ancla.** Entra como predicado y
    // no lleva `@target`, que es el reparto inverso al de `interface`: el contrato de
    // un endpoint no incluye cómo se llama el método que lo sirve, así que
    // renombrarlo es una relocalización. Y la ruta, que era el ancla cuando la
    // anotación llevaba literal, es lo que el fragmento captura: siendo ancla, al
    // cambiar se perdía el puntero en vez de verse el diff.
    //
    // Los `@target` van con `(_)`: el kind del tipo de retorno es parte de lo
    // capturado, y `List<Dto>` → `Dto` es un cambio de contenido.
    //
    // Las partes salen en el orden de la gramática, que en Java es
    // `modifiers, type, name, parameters`. El nombre va en el medio, no al final.
    for field in ["type", "name", "parameters"] {
        let Some(child) = method.child_by_field_name(field) else { continue };
        if field == "parameters" && is_overloaded(class, method, source) {
            method_parts.push(parameter_types_pattern(&mut cap, child, source));
            continue;
        }
        if field == "name" {
            let c = cap();
            method_parts.push(format!(
                "name: ({kind}) {c} (#eq? {c} \"{n}\")",
                kind = child.kind(), n = query::escape_query_string(&source[child.byte_range()])));
        } else {
            method_parts.push(format!("{field}: (_) @target"));
        }
    }
    let method_pat = format!("(method_declaration\n      {})", method_parts.join("\n      "));

    match (class, class_pat) {
        (Some(class), Some(cp)) => format!(
            "({kind}\n  {cp}\n  body: (class_body\n    {method_pat}))", kind = class.kind()),
        (Some(class), None) => format!(
            "({kind}\n  body: (class_body\n    {method_pat}))", kind = class.kind()),
        _ => method_pat,
    }
}
