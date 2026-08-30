//! El esquema JSON del formato, generado desde los tipos.
//!
//! La dirección es Rust → esquema y no al revés (ADR-0006): los tipos con serde son
//! la fuente. Se publica como artefacto de la release para que un consumidor de la
//! frontera valide antes de interpretar **sin adoptar bilinker**.

use schemars::{schema_for, Schema};
use serde_json::json;

use crate::bilink::BiLink;
use crate::capture::Capture;

/// El esquema de los dos archivos del formato, bajo `$defs`.
///
/// Un solo documento y no dos: comparten tipos —`LinkEndpoint`, `ByteRange`— y
/// separarlos obligaría a publicar los comunes dos veces.
pub fn schema() -> Schema {
    Schema::try_from(json!({
        "$schema":     "https://json-schema.org/draft/2020-12/schema",
        "title":       "bilink-format",
        "description": "Formato de los archivos de bilinker: <uuid>.yaml y capture/<id>.yaml.",
        "version":     crate::VERSION,
        "$defs": {
            "BiLink":  schema_for!(BiLink),
            "Capture": schema_for!(Capture),
        }
    }))
    .expect("el esquema generado es un objeto JSON")
}

/// El esquema como el texto que se publica: JSON indentado, con salto final.
///
/// Es lo que se hashea, así que la representación tiene que ser una sola.
pub fn schema_json() -> String {
    let mut out = serde_json::to_string_pretty(&schema()).expect("un esquema siempre serializa");
    out.push('\n');
    out
}

// ─── el guard de versión ──────────────────────────────────────────────────────

/// El hash del esquema publicado de cada versión de formato.
///
/// **Es de sólo-agregar.** Una entrada registra lo que se publicó bajo esa versión;
/// corregirla en vez de agregar una nueva reescribiría el pasado, y el hash dejaría
/// de certificar el artefacto que alguien ya bajó.
///
/// La regla protege **lo publicado**. Mientras una versión no salió —no hay release,
/// nadie la bajó— su entrada todavía se está escribiendo y corregirla no reescribe
/// nada. La línea es la publicación, no el número.
pub const SCHEMA_HASHES: &[(&str, &str)] = &[
    ("2.0.0", "39396775ea75eec5ac760b51a450a4315b2d1a8275860daedf7e4468e5acc21f"),
];

pub fn registered_hash(version: &str) -> Option<&'static str> {
    SCHEMA_HASHES.iter().find(|(v, _)| *v == version).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_covers_both_files() {
        let s = schema_json();
        assert!(s.contains("\"BiLink\""),  "falta el bilink:\n{s}");
        assert!(s.contains("\"Capture\""), "falta el capture:\n{s}");
        assert!(s.contains(crate::VERSION), "el esquema no dice su versión");
    }

    /// Los prefijos reconocidos se publican: agregar un tipo de endpoint cambia el
    /// esquema, y el guard lo detecta. Con `{"type": "string"}` a secas no lo haría.
    #[test]
    fn the_endpoint_publishes_its_prefixes() {
        let v: serde_json::Value = serde_json::from_str(&schema_json()).unwrap();
        let ep = &v["$defs"]["BiLink"]["$defs"]["LinkEndpoint"];
        assert_eq!(ep["type"], "string");
        let listed: Vec<&str> = ep["prefixes"].as_array().unwrap()
            .iter().map(|x| x.as_str().unwrap()).collect();
        assert_eq!(listed, crate::ENDPOINT_PREFIXES);
    }

    #[test]
    fn the_schema_is_deterministic() {
        assert_eq!(schema_json(), schema_json());
    }

    /// **El guard.** Cambiar los tipos sin subir la versión falla acá.
    #[test]
    fn the_schema_hash_matches_the_registered_version() {
        use sha2::{Digest, Sha256};
        let actual = hex::encode(Sha256::digest(schema_json().as_bytes()));

        let Some(expected) = registered_hash(crate::VERSION) else {
            panic!("la versión de formato {} no está registrada.\n\
                    Agregar a SCHEMA_HASHES:\n    (\"{}\", \"{actual}\"),",
                   crate::VERSION, crate::VERSION);
        };
        assert_eq!(actual, expected,
            "\nel esquema cambió y la versión de formato sigue en {}.\n\
             Subir la versión en crates/bilink-format/Cargo.toml y registrar el hash nuevo.\n",
            crate::VERSION);
    }
}
