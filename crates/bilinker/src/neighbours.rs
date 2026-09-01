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
pub trait Neighbours {
    fn of(&self, layer: &Path, file: &str, ranges: &Ranges) -> Result<Option<Vec<Location>>>;
}

/// Quién resuelve el vecindario en esta corrida, si hay alguien.
pub type Provider<'a> = Option<&'a dyn Neighbours>;

/// Los dos hashes plegados de un vecindario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folded {
    pub hash: String,
    /// Presente **sólo si todos** los vecinos tienen gramática. Ver el módulo.
    pub hash_ast: Option<String>,
}

/// Un solo orden, y dos folds sobre ese orden.
///
/// **La clave de orden es identidad, nunca contenido.** Ordenando por el texto, un
/// reformateo le cambiaría el puesto a un vecino, la lista se reordenaría, y
/// `hash_ast_n1` se movería sin que ningún AST cambiara — un falso *"cambió de
/// verdad"* producido por el orden. Ordenando por identidad nadie se mueve de puesto
/// salvo que un vecino entre, salga o se renombre, y esas tres cosas **son** cambios
/// de contrato.
///
/// Tampoco puede ordenar el rango: lleva offsets, que se corren con cualquier
/// edición más arriba del archivo.
pub fn fold(layer: &Path, locs: &[Location]) -> Result<Folded> {
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

    Ok(Folded {
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
        fn of(&self, _l: &Path, _f: &str, _r: &Ranges) -> Result<Option<Vec<Location>>> {
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
        let r = Ranges::one(0, 10);
        assert!(mudo.of(d.path(), "Svc.rs", &r).unwrap().is_none(),
                "sin proveedor no hay vecindario, y eso no es un vecindario vacío");
    }

    /// Se le pregunta una vez por endpoint, no una por vecino.
    #[test]
    fn the_provider_is_asked_once() {
        let d = repo();
        let p = Fake { locs: Some(vec![dto(), dto()]), asked: Cell::new(0) };
        let r = Ranges::one(0, 10);
        let locs = p.of(d.path(), "Svc.rs", &r).unwrap().unwrap();
        assert_eq!(p.asked.get(), 1);
        // Y dos veces el mismo vecino es un vecino.
        assert_eq!(fold(d.path(), &locs).unwrap(), fold(d.path(), &[dto()]).unwrap());
    }
}
