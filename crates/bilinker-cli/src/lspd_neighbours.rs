//! El vecindario resuelto contra [`lspd`](https://github.com/anibalanto/lspd).
//!
//! **Éste es el único archivo de bilinker que nombra al daemon.** La librería define
//! el puerto y no sabe quién lo implementa; el binario elige. No es para evitar un
//! ciclo —desde que el daemon salió de lattice no hay ninguno— sino para que
//! bilinker no quede atado a *ese* daemon: mañana puede ser SCIP, un índice propio,
//! o un language server hablado directo.

use std::path::Path;

use anyhow::Result;
use bilink_format::Ranges;
use bilinker::neighbours::{Location, Neighbours};

pub struct Lspd;

impl Neighbours for Lspd {
    /// Los tipos que la firma menciona, un salto.
    ///
    /// Devuelve `None` cuando no hay daemon: **no pude mirar**, que es distinto de
    /// *no hay vecinos*. Se pregunta con un `ping`, que falla en el acto si el socket
    /// no está — por eso `check` sin daemon no se pone más lento.
    ///
    /// **Y también cuando el daemon está pero el servidor de atrás sigue indexando.**
    /// Un `ping` contesta antes que el language server esté listo, así que "hay
    /// daemon" no alcanza: en esa ventana `definitions` devolvía `[]`, que llega como
    /// `Some(vec![])` —*"miré y esta firma no menciona ningún tipo"*— y se escribe
    /// como vecindario adquirido. Es una cobertura afirmada que no existe.
    ///
    /// La distinción no se puede hacer de este lado —una firma de puros primitivos
    /// tiene vecindario vacío, y es legítimo—, así que la da el daemon con
    /// [`NOT_READY`](lspd_client::NOT_READY) y acá sólo se traduce.
    fn of(&self, layer: &Path, file: &str, ranges: &Ranges) -> Result<Option<Vec<Location>>> {
        if !lspd_client::responds() { return Ok(None); }

        let abs = layer.join(file);
        let source = std::fs::read_to_string(&abs)?;

        let mut out: Vec<Location> = Vec::new();
        for r in ranges.parts() {
            // El daemon habla en línea/columna 0-based, como LSP. La conversión es de
            // este lado: traducirla allá sería ponerle al daemon una convención que
            // no es suya.
            let (line, col) = line_col_of(&source, r.start);
            let val = match lspd_client::rpc("definitions", serde_json::json!({
                "file": abs.to_string_lossy(), "line": line, "col": col,
            })) {
                Ok(v) => v,
                // **Un rango sin resolver invalida el vecindario entero**, no sólo el
                // suyo: el fold es sobre el conjunto, y un conjunto al que le falta
                // un miembro hashea distinto que el completo. Devolver lo que se
                // alcanzó a juntar sería exactamente el vacío que este camino existe
                // para no escribir.
                Err(e) if not_ready(&e) => return Ok(None),
                Err(e) => return Err(e),
            };
            for d in val.as_array().into_iter().flatten() {
                let Some(loc) = location_of(layer, d) else { continue };
                out.push(loc);
            }
        }
        Ok(Some(out))
    }
}

/// Si el daemon contestó *"todavía no puedo"*.
///
/// **Por código y no por mensaje.** El texto del error es prosa que alguien va a
/// mejorar; el código es el contrato.
fn not_ready(e: &anyhow::Error) -> bool {
    e.downcast_ref::<lspd_client::RpcError>().is_some_and(|r| r.is_not_ready())
}

/// Una definición del daemon, traducida a la forma que bilinker foldea.
///
/// Se descarta lo que cae fuera de la capa: el vecindario de un contrato son los
/// tipos del proyecto, y un `String` de la stdlib no es algo que nadie vaya a
/// aceptar ni a mirar cuando cambie.
fn location_of(layer: &Path, d: &serde_json::Value) -> Option<Location> {
    let file   = d.get("file")?.as_str()?;
    let symbol = d.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rel = Path::new(file).strip_prefix(layer).ok()?.to_string_lossy().to_string();

    let source = std::fs::read_to_string(file).ok()?;
    let start = byte_of(&source, d.get("line")?.as_u64()? as usize,
                                 d.get("col")?.as_u64()? as usize)?;
    let end = d.get("end_line").and_then(|v| v.as_u64())
        .zip(d.get("end_col").and_then(|v| v.as_u64()))
        .and_then(|(l, c)| byte_of(&source, l as usize, c as usize))
        .unwrap_or(source.len());

    Some(Location { file: rel, symbol, start, end })
}

/// Línea y columna 0-based de un offset, contando bytes.
fn line_col_of(source: &str, byte: usize) -> (usize, usize) {
    let end = byte.min(source.len());
    let head = &source.as_bytes()[..end];
    let line = head.iter().filter(|&&b| b == b'\n').count();
    let col  = end - head.iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
    (line, col)
}

fn byte_of(source: &str, line: usize, col: usize) -> Option<usize> {
    let mut at = 0usize;
    for (i, l) in source.split_inclusive('\n').enumerate() {
        if i == line { return Some((at + col).min(source.len())); }
        at += l.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "uno\ndos\ntres\n";

    #[test]
    fn line_and_col_round_trip() {
        for byte in [0usize, 3, 4, 7, 8, 11] {
            let (l, c) = line_col_of(SRC, byte);
            assert_eq!(byte_of(SRC, l, c), Some(byte), "byte {byte} → {l}:{c}");
        }
    }

    /// Cuenta bytes y no caracteres: una `ó` en una spec en castellano alcanza para
    /// que las dos cosas dejen de coincidir.
    #[test]
    fn it_counts_bytes() {
        let src = "canción\nsiguiente\n";
        let byte = src.find("siguiente").unwrap();
        let (l, c) = line_col_of(src, byte);
        assert_eq!((l, c), (1, 0));
        assert_eq!(byte_of(src, l, c), Some(byte));
    }
}
