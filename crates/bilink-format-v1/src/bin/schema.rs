//! Emite el esquema JSON del formato por stdout.
//!
//! Es el artefacto que se publica con la release y el que un consumidor de la
//! frontera usa para validar antes de interpretar (ADR-0006, decisión 3).
//!
//! ```sh
//! cargo run -q -p bilink-format --bin schema > bilink-format-$(version).json
//! ```

fn main() {
    print!("{}", bilink_format_v1::schema_json());
}
