//! El formato de los archivos de bilinker: `<uuid>.yaml` y `capture/<id>.yaml`.
//!
//! Sólo los tipos y su serialización. No resuelve queries, no consulta git y no
//! sabe qué es un estado válido para un fragmento — eso lo hace `bilinker`, que
//! depende de este crate como cualquier otro consumidor.
//!
//! **La versión del crate es la versión del formato** (ADR-0006): cambiar los tipos
//! obliga a releasear, y releasear obliga a bumpear, así que `.bilink/version` deja
//! de ser una promesa y pasa a ser una propiedad del artefacto.

pub mod bilink;
pub mod capture;
pub mod ignore;
pub mod link;
pub mod schema;
pub mod version;

pub use bilink::{Accepted, BiLink, CaptureSet, DeclaredLevel, DeclaredN, DeclinedMark, Endpoint, Endpoints, Neighbourhood, N};
pub use capture::Capture;
pub use ignore::write_ignore;
pub use link::{ByteRange, LinkEndpoint, Ranges, ENDPOINT_PREFIXES, FRAGMENT_SEPARATOR};
pub use schema::{schema, schema_json};
pub use version::{ensure_layer, ensure_readable, ensure_version, major, read_version, write_version, Mismatch, VERSION_FILE};

/// La versión del formato, tomada del `Cargo.toml` de este crate.
///
/// No se declara a mano en ningún otro lado: cualquier otra copia podría quedar
/// desincronizada, y ésta no puede.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
