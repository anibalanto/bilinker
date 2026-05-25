use anyhow::{bail, Result};
use tree_sitter::Language;

pub fn language_for_file(file: &str) -> &'static str {
    match std::path::Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("java") => "java",
        Some("rs")   => "rust",
        _            => "text",
    }
}

pub fn for_language(lang: &str) -> Result<Language> {
    match lang {
        "java" => Ok(tree_sitter_java::language()),
        other => bail!("unsupported language: '{other}' (supported: java)"),
    }
}

/// Node kinds that are considered stable anchors for a given language.
/// A stable anchor is a named declaration that identifies itself (class, method, etc.).
pub fn stable_anchor_kinds(lang: &str) -> &'static [&'static str] {
    match lang {
        "java" => &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "method_declaration",
            "constructor_declaration",
            "field_declaration",
        ],
        _ => &[],
    }
}

/// Returns the field name that holds the "name" identifier for a given node kind.
pub fn name_field(lang: &str, kind: &str) -> Option<&'static str> {
    match (lang, kind) {
        ("java", "class_declaration")       => Some("name"),
        ("java", "interface_declaration")   => Some("name"),
        ("java", "enum_declaration")        => Some("name"),
        ("java", "method_declaration")      => Some("name"),
        ("java", "constructor_declaration") => Some("name"),
        _ => None,
    }
}

/// Returns the tree-sitter node kind used for the name of a given declaration kind.
/// In Java, class/interface names use `type_identifier`; methods use `identifier`.
pub fn name_node_type(lang: &str, _kind: &str) -> &'static str {
    match lang {
        "java" => "identifier",
        _      => "identifier",
    }
}
