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

pub mod cut;
pub mod partition;

use accreta_migrate::Migration;

/// Las migraciones de bilinker, en orden.
///
/// **El orden importa y no es el obvio.** La partición va primera: mientras
/// `range`, `state` y `resolved_at` sigan dentro del capture, no se le puede
/// calcular un id estable.
pub fn all() -> Vec<Migration> {
    vec![
        Migration {
            id:          "bilinker-002-file-partition",
            description: "reescribe los archivos a YAML y saca lo derivable del formato",
            run:         partition::run,
        },
    ]
}
