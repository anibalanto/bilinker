//! Las partes del contenido: cómo se compara cada dimensión, y cómo califican al
//! estado del endpoint.
//!
//! **El estado sigue siendo una palabra.** Lo que consumen el filtro por estado, el
//! código de salida y el agrupado de `status` es [`EndpointState`], y su vocabulario
//! es cerrado. Los nombres de dimensión son del generador, así que van al lado, como
//! calificación, y nunca adentro de la palabra.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use bilink_format::{AcceptedDimension, ByteRange, Capture, DeclaredDimension, Ranges};

use crate::check::CommitSource;
use crate::state::EndpointState;
use crate::{grammar, hash, query};

/// El estado de una dimensión, por nombre.
pub type DimensionStates = Vec<(String, EndpointState)>;

/// Qué tan grave es un estado del contenido, por el arreglo que pide: `ALTERED`
/// pide revisar y falla, `EXPANDED` pide revisar y no falla, `RESTYLED` sólo pide
/// aceptar.
fn severity(s: EndpointState) -> u8 {
    match s {
        EndpointState::Ok       => 0,
        EndpointState::Restyled => 1,
        EndpointState::Expanded => 2,
        _                       => 3,
    }
}

/// La palabra del endpoint: la de la dimensión más severa, y `OK` sin ninguna.
pub fn word(dims: &DimensionStates) -> EndpointState {
    dims.iter().map(|(_, s)| *s).max_by_key(|s| severity(*s)).unwrap_or(EndpointState::Ok)
}

/// Las dimensiones que califican a la palabra: las que no están `OK`.
///
/// **También las que no hacen fallar.** Aceptar aprueba todas las partes juntas, así
/// que quien revisa una cambiada tiene que saber que otra se reformateó.
pub fn qualifying(dims: DimensionStates) -> DimensionStates {
    dims.into_iter().filter(|(_, s)| !s.is_ok()).collect()
}

/// La palabra con sus dimensiones al lado: `ALTERED(body, route)`.
pub fn qualified(state: EndpointState, names: &[&str]) -> String {
    if names.is_empty() { state.to_string() } else { format!("{state}({})", names.join(", ")) }
}

/// Compara cada dimensión contra lo que se aprobó de ella, por nombre, en el orden
/// de los nombres.
///
/// `range` es el fragmento que resolvió el capture, y cada parte se busca adentro:
/// su query no dice de qué método se trata, porque eso ya lo dijo el capture.
///
/// **Una parte de un lado solo es `ALTERED`**, declarada y no aprobada o al revés:
/// cambió qué se vigila, y nadie aprobó ese cambio. Y una que no se encuentra
/// también: lo aprobado ya no está donde estaba.
pub(crate) fn compare(
    layer: &Path,
    cap: &Capture,
    declared: &BTreeMap<String, DeclaredDimension>,
    accepted: &BTreeMap<String, AcceptedDimension>,
    range: &Ranges,
    commit: &mut CommitSource<'_>,
) -> Result<DimensionStates> {
    let source = std::fs::read_to_string(layer.join(&cap.file))?;
    let lang = grammar::language_for_file(&cap.file);
    let language = grammar::for_language(lang)?;
    let within = ByteRange { start: range.start(), end: range.end() };

    // El archivo en el commit aceptado, para EXPANDED: se busca una vez, y sólo si
    // alguna parte difiere.
    let mut then: Option<Option<(String, ByteRange)>> = None;

    let names: BTreeSet<&String> = declared.keys().chain(accepted.keys()).collect();
    let mut out = Vec::new();
    for name in names {
        let (Some(d), Some(a)) = (declared.get(name), accepted.get(name)) else {
            out.push((name.clone(), EndpointState::Altered));
            continue;
        };
        let Some(f) = query::find_fragment_within(language.clone(), &source, &d.query, &within)? else {
            out.push((name.clone(), EndpointState::Altered));
            continue;
        };
        let text = f.ranges.text(&source);
        if hash::sha256(text.as_bytes()) == a.hash {
            out.push((name.clone(), EndpointState::Ok));
            continue;
        }

        let then = then.get_or_insert_with(|| {
            let old = crate::capture::source_at(layer, cap, &(commit.derive)()?)?;
            let hull = match &cap.query {
                None => ByteRange { start: 0, end: old.len() },
                Some(q) => {
                    let r = query::find_fragment(language.clone(), &old, q).ok()??.ranges;
                    ByteRange { start: r.start(), end: r.end() }
                }
            };
            Some((old, hull))
        });
        let approved = then.as_ref().and_then(|(old, hull)| {
            let f = query::find_fragment_within(language.clone(), old, &d.query, hull).ok()??;
            let t = f.ranges.text(old);
            (hash::sha256(t.as_bytes()) == a.hash).then_some(t)
        });
        if let Some(t) = approved {
            if !t.is_empty() && text.len() > t.len() && text.contains(&t) {
                out.push((name.clone(), EndpointState::Expanded));
                continue;
            }
        }

        // Sólo formato, con la misma regla que el fragmento entero: la gramática
        // decide antes que `accepted` si el AST discrimina contenido.
        if grammar::ast_discriminates_content(lang)
            && a.hash_ast.as_deref() == Some(hash::sha256(f.sexp.as_bytes()).as_str())
        {
            out.push((name.clone(), EndpointState::Restyled));
            continue;
        }
        out.push((name.clone(), EndpointState::Altered));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use EndpointState::*;

    const SRC: &str = "fn a(x: u8) -> u8 { x + 1 }\n\nfn b(y: u16) -> u16 { y }\n";
    const FN_A: &str = r#"(function_item name: (identifier) @n0 (#eq? @n0 "a")) @target"#;
    const PARAMS: &str = "(function_item parameters: (parameters) @target)";
    const BODY: &str = "(function_item body: (block) @target)";

    fn dims(pairs: &[(&str, EndpointState)]) -> DimensionStates {
        pairs.iter().map(|(n, s)| (n.to_string(), *s)).collect()
    }

    #[test]
    fn without_dimensions_the_word_is_ok() {
        assert_eq!(word(&dims(&[])), Ok);
    }

    /// Una reformateada y otra cambiada dan la cambiada.
    #[test]
    fn the_word_is_the_most_severe() {
        assert_eq!(word(&dims(&[("body", Restyled), ("route", Altered)])), Altered);
        assert_eq!(word(&dims(&[("body", Restyled), ("route", Expanded)])), Expanded);
        assert_eq!(word(&dims(&[("body", Ok), ("route", Restyled)])), Restyled);
    }

    /// La calificación lista todas las que no están `OK`, también las que no fallan.
    #[test]
    fn every_part_that_is_not_ok_qualifies() {
        let q = qualifying(dims(&[("body", Restyled), ("params", Ok), ("route", Altered)]));
        assert_eq!(q, dims(&[("body", Restyled), ("route", Altered)]));
    }

    #[test]
    fn the_qualification_goes_beside_the_word() {
        assert_eq!(qualified(Altered, &["body", "route"]), "ALTERED(body, route)");
        assert_eq!(qualified(Ok, &[]), "OK");
    }

    /// La palabra sigue siendo del vocabulario cerrado: califica, no se vuelve otra.
    #[test]
    fn the_qualified_word_still_parses_as_its_word() {
        let q = qualified(Altered, &["body"]);
        let w = q.split('(').next().unwrap();
        assert_eq!(w.parse::<EndpointState>().unwrap(), Altered);
    }

    // ─── la comparación ───────────────────────────────────────────────────────

    /// La capa con `SRC` reescrito, y el fragmento de `a` resuelto hoy.
    fn layer(source: &str) -> (tempfile::TempDir, Capture, Ranges) {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join("lib.rs"), source).unwrap();
        let cap = Capture { file: "lib.rs".into(), query: Some(FN_A.into()) };
        let range = resolve(source, FN_A);
        (d, cap, range)
    }

    fn resolve(source: &str, q: &str) -> Ranges {
        let language = grammar::for_language("rust").unwrap();
        query::find_fragment(language, source, q).unwrap().unwrap().ranges
    }

    fn declared(names: &[(&str, &str)]) -> BTreeMap<String, DeclaredDimension> {
        names.iter().map(|(n, q)| (n.to_string(), DeclaredDimension { query: q.to_string() })).collect()
    }

    /// Lo que se aprobó de cada parte, sobre `source`.
    fn approved(source: &str, names: &[(&str, &str)]) -> BTreeMap<String, AcceptedDimension> {
        let language = grammar::for_language("rust").unwrap();
        let within = resolve(source, FN_A);
        let within = ByteRange { start: within.start(), end: within.end() };
        names.iter().map(|(n, q)| {
            let f = query::find_fragment_within(language.clone(), source, q, &within).unwrap().unwrap();
            (n.to_string(), AcceptedDimension {
                hash: hash::sha256(f.ranges.text(source).as_bytes()),
                hash_ast: Some(hash::sha256(f.sexp.as_bytes())),
            })
        }).collect()
    }

    fn run(today: &str, decl: &[(&str, &str)], acc: &BTreeMap<String, AcceptedDimension>) -> DimensionStates {
        let (d, cap, range) = layer(today);
        let mut derive = || None;
        let mut src = CommitSource { derive: &mut derive };
        compare(d.path(), &cap, &declared(decl), acc, &range, &mut src).unwrap()
    }

    const BOTH: &[(&str, &str)] = &[("body", BODY), ("parameters", PARAMS)];

    #[test]
    fn an_untouched_fragment_is_ok_in_every_part() {
        let acc = approved(SRC, BOTH);
        assert_eq!(run(SRC, BOTH, &acc), dims(&[("body", Ok), ("parameters", Ok)]));
    }

    /// El caso de la spec: el cuerpo sólo reformateado, y un parámetro cambiado.
    #[test]
    fn a_restyled_part_beside_an_altered_one_is_reported_with_its_own_state() {
        let acc = approved(SRC, BOTH);
        let today = SRC.replace("fn a(x: u8) -> u8 { x + 1 }", "fn a(x: u32) -> u8 {\n    x + 1\n}");
        let got = run(&today, BOTH, &acc);
        assert_eq!(got, dims(&[("body", Restyled), ("parameters", Altered)]));
        assert_eq!(word(&got), Altered);
    }

    /// Lo que cambia afuera de toda parte declarada no avisa: nadie pidió vigilarlo.
    #[test]
    fn a_change_outside_every_part_says_nothing() {
        let only = &[("parameters", PARAMS)];
        let acc = approved(SRC, only);
        let today = SRC.replace("{ x + 1 }", "{ x * 2 }");
        assert_eq!(run(&today, only, &acc), dims(&[("parameters", Ok)]));
    }

    /// Una parte se busca adentro del nodo: la de `b` no es la de `a`.
    #[test]
    fn a_part_is_looked_for_inside_the_captured_node() {
        let only = &[("parameters", PARAMS)];
        let acc = approved(SRC, only);
        let today = SRC.replace("fn b(y: u16)", "fn b(y: u64)");
        assert_eq!(run(&today, only, &acc), dims(&[("parameters", Ok)]));
    }

    /// Declarada y no aprobada, o aprobada y no declarada: cambió qué se vigila.
    #[test]
    fn a_part_on_one_side_only_is_altered() {
        let acc = approved(SRC, &[("parameters", PARAMS)]);
        assert_eq!(run(SRC, BOTH, &acc), dims(&[("body", Altered), ("parameters", Ok)]));
        let acc = approved(SRC, BOTH);
        assert_eq!(run(SRC, &[("parameters", PARAMS)], &acc),
                   dims(&[("body", Altered), ("parameters", Ok)]));
    }

    #[test]
    fn a_part_that_is_not_found_is_altered() {
        let only = &[("return", "(function_item return_type: (_) @target)")];
        let acc = approved(SRC, only);
        let today = SRC.replace("fn a(x: u8) -> u8", "fn a(x: u8)");
        assert_eq!(run(&today, only, &acc), dims(&[("return", Altered)]));
    }
}
