//! El esquema JSON del formato, generado desde los tipos.
//!
//! La dirección es Rust → esquema y no al revés (ADR-0006, decisión 3): los tipos
//! con serde son la fuente, y el esquema sale generado. Se publica como artefacto
//! de la release para que un consumidor de la frontera valide antes de interpretar
//! **sin adoptar bilinker**, con cualquier validador de JSON Schema.

use schemars::{schema_for, Schema};
use serde_json::json;

use crate::bilink::BiLinkFile;
use crate::capture::CaptureFile;

/// El esquema de los dos archivos del formato, bajo `$defs`.
///
/// Un solo documento y no dos: los dos archivos comparten tipos —`LinkEndpoint`,
/// `ByteRange`— y separarlos obligaría a publicar los comunes dos veces o a
/// referenciarlos entre documentos.
pub fn schema() -> Schema {
    let bilink: Schema  = schema_for!(BiLinkFile);
    let capture: Schema = schema_for!(CaptureFile);
    Schema::try_from(json!({
        "$schema":     "https://json-schema.org/draft/2020-12/schema",
        "title":       "bilink-format",
        "description": "Formato de los archivos .bilink y .capture de bilinker.",
        "version":     crate::VERSION,
        "$defs": {
            "BiLinkFile":  bilink,
            "CaptureFile": capture,
        }
    }))
    .expect("el esquema generado es un objeto JSON")
}

/// El esquema como el texto que se publica: JSON indentado, con salto final.
///
/// Es lo que se hashea, así que la representación tiene que ser una sola. Si el
/// hash se calculara sobre el `Schema` en memoria, dos formas de imprimirlo darían
/// dos hashes para el mismo esquema.
pub fn schema_json() -> String {
    let mut out = serde_json::to_string_pretty(&schema())
        .expect("un esquema siempre serializa");
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
/// El esquema lleva su propia versión adentro, así que el hash certifica las dos
/// cosas a la vez: qué tipos describe y bajo qué nombre se publicó.
pub const SCHEMA_HASHES: &[(&str, &str)] = &[
    ("0.1.0", "21e2603f57bac65a1129ed4b1a4f4726d9e54997a459321d6393ec8aa43d274e"),
];

/// El hash registrado para una versión, si está registrada.
pub fn registered_hash(version: &str) -> Option<&'static str> {
    SCHEMA_HASHES.iter().find(|(v, _)| *v == version).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El esquema describe los dos archivos y se identifica con la versión.
    #[test]
    fn the_schema_covers_both_files() {
        let s = schema_json();
        assert!(s.contains("\"BiLinkFile\""),  "falta el .bilink:\n{s}");
        assert!(s.contains("\"CaptureFile\""), "falta el .capture:\n{s}");
        assert!(s.contains(crate::VERSION),    "el esquema no dice su versión:\n{s}");
    }

    /// Un endpoint es un string en el esquema, igual que en el archivo.
    #[test]
    fn an_endpoint_is_a_string_in_the_schema() {
        let s = schema_json();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        let ep = &v["$defs"]["BiLinkFile"]["$defs"]["LinkEndpoint"];
        assert_eq!(ep["type"], "string", "el endpoint debería ser un string:\n{ep:#}");
    }

    /// Generar dos veces da el mismo texto — si no, el hash no serviría de guarda.
    #[test]
    fn the_schema_is_deterministic() {
        assert_eq!(schema_json(), schema_json());
    }

    /// **El guard.** Cambiar los tipos sin subir la versión falla acá.
    ///
    /// Es lo que convierte `.bilink/version` de una promesa en una propiedad del
    /// artefacto: no hay forma de cambiar el formato y olvidarse de decirlo. El caso
    /// que lo motiva es un cambio **aditivo** —ADR-0005 agrega endpoints sin
    /// migración— que un parser viejo leería mal sin fallar y que nadie recordaría
    /// bumpear.
    #[test]
    fn the_schema_hash_matches_the_registered_version() {
        use sha2::{Digest, Sha256};
        let actual = hex::encode(Sha256::digest(schema_json().as_bytes()));

        let Some(expected) = registered_hash(crate::VERSION) else {
            panic!(
                "la versión de formato {} no está registrada.\n\
                 Si el cambio es deliberado, agregar a SCHEMA_HASHES:\n    \
                 (\"{}\", \"{actual}\"),",
                crate::VERSION, crate::VERSION
            );
        };

        assert_eq!(
            actual, expected,
            "\nel esquema cambió y la versión de formato sigue en {}.\n\
             Subir la versión en crates/bilink-format/Cargo.toml y registrar el hash \
             nuevo en SCHEMA_HASHES.\n\
             Si el cambio no era deliberado, revisar qué tipo se tocó.\n",
            crate::VERSION
        );
    }
}
