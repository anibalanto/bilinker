//! El vecindario de una firma: los tipos que menciona, un salto.
//!
//! **Bilinker no sale a buscarlos.** Resolver un tipo hasta su declaración es
//! trabajo de language server, y la frontera de este subsistema es git y
//! tree-sitter. Lo que hay acá es el **puerto** por el que entran las ubicaciones y
//! el plegado que las convierte en los dos hashes que se guardan.
//!
//! El puerto no nombra a nadie. **No es para evitar un ciclo** —desde que el daemon
//! salió de lattice no hay ninguno— sino para que bilinker no quede atado a *ese*
//! daemon: mañana puede ser SCIP, un índice propio, o un language server hablado
//! directo.
//!
//! Ver `concepts/accept.md` § "El cierre de firma".

use std::path::Path;

use anyhow::Result;
use bilink_format::Ranges;

use crate::{grammar, hash, query};

/// Dónde vive un vecino.
///
/// `file` y `symbol` son **su identidad**, y es por ahí que se ordena el fold. El
/// rango es crudo: bilinker le aplica el mismo recorte de bordes que a un fragmento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Path relativo a la raíz de la capa.
    pub file:   String,
    /// El nombre del símbolo declarado.
    pub symbol: String,
    pub start:  usize,
    pub end:    usize,
}

/// Quién resuelve el vecindario.
///
/// **`None` es *no pude mirar*, y no *no hay vecinos*.** De esa distinción sale el
/// estado `CONTRACT_UNVERIFIED`: sin ella, un daemon apagado se leería como un
/// contrato que no menciona ningún tipo, que es la confusión más cara del
/// ecosistema con otro disfraz.
/// **Y recibe posiciones, no el rango del fragmento.** Dónde hay un tipo que
/// preguntar es gramática, y la gramática es de bilinker; qué declara ese tipo es del
/// proveedor. Pasarle el rango lo obligaba a inventar dónde preguntar adentro, y lo
/// que inventaba era *"el byte donde arranca"* — que sobre un capture de nodo entero
/// cae en `pub` y no declara nada.
pub trait Neighbours {
    fn of(&self, layer: &Path, file: &str, at: &[usize]) -> Result<Option<Vec<Location>>>;
}

/// Quién resuelve el vecindario en esta corrida, si hay alguien.
pub type Provider<'a> = Option<&'a dyn Neighbours>;

/// Qué se puede saber del vecindario de un fragmento **antes de preguntarle a nadie**.
///
/// Tres valores y no un booleano, que es lo que separa *"no hay"* de *"no pude"*. Es
/// la misma figura que la readiness de `lspd`, y por el mismo motivo: el tercer valor
/// es el que hace honestos a los otros dos. Ver `concepts/accept.md` § "Cuándo se
/// adquiere el vecindario".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// No hay vecindario: prosa, YAML, un lenguaje sin tipos. La ausencia de `n` es
    /// la correcta y no hay nada que pedir.
    None,
    /// Hay, y se sabe dónde preguntar: los offsets de los campos de la firma que
    /// llevan tipos.
    At(Vec<usize>),
    /// Hay, y **no se alcanza desde este fragmento**. El archivo entero, un `enum`, un
    /// `impl`: tienen firmas adentro y ninguna es la suya.
    ///
    /// Lleva con qué explicarlo, porque quien lea el error no tiene cómo deducirlo.
    Unreachable { what: String },
}

/// Dónde preguntar por el vecindario de este fragmento, si en algún lado.
///
/// Se contesta con la gramática y sin proveedor, y de eso dependen dos cosas: que el
/// aviso de `accept` aparezca sólo donde corresponde —sobre prosa sería ruido, porque
/// ahí la ausencia de `n` ya era la correcta— y que una ausencia sin marca tenga un
/// solo significado.
///
/// Se camina hacia **arriba** desde cada `@target` hasta la firma que lo contiene: un
/// capture de contrato señala el tipo de retorno y los parámetros, y ninguno de esos
/// nodos *es* la firma — todos son hijos suyos. Y desde la firma se baja a los campos
/// que llevan tipos, que es lo que hace que un capture de contrato y uno de la función
/// entera pregunten en las **mismas** posiciones.
pub fn reach(layer: &Path, file: &str, ranges: &Ranges) -> Reach {
    use tree_sitter::Parser;

    let lang = grammar::language_for_file(file);
    let kinds = grammar::signature_kinds(lang);
    // Un lenguaje sin firmas no tiene vecindario que alcanzar, y eso no es una
    // limitación: es que ahí la pregunta no existe.
    if kinds.is_empty() { return Reach::None; }

    let unreachable = |what: &str| Reach::Unreachable { what: what.to_string() };

    let Ok(language) = grammar::for_language(lang) else { return Reach::None };
    let Ok(source) = std::fs::read_to_string(layer.join(file)) else { return Reach::None };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() { return Reach::None }
    let Some(tree) = parser.parse(&source, None) else { return Reach::None };

    let fields = grammar::signature_fields(lang);
    let mut at: Vec<usize> = Vec::new();

    for r in ranges.parts() {
        // La firma que **contiene** a esta parte, si hay alguna.
        let mut node = tree.root_node().descendant_for_byte_range(r.start, r.end);
        let mut firma = None;
        while let Some(n) = node {
            if kinds.contains(&n.kind()) { firma = Some(n); break }
            node = n.parent();
        }
        let Some(firma) = firma else {
            // No es una firma ni está adentro de una. Lo que decide entre *"no hay"* y
            // *"no pude"* es si **contiene** firmas que quedan sin cubrir: un DTO no
            // tiene ninguna adentro y su ausencia es completa; un archivo entero tiene
            // muchas y ninguna es la suya.
            let Some(n) = tree.root_node().descendant_for_byte_range(r.start, r.end)
            else { continue };
            if !contains_signature(n, kinds) { continue }
            return unreachable(&if r.start == 0 && r.end >= source.len() {
                "es el archivo entero".to_string()
            } else {
                format!("es un `{}` y las firmas que tiene adentro no son la suya", n.kind())
            });
        };
        for f in fields {
            if let Some(child) = firma.child_by_field_name(f) {
                at.push(child.start_byte());
            }
        }
    }

    at.sort_unstable();
    at.dedup();
    // Una firma sin ningún campo de tipo —`fn f() {}`— no tiene a quién preguntarle, y
    // eso **sí** es un vecindario vacío legítimo.
    if at.is_empty() { Reach::None } else { Reach::At(at) }
}

// **El tipo que calcula el fold *es* el que se guarda.** Antes había un `Folded`
// propio que se desarmaba en dos campos al serializar y se volvía a armar al
// comparar; con `n1` plegado, `Neighbourhood` sirve para las dos cosas.
pub use bilink_format::Neighbourhood;

/// Un solo orden, y dos folds sobre ese orden.
///
/// **La clave de orden es identidad, nunca contenido.** Ordenando por el texto, un
/// reformateo le cambiaría el puesto a un vecino, la lista se reordenaría, y
/// `n1.hash_ast` se movería sin que ningún AST cambiara — un falso *"cambió de
/// verdad"* producido por el orden. Ordenando por identidad nadie se mueve de puesto
/// salvo que un vecino entre, salga o se renombre, y esas tres cosas **son** cambios
/// de contrato.
///
/// Tampoco puede ordenar el rango: lleva offsets, que se corren con cualquier
/// edición más arriba del archivo.
pub fn fold(layer: &Path, locs: &[Location]) -> Result<Neighbourhood> {
    let mut locs: Vec<&Location> = locs.iter().collect();
    locs.sort_by(|a, b| (&a.file, &a.symbol).cmp(&(&b.file, &b.symbol)));
    locs.dedup_by(|a, b| a.file == b.file && a.symbol == b.symbol);

    let mut texts = String::new();
    let mut sexps = String::new();
    let mut every_one_has_a_grammar = true;

    for loc in &locs {
        let source = std::fs::read_to_string(layer.join(&loc.file)).unwrap_or_default();
        // El mismo recorte que aplica la resolución de un fragmento: un vecino sin
        // recortar mueve su hash cuando le agregan algo abajo.
        let (s, e) = query::trim_edges(&source, loc.start, loc.end);
        let text = source.get(s..e).unwrap_or_default();
        texts.push_str(&hash::sha256(text.as_bytes()));
        texts.push('\0');

        let lang = grammar::language_for_file(&loc.file);
        if grammar::ast_discriminates_content(lang) {
            if let Some(sexp) = sexp_of(lang, &source, s, e) {
                sexps.push_str(&hash::sha256(sexp.as_bytes()));
                sexps.push('\0');
                continue;
            }
        }
        every_one_has_a_grammar = false;
    }

    // **`link` vacío por ahora.** Acuñar un capture por vecino y escribirlo acá es
    // la task `3u`: es `accept` quien tiene el proveedor y quien acuña. Hasta
    // entonces el fold es el de antes, con el campo declarado y sin poblar.
    Ok(Neighbourhood {
        link: Default::default(),
        hash:     hash::sha256(texts.as_bytes()),
        hash_ast: every_one_has_a_grammar.then(|| hash::sha256(sexps.as_bytes())),
    })
}

/// La huella del nodo más chico que cubre el rango del vecino.
fn sexp_of(lang: &str, source: &str, start: usize, end: usize) -> Option<String> {
    use tree_sitter::Parser;
    let language = grammar::for_language(lang).ok()?;
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(source, None)?;
    let node = tree.root_node().descendant_for_byte_range(start, end)?;
    Some(query::shape_and_tokens(node, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn loc(file: &str, symbol: &str, start: usize, end: usize) -> Location {
        Location { file: file.into(), symbol: symbol.into(), start, end }
    }

    fn layer_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        for (name, body) in files { fs::write(d.path().join(name), body).unwrap(); }
        d
    }

    /// El orden del fold no depende de en qué orden llegaron los vecinos.
    #[test]
    fn the_order_of_arrival_does_not_change_the_fold() {
        let d = layer_with(&[("a.rs", "struct A { x: u8 }\n"), ("b.rs", "struct B { y: u8 }\n")]);
        let a = loc("a.rs", "A", 0, 18);
        let b = loc("b.rs", "B", 0, 18);
        assert_eq!(fold(d.path(), &[a.clone(), b.clone()]).unwrap(),
                   fold(d.path(), &[b, a]).unwrap());
    }

    /// Un vecino repetido es un vecino: `Persona f(Persona a, Persona b)` menciona un
    /// tipo, no tres.
    #[test]
    fn the_same_neighbour_twice_is_one() {
        let d = layer_with(&[("a.rs", "struct A { x: u8 }\n")]);
        let a = loc("a.rs", "A", 0, 18);
        assert_eq!(fold(d.path(), &[a.clone()]).unwrap(),
                   fold(d.path(), &[a.clone(), a]).unwrap());
    }

    /// Que un vecino cambie mueve los dos hashes.
    #[test]
    fn a_changed_neighbour_moves_both() {
        let d1 = layer_with(&[("a.rs", "struct A { x: u8 }\n")]);
        let d2 = layer_with(&[("a.rs", "struct A { x: u8, y: u8 }\n")]);
        let f1 = fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap();
        let f2 = fold(d2.path(), &[loc("a.rs", "A", 0, 25)]).unwrap();
        assert_ne!(f1.hash, f2.hash);
        assert_ne!(f1.hash_ast, f2.hash_ast);
    }

    /// Reformatearlo mueve el texto y no el AST — el cuadrante "el vecindario se
    /// reformateó".
    #[test]
    fn a_reformatted_neighbour_moves_only_the_text() {
        let d1 = layer_with(&[("a.rs", "struct A { x: u8 }\n")]);
        // Sin coma final: una coma es un token, y agregarla es contenido y no formato.
        let d2 = layer_with(&[("a.rs", "struct A {\n    x: u8\n}\n")]);
        let f1 = fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap();
        let f2 = fold(d2.path(), &[loc("a.rs", "A", 0, 22)]).unwrap();
        assert_ne!(f1.hash, f2.hash);
        assert_eq!(f1.hash_ast, f2.hash_ast);
    }

    /// Un vecino sin gramática deja `hash_ast` **ausente para todo el fold**, no
    /// afuera: un cambio real en ése movería el texto y no el AST, y eso se leería
    /// como "sólo formateo" cuando no lo fue.
    #[test]
    fn one_neighbour_without_a_grammar_removes_the_ast_hash_entirely() {
        let d = layer_with(&[("a.rs", "struct A { x: u8 }\n"), ("nota.txt", "hola\n")]);
        let con = fold(d.path(), &[loc("a.rs", "A", 0, 18)]).unwrap();
        assert!(con.hash_ast.is_some());

        let sin = fold(d.path(), &[loc("a.rs", "A", 0, 18), loc("nota.txt", "nota", 0, 5)]).unwrap();
        assert!(sin.hash_ast.is_none(), "todo-o-nada");
    }

    /// El recorte de bordes vale igual acá: agregarle algo abajo al archivo no le
    /// mueve el hash a un vecino que nadie tocó.
    #[test]
    fn the_edges_are_trimmed_like_any_fragment() {
        let d1 = layer_with(&[("a.rs", "struct A { x: u8 }")]);
        let d2 = layer_with(&[("a.rs", "struct A { x: u8 }\n\n\n")]);
        assert_eq!(fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap().hash,
                   fold(d2.path(), &[loc("a.rs", "A", 0, 21)]).unwrap().hash);
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use tempfile::tempdir;

    /// Un proveedor de mentira: contesta lo que se le dijo, o que no pudo mirar.
    struct Fake {
        locs:  Option<Vec<Location>>,
        asked: Cell<usize>,
    }
    impl Neighbours for Fake {
        fn of(&self, _l: &Path, _f: &str, _at: &[usize]) -> Result<Option<Vec<Location>>> {
            self.asked.set(self.asked.get() + 1);
            Ok(self.locs.clone())
        }
    }

    fn repo() -> tempfile::TempDir {
        let d = tempdir().unwrap();
        fs::write(d.path().join("Svc.rs"),
            "pub struct Dto { pub x: u8 }\n\npub fn get() -> Dto { Dto { x: 1 } }\n").unwrap();
        for args in [vec!["init","-q"], vec!["config","user.email","t@t"],
                     vec!["config","user.name","t"], vec!["add","-A"], vec!["commit","-qm","i"]] {
            std::process::Command::new("git").current_dir(d.path()).args(&args).output().unwrap();
        }
        d
    }

    fn dto() -> Location {
        Location { file: "Svc.rs".into(), symbol: "Dto".into(), start: 0, end: 28 }
    }

    /// **`None` es "no pude mirar" y no "no hay vecinos".** Los dos casos tienen que
    /// producir cosas distintas, o un daemon apagado se leería como un contrato que
    /// no menciona ningún tipo.
    #[test]
    fn not_being_able_to_look_is_not_an_empty_neighbourhood() {
        let d = repo();
        let vacio = fold(d.path(), &[]).unwrap();
        let con   = fold(d.path(), &[dto()]).unwrap();
        assert_ne!(vacio.hash, con.hash);

        let mudo = Fake { locs: None, asked: Cell::new(0) };
        assert!(mudo.of(d.path(), "Svc.rs", &[0]).unwrap().is_none(),
                "sin proveedor no hay vecindario, y eso no es un vecindario vacío");
    }

    /// Se le pregunta una vez por endpoint, no una por vecino.
    #[test]
    fn the_provider_is_asked_once() {
        let d = repo();
        let p = Fake { locs: Some(vec![dto(), dto()]), asked: Cell::new(0) };
        let locs = p.of(d.path(), "Svc.rs", &[0]).unwrap().unwrap();
        assert_eq!(p.asked.get(), 1);
        // Y dos veces el mismo vecino es un vecino.
        assert_eq!(fold(d.path(), &locs).unwrap(), fold(d.path(), &[dto()]).unwrap());
    }
}

/// Si este nodo tiene alguna firma adentro.
///
/// **Es lo que separa "no hay vecindario" de "no pude recorrer hacia el próximo
/// nivel".** Un DTO no tiene ninguna y su ausencia es completa; un archivo entero
/// tiene muchas, y ninguna es la suya.
fn contains_signature(node: tree_sitter::Node<'_>, kinds: &[&str]) -> bool {
    let mut cur = node.walk();
    let mut pila = vec![node];
    while let Some(n) = pila.pop() {
        if n != node && kinds.contains(&n.kind()) { return true }
        pila.extend(n.children(&mut cur));
    }
    false
}

#[cfg(test)]
mod reach_tests {
    use super::*;
    use bilink_format::Ranges;
    use tempfile::tempdir;

    const SRC: &str = "pub struct Dto { pub x: u8 }\n\npub enum E { A }\n\npub fn get(d: Dto) -> Dto { todo!() }\n";

    fn en(name: &str, body: &str) -> tempfile::TempDir {
        let d = tempdir().unwrap();
        std::fs::write(d.path().join(name), body).unwrap();
        d
    }

    fn de(d: &tempfile::TempDir, name: &str, needle: &str, len: usize) -> Reach {
        let src = std::fs::read_to_string(d.path().join(name)).unwrap();
        let at = src.find(needle).unwrap();
        reach(d.path(), name, &Ranges::one(at, at + len))
    }

    /// Un lenguaje sin firmas no tiene vecindario que alcanzar, y eso no es una
    /// limitación: ahí la pregunta no existe.
    #[test]
    fn prose_has_no_neighbourhood() {
        let d = en("spec.md", "# Título\n\nUn párrafo.\n");
        assert_eq!(de(&d, "spec.md", "Título", 6), Reach::None);
    }

    /// **Un DTO no tiene firma adentro, así que su ausencia es completa.** Es lo que
    /// separa "no hay" de "no pude": no hay nada del próximo nivel que quede sin
    /// cubrir.
    #[test]
    fn a_dto_has_none_and_that_is_the_whole_truth() {
        let d = en("Svc.rs", SRC);
        assert_eq!(de(&d, "Svc.rs", "pub struct Dto", 28), Reach::None);
    }

    /// Un `enum` tampoco: sus variantes no son callables.
    #[test]
    fn an_enum_has_none() {
        let d = en("Svc.rs", SRC);
        assert_eq!(de(&d, "Svc.rs", "pub enum E", 16), Reach::None);
    }

    /// **La firma se alcanza, y las posiciones son las de sus tipos** — no la del
    /// `pub` donde el fragmento arranca, que no declara nada.
    #[test]
    fn a_signature_is_reached_at_its_types_and_not_at_its_start() {
        let d = en("Svc.rs", SRC);
        let arranca = SRC.find("pub fn get").unwrap();
        let firma = "pub fn get(d: Dto) -> Dto { todo!() }";
        let Reach::At(at) = de(&d, "Svc.rs", firma, firma.len()) else {
            panic!("una firma se alcanza");
        };
        assert!(!at.contains(&arranca), "preguntar en `pub` es el defecto que esto arregla");
        assert_eq!(at.len(), 2, "el retorno y los parámetros: {at:?}");
        // En Rust el campo `return_type` es **el tipo**, no la flecha: la posición cae
        // sobre `Dto` y no sobre `->`.
        for byte in &at {
            assert!(SRC[*byte..].starts_with("(d: Dto)") || SRC[*byte..].starts_with("Dto {"),
                    "cae sobre un campo de tipo, y no sobre {:?}", &SRC[*byte..*byte + 8]);
        }
    }

    /// **El archivo entero tiene firmas y ninguna es la suya.** Es *"no pude recorrer
    /// hacia el próximo nivel"*, no *"no hay vecindario"*, y por eso pide que se lo
    /// diga en vez de escribirse como ausencia.
    #[test]
    fn the_whole_file_is_unreachable_and_says_why() {
        let d = en("Svc.rs", SRC);
        let r = reach(d.path(), "Svc.rs", &Ranges::one(0, SRC.len()));
        let Reach::Unreachable { what } = r else { panic!("el archivo entero no se alcanza: {r:?}") };
        assert!(what.contains("archivo entero"), "el error tiene que decir por qué: {what}");
    }

    /// Y un archivo **sin** ninguna firma adentro sí es ausencia: no queda nada sin
    /// cubrir.
    #[test]
    fn a_whole_file_without_signatures_is_absence_and_not_a_refusal() {
        let body = "pub struct A { pub x: u8 }\npub struct B { pub y: u8 }\n";
        let d = en("Dtos.rs", body);
        assert_eq!(reach(d.path(), "Dtos.rs", &Ranges::one(0, body.len())), Reach::None);
    }

    /// Una firma sin ningún tipo que mencionar **sí se alcanza**, y su vecindario
    /// vacío es verdadero.
    ///
    /// Es el caso que hace que bilinker no pueda defenderse solo del vacío: acá el
    /// vacío es la respuesta correcta, así que una guarda contra él rompería esto. Por
    /// eso quien no puede contestar tiene que decirlo, y no callarse.
    #[test]
    fn a_signature_that_mentions_nothing_is_reached_and_its_emptiness_is_true() {
        let body = "pub fn go() { }\n";
        let d = en("Svc.rs", body);
        let Reach::At(at) = de(&d, "Svc.rs", "pub fn go() { }", 15) else {
            panic!("una firma se alcanza aunque no mencione nada");
        };
        assert_eq!(at.len(), 1, "la lista de parámetros, vacía: {at:?}");
        assert!(body[at[0]..].starts_with("()"));
    }
}
