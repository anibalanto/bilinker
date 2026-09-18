//! Los generadores que este binario conoce.
//!
//! Uno es del núcleo —[`Interface`], que sólo sabe de gramática— y el otro sabe de
//! un framework —[`SpringController`]. Los dos se piden igual, `--as <nombre>`, y
//! los dos **desaparecen**: el capture es el del núcleo, y lo que queda escrito en el
//! endpoint son las dimensiones que declararon, cada una con su query.
//!
//! Agregar otro framework es agregar otra `impl` acá. Ver
//! [`CaptureGenerator`](crate::capture::CaptureGenerator).

use anyhow::{bail, Result};
use tree_sitter::Node;

use bilink_format::Ranges;

use crate::capture::{CaptureGenerator, GenCtx, GeneratedDimension};
use crate::grammar;

// ─── interface: la firma sin el cuerpo ────────────────────────────────────────

/// La firma: los hijos del nodo señalado **menos su cuerpo**, una dimensión cada uno.
///
/// Es lo único que se puede saber sin saber de ningún framework: la gramática nombra
/// el campo del cuerpo, y la firma es todo lo demás.
pub struct Interface;

impl CaptureGenerator for Interface {
    fn name(&self) -> &'static str { "interface" }

    fn describe(&self) -> &'static str {
        "la firma sin el cuerpo: una dimensión por cada hijo del nodo menos `body`"
    }

    /// Aplica donde haya un cuerpo que sacar. No aplica sobre un nodo que ya es sólo
    /// firma: no habría nada que el modo agregue.
    fn applies(&self, file: &str, _source: &str, node: Node) -> bool {
        grammar::body_field(grammar::language_for_file(file))
            .and_then(|f| node.child_by_field_name(f))
            .is_some()
    }

    fn dimensions<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Vec<GeneratedDimension<'t>>> {
        let field = grammar::body_field(ctx.lang).ok_or_else(|| anyhow::anyhow!(
            "`--as interface` no sabe qué es el cuerpo en {}.\n       \
             Señalar las partes a mano, o agregar {} a la tabla.", ctx.lang, ctx.lang
        ))?;

        // Un nodo sin cuerpo declara todos sus hijos: si la gramática no le da campo
        // `body` —la firma de un método en una interface de TypeScript— la firma
        // *es* el nodo, y no hay nada que sacarle.
        let dims = children_dimensions(node, &[field]);
        if dims.is_empty() {
            bail!(
                "el `{}` de la línea {} es todo cuerpo: no hay firma que vigilar.",
                node.kind(), node.start_position().row + 1
            );
        }
        Ok(dims)
    }

    /// Una firma se nombra por su método: el texto de su dimensión `name`.
    fn alias(&self, source: &str, parts: &[(String, Ranges)]) -> Option<String> {
        let (_, r) = parts.iter().find(|(n, _)| n == "name")?;
        let nombre = r.text(source);
        (!nombre.is_empty() && !nombre.contains('\n')).then_some(nombre)
    }
}

/// Una dimensión por cada hijo con nombre de `node`, salvo los campos de `except`.
///
/// **El nombre lo da la gramática**: el del campo —`type`, `name`, `parameters`— o,
/// si el hijo no tiene campo, su tipo de nodo —`modifiers`—. No hay tabla que
/// mantener, y en otro lenguaje son otros solos. Dos hijos con el mismo nombre son
/// una dimensión con dos partes, porque la query los matchea a los dos.
///
/// **Con `(_)` y no con el tipo del hijo**: el tipo es parte de lo vigilado, y un
/// `List<Dto>` que pasa a `Dto` es la firma que cambió, no una dimensión que se fue.
fn children_dimensions<'t>(node: Node<'t>, except: &[&str]) -> Vec<GeneratedDimension<'t>> {
    let mut out: Vec<GeneratedDimension<'t>> = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            let field = cursor.field_name();
            if child.is_named() && !field.is_some_and(|f| except.contains(&f)) {
                let (name, query) = match field {
                    Some(f) => (f.to_string(), format!("({} {f}: (_) @target) @anchor", node.kind())),
                    None    => (child.kind().to_string(), format!("({} ({}) @target) @anchor", node.kind(), child.kind())),
                };
                match out.iter_mut().find(|d| d.name == name) {
                    Some(d) => d.targets.push(child),
                    None    => out.push(GeneratedDimension { name, query, targets: vec![child] }),
                }
            }
            if !cursor.goto_next_sibling() { break; }
        }
    }
    out
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
/// Señalás **el método** y salen tres dimensiones: `route` —el `@RequestMapping` de
/// la clase y la anotación de ruta del método—, `type` y `parameters`, que son las
/// de [`Interface`] con el mismo nombre y la misma query.
///
/// **La ruta compuesta es el caso que no tenía salida.** Sale de dos anotaciones en
/// nodos distintos, y el literal completo no aparece en ningún lado del archivo.
///
/// **El nombre del método no se vigila.** Renombrarlo no cambia el contrato del
/// endpoint; meterlo haría que un refactor interno disparara drift, que es lo que
/// vigilar la firma existe para dejar de hacer.
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

    fn dimensions<'t>(&self, ctx: &GenCtx<'_>, node: Node<'t>) -> Result<Vec<GeneratedDimension<'t>>> {
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

        // **La clase que lo contiene directamente**: la de un método de una clase
        // anidada es la anidada, que es la que su `@anchor` alcanza por `body:`.
        let class = node.parent()
            .filter(|b| b.kind() == "class_body")
            .and_then(|b| b.parent())
            .filter(|c| c.kind() == "class_declaration");
        let class_mapping = class.and_then(|c| class_annotation(c, ctx.source));

        let mut dims = vec![route_dimension(route, class_mapping)];
        dims.extend(children_dimensions(node, &[]).into_iter()
            .filter(|d| d.name == "type" || d.name == "parameters"));
        Ok(dims)
    }

    /// `GET /public-api/user/info/from-token`, compuesto de lo vigilado.
    ///
    /// La ruta de clase y el literal del método son las partes de `route`; el verbo
    /// sale del nombre de la anotación. Nada de esto se busca afuera de las
    /// dimensiones, que es lo que hace que no pueda mentir.
    fn alias(&self, source: &str, parts: &[(String, Ranges)]) -> Option<String> {
        let dim = |nombre: &str| parts.iter().find(|(n, _)| n == nombre).map(|(_, r)| r);
        let partes: Vec<&str> = dim("route")?.parts().iter()
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
        // que distingue es el nombre del método: donde falta el literal sobra el
        // nombre, y viceversa.
        let nombre = match (dim("type"), dim("parameters")) {
            (Some(t), Some(p)) => nombre_entre(source, t.end(), p.start()),
            _ => None,
        };
        match (literal_de(partes[verbo_i]), nombre) {
            (None, Some(m)) => Some(format!("{verbo} {ruta}  ·  {m}")),
            _               => Some(format!("{verbo} {ruta}")),
        }
    }
}

/// `route`: la anotación de ruta del método, y la de la clase si la había.
///
/// **Pide lo que había al capturar.** Con la anotación de la clase en la query,
/// mudarla al método deja `route` sin resolver: la dimensión dice qué forma tenía la
/// ruta que se aprobó, y el aviso es el que pide volver a generarla.
///
/// **Una anotación de ruta se reconoce por clase, no por nombre**, con `#match?`: el
/// verbo es contenido. **Y sin kind**: `annotation` y `marker_annotation` son la
/// misma anotación con y sin literal, y sacarle el literal es cambiar la ruta.
fn route_dimension<'t>(route: Node<'t>, class_mapping: Option<Node<'t>>) -> GeneratedDimension<'t> {
    let annotation = |c: &str, names: &[&str], indent: &str| format!(
        "(modifiers\n{indent}  (_\n{indent}    name: (identifier) @{c} (#match? @{c} \"^({})$\")) @target)",
        names.join("|"));
    let (query, targets) = match class_mapping {
        Some(m) => (format!(
            "(class_declaration\n  {}\n  body: (class_body\n    (method_declaration\n      {}) @anchor))",
            annotation("c", &[CLASS_MAPPING], "  "), annotation("m", MAPPINGS, "      ")),
            vec![m, route]),
        None => (format!("(method_declaration\n  {}) @anchor", annotation("m", MAPPINGS, "  ")),
            vec![route]),
    };
    GeneratedDimension { name: "route".into(), query, targets }
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
/// **No se lee de la query**: `name: (identifier)` aparece también en la clase, que
/// va más arriba del árbol y por lo tanto antes en el patrón. Entre el fin de `type`
/// y el comienzo de `parameters` no hay nada más que el nombre: es la forma que la
/// gramática le da al método, no una heurística sobre texto.
fn nombre_entre(source: &str, desde: usize, hasta: usize) -> Option<String> {
    let entre = source.get(desde..hasta)?.trim();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{compute, compute_as, generator_named, Computed};
    use tempfile::tempdir;

    const CTL: &str = "\
@RestController
@RequestMapping(\"/public-api/user\")
public class Service {

    @GetMapping(\"/permissions/from-token\")
    public List<PublicAuthorityDto> getPermissions(String token)
    {
        return svc.permissionsOf(token);
    }
}
";

    fn layer(src: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Service.java"), src).unwrap();
        d
    }

    fn as_mode(src: &str, mode: &str) -> Computed {
        let d = layer(src);
        let g = generator_named(mode).unwrap();
        compute_as(d.path(), "Service.java", &[((6, 5), (6, 5))], Some(g.as_ref())).unwrap()
    }

    fn names(c: &Computed) -> Vec<&str> {
        c.dimensions.keys().map(String::as_str).collect()
    }

    fn part(c: &Computed, src: &str, name: &str) -> String {
        c.parts.iter().find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no hay dimensión {name}")).1.text(src)
    }

    /// Las dimensiones de la firma son los hijos del método, nombrados como los
    /// nombra la gramática, **menos el cuerpo**.
    #[test]
    fn interface_declares_every_child_but_the_body() {
        let c = as_mode(CTL, "interface");
        assert_eq!(names(&c), ["modifiers", "name", "parameters", "type"]);
        assert_eq!(c.dimensions["type"].query, "(method_declaration type: (_) @target) @anchor");
        assert_eq!(c.dimensions["modifiers"].query, "(method_declaration (modifiers) @target) @anchor");
        assert_eq!(part(&c, CTL, "name"), "getPermissions");
        assert_eq!(part(&c, CTL, "type"), "List<PublicAuthorityDto>");
    }

    /// El capture es el del núcleo, con cualquier modo: la misma ubicación, el mismo
    /// archivo.
    #[test]
    fn a_generator_writes_the_core_anchor() {
        let d = layer(CTL);
        let (core, _, _) = compute(d.path(), "Service.java", &[((6, 5), (6, 5))], None).unwrap();
        for mode in ["interface", "spring-controller"] {
            let c = as_mode(CTL, mode);
            assert_eq!(c.capture.id(), core.id(), "{mode}:\n{:?}", c.capture.query);
            assert_eq!(c.capture.query.as_deref().unwrap().matches("@target").count(), 1);
        }
    }

    /// `route` junta las dos anotaciones; `type` y `parameters` son las de la firma.
    #[test]
    fn spring_declares_the_route_the_type_and_the_parameters() {
        let c = as_mode(CTL, "spring-controller");
        assert_eq!(names(&c), ["parameters", "route", "type"]);
        let firma = as_mode(CTL, "interface");
        assert_eq!(c.dimensions["type"], firma.dimensions["type"]);
        assert_eq!(c.dimensions["parameters"], firma.dimensions["parameters"]);
        assert_eq!(part(&c, CTL, "route"),
                   "@RequestMapping(\"/public-api/user\")\n@GetMapping(\"/permissions/from-token\")");
        assert!(c.dimensions["route"].query.starts_with("(class_declaration"), "{}", c.dimensions["route"].query);
    }

    /// Sin prefijo en la clase, `route` es la anotación del método y nada más.
    #[test]
    fn without_a_class_prefix_the_route_is_the_method_annotation() {
        let src = CTL.replace("@RequestMapping(\"/public-api/user\")\n", "\n");
        let c = as_mode(&src, "spring-controller");
        assert!(c.dimensions["route"].query.starts_with("(method_declaration"), "{}", c.dimensions["route"].query);
        assert_eq!(part(&c, &src, "route"), "@GetMapping(\"/permissions/from-token\")");
    }

    /// **`route` pide lo que había al capturar.** Mudar el prefijo de la clase al
    /// método deja el capture resuelto y la ruta sin resolver.
    #[test]
    fn moving_the_class_prefix_to_the_method_leaves_the_route_unresolved() {
        let c = as_mode(CTL, "spring-controller");
        let moved = CTL
            .replace("@RequestMapping(\"/public-api/user\")\npublic class", "public class")
            .replace("    @GetMapping", "    @RequestMapping(\"/public-api/user\")\n    @GetMapping");
        let language = crate::grammar::for_language("java").unwrap();
        let node = crate::query::find_fragment(language.clone(), &moved, c.capture.query.as_deref().unwrap())
            .unwrap().expect("el capture sigue resolviendo").ranges;
        let route = crate::query::dimension(language, &moved, &c.dimensions["route"].query, (node.start(), node.end()))
            .unwrap();
        assert!(route.is_none(), "la ruta no resuelve");
    }

    /// El alias sale de las dimensiones: el verbo y la ruta de `route`.
    #[test]
    fn the_spring_alias_comes_from_the_dimensions() {
        let c = as_mode(CTL, "spring-controller");
        assert_eq!(SpringController.alias(CTL, &c.parts).as_deref(),
                   Some("GET /public-api/user/permissions/from-token"));
    }

    /// Sin literal propio, el nombre se lee entre `type` y `parameters`.
    #[test]
    fn a_markerless_alias_carries_the_method_name() {
        let src = CTL.replace("@GetMapping(\"/permissions/from-token\")", "@GetMapping");
        let c = as_mode(&src, "spring-controller");
        assert_eq!(SpringController.alias(&src, &c.parts).as_deref(),
                   Some("GET /public-api/user  ·  getPermissions"));
    }

    /// Una firma se nombra por su método: el texto de la dimensión `name`.
    #[test]
    fn the_interface_alias_is_the_name() {
        let c = as_mode(CTL, "interface");
        assert_eq!(Interface.alias(CTL, &c.parts).as_deref(), Some("getPermissions"));
    }
}
