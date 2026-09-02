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

        // **El ancla es la ruta, no el nombre del método.** Un refactor renombra el
        // método y no la ruta, y lo que el bilink describe es el contrato. Es el
        // reverso de que una ruta compuesta no sirva de ancla: el pedazo que aporta
        // el método sí existe como literal.
        let query = spring_pattern(ctx, class, node, class_mapping, route);

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
/// Toma el **primer** string de los argumentos: `("/x")` y `(value = "/x", produces
/// = "…")` dan lo mismo. `produces` y `consumes` van después y no son ruta.
fn literal_de(texto: &str) -> Option<String> {
    let (_, resto) = texto.split_once('"')?;
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

fn enclosing_class<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "class_declaration" { return Some(n); }
        cur = n.parent();
    }
    None
}

/// El patrón de un endpoint, anclado por el literal de su ruta.
///
/// Las capturas se numeran `@nK` de afuera hacia adentro, como las que escribe el
/// núcleo: así el **último** predicado es el de la ruta, y es el que `recapture` y
/// la búsqueda de anclas renombradas van a mirar. El ancla de un endpoint es su
/// ruta, así que eso es exactamente lo que corresponde.
fn spring_pattern(
    ctx:           &GenCtx<'_>,
    class:         Option<Node>,
    method:        Node,
    class_mapping: Option<Node>,
    route:         Node,
) -> String {
    let source = ctx.source;
    let esc = query::escape_query_string;
    let mut k = 0usize;
    let mut cap = || { let c = format!("@n{k}"); k += 1; c };

    // La anotación de la clase: aporta el prefijo de la ruta.
    let class_pat = class_mapping.map(|m| {
        let c = cap();
        let name = m.child_by_field_name("name")
            .map(|n| &source[n.byte_range()]).unwrap_or(CLASS_MAPPING);
        format!("(modifiers\n    ({kind}\n      name: (identifier) {c} (#eq? {c} \"{name}\")) @target)",
                kind = m.kind(), name = esc(name))
    });

    // La anotación del método: aporta el resto de la ruta, y es el ancla.
    let route_name = route.child_by_field_name("name")
        .map(|n| &source[n.byte_range()]).unwrap_or("RequestMapping");
    let cn = cap();
    let route_pat = match route.child_by_field_name("arguments") {
        Some(args) => {
            let ca = cap();
            let fields = format!(
                "name: (identifier) {cn} (#eq? {cn} \"{name}\")\n          \
                 arguments: ({akind}) {ca} (#eq? {ca} \"{lit}\")",
                name  = esc(route_name),
                akind = args.kind(),
                lit   = esc(&source[args.byte_range()]));
            format!("({kind}\n          {fields}) @target", kind = route.kind())
        }
        None => format!("({kind}\n          name: (identifier) {cn} (#eq? {cn} \"{name}\")) @target",
                        kind = route.kind(), name = esc(route_name)),
    };

    let mut method_parts = vec![format!("(modifiers\n        {route_pat})")];

    // **Cuando la anotación del método no lleva literal, el ancla es su nombre.**
    //
    // `@GetMapping` a secas es la mitad de los endpoints de una api real: la ruta la
    // aporta entera el `@RequestMapping` de la clase, y el método no agrega ningún
    // literal. Sin esto el único predicado sería el nombre de la anotación, que
    // matchea cualquier hermano con la misma anotación pelada — un capture que
    // depende de que hoy haya uno solo, y que se muda al vecino en cuanto aparece
    // otro. `verify_query_identifies` no lo agarra: pregunta si es única *ahora*.
    //
    // **Entra como predicado y no lleva `@target`**, que es el reparto inverso al de
    // `interface`. Los dos salen del mismo criterio —qué describe el fragmento—: el
    // contrato de un endpoint no incluye cómo se llama el método que lo sirve, así
    // que renombrarlo tiene que ser una relocalización y no un cambio de contenido.
    // Las partes salen en el orden de la gramática, que en Java es
    // `modifiers, type, name, parameters`. El nombre va en el medio, no al final.
    let anclar_por_nombre = route.child_by_field_name("arguments").is_none();
    for field in ["type", "name", "parameters"] {
        let Some(child) = method.child_by_field_name(field) else { continue };
        if field == "name" {
            if !anclar_por_nombre { continue }
            let c = cap();
            method_parts.push(format!(
                "name: ({kind}) {c} (#eq? {c} \"{n}\")",
                kind = child.kind(), n = esc(&source[child.byte_range()])));
        } else {
            method_parts.push(format!("{field}: ({}) @target", child.kind()));
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
