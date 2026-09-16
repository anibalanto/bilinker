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
/// **Contesta o falla.** No hay un tercer valor para *"no pude mirar"*: un proveedor
/// que no puede contestar devuelve `Err`, y el comando que preguntó falla. Leerlo como
/// un vacío haría pasar un daemon apagado por un contrato que no menciona ningún tipo.
///
/// **Y recibe posiciones, no el rango del fragmento.** Dónde hay un tipo que
/// preguntar es gramática, y la gramática es de bilinker; qué declara ese tipo es del
/// proveedor. Pasarle el rango lo obligaba a inventar dónde preguntar adentro, y lo
/// que inventaba era *"el byte donde arranca"* — que sobre un capture de nodo entero
/// cae en `pub` y no declara nada.
///
/// **Cada posición es un identificador de tipo**, y de eso depende que el proveedor
/// pueda contestar algo útil: sobre un paréntesis o sobre el `ResponseEntity` de un
/// genérico, un proveedor que resuelve perfecto devuelve la función misma o un tipo
/// de otra capa. Ver `concepts/accept.md` § "Dónde se pregunta".
pub trait Neighbours {
    /// Si hay a quién preguntarle en esta capa. Se consulta antes de trabajar.
    fn available(&self, layer: &Path) -> bool;
    fn of(&self, layer: &Path, file: &str, at: &[usize]) -> Result<Vec<Location>>;
}

/// A quién se le pregunta en esta corrida. `None` es no preguntarle a nadie.
pub type Provider<'a> = Option<&'a dyn Neighbours>;

/// Cuántos endpoints de una corrida tienen nivel 1 que resolver, y de qué lenguajes.
///
/// Es lo que dice el error cuando no hay a quién preguntarle: cuántos, y qué
/// lenguajes tiene que calentar quien levante el proveedor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Demand {
    pub endpoints: usize,
    pub languages: std::collections::BTreeSet<&'static str>,
}

impl Demand {
    pub fn add(&mut self, file: &str) {
        self.endpoints += 1;
        // `tsx` es una gramática y no un lenguaje: lo atiende el mismo servidor.
        match grammar::language_for_file(file) {
            "tsx" => { self.languages.insert("typescript"); }
            lang if !grammar::signature_kinds(lang).is_empty() => { self.languages.insert(lang); }
            _ => {}
        }
    }
    pub fn is_empty(&self) -> bool { self.endpoints == 0 }
}

/// Hay nivel 1 que resolver y nadie a quién preguntarle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoProvider(pub Demand);

impl std::fmt::Display for NoProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} endpoint(s) tienen nivel 1 y no hay daemon en esta capa.", self.0.endpoints)
    }
}

impl std::error::Error for NoProvider {}

/// El proveedor falló mientras se le preguntaba: el comando no terminó.
///
/// Va como contexto del error del proveedor, para que quien sale del proceso lo
/// distinga de un error del árbol sin mirar el texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderFailed;

impl std::fmt::Display for ProviderFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("el proveedor del vecindario no contestó")
    }
}

/// Falla antes de trabajar si hay nivel 1 que resolver y el proveedor no está.
///
/// Sin proveedor —no preguntarle a nadie— y sin demanda no hay nada que exigir.
pub fn require(nb: Provider<'_>, layer: &Path, demand: Demand) -> Result<()> {
    match nb {
        Some(p) if !demand.is_empty() && !p.available(layer) => Err(NoProvider(demand).into()),
        _ => Ok(()),
    }
}

/// Le pregunta al proveedor, y marca su falla como [`ProviderFailed`].
pub fn ask(p: &dyn Neighbours, layer: &Path, file: &str, at: &[usize]) -> Result<Vec<Location>> {
    p.of(layer, file, at).map_err(|e| e.context(ProviderFailed))
}

/// Si el error viene de que el proveedor no está o falló.
pub fn is_provider_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<NoProvider>().is_some() || e.downcast_ref::<ProviderFailed>().is_some()
}

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
    /// Hay, y se sabe dónde preguntar: el byte inicial de cada identificador de tipo
    /// que los campos de la firma mencionan. Un campo puede aportar varios
    /// —`ResponseEntity<List<Dto>>` aporta tres— y `void` ninguno.
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
///
/// **De cada campo salen los identificadores de tipo que contiene, no su primer
/// byte.** Un campo no empieza en un tipo —`parameters` empieza en el paréntesis— así
/// que proyectarlo pregunta donde no hay nada que declarar. Ver [`type_positions`].
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
    let tipos = grammar::type_identifier_kinds(lang);
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
                type_positions(child, tipos, &mut at);
            }
        }
    }

    at.sort_unstable();
    at.dedup();
    // Una firma que no menciona ningún tipo —`fn f() {}`, `void f()`, o una de puros
    // primitivos— no tiene a quién preguntarle, y eso **sí** es un vecindario vacío
    // legítimo. Los campos pueden estar y no aportar ninguna posición.
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
/// El fold, y los captures que lo componen.
///
/// **Van juntos porque se calculan juntos y se escriben aparte.** `check` llama a
/// esto y descarta los captures —no escribe nada versionado—; `accept` los escribe,
/// porque sin los archivos el `n.1.link` que va a guardar apuntaría a captures que no
/// existen. Devolverlos en vez de escribirlos acá es lo que deja esa decisión en
/// quien la puede tomar.
pub struct Folded {
    pub n: Neighbourhood,
    pub captures: Vec<bilink_format::Capture>,
}

/// `None` es *"el vecindario no se puede representar"*.
///
/// Pasa cuando **algún** vecino no se puede capturar: un archivo sin gramática, un
/// nodo sin ancla estable. Saltearlo diría que hay menos vecinos de los que hay, y
/// rompería la correspondencia entre `link` y `hash` — el fold cubriría un conjunto
/// y la lista nombraría otro.
///
/// Así que es todo o nada, y quien lo pide decide qué hace con "nada": `accept` falla,
/// y `check` lo lee como un conjunto que ya no es el aceptado.
pub fn fold(layer: &Path, locs: &[Location]) -> Result<Option<Folded>> {
    let mut captures: Vec<(String, String, Option<String>, bilink_format::Capture)> = Vec::new();

    for loc in locs {
        // **La ubicación sirve para encontrar el nodo, y después se descarta.** Es la
        // regla de cualquier selección, y acá es la que arregla el defecto de fondo:
        // `definitions` devuelve el rango del *nombre* del tipo, y `capture` camina de
        // ahí al ancla estable que lo contiene — su declaración. Hashear el nombre
        // cubría *"el tipo sigue llamándose igual"*; hashear la declaración cubre su
        // forma, que es el caso por el que el nivel 1 existe.
        let source = std::fs::read_to_string(layer.join(&loc.file)).unwrap_or_default();
        let pos = line_col_1based(&source, loc.start);
        // Un vecino que no se puede capturar no se inventa: se saltea, y el conjunto
        // queda sin él. El eje de ubicación lo va a decir, porque el id no está.
        // **Sin commit y por lo tanto sin git.** Lo único que hace falta de un vecino
        // es su id y su hash; pedir el commit del archivo ataría el cálculo a que
        // esté versionado, que no tiene nada que ver.
        let Ok((c, h, rr)) = crate::capture::compute(layer, &loc.file, &[(pos, pos)], None)
            else { return Ok(None) };

        let ast = ast_of(&loc.file, &source, &rr);
        captures.push((c.id(), h, ast, c));
    }
    Ok(Some(plegar(captures)))
}

/// El fold de los vecinos aceptados, resolviendo cada capture por su query.
///
/// **Es el mismo fold que calcula [`fold`]**, con el mismo orden, el mismo recorte y
/// el mismo `hash_ast`: sólo cambia de dónde sale cada rango. Con esto el contenido de
/// un nivel 1 se verifica sin preguntarle a nadie.
///
/// `None` es que algún capture no se pudo leer o no resuelve: la declaración aceptada
/// ya no está donde estaba.
pub fn fold_captures(layer: &Path, ids: &[String]) -> Result<Option<Folded>> {
    let mut captures = Vec::new();
    for id in ids {
        let Ok(c) = bilink_format::Capture::load_in(layer, id) else { return Ok(None) };
        let (state, ranges) = crate::check::resolve_capture(layer, &c, None, None)?;
        let (true, Some(rr)) = (state.is_resolved(), ranges) else { return Ok(None) };
        let Ok(source) = std::fs::read_to_string(layer.join(&c.file)) else { return Ok(None) };
        let h = hash::sha256(rr.text(&source).as_bytes());
        let ast = ast_of(&c.file, &source, &rr);
        captures.push((c.id(), h, ast, c));
    }
    Ok(Some(plegar(captures)))
}

/// La huella de un vecino, donde la gramática discrimina contenido.
fn ast_of(file: &str, source: &str, rr: &Ranges) -> Option<String> {
    let lang = grammar::language_for_file(file);
    grammar::ast_discriminates_content(lang)
        .then(|| {
            let rs = rr.parts().first()?;
            sexp_of(lang, source, rs.start, rs.end).map(|x| hash::sha256(x.as_bytes()))
        })
        .flatten()
}

/// Los dos hashes de un conjunto de vecinos, ya capturados.
fn plegar(mut captures: Vec<(String, String, Option<String>, bilink_format::Capture)>) -> Folded {

    // **El orden es por id de capture**, que es la identidad: `sha256(file \0 query \0)`.
    //
    // No lleva contenido, así que un reformateo no lo mueve; y cambia exactamente
    // cuando un vecino entra, sale, se muda de archivo o se renombra — que son
    // cambios de contrato. La clave vieja era `<path>` más el nombre del símbolo, dos
    // cosas concatenadas para decir lo que el id dice solo.
    captures.sort_by(|a, b| a.0.cmp(&b.0));
    captures.dedup_by(|a, b| a.0 == b.0);

    let mut texts = String::new();
    let mut sexps = String::new();
    let mut every_one_has_a_grammar = true;
    for (_, h, ast, _) in &captures {
        texts.push_str(h);
        texts.push('\0');
        match ast {
            Some(a) => { sexps.push_str(a); sexps.push('\0'); }
            None    => every_one_has_a_grammar = false,
        }
    }

    Folded {
        n: Neighbourhood {
            link: bilink_format::CaptureSet::new(captures.iter().map(|(id, ..)| id.clone()).collect()).into(),
            hash:     hash::sha256(texts.as_bytes()),
            hash_ast: every_one_has_a_grammar.then(|| hash::sha256(sexps.as_bytes())),
        },
        captures: captures.into_iter().map(|(.., c)| c).collect(),
    }
}

/// Línea y columna **1-based** de un offset, que es lo que `capture` toma.
fn line_col_1based(source: &str, byte: usize) -> (usize, usize) {
    let end = byte.min(source.len());
    let head = &source.as_bytes()[..end];
    let line = head.iter().filter(|&&b| b == b'\n').count() + 1;
    let col  = end - head.iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0) + 1;
    (line, col)
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
        assert_eq!(fold(d.path(), &[a.clone(), b.clone()]).unwrap().unwrap().n,
                   fold(d.path(), &[b, a]).unwrap().unwrap().n);
    }

    /// Un vecino repetido es un vecino: `Persona f(Persona a, Persona b)` menciona un
    /// tipo, no tres.
    #[test]
    fn the_same_neighbour_twice_is_one() {
        let d = layer_with(&[("a.rs", "struct A { x: u8 }\n")]);
        let a = loc("a.rs", "A", 0, 18);
        assert_eq!(fold(d.path(), &[a.clone()]).unwrap().unwrap().n,
                   fold(d.path(), &[a.clone(), a]).unwrap().unwrap().n);
    }

    /// Que un vecino cambie mueve los dos hashes.
    #[test]
    fn a_changed_neighbour_moves_both() {
        let d1 = layer_with(&[("a.rs", "struct A { x: u8 }\n")]);
        let d2 = layer_with(&[("a.rs", "struct A { x: u8, y: u8 }\n")]);
        let f1 = fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap().unwrap().n;
        let f2 = fold(d2.path(), &[loc("a.rs", "A", 0, 25)]).unwrap().unwrap().n;
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
        let f1 = fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap().unwrap().n;
        let f2 = fold(d2.path(), &[loc("a.rs", "A", 0, 22)]).unwrap().unwrap().n;
        assert_ne!(f1.hash, f2.hash);
        assert_eq!(f1.hash_ast, f2.hash_ast);
    }

    /// **Un vecino que no se puede capturar vuelve el vecindario irrepresentable.**
    ///
    /// Antes se hasheaba por texto y sólo se caía el `hash_ast`. Con los vecinos
    /// siendo captures eso ya no alcanza: saltearlo diría que hay menos vecinos de los
    /// que hay, y el fold cubriría un conjunto mientras `link` nombraría otro.
    ///
    /// **Lo que se pierde es teórico y lo que se gana es estructural.** Un tipo se
    /// declara en un archivo del lenguaje del tipo, y ésos tienen gramática; que
    /// `definitions` apunte a un `.txt` sería raro. Lo que se gana es que `link` y
    /// `hash` cubran siempre exactamente el mismo conjunto.
    #[test]
    fn a_neighbour_that_cannot_be_captured_makes_the_neighbourhood_unavailable() {
        let d = layer_with(&[("a.rs", "struct A { x: u8 }\n"), ("nota.txt", "hola\n")]);
        let con = fold(d.path(), &[loc("a.rs", "A", 0, 18)]).unwrap().unwrap().n;
        assert!(con.hash_ast.is_some());
        assert_eq!(con.link.known_ids().expect("resuelto, no unknown").len(), 1,
                   "el vecino capturable está nombrado");

        let sin = fold(d.path(), &[loc("a.rs", "A", 0, 18), loc("nota.txt", "nota", 0, 5)]).unwrap();
        assert!(sin.is_none(), "todo o nada: no se saltea un vecino");
    }

    /// El recorte de bordes vale igual acá: agregarle algo abajo al archivo no le
    /// mueve el hash a un vecino que nadie tocó.
    #[test]
    fn the_edges_are_trimmed_like_any_fragment() {
        let d1 = layer_with(&[("a.rs", "struct A { x: u8 }")]);
        let d2 = layer_with(&[("a.rs", "struct A { x: u8 }\n\n\n")]);
        assert_eq!(fold(d1.path(), &[loc("a.rs", "A", 0, 18)]).unwrap().unwrap().n.hash,
                   fold(d2.path(), &[loc("a.rs", "A", 0, 21)]).unwrap().unwrap().n.hash);
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use tempfile::tempdir;

    /// Un proveedor de mentira: contesta lo que se le dijo, o falla.
    struct Fake {
        alive: bool,
        locs:  Option<Vec<Location>>,
        asked: Cell<usize>,
    }
    impl Neighbours for Fake {
        fn available(&self, _l: &Path) -> bool { self.alive }
        fn of(&self, _l: &Path, _f: &str, _at: &[usize]) -> Result<Vec<Location>> {
            self.asked.set(self.asked.get() + 1);
            self.locs.clone().ok_or_else(|| anyhow::anyhow!("el daemon no contestó"))
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

    /// **Un proveedor que no puede contestar falla, y la falla se distingue.** No hay
    /// un vacío que lo disfrace de contrato sin tipos.
    #[test]
    fn a_provider_that_cannot_answer_fails_and_is_told_apart() {
        let d = repo();
        let mudo = Fake { alive: true, locs: None, asked: Cell::new(0) };
        let e = ask(&mudo, d.path(), "Svc.rs", &[0]).unwrap_err();
        assert!(is_provider_error(&e), "la falla del proveedor se marca: {e:#}");

        let vacio = fold(d.path(), &[]).unwrap().unwrap().n;
        let con   = fold(d.path(), &[dto()]).unwrap().unwrap().n;
        assert_ne!(vacio.hash, con.hash, "y un vacío legítimo sigue siendo otra cosa");
    }

    /// Se le pregunta una vez por endpoint, no una por vecino.
    #[test]
    fn the_provider_is_asked_once() {
        let d = repo();
        let p = Fake { alive: true, locs: Some(vec![dto(), dto()]), asked: Cell::new(0) };
        let locs = ask(&p, d.path(), "Svc.rs", &[0]).unwrap();
        assert_eq!(p.asked.get(), 1);
        // Y dos veces el mismo vecino es un vecino.
        assert_eq!(fold(d.path(), &locs).unwrap().unwrap().n, fold(d.path(), &[dto()]).unwrap().unwrap().n);
    }

    /// **Sin proveedor que conteste, falla antes de trabajar, y dice cuántos y de qué
    /// lenguajes.** Sin demanda, o sin preguntarle a nadie, no hay nada que exigir.
    #[test]
    fn requiring_a_provider_fails_only_with_demand_and_nobody_to_ask() {
        let d = repo();
        let apagado = Fake { alive: false, locs: None, asked: Cell::new(0) };
        let mut demand = Demand::default();
        demand.add("src/A.java");
        demand.add("web/b.tsx");
        demand.add("docs/spec.md");

        let e = require(Some(&apagado), d.path(), demand.clone()).unwrap_err();
        let NoProvider(dicho) = e.downcast_ref::<NoProvider>().expect("es un NoProvider").clone();
        assert_eq!(dicho.endpoints, 3);
        assert_eq!(dicho.languages.iter().copied().collect::<Vec<_>>(), vec!["java", "typescript"],
                   "los lenguajes que lspd entiende, sin prosa");
        assert!(is_provider_error(&e));

        assert!(require(Some(&apagado), d.path(), Demand::default()).is_ok(), "sin demanda");
        assert!(require(None, d.path(), demand.clone()).is_ok(), "sin preguntarle a nadie");
        let vivo = Fake { alive: true, locs: None, asked: Cell::new(0) };
        assert!(require(Some(&vivo), d.path(), demand).is_ok());
        assert_eq!(apagado.asked.get() + vivo.asked.get(), 0, "exigir no pregunta nada");
    }

    /// **El fold de los captures de los vecinos, resueltos por su query, es el mismo
    /// que calcula `accept`.** Es lo que deja verificar el contenido del nivel 1 sin
    /// preguntarle a nadie.
    #[test]
    fn folding_the_neighbour_captures_equals_the_fold_accept_computes() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("Svc.rs"),
            "pub struct Dto {\n    pub x: u8,\n}\n\npub enum Kind { A, B }\n\npub fn get(k: Kind) -> Dto { todo!() }\n").unwrap();
        fs::write(d.path().join("Otro.java"),
            "class Otro {\n\tprivate String y;\n}\n").unwrap();
        let src = fs::read_to_string(d.path().join("Svc.rs")).unwrap();
        let locs = vec![
            Location { file: "Svc.rs".into(), symbol: "Dto".into(),
                       start: src.find("Dto").unwrap(), end: src.find("Dto").unwrap() + 3 },
            Location { file: "Svc.rs".into(), symbol: "Kind".into(),
                       start: src.find("Kind").unwrap(), end: src.find("Kind").unwrap() + 4 },
            Location { file: "Otro.java".into(), symbol: "Otro".into(), start: 6, end: 10 },
        ];

        let aceptado = fold(d.path(), &locs).unwrap().unwrap();
        for c in &aceptado.captures { c.write_in(d.path()).unwrap(); }
        let ids = aceptado.n.link.known_ids().unwrap().to_vec();

        let hoy = fold_captures(d.path(), &ids).unwrap().expect("los captures resuelven");
        assert_eq!(hoy.n, aceptado.n, "mismo conjunto, mismo hash y mismo hash_ast");
    }

    /// Un capture de vecino que ya no resuelve no se pliega: es un cambio.
    #[test]
    fn a_neighbour_capture_that_no_longer_resolves_does_not_fold() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("Svc.rs"), "pub struct Dto { pub x: u8 }\n").unwrap();
        let aceptado = fold(d.path(), &[Location {
            file: "Svc.rs".into(), symbol: "Dto".into(), start: 11, end: 14 }]).unwrap().unwrap();
        for c in &aceptado.captures { c.write_in(d.path()).unwrap(); }
        let ids = aceptado.n.link.known_ids().unwrap().to_vec();

        fs::write(d.path().join("Svc.rs"), "pub struct Otro { pub x: u8 }\n").unwrap();
        assert!(fold_captures(d.path(), &ids).unwrap().is_none());
    }
}

/// Los bytes iniciales de los identificadores de tipo que este nodo contiene.
///
/// **Un recorrido y no una proyección.** El primer byte de un campo de la firma casi
/// nunca es un tipo: el de `parameters` es el paréntesis —y preguntar ahí devuelve la
/// función que lo contiene, o sea el propio fragmento— y el de `Result<Checked>` o
/// `ResponseEntity<List<Dto>>` es el tipo de más afuera, que suele estar en otra capa
/// y se descarta, dejando sin preguntar justo al que importa.
///
/// Va a cualquier profundidad porque la anidación no tiene tope: `ResponseEntity<
/// List<Dto>>` esconde su DTO dos niveles adentro. Ver `concepts/accept.md`
/// § "Dónde se pregunta".
fn type_positions(node: tree_sitter::Node<'_>, kinds: &[&str], at: &mut Vec<usize>) {
    let mut cur = node.walk();
    let mut pila = vec![node];
    while let Some(n) = pila.pop() {
        if kinds.contains(&n.kind()) { at.push(n.start_byte()) }
        pila.extend(n.children(&mut cur));
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

    /// **La firma se alcanza, y cada posición cae sobre un identificador de tipo** —
    /// no sobre el `pub` donde el fragmento arranca, ni sobre el paréntesis donde
    /// arranca la lista de parámetros. Ninguno de los dos declara nada.
    #[test]
    fn a_signature_is_reached_at_its_types_and_not_at_its_start() {
        let d = en("Svc.rs", SRC);
        let arranca = SRC.find("pub fn get").unwrap();
        let firma = "pub fn get(d: Dto) -> Dto { todo!() }";
        let Reach::At(at) = de(&d, "Svc.rs", firma, firma.len()) else {
            panic!("una firma se alcanza");
        };
        assert!(!at.contains(&arranca), "preguntar en `pub` es el defecto que esto arregla");
        assert_eq!(at.len(), 2, "el tipo del parámetro y el del retorno: {at:?}");
        // **Las dos caen sobre `Dto`.** Antes una caía sobre el `(` de `parameters`, y
        // preguntar ahí devuelve la función que lo contiene: el fragmento se declaraba
        // vecino de sí mismo, y eso no fallaba — cubría.
        for byte in &at {
            assert!(SRC[*byte..].starts_with("Dto"),
                    "cae sobre un identificador de tipo, y no sobre {:?}", &SRC[*byte..*byte + 8]);
        }
    }

    /// Y el paréntesis **no es una posición**, dicho sobre el byte y no sobre el
    /// conteo: es el defecto que producía el vecino de sí mismo.
    #[test]
    fn the_parameter_paren_is_never_asked() {
        let d = en("Svc.rs", SRC);
        let firma = "pub fn get(d: Dto) -> Dto { todo!() }";
        let paren = SRC.find("(d: Dto)").unwrap();
        let Reach::At(at) = de(&d, "Svc.rs", firma, firma.len()) else { panic!() };
        assert!(!at.contains(&paren), "el `(` no declara ningún tipo: {at:?}");
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

    /// Una firma que **no menciona ningún tipo** no tiene a quién preguntarle, y eso
    /// se sabe con la gramática sola: `Reach::None`, sin proveedor y sin aviso.
    ///
    /// **Antes daba `At([el paréntesis])`**, o sea que iba a preguntar por un
    /// vecindario que la gramática ya sabía vacío. Que ahora se decida de este lado
    /// achica el vacío indefendible del puerto a lo que de verdad lo es: un proveedor
    /// que resolvió y no encontró nada **en la capa**.
    #[test]
    fn a_signature_that_mentions_no_type_has_none_and_nobody_is_asked() {
        let body = "pub fn go() { }\n";
        let d = en("Svc.rs", body);
        assert_eq!(de(&d, "Svc.rs", "pub fn go() { }", 15), Reach::None);
    }

    /// Y una de puros primitivos es el mismo caso: `u8` es `primitive_type`, no un
    /// identificador de tipo, y no tiene declaración a la que ir.
    #[test]
    fn a_signature_of_only_primitives_has_none() {
        let body = "pub fn add(a: u8, b: u8) -> u8 { a + b }\n";
        let d = en("Svc.rs", body);
        let m = "pub fn add(a: u8, b: u8) -> u8 { a + b }";
        assert_eq!(de(&d, "Svc.rs", m, m.len()), Reach::None);
    }

    /// **El retorno genérico es el caso de `hsi`, y es el que más aparece.**
    ///
    /// El campo `return_type` de `Result<Checked>` arranca en `Result`, que está en
    /// otra capa y se descarta — así que proyectando el primer byte, `Checked` no se
    /// preguntaba nunca. Recorriendo, los dos se preguntan y el proveedor decide.
    #[test]
    fn a_generic_return_asks_for_the_type_inside_and_not_only_the_outer_one() {
        let body = "pub struct Checked { pub x: u8 }\n\npub fn check() -> Result<Checked> { todo!() }\n";
        let d = en("Svc.rs", body);
        let m = "pub fn check() -> Result<Checked> { todo!() }";
        let Reach::At(at) = de(&d, "Svc.rs", m, m.len()) else { panic!("se alcanza") };
        let ve: Vec<&str> = at.iter().map(|b| body[*b..].split(|c: char| !c.is_alphanumeric()).next().unwrap()).collect();
        assert!(ve.contains(&"Checked"), "el tipo de adentro se pregunta: {ve:?}");
        assert!(ve.contains(&"Result"), "y el de afuera también: el proveedor decide, no la gramática: {ve:?}");
    }

    /// Y en java anida dos veces, que es la forma de 28 de los 98 endpoints de `hsi`.
    #[test]
    fn a_doubly_nested_java_return_asks_for_the_dto() {
        let body = "class C {\n\tpublic ResponseEntity<List<Dto>> get() { return null; }\n}\n";
        let d = en("C.java", body);
        let m = "public ResponseEntity<List<Dto>> get() { return null; }";
        let Reach::At(at) = de(&d, "C.java", m, m.len()) else { panic!("se alcanza") };
        let ve: Vec<&str> = at.iter().map(|b| body[*b..].split(|c: char| !c.is_alphanumeric()).next().unwrap()).collect();
        assert!(ve.contains(&"Dto"), "el DTO está tres capas adentro y es el que importa: {ve:?}");
        assert_eq!(ve.len(), 3, "ResponseEntity, List y Dto: {ve:?}");
    }

    /// Un `void` no aporta ninguna posición, y su lista de parámetros vacía tampoco:
    /// son 7 de los 98 de `hsi`.
    #[test]
    fn a_void_java_signature_with_no_parameters_has_none() {
        let body = "class C {\n\tpublic void go() { }\n}\n";
        let d = en("C.java", body);
        let m = "public void go() { }";
        assert_eq!(de(&d, "C.java", m, m.len()), Reach::None);
    }
}
