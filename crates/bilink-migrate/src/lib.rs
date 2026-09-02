//! Las migraciones del formato de bilinker.
//!
//! Viven acá y no en `bilinker` porque **dependen de los dos formatos que
//! puentean**, y `bilinker` depende de uno solo: el vigente. Cargar todos los
//! parsers históricos en el camino de lectura es lo que `concepts/migration.md`
//! descarta — funciona hasta el segundo cambio.
//!
//! # Determinismo
//!
//! Una migración es **una función pura de los archivos de entrada**. No consulta
//! git, no resuelve queries tree-sitter, no lee la hora. Correrla dos veces sobre
//! la misma entrada produce bytes idénticos, y por eso la carpeta de salida se
//! puede regenerar en cualquier momento — que es lo único que la vuelve segura.
//!
//! # El conjunto es de sólo-agregar
//!
//! Nunca se borra una migración, ni cuando parece que ya nadie está en ese formato.
//! Es lo único que permite que alguien parado en una versión vieja llegue a la
//! actual corriendo la cadena entera.

pub mod accepted_list;
pub mod cut;
pub mod partition;

use accreta_migrate::Migration;

/// Las migraciones de bilinker, en orden.
///
/// Hay una sola, y `bilinker-001-capture-split` no está: se retiró. Sigue en el
/// ledger de todos los repos que la corrieron —eso registra qué les pasó, no qué
/// sabe hacer este binario— pero su código ya no existe, porque `002` lee la
/// forma embebida además de la que `001` producía. Un repo que nunca corrió
/// `001` se migra igual, en un paso.
pub fn all() -> Vec<Migration> {
    vec![
        Migration {
            id:          "bilinker-002-file-partition",
            description: "reescribe los archivos a YAML y saca lo derivable del formato",
            run:         partition::run,
        },
        Migration {
            id:          "bilinker-003-accepted-list",
            description: "`accepted` pasa a lista, y un vecindario sin captures pasa a `declined`",
            run:         accepted_list::run,
        },
    ]
}
