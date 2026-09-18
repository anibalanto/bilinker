//! Un capture escrito cuando la query componía el fragmento, en la forma de hoy.
//!
//! Esa query lleva varios `@target` —la ruta de la clase, la del método, el retorno
//! y los parámetros de un endpoint de Spring— y su `hash` aceptado es el de la
//! concatenación. Hoy el capture es el nodo y las partes son dimensiones del
//! endpoint, así que partirlo es volver a correr el generador sobre el nodo y
//! repartir la aceptación entre sus partes.
//!
//! **No acepta nada.** Sólo parte una aceptación cuando lo que hay en el archivo es
//! lo aprobado y las partes de hoy reproducen el fragmento de entonces byte a byte:
//! ahí el hash de cada parte es un pedazo de lo que alguien ya aprobó.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

use bilink_format::{AcceptedDimension, Capture, DeclaredDimension};

use crate::capture::{compute_at, CaptureGenerator};
use crate::{grammar, hash, query};

/// Un capture compuesto, partido: el capture del nodo, lo que se vigila de él, y
/// la aceptación repartida.
#[derive(Debug)]
pub struct Split {
    pub capture: Capture,
    pub dimensions: BTreeMap<String, DeclaredDimension>,
    pub accepted: BTreeMap<String, AcceptedDimension>,
    /// El del nodo entero, como lo escribe `accept` con dimensiones.
    pub hash: String,
    pub hash_ast: Option<String>,
}

/// Si la query compone: más de un `@target`.
pub fn is_composed(query_str: &str) -> bool {
    query_str.matches("@target").count() > 1
}

/// Parte `cap`, cuyo fragmento aprobado hashea a `accepted_hash`.
///
/// Falla, sin escribir nada, si la query vieja no resuelve, si lo que resuelve no
/// es lo aprobado, o si las dimensiones del generador no reproducen el fragmento.
pub fn split(
    layer: &Path,
    cap: &Capture,
    generator: &dyn CaptureGenerator,
    accepted_hash: &str,
) -> Result<Split> {
    let Some(old_query) = &cap.query else { bail!("el capture es un archivo entero: no compone") };
    let source = std::fs::read_to_string(layer.join(&cap.file))
        .with_context(|| format!("leyendo {}", cap.file))?;
    let lang = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;

    let Some((old, node)) = query::declaration(language.clone(), &source, old_query)? else {
        bail!("la query no resuelve en {}", cap.file);
    };
    let old_text = old.text(&source);
    if hash::sha256(old_text.as_bytes()) != accepted_hash {
        bail!("el fragmento no es el aceptado");
    }

    let c = compute_at(layer, &cap.file, (node.start, node.end), Some(generator))?;
    if c.ranges.text(&source) != old_text {
        bail!("las dimensiones de `{}` no reproducen el fragmento", generator.name());
    }

    let whole = query::find_fragment(language.clone(), &source,
        c.capture.query.as_deref().context("el capture del nodo lleva query")?)?
        .context("el capture del nodo no resuelve")?;
    let anchor = (whole.ranges.start(), whole.ranges.end());
    let discriminates = grammar::ast_discriminates_content(lang);

    let accepted = c.dimensions.iter().map(|(name, d)| {
        let part = query::dimension(language.clone(), &source, &d.query, anchor)?
            .with_context(|| format!("la dimensión {name} no resuelve"))?;
        Ok((name.clone(), AcceptedDimension {
            query: d.query.clone(),
            hash: hash::sha256(part.ranges.text(&source).as_bytes()),
            hash_ast: discriminates.then(|| hash::sha256(part.sexp.as_bytes())),
        }))
    }).collect::<Result<_>>()?;

    Ok(Split {
        hash: hash::sha256(whole.ranges.text(&source).as_bytes()),
        hash_ast: discriminates.then(|| hash::sha256(whole.sexp.as_bytes())),
        capture: c.capture,
        dimensions: c.dimensions,
        accepted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::generator_named;
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

    /// La query que escribía `--as spring-controller` cuando componía el fragmento.
    const OLD: &str = r#"(class_declaration
  (modifiers
    (_
          name: (identifier) @n0 (#match? @n0 "^(RequestMapping)$")) @target)
  body: (class_body
    (method_declaration
      (modifiers
        (_
          name: (identifier) @n1 (#match? @n1 "^(GetMapping|PostMapping|PutMapping|DeleteMapping|PatchMapping|RequestMapping)$")) @target)
      type: (_) @target
      name: (identifier) @n2 (#eq? @n2 "getPermissions")
      parameters: (_) @target)))"#;

    const FRAGMENT: &str = "@RequestMapping(\"/public-api/user\")\n@GetMapping(\"/permissions/from-token\")\nList<PublicAuthorityDto>\n(String token)";

    fn layer(src: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("Service.java"), src).unwrap();
        d
    }

    fn old() -> Capture {
        Capture { file: "Service.java".into(), query: Some(OLD.into()) }
    }

    fn spring() -> Box<dyn CaptureGenerator> { generator_named("spring-controller").unwrap() }

    #[test]
    fn a_query_with_several_targets_composes() {
        assert!(is_composed(OLD));
        assert!(!is_composed("(method_declaration) @target"));
    }

    /// El capture pasa a ser el del núcleo: el mismo que escribe hoy un capture del
    /// método, con generador o sin él.
    #[test]
    fn the_split_anchors_the_method() {
        let d = layer(CTL);
        let s = split(d.path(), &old(), spring().as_ref(), &hash::sha256(FRAGMENT.as_bytes())).unwrap();
        let hoy = crate::capture::compute_as(d.path(), "Service.java", &[((5, 5), (5, 5))], None).unwrap();
        assert_eq!(s.capture, hoy.capture);
        assert_eq!(s.dimensions.keys().collect::<Vec<_>>(), ["parameters", "route", "type"]);
    }

    /// Cada dimensión aprobada es un pedazo del fragmento aprobado, y el `hash` es
    /// el del nodo entero.
    #[test]
    fn the_acceptance_is_split_among_the_parts() {
        let d = layer(CTL);
        let s = split(d.path(), &old(), spring().as_ref(), &hash::sha256(FRAGMENT.as_bytes())).unwrap();
        assert_eq!(s.accepted["route"].hash,
                   hash::sha256(b"@RequestMapping(\"/public-api/user\")\n@GetMapping(\"/permissions/from-token\")"));
        assert_eq!(s.accepted["type"].hash, hash::sha256(b"List<PublicAuthorityDto>"));
        assert_eq!(s.accepted["parameters"].hash, hash::sha256(b"(String token)"));
        assert!(s.accepted.values().all(|a| a.hash_ast.is_some()));
        let metodo = &CTL[CTL.find("@GetMapping").unwrap()..CTL.rfind("    }").unwrap() + 5];
        assert_eq!(s.hash, hash::sha256(metodo.as_bytes()));
    }

    /// Si lo que hay en el archivo no es lo aprobado, no hay qué partir.
    #[test]
    fn a_fragment_that_is_not_the_accepted_one_is_refused() {
        let d = layer(CTL);
        let e = split(d.path(), &old(), spring().as_ref(), &hash::sha256(b"otra cosa")).unwrap_err();
        assert!(e.to_string().contains("no es el aceptado"), "{e}");
    }

    /// Un generador que hoy vigila otra cosa que el de entonces no puede heredar la
    /// aprobación: `interface` no vigila la ruta de la clase.
    #[test]
    fn parts_that_do_not_reproduce_the_fragment_are_refused() {
        let d = layer(CTL);
        let interface = generator_named("interface").unwrap();
        let e = split(d.path(), &old(), interface.as_ref(), &hash::sha256(FRAGMENT.as_bytes())).unwrap_err();
        assert!(e.to_string().contains("no reproducen el fragmento"), "{e}");
    }

    #[test]
    fn a_query_that_does_not_resolve_is_refused() {
        let d = layer(&CTL.replace("getPermissions", "otro"));
        let e = split(d.path(), &old(), spring().as_ref(), &hash::sha256(FRAGMENT.as_bytes())).unwrap_err();
        assert!(e.to_string().contains("no resuelve"), "{e}");
    }
}
