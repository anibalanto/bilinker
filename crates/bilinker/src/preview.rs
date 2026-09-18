//! Qué se capturó, y qué no — antes de escribir.
//!
//! Un capture es opaco después de escrito: el archivo dice `file` y `query`, y si la
//! query agarró el nodo equivocado eso se descubre cuando alguien va a leer el
//! fragmento, mucho después. La vista previa es lo que mueve ese descubrimiento al
//! momento en que todavía se puede no escribir.
//!
//! **Lo que se muestra marcado es lo capturado, y lo demás se ve sin marcar.** Eso
//! es deliberado: el error que se busca atrapar no es que falte algo, es que sobre o
//! esté en otro lado, y para verlo hace falta ver el vecindario.

use bilink_format::Ranges;

/// La marca de una línea capturada. Va en la vista y se lee de vuelta al editarla.
pub const MARK: char = '▸';

/// Las líneas que se saltan.
pub const SKIP: char = '⋮';

/// Cuántas líneas de contexto se muestran alrededor de cada parte.
pub const CONTEXT: usize = 2;

/// La vista de un fragmento sobre su archivo.
///
/// El archivo va **una vez**, como encabezado, y no repetido por parte: un
/// fragmento de cuatro partes con cuatro encabezados se lee como cuatro captures.
pub struct Preview {
    /// Cómo nombrar el archivo en el encabezado — `impl :: src/query.rs`.
    pub label: String,
    /// Las líneas capturadas, 1-based.
    pub marked: Vec<usize>,
    /// Los tramos contiguos de líneas capturadas, 1-based e inclusivos.
    ///
    /// **No son las partes.** Dos partes pegadas —el tipo de retorno y los
    /// parámetros de una firma— caen en el mismo tramo de líneas, así que los tramos
    /// dicen dónde mirar y `parts` cuántos fragmentos hay.
    pub spans: Vec<(usize, usize)>,
    /// Cuántas partes tiene el fragmento: una por `@target`.
    pub parts: usize,
    /// Una línea al pie: qué otra forma de capturar esto había. Se **sugiere**.
    pub note: Option<String>,
}

impl Preview {
    /// Qué líneas cubre cada parte del fragmento.
    pub fn of(label: &str, source: &str, ranges: &Ranges) -> Preview {
        let mut marked = Vec::new();
        for r in ranges.parts() {
            let from = line_of(source, r.start);
            let to   = line_of(source, r.end.saturating_sub(1));
            for l in from..=to { marked.push(l); }
        }
        marked.sort_unstable();
        marked.dedup();

        Preview {
            label:  label.to_string(),
            marked: marked.clone(),
            spans:  spans_of(&marked),
            parts:  ranges.parts().len(),
            note:   None,
        }
    }

    /// La misma vista, con una línea al pie.
    pub fn with_note(mut self, note: Option<String>) -> Preview {
        self.note = note;
        self
    }

    /// La vista, lista para imprimir o para abrir en un editor.
    pub fn render(&self, source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let width = lines.len().to_string().len().max(2);

        // Qué líneas se muestran: las marcadas y su contexto. El resto se resume con
        // un `⋮`, así el salto es visible en vez de silencioso.
        let mut shown: Vec<usize> = Vec::new();
        for &m in &self.marked {
            let from = m.saturating_sub(CONTEXT).max(1);
            let to   = (m + CONTEXT).min(lines.len());
            for l in from..=to { shown.push(l); }
        }
        shown.sort_unstable();
        shown.dedup();

        let mut out = format!("{}\n\n", self.label);
        let mut prev = 0usize;
        for l in shown {
            if prev != 0 && l > prev + 1 {
                out.push_str(&format!("     {SKIP}\n"));
            }
            let mark = if self.marked.contains(&l) { MARK } else { ' ' };
            out.push_str(&format!("  {mark} {:>width$}   {}\n", l, lines[l - 1], width = width));
            prev = l;
        }

        out.push('\n');
        out.push_str(&format!("{} · {}\n", partes(self.parts), fmt_spans(&self.spans)));
        out.push_str("queda afuera: todo lo que no está marcado\n");
        if let Some(note) = &self.note {
            out.push_str(&format!("{note}\n"));
        }
        out
    }

    /// Las líneas marcadas de una vista editada a mano.
    ///
    /// **Las marcas son señales, no rangos.** Cada línea marcada resuelve a su nodo,
    /// igual que una posición de la línea de comandos: marcar tres líneas de una
    /// función marca la función una vez. Por eso alcanza con devolver los números y
    /// volver a capturar, en vez de intentar leer rangos del buffer.
    pub fn marks_in(buffer: &str) -> Vec<usize> {
        let mut out = Vec::new();
        for line in buffer.lines() {
            let Some(rest) = line.strip_prefix("  ") else { continue };
            let Some(rest) = rest.strip_prefix(MARK) else { continue };
            if let Some(n) = rest.trim_start().split_whitespace().next() {
                if let Ok(n) = n.parse::<usize>() { out.push(n); }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn partes(n: usize) -> String {
    if n == 1 { "1 fragmento".into() } else { format!("{n} fragmentos") }
}

fn fmt_spans(spans: &[(usize, usize)]) -> String {
    spans.iter()
        .map(|(a, b)| if a == b { a.to_string() } else { format!("{a}–{b}") })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Los tramos contiguos de una lista de líneas ordenada.
fn spans_of(lines: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for &l in lines {
        match out.last_mut() {
            Some(last) if last.1 + 1 == l => last.1 = l,
            _ => out.push((l, l)),
        }
    }
    out
}

/// La línea 1-based de un offset. Cuenta sobre bytes: un offset puede caer en medio
/// de un carácter multibyte y cortar el `&str` ahí es un panic.
fn line_of(source: &str, byte: usize) -> usize {
    let end = byte.min(source.len());
    source.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use bilink_format::{ByteRange, Ranges};

    const SRC: &str = "uno\ndos\ntres\ncuatro\ncinco\nseis\nsiete\nocho\nnueve\ndiez\n";

    fn ranges(pairs: &[(usize, usize)]) -> Ranges {
        Ranges::new(pairs.iter().map(|&(s, e)| ByteRange { start: s, end: e }).collect()).unwrap()
    }

    /// Un byte range de dos líneas marca las dos.
    #[test]
    fn a_part_marks_every_line_it_covers() {
        // "dos\ntres" — líneas 2 y 3
        let p = Preview::of("x", SRC, &ranges(&[(4, 12)]));
        assert_eq!(p.marked, vec![2, 3]);
        assert_eq!(p.spans, vec![(2, 3)]);
    }

    /// Dos partes lejanas son dos tramos, y en el medio va un `⋮`.
    #[test]
    fn distant_parts_are_two_spans_with_a_gap() {
        let p = Preview::of("x", SRC, &ranges(&[(0, 3), (44, 48)]));
        assert_eq!(p.spans, vec![(1, 1), (9, 9)]);
        let out = p.render(SRC);
        assert!(out.contains(SKIP), "falta el salto:\n{out}");
        assert!(out.contains("2 fragmentos · 1, 9"), "{out}");
    }

    /// Lo no capturado se muestra **sin marcar**, que es cómo se ve que no entra.
    #[test]
    fn the_context_is_shown_unmarked() {
        let out = Preview::of("x", SRC, &ranges(&[(0, 3)])).render(SRC);
        let dos = out.lines().find(|l| l.ends_with("dos")).expect("la línea 2 se muestra");
        assert!(!dos.contains(MARK), "el contexto no va marcado: {dos}");
        let uno = out.lines().find(|l| l.ends_with("uno")).unwrap();
        assert!(uno.contains(MARK), "lo capturado sí: {uno}");
    }

    /// Lo que se imprime se puede volver a leer: es lo que hace editable la vista.
    #[test]
    fn the_marks_of_a_rendered_view_read_back() {
        let p = Preview::of("x", SRC, &ranges(&[(0, 3), (44, 48)]));
        assert_eq!(Preview::marks_in(&p.render(SRC)), p.marked);
    }

    /// Y una marca agregada a mano se lee igual.
    #[test]
    fn a_mark_added_by_hand_is_read() {
        let p = Preview::of("x", SRC, &ranges(&[(0, 3)]));
        let edited: String = p.render(SRC).lines()
            .map(|l| if l.ends_with("dos") { format!("  {MARK}{}", &l[3..]) } else { l.to_string() })
            .collect::<Vec<_>>().join("\n");
        assert_eq!(Preview::marks_in(&edited), vec![1, 2]);
    }
}

// ─── la vista de `get`: el fragmento sobre sus líneas ─────────────────────────

/// Lo que no entró **adentro** de una línea.
///
/// Mismo signo para el mismo hecho en las dos escalas: [`SKIP`] para las líneas que
/// no entran, esto para lo que no entra adentro de una. Un hueco es un hueco.
pub const HOLE: &str = "...";

/// El fragmento como lo imprime `get`: una línea por línea, con su número.
///
/// **No es [`Preview`], y no puede serlo.** Aquella marca líneas porque su vista es
/// *editable* —`marks_in` la lee de vuelta, y cada línea marcada resuelve a su
/// nodo—, así que meterle precisión de columna rompería esa simetría. Ésta no se lee
/// de vuelta, y ahí la precisión no cuesta nada.
///
/// Lo que comparten es el cálculo de tramos y saltos, que es lo que se
/// desincronizaría si hubiera dos renderers.
///
/// La sangría se conserva tal cual y **nunca se vuelve un hueco**: es espacio en
/// blanco, no aporta contenido, y sin ella el código no se lee.
pub fn fragment_view(source: &str, ranges: &Ranges, before: usize, after: usize) -> String {
    let partes: Vec<(usize, usize)> = ranges.parts().iter().map(|r| (r.start, r.end)).collect();
    let mut out = String::new();
    for fila in filas(source, &partes, before, after) {
        match fila {
            Fila::Salto(width)        => out.push_str(&format!("{:>width$} {SKIP}\n", "")),
            Fila::Linea(_, texto)     => { out.push_str(&texto); out.push('\n'); }
        }
    }
    out
}

/// Una fila de la vista: un salto, con el ancho de la columna de números, o una
/// línea del archivo ya numerada y con sus huecos.
enum Fila {
    Salto(usize),
    Linea(usize, String),
}

/// Las filas de la vista de unas partes: las líneas que tocan, con el contexto
/// pedido, y un salto donde no son contiguas.
///
/// Es lo que comparten la vista de un fragmento y la de sus dimensiones, y lo que se
/// desincronizaría si cada una calculara sus líneas.
fn filas(source: &str, partes: &[(usize, usize)], before: usize, after: usize) -> Vec<Fila> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() { return Vec::new() }

    // El offset donde arranca cada línea, 1-based.
    let mut start_of = Vec::with_capacity(lines.len() + 1);
    let mut at = 0usize;
    for l in &lines { start_of.push(at); at += l.len() + 1; }

    // Qué líneas se muestran: las que toca alguna parte, más el contexto pedido.
    let mut shown: Vec<usize> = Vec::new();
    for (s, e) in partes {
        let from = line_of(source, *s);
        let to   = line_of(source, e.saturating_sub(1));
        let from = from.saturating_sub(before).max(1);
        let to   = (to + after).min(lines.len());
        for l in from..=to { shown.push(l); }
    }
    shown.sort_unstable();
    shown.dedup();

    let width = shown.last().copied().unwrap_or(1).to_string().len();
    let mut out = Vec::new();
    let mut prev = 0usize;
    for l in shown {
        // **Un salto de líneas es visible o es una mentira.** Sin el `⋮`, dos tramos
        // lejanos se leen como si fueran contiguos.
        if prev != 0 && l > prev + 1 {
            out.push(Fila::Salto(width));
        }
        let texto = lines[l - 1];
        let ls = start_of[l - 1];
        out.push(Fila::Linea(l, format!("{l:>width$}:   {}", con_huecos(texto, ls, partes))));
        prev = l;
    }
    out
}

/// La vista de las dimensiones de un endpoint: sus partes sobre sus líneas, y al
/// final de cada línea los nombres de las dimensiones que la tocan.
///
/// **Una línea sale una sola vez**, aunque la toquen varias dimensiones: el retorno y
/// los parámetros comparten línea siempre que la firma quepa en una, y un bloque por
/// dimensión la imprimiría dos veces. Los nombres van en el orden en que sus partes
/// aparecen en la línea, así que con los `...` se lee cuál es cuál sin tocar el texto.
///
/// Dos dimensiones pueden superponerse —el formato sólo hace disjuntas las partes de
/// una misma—, así que las partes se unen antes de recortar: el rango compartido sale
/// una vez, y su línea lleva los dos nombres.
pub fn dimension_view(source: &str, dims: &[(String, Ranges)], before: usize, after: usize) -> String {
    let mut partes: Vec<(usize, usize)> = dims.iter()
        .flat_map(|(_, r)| r.parts().iter().map(|p| (p.start, p.end)))
        .collect();
    partes.sort_unstable();
    let mut unidas: Vec<(usize, usize)> = Vec::new();
    for (s, e) in partes {
        match unidas.last_mut() {
            Some(u) if s < u.1 => u.1 = u.1.max(e),
            _ => unidas.push((s, e)),
        }
    }

    let filas = filas(source, &unidas, before, after);
    let columna = filas.iter()
        .map(|f| match f { Fila::Linea(_, t) => t.chars().count(), Fila::Salto(_) => 0 })
        .max().unwrap_or(0) + 2;

    let mut out = String::new();
    for fila in filas {
        match fila {
            Fila::Salto(width) => out.push_str(&format!("{:>width$} {SKIP}\n", "")),
            Fila::Linea(l, texto) => {
                let nombres = nombres_de(source, l, dims);
                if nombres.is_empty() {
                    out.push_str(&texto);
                } else {
                    let pad = columna - texto.chars().count();
                    out.push_str(&format!("{texto}{:pad$}‹{}›", "", nombres.join(" · ")));
                }
                out.push('\n');
            }
        }
    }
    out
}

/// Los nombres de las dimensiones que tocan la línea `l`, 1-based, en el orden en
/// que aparece la primera parte de cada una en esa línea.
fn nombres_de<'a>(source: &str, l: usize, dims: &'a [(String, Ranges)]) -> Vec<&'a str> {
    let Some(texto) = source.lines().nth(l - 1) else { return Vec::new() };
    let inicio: usize = source.lines().take(l - 1).map(|t| t.len() + 1).sum();
    let fin = inicio + texto.len();
    let mut tocan: Vec<(usize, &str)> = dims.iter()
        .filter_map(|(nombre, r)| r.parts().iter()
            .filter(|p| p.start < fin.max(inicio + 1) && p.end > inicio)
            .map(|p| p.start.max(inicio))
            .min()
            .map(|at| (at, nombre.as_str())))
        .collect();
    tocan.sort();
    tocan.into_iter().map(|(_, n)| n).collect()
}

/// Una línea con `...` donde no la cubre ninguna parte.
///
/// **Los `...` son el límite entre partes, y por eso no hace falta marcarlo aparte**:
/// dónde termina una y arranca la otra se ve porque lo que hay en el medio no está.
///
/// Una línea que ninguna parte toca es contexto —la pidió `-B` o `-A`— y sale
/// entera: rellenarla de huecos diría que se capturó un pedazo, y no se capturó
/// ninguno.
fn con_huecos(texto: &str, line_start: usize, partes: &[(usize, usize)]) -> String {
    let fin = line_start + texto.len();
    let cubierto: Vec<(usize, usize)> = partes.iter()
        .filter_map(|(s, e)| {
            let (s, e) = ((*s).max(line_start), (*e).min(fin));
            (s < e).then_some((s, e))
        })
        .collect();
    if cubierto.is_empty() { return texto.to_string() }

    // La sangría va siempre y nunca es hueco.
    let sangria = texto.len() - texto.trim_start().len();
    let cuerpo  = line_start + sangria;

    let mut piezas: Vec<String> = Vec::new();
    let mut at = cuerpo;
    for (s, e) in &cubierto {
        if *s > at { piezas.push(HOLE.into()); }
        piezas.push(texto[s - line_start..e - line_start].to_string());
        at = *e;
    }
    if at < fin { piezas.push(HOLE.into()); }

    // Sin huecos, la línea sale igual que en el archivo.
    if piezas.len() == 1 { return texto.to_string() }
    format!("{}{}", &texto[..sangria], piezas.join(" "))
}

#[cfg(test)]
mod fragment_view_tests {
    use super::*;

    /// El caso que levantó esto: dos partes en la misma línea.
    ///
    /// Concatenadas, esa línea salía **dos veces** y se leía como una duplicación que
    /// no existe. Son todos los captures de `spring-controller`: el tipo de retorno y
    /// los parámetros comparten línea siempre que la firma quepa en una.
    const FIRMA: &str = "class C {\n\tpublic Dto get(String t) {\n\t\treturn null;\n\t}\n}\n";

    /// Las dos partes de la línea 2: `Dto` y `(String t)`.
    fn dos_partes() -> Ranges {
        let tipo = FIRMA.find("Dto").unwrap();
        let par  = FIRMA.find("(String t)").unwrap();
        rangos(&[(tipo, tipo + 3), (par, par + 10)])
    }

    fn rangos(pares: &[(usize, usize)]) -> Ranges {
        Ranges::new(pares.iter()
            .map(|&(start, end)| bilink_format::ByteRange { start, end })
            .collect()).unwrap()
    }

    #[test]
    fn una_linea_compartida_por_dos_partes_sale_una_vez() {
        let v = fragment_view(FIRMA, &dos_partes(), 0, 0);
        assert_eq!(v.lines().filter(|l| l.contains("Dto")).count(), 1, "{v}");
    }

    /// Y lo que no entró se ve, que es lo que vuelve legible el límite entre partes.
    #[test]
    fn lo_que_no_entra_adentro_de_una_linea_sale_como_hueco() {
        let v = fragment_view(FIRMA, &dos_partes(), 0, 0);
        let linea = v.lines().next().unwrap();
        assert!(linea.contains("... Dto ... (String t) ..."),
                "el `public`, el nombre y el `{{` son huecos: {linea}");
    }

    /// **La sangría se conserva y nunca es hueco.** Es espacio en blanco, no aporta
    /// contenido, y sin ella el código no se lee.
    #[test]
    fn la_sangria_se_conserva_y_no_se_vuelve_hueco() {
        let v = fragment_view(FIRMA, &dos_partes(), 0, 0);
        let linea = v.lines().next().unwrap();
        let tras_numero = linea.split_once(":   ").unwrap().1;
        assert!(tras_numero.starts_with('\t'), "la sangría va tal cual: {linea:?}");
        assert!(!tras_numero.starts_with("..."), "y no se cuenta como hueco: {linea:?}");
    }

    /// La línea lleva su número, que es lo que `get` no daba y la vista previa sí.
    #[test]
    fn cada_linea_lleva_su_numero() {
        let v = fragment_view(FIRMA, &dos_partes(), 0, 0);
        assert!(v.starts_with("2:"), "{v}");
    }

    /// Una parte que cubre la línea entera sale igual que en el archivo.
    #[test]
    fn una_parte_que_cubre_la_linea_entera_no_lleva_huecos() {
        let src = "uno\ndos\ntres\n";
        let at = src.find("dos").unwrap();
        let v = fragment_view(src, &Ranges::one(at, at + 3), 0, 0);
        assert_eq!(v, "2:   dos\n", "{v:?}");
    }

    /// **Dos tramos lejanos llevan `⋮` en el medio**, o se leen como contiguos.
    #[test]
    fn dos_tramos_lejanos_muestran_el_salto() {
        let a = FIRMA.find("class").unwrap();
        let b = FIRMA.find("return null;").unwrap();
        let v = fragment_view(FIRMA, &rangos(&[(a, a + 5), (b, b + 12)]), 0, 0);
        assert!(v.contains(SKIP), "el salto tiene que verse: {v}");
    }

    /// Una línea de contexto no la toca ninguna parte, y sale **entera**: llenarla de
    /// huecos diría que se capturó un pedazo, y no se capturó ninguno.
    #[test]
    fn el_contexto_sale_entero_y_sin_huecos() {
        let src = "uno\ndos\ntres\n";
        let at = src.find("dos").unwrap();
        let v = fragment_view(src, &Ranges::one(at, at + 3), 1, 1);
        assert_eq!(v, "1:   uno\n2:   dos\n3:   tres\n", "{v:?}");
    }
}

#[cfg(test)]
mod dimension_view_tests {
    use super::*;

    const FIRMA: &str = "class C {\n\tpublic Dto get(String t) {\n\t\treturn null;\n\t}\n}\n";

    fn en(src: &str, text: &str) -> Ranges {
        let at = src.find(text).unwrap();
        Ranges::one(at, at + text.len())
    }

    /// Las dimensiones en el orden de sus nombres, que es como las da el bilink.
    fn firma() -> Vec<(String, Ranges)> {
        vec![
            ("parameters".into(), en(FIRMA, "(String t)")),
            ("return".into(), en(FIRMA, "Dto")),
        ]
    }

    /// El caso que obliga a marcar por línea: el retorno y los parámetros comparten
    /// una, y la línea sale una vez, con los dos nombres en el orden en que aparecen.
    #[test]
    fn una_linea_lleva_los_nombres_de_sus_partes_en_orden_de_archivo() {
        let v = dimension_view(FIRMA, &firma(), 0, 0);
        assert_eq!(v.lines().count(), 1, "{v}");
        let linea = v.lines().next().unwrap();
        assert!(linea.contains("... Dto ... (String t) ..."), "el texto no se toca: {linea}");
        assert!(linea.ends_with("‹return · parameters›"), "{linea}");
    }

    /// Los nombres se alinean en una columna, a dos espacios de la línea más larga.
    #[test]
    fn los_nombres_van_en_una_columna() {
        let dims = vec![
            ("head".to_string(), en(FIRMA, "class C {")),
            ("return".to_string(), en(FIRMA, "Dto")),
        ];
        let v = dimension_view(FIRMA, &dims, 0, 0);
        let columnas: Vec<usize> = v.lines()
            .filter(|l| l.contains('‹'))
            .map(|l| l.chars().take_while(|c| *c != '‹').count())
            .collect();
        assert_eq!(columnas.len(), 2, "{v}");
        assert_eq!(columnas[0], columnas[1], "{v}");
        let larga = v.lines().map(|l| l.split('‹').next().unwrap().trim_end().chars().count()).max().unwrap();
        assert_eq!(columnas[0], larga + 2, "{v}");
    }

    /// Dos dimensiones que se superponen: el rango sale una vez, con los dos nombres.
    #[test]
    fn dos_dimensiones_superpuestas_salen_una_vez_con_los_dos_nombres() {
        let dims = vec![
            ("firma".to_string(), en(FIRMA, "Dto get(String t)")),
            ("parameters".to_string(), en(FIRMA, "(String t)")),
        ];
        let v = dimension_view(FIRMA, &dims, 0, 0);
        assert_eq!(v.matches("(String t)").count(), 1, "{v}");
        assert!(v.contains("... Dto get(String t) ..."), "{v}");
        assert!(v.contains("‹firma · parameters›"), "{v}");
    }

    /// El contexto de `-B` y `-A` sale entero y sin marcas: no es de ninguna dimensión.
    #[test]
    fn el_contexto_no_lleva_nombres() {
        let v = dimension_view(FIRMA, &firma(), 1, 1);
        let lineas: Vec<&str> = v.lines().collect();
        assert_eq!(lineas.len(), 3, "{v}");
        assert!(!lineas[0].contains('‹') && !lineas[2].contains('‹'), "{v}");
        assert!(lineas[1].contains('‹'), "{v}");
    }

    /// Las líneas lejanas llevan `⋮` en el medio, como en cualquier fragmento.
    #[test]
    fn el_salto_sigue_viendose() {
        let dims = vec![
            ("head".to_string(), en(FIRMA, "class C {")),
            ("body".to_string(), en(FIRMA, "return null;")),
        ];
        let v = dimension_view(FIRMA, &dims, 0, 0);
        assert!(v.contains(SKIP), "{v}");
    }
}
