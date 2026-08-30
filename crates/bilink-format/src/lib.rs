//! El formato de los archivos de bilinker: `.bilink` y `.capture`.
//!
//! Sólo los tipos y su serialización. No resuelve queries, no consulta git y no
//! sabe qué es un estado válido para un fragmento — eso lo hace `bilinker`, que
//! depende de este crate como cualquier otro consumidor.
//!
//! **La versión del crate es la versión del formato** (ADR-0006): cambiar los
//! tipos obliga a releasear, y releasear obliga a bumpear, así que
//! `.bilink/version` deja de ser una promesa y pasa a ser una propiedad del
//! artefacto.

pub mod bilink;
pub mod capture;
pub mod link;
pub mod schema;

pub use link::state_str;
pub use schema::{schema, schema_json};

/// La versión del formato, tomada del `Cargo.toml` de este crate.
///
/// No se declara a mano en ningún otro lado: cualquier otra copia podría quedar
/// desincronizada, y ésta no puede.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
