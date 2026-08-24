use sha2::{Digest, Sha256};

pub fn sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Similitud entre dos fragmentos, de 0.0 a 1.0.
///
/// Coeficiente de Dice sobre líneas: `2·comunes / (total_a + total_b)`. Es la
/// misma idea que usa git para detectar renames de archivos, y por eso el umbral
/// del 50% significa acá lo mismo que en `git diff -M`.
///
/// Para fragmentos de una sola línea —un heading markdown, una firma— las líneas
/// no discriminan nada, así que cae a bigramas de caracteres.
pub fn similarity(a: &str, b: &str) -> f64 {
    if a == b { return 1.0; }
    if a.is_empty() || b.is_empty() { return 0.0; }

    let lines_a: Vec<&str> = a.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let lines_b: Vec<&str> = b.lines().map(str::trim).filter(|l| !l.is_empty()).collect();

    if lines_a.len() > 1 && lines_b.len() > 1 {
        return dice(&lines_a, &lines_b);
    }

    let bi_a: Vec<&str> = bigrams(a);
    let bi_b: Vec<&str> = bigrams(b);
    if bi_a.is_empty() || bi_b.is_empty() { return 0.0; }
    dice(&bi_a, &bi_b)
}

fn bigrams(s: &str) -> Vec<&str> {
    let idx: Vec<usize> = s.char_indices().map(|(i, _)| i).chain(std::iter::once(s.len())).collect();
    idx.windows(3).map(|w| &s[w[0]..w[2]]).collect()
}

/// Dice sobre multiconjuntos: cada elemento de `a` puede aparearse con uno solo
/// de `b`, así que repetir una línea no infla el puntaje.
fn dice(a: &[&str], b: &[&str]) -> f64 {
    let mut pool: Vec<&str> = b.to_vec();
    let mut common = 0usize;
    for item in a {
        if let Some(pos) = pool.iter().position(|x| x == item) {
            pool.swap_remove(pos);
            common += 1;
        }
    }
    (2.0 * common as f64) / (a.len() + b.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_one() {
        assert_eq!(similarity("fn foo() {}", "fn foo() {}"), 1.0);
    }

    #[test]
    fn rename_keeps_high_similarity() {
        let a = "fn foo() {\n    let x = 1;\n    println!(\"{}\", x);\n}";
        let b = "fn bar() {\n    let x = 1;\n    println!(\"{}\", x);\n}";
        assert!(similarity(a, b) > 0.5, "renombrar la firma no debería hundir la similitud");
    }

    #[test]
    fn rename_plus_small_edit_still_matches() {
        let a = "fn foo() {\n    let x = 1;\n    let y = 2;\n    x + y\n}";
        let b = "fn bar() {\n    let x = 1;\n    let y = 3;\n    x + y\n}";
        assert!(similarity(a, b) > 0.5);
    }

    #[test]
    fn unrelated_blocks_score_low() {
        let a = "fn foo() {\n    let x = 1;\n    x\n}";
        let b = "struct Otra {\n    campo: String,\n    otro: u32,\n}";
        assert!(similarity(a, b) < 0.5, "bloques distintos no deberían pasar el umbral");
    }

    #[test]
    fn single_line_falls_back_to_bigrams() {
        // Un heading markdown renombrado apenas.
        let s = similarity("## Escritura de cache", "## Escritura de la cache");
        assert!(s > 0.5, "similitud fue {s}");
    }

    #[test]
    fn repeated_lines_do_not_inflate() {
        let a = "x\nx\nx\ny";
        let b = "x\nz\nw\nv";
        assert!(similarity(a, b) < 0.5);
    }
}
