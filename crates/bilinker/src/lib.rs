//! Bilinker: verificación y mantenimiento de bilinks.
//!
//! El **formato** de los archivos vive aparte, en `bilink-format`. Acá está todo lo
//! que los interpreta: resolución tree-sitter, estados, git. Los dos módulos del
//! formato se re-exportan para que el resto del crate y sus consumidores sigan
//! diciendo `bilinker::link` y `bilinker::bilink`.
pub use bilink_format::{bilink, link, VERSION as FORMAT_VERSION};

pub mod accept;
pub mod apply;
pub mod bilink_ref;
pub mod cache;
pub mod capture;
pub mod chain;
pub mod check;
pub mod config;
pub mod get;
pub mod git;
pub mod grammar;
pub mod hash;
pub mod index;
pub mod init;
pub mod issue;
pub mod query;
pub mod state;
pub mod sync;
pub mod track;
