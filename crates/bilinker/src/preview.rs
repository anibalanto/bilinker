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
    pub spans: Vec<(usize, usize)>,
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

        Preview { label: label.to_string(), marked: marked.clone(), spans: spans_of(&marked) }
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
        out.push_str(&format!("{} · {}\n", partes(self.spans.len()), fmt_spans(&self.spans)));
        out.push_str("queda afuera: todo lo que no está marcado\n");
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
