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
    // 3.0.0 saca el `offset` del capture: un fragmento deja de ser un rango adentro
    // de un nodo. Sacar un campo no es aditivo, así que sube el major — y con él se
    // va `DISPLACED`, el único estado que hablaba de un sub-rango.
    ("3.0.0", "faf08a34795e9915d0e9dc7b0c881936e8446f1c027849bb8f9e6ae5702db9e0"),
    // 3.1.0 agrega los endpoints `repo <alias>` y `abstract` — la frontera entre
    // proyectos. **Es aditivo y sube la versión igual**, que es lo contraintuitivo
    // y la razón de fondo de que este registro exista además del ledger: ningún
    // archivo existente los usa, no hubo migración que registrar, y sin embargo un
    // parser de 3.0.0 leería `abstract` como un path de capa — en silencio y sin
    // fallar. Entre proyectos con releases independientes, ése es *el* modo de
    // falla que importa.
    ("3.1.0", "70d09000aaf76f14664c49e71a4307b7f8dd0077712683d08c3e4d32175f134e"),
    // 3.2.0 agrega `accepted.agree`: quiénes aprobaron **estos** valores. Aditivo
    // —ningún archivo existente lo lleva, y su ausencia es un set vacío— y sube la
    // versión igual, por el mismo motivo que 3.1.0: un parser de 3.1.0 tiene
    // `deny_unknown_fields` sobre `Accepted`, así que un archivo con `agree` no le
    // falla en silencio, le falla y basta. La versión es lo que le permite decir
    // **por qué**.
    ("3.2.0", "32ccee2d79db3f2657ed4137d2088bbf7b2508eba95f7ec1ec013f54b309e2d8"),
    // 3.3.0: la `query` de un capture puede llevar **varios** `@target`, y el
    // fragmento pasa a ser su concatenación unida por `\n`. Los tipos no cambian
    // —`query` sigue siendo un string— y el archivo tampoco, así que no hay
    // migración: un capture de un solo `@target` hashea exactamente lo que hasheaba.
    //
    // **Sube igual, y por el motivo de siempre**: un parser de 3.2.0 lee una query
    // de tres `@target`, se queda con el primero, y hashea otro fragmento — en
    // silencio y sin fallar. Es el mismo modo de falla que 3.1.0, con el agravante
    // de que acá el esquema no puede describirlo: lo que discrimina está adentro de
    // un string, y ningún tipo lo hace visible. La versión es lo único que queda.
    ("3.3.0", "58e2f5f9259afc1fee7d15911067ac246fabb439c7a89bdf302ba3312cca82ae"),
    // 3.4.0 agrega `accepted.hash_n1` y `accepted.hash_ast_n1`: el vecindario de la
    // firma —los tipos que menciona, un salto— plegado en dos hashes.
    //
    // **Aditivo, y sube igual**, por el motivo de 3.1.0: ningún archivo existente los
    // lleva y su ausencia significa "este endpoint no tiene vecindario aceptado",
    // así que no hay migración. Lo que un parser de 3.3.0 no puede hacer es
    // *conservarlos*: `Accepted` tiene `deny_unknown_fields`, así que le falla y le
    // falla explícito — la versión es lo que le permite decir por qué.
    ("3.4.0", "d9f899c1df0be5b3b18691ed67ced79d9b82f31ecacbf8728df0c0c6c28083db"),
    // 3.5.0 agrega `accepted.n1`, con un solo valor: `declined`. Dice que alguien
    // **renunció** al vecindario, y no que el fragmento no tenga uno.
    //
    // **Aditivo, y sube igual.** Un parser de 3.4.0 no puede leer el campo —
    // `deny_unknown_fields`— así que falla explícito y la versión le dice por qué.
    // Lo que no puede pasar es que lo ignore: leería la renuncia como *"este
    // fragmento no tiene firma resoluble"*, que es la confusión que el campo existe
    // para cerrar.
    //
    // No hay migración: ningún archivo existente lo lleva, y su ausencia sigue
    // significando lo que significaba.
    ("3.5.0", "685a031d0ef3c40f6af7549f092bb9a244b2c17f9b514ee215ecb02d7d3ccb3c"),
    // 3.6.0 pliega el vecindario en **un** campo con tres estados: `n1` adquirido
    // —`{hash, hash_ast}`—, `n1: declined`, o ausente. Reemplaza a `hash_n1`,
    // `hash_ast_n1` y la marca suelta que 3.5.0 había agregado.
    //
    // **No es aditivo, y aun así no hay migración**: al 2026-09-01 no existe un solo
    // archivo con `hash_n1` escrito. Se verificó sobre `.bilink/` de accreta y de las
    // cinco capas impl, y la razón de fondo es que escribirlo pide un language server
    // corriendo, que todavía no pasó en ningún repo. La ventana se cierra sola el día
    // que alguien acepte con el daemon prendido — de ahí que el cambio de forma vaya
    // ahora y no después.
    ("3.6.0", "cb3afb41005b3a860b6f7595af3978fcdd08e83c5e0fd55bf46100d0ac432366"),
    // 3.7.0 mete los niveles adentro del campo: `n: {1: {hash, hash_ast}}`, y la
    // renuncia pasa al contenedor — `n: declined` en vez de `n1: declined`.
    //
    // **La renuncia es del vecindario entero**, del nivel 1 para arriba, y escrita
    // adentro del nivel 1 decía otra cosa. Con un solo nivel las dos frases coinciden;
    // con dos, la de adentro obliga a preguntar qué pasa con el 2.
    //
    // Sigue sin haber migración, y por tercera y última vez por el mismo motivo: no
    // existe un solo archivo con vecindario escrito. La ventana se cierra el día que
    // alguien acepte con un language server prendido.
    ("3.7.0", "e1e221f6b62f0b60aadcf7deef97b7134e7795823c99256f10b7f077548df0f1"),
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
