use anyhow::{bail, Result};
use tree_sitter::Language;

pub fn language_for_file(file: &str) -> &'static str {
    match std::path::Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("java")         => "java",
        Some("rs")           => "rust",
        Some("yaml" | "yml") => "yaml",
        Some("md")           => "markdown",
        Some("ts" | "js")    => "typescript",
        Some("tsx" | "jsx")  => "tsx",
        _                    => "text",
    }
}

pub fn for_language(lang: &str) -> Result<Language> {
    match lang {
        "java"     => Ok(tree_sitter_java::LANGUAGE.into()),
        "rust"     => Ok(tree_sitter_rust::LANGUAGE.into()),
        "yaml"     => Ok(tree_sitter_yaml::LANGUAGE.into()),
        "markdown"   => Ok(tree_sitter_md::LANGUAGE.into()),
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx"        => Ok(tree_sitter_typescript::LANGUAGE_TSX.into()),
        other        => bail!("unsupported language: '{other}' (supported: java, rust, yaml, markdown, typescript, tsx)"),
    }
}

/// ¿El AST de este lenguaje discrimina el contenido?
///
/// La s-expression de tree-sitter lleva **tipos de nodo, no texto**. En código eso
/// alcanza: reindentar no cambia el árbol, pero cambiar una expresión sí. En prosa
/// no: dos párrafos distintos bajo el mismo heading dan la misma s-expression, así
/// que `hash_ast` no distingue nada.
///
/// Donde devuelve `false` no se calcula `hash_ast`, y entonces `RESTYLED` no existe
/// y todo cambio de texto es `ALTERED` — que en prosa es lo correcto.
pub fn ast_discriminates_content(lang: &str) -> bool {
    !matches!(lang, "markdown" | "text")
}

/// Cómo llama esta gramática al campo que lleva **el cuerpo** de una declaración.
///
/// Es todo lo que `--as interface` necesita saber: la firma es el nodo menos ese
/// campo. No es conocimiento de framework —es de la gramática, y la gramática ya es
/// una dependencia— y es una tabla de la misma clase que [`stable_anchor_kinds`].
///
/// **Existe para que un lenguaje que no está falle en vez de adivinar.** Que hoy
/// todas las entradas digan `body` no la vuelve superflua: lo que la tabla decide no
/// es cómo se llama el campo sino de qué lenguajes bilinker puede afirmarlo.
pub fn body_field(lang: &str) -> Option<&'static str> {
    match lang {
        "java" | "rust" | "typescript" | "tsx" => Some("body"),
        _ => None,
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
        "rust" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            // Una constante es una declaración con nombre propio como cualquier otra,
            // y en un crate de formato suele ser el dato que la spec describe: una
            // tabla de prefijos, un registro de versiones.
            "const_item",
            "static_item",
        ],
        "yaml" => &[
            "block_sequence_item",
            "block_mapping_pair",
        ],
        "markdown" => &[
            "section",
            // Una fila de tabla no tiene nombre propio, pero su primera celda la
            // discrimina — igual que el `id:` de un item de secuencia YAML. Sin
            // ella, capturar una fila obliga a un rango de bytes dentro de la
            // sección, que se corre con cualquier fila agregada más arriba.
            "pipe_table_row",
        ],
        "typescript" | "tsx" => &[
            "class_declaration",
            "abstract_class_declaration",
            "function_declaration",
            "generator_function_declaration",
            "enum_declaration",
            "interface_declaration",
            "type_alias_declaration",
            "method_definition",
            "method_signature",
        ],
        _ => &[],
    }
}

/// Los node kinds que **llevan una firma resoluble**: un callable con tipo de
/// retorno y parámetros.
///
/// Es lo que separa *"este fragmento no tiene vecindario"* de *"nadie pudo mirarlo"*,
/// y se contesta con la gramática y sin proveedor — que es lo que permite que el
/// aviso de `accept` sea preciso en vez de ruido. Ver `concepts/accept.md`
/// § "Cuándo se adquiere el vecindario".
///
/// **Una clase o un DTO no están acá**, y es deliberado: su declaración no menciona
/// tipos del modo en que los menciona una firma. Es el mismo corte que hace la spec
/// cuando enumera dónde `n1` está legítimamente ausente.
pub fn signature_kinds(lang: &str) -> &'static [&'static str] {
    match lang {
        "java" => &["method_declaration", "constructor_declaration"],
        "rust" => &["function_item", "function_signature_item"],
        "typescript" | "tsx" => &[
            "function_declaration",
            "generator_function_declaration",
            "method_definition",
            "method_signature",
        ],
        // Prosa, YAML, TOML, texto: no hay firma que resolver.
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
        ("rust", "function_item") => Some("name"),
        ("rust", "struct_item")   => Some("name"),
        ("rust", "enum_item")     => Some("name"),
        ("rust", "trait_item")    => Some("name"),
        ("rust", "mod_item")      => Some("name"),
        ("rust", "impl_item")     => Some("type"),
        ("rust", "const_item")    => Some("name"),
        ("rust", "static_item")   => Some("name"),
        ("typescript" | "tsx", "class_declaration")          => Some("name"),
        ("typescript" | "tsx", "abstract_class_declaration") => Some("name"),
        ("typescript" | "tsx", "function_declaration")       => Some("name"),
        ("typescript" | "tsx", "generator_function_declaration") => Some("name"),
        ("typescript" | "tsx", "enum_declaration")           => Some("name"),
        ("typescript" | "tsx", "interface_declaration")      => Some("name"),
        ("typescript" | "tsx", "type_alias_declaration")     => Some("name"),
        ("typescript" | "tsx", "method_definition")          => Some("name"),
        ("typescript" | "tsx", "method_signature")           => Some("name"),
        _ => None,
    }
}

/// Returns the tree-sitter node kind used for the name of a given declaration kind.
/// In Java, class/interface names use `type_identifier`; methods use `identifier`.
pub fn name_node_type(lang: &str, kind: &str) -> &'static str {
    match (lang, kind) {
        ("typescript" | "tsx", "class_declaration")
        | ("typescript" | "tsx", "abstract_class_declaration")
        | ("typescript" | "tsx", "interface_declaration")
        | ("typescript" | "tsx", "type_alias_declaration") => "type_identifier",
        ("typescript" | "tsx", "method_definition")
        | ("typescript" | "tsx", "method_signature")       => "property_identifier",
        _ => "identifier",
    }
}

#[cfg(test)]
mod ast_tests {
    use super::*;

    /// En prosa el AST no distingue contenido, así que no se hashea.
    ///
    /// La s-expression lleva tipos de nodo y no texto: dos párrafos distintos bajo
    /// el mismo heading dan el mismo árbol. Calcular `hash_ast` ahí haría que
    /// cualquier reescritura de prosa se reportara como RESTYLED —"sólo cambió el
    /// formato"— cuando lo que cambió es lo que el documento dice.
    #[test]
    fn prose_does_not_get_an_ast_hash() {
        assert!(!ast_discriminates_content("markdown"));
        assert!(!ast_discriminates_content("text"));
    }

    /// En código sí: reindentar no cambia el árbol, cambiar una expresión sí.
    #[test]
    fn code_gets_an_ast_hash() {
        for lang in ["rust", "java", "typescript", "tsx", "yaml"] {
            assert!(ast_discriminates_content(lang), "{lang} debería discriminar");
        }
    }
}
