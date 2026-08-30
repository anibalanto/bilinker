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
/// Formato 1. Ya no se escribe, pero se sigue leyendo, y leerlo mejor cambia el
/// esquema: `1.1.0` agrega `kind` y `name.N`, que estaban en el formato desde
/// siempre y este lector no modelaba. La migración decía preservarlos y no podía,
/// porque nadie se los pasaba.
///
/// `1.0.0` se queda. La regla es de sólo-agregar justamente para esto: describe
/// lo que se publicó bajo ese número, y corregirlo en el lugar reescribiría el
/// pasado. Que el comentario anterior dijera "nunca va a tener otra entrada" era
/// una predicción, no una invariante.
///
/// El `0.1.0` que este crate llevó mientras se lo escribía no está: nombraba el
/// mismo formato con otro número y no se publicó nunca. La regla protege lo que
/// alguien pudo haber bajado, no los números que no salieron.
pub const SCHEMA_HASHES: &[(&str, &str)] = &[
    ("1.0.0", "0be624dfa3d2fc1ca6bb5e5d662cac0300fead3036f7ab570973ecd874425bc0"),
    ("1.1.0", "ec223a1a22c52f0545bbea0affb6060bb77dc5e3738f6041062ccc36e50c96ea"),
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
