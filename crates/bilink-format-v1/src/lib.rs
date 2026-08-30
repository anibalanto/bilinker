//! **Formato 1, congelado.** El formato de texto plano `clave: valor`, anterior al
//! YAML de ADR-0003.
//!
//! Existe sólo para que `bilinker-002-file-partition` pueda leerlo. No se usa en
//! ningún camino de lectura del día a día: eso sería cargar toda la historia del
//! formato en cada lectura, que es lo que `concepts/migration.md` descarta.
//!
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
