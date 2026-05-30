use std::path::Path;
use anyhow::{Context, Result};
use tree_sitter::{Node, Parser, Point};

use crate::git;
use crate::grammar::{self, stable_anchor_kinds};
use crate::hash;
use crate::link::StructuralRef;

pub struct CaptureResult {
    pub endpoint: StructuralRef,
    pub hash: String,
    pub commit: String,
}

pub fn capture(
    root: &Path,
    file: &str,
    start: (usize, usize), // (line, col) 1-based
    end: (usize, usize),
) -> Result<CaptureResult> {
    let commit = git::head_commit_for_file(root, file)?;
    let file_path = root.join(file);
    let source = std::fs::read_to_string(&file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;

    let lang = grammar::language_for_file(file);
    let language = grammar::for_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&language).context("set language")?;
    let tree = parser.parse(&source, None).context("parse failed")?;

    let start_point = Point { row: start.0 - 1, column: start.1 - 1 };
    let end_point   = Point { row: end.0 - 1,   column: end.1 - 1 };

    let root_node = tree.root_node();
    let node = root_node
        .named_descendant_for_point_range(start_point, end_point)
        .context("no named node at selection")?;

    let anchors = stable_anchor_kinds(lang);
    let target = walk_up_to_anchor(node, anchors).unwrap_or(node);

    let anchor = target.parent()
        .and_then(|p| walk_up_to_anchor(p, anchors));

    let query = match anchor {
        None => query_for_node(target, &source, &mut 0),
        Some(a) if a.id() == target.id() => query_for_node(target, &source, &mut 0),
        Some(a) => {
            let path = build_path(a, target);
            query_from_path(&path, &source, &mut 0)
        }
    };

    let start_byte = byte_for_point(&source, start_point);
    let end_byte   = byte_for_point(&source, end_point);
    // Single-point selection → capture the whole anchor; use relative offsets only for real ranges.
    let range = if start_byte == end_byte {
        None
    } else if start_byte != target.start_byte() || end_byte != target.end_byte() {
        let rel_start = start_byte.saturating_sub(target.start_byte());
        let rel_end   = end_byte.saturating_sub(target.start_byte());
        Some(crate::link::ByteRange { start: rel_start, end: rel_end })
    } else {
        None
    };

    let (frag_start, frag_end) = match &range {
        Some(r) => (target.start_byte() + r.start, target.start_byte() + r.end),
        None    => (target.start_byte(), target.end_byte()),
    };
    let fragment = &source[frag_start..frag_end.min(source.len())];
    let hash = hash::sha256(fragment.as_bytes());

    Ok(CaptureResult {
        endpoint: StructuralRef {
            file: file.to_string(),
            query: Some(query),
            range,
        },
        hash,
        commit,
    })
}

fn build_path<'a>(ancestor: Node<'a>, descendant: Node<'a>) -> Vec<Node<'a>> {
    if ancestor.id() == descendant.id() {
        return vec![ancestor];
    }
    for i in 0..ancestor.child_count() {
        let child = ancestor.child(i).unwrap();
        if node_contains(child, descendant.id()) {
            let mut path = vec![ancestor];
            path.extend(build_path(child, descendant));
            return path;
        }
    }
    vec![ancestor]
}

fn node_contains(node: Node, target_id: usize) -> bool {
    if node.id() == target_id { return true; }
    for i in 0..node.child_count() {
        if node_contains(node.child(i).unwrap(), target_id) {
            return true;
        }
    }
    false
}

fn query_for_node(node: Node, source: &str, counter: &mut usize) -> String {
    let name_pred = real_name_predicate(node, source, counter);
    format!("({}{}) @target", node.kind(), name_pred)
}

fn query_from_path(path: &[Node], source: &str, counter: &mut usize) -> String {
    assert!(!path.is_empty());
    let node = path[0];
    let name_pred = real_name_predicate(node, source, counter);

    if path.len() == 1 {
        return format!("({}{}) @target", node.kind(), name_pred);
    }

    let next = path[1];
    let field = field_name_for_child(node, next.id())
        .map(|f| format!("{f}: "))
        .unwrap_or_default();

    let inner = query_from_path(&path[1..], source, counter);
    format!("({}{}\n  {field}{inner})", node.kind(), name_pred)
}

fn real_name_predicate(node: Node, source: &str, counter: &mut usize) -> String {
    let Some(name_child) = node.child_by_field_name("name") else {
        return String::new();
    };
    let name_type = name_child.kind();
    let name_text = &source[name_child.byte_range()];
    let cap = format!("@n{counter}");
    *counter += 1;
    format!("\n  name: ({name_type}) {cap} (#eq? {cap} \"{name_text}\")")
}

fn field_name_for_child<'a>(parent: Node<'a>, child_id: usize) -> Option<&'a str> {
    for i in 0..parent.child_count() as u32 {
        if let Some(c) = parent.child(i as usize) {
            if c.id() == child_id {
                return parent.field_name_for_child(i);
            }
        }
    }
    None
}

fn walk_up_to_anchor<'a>(node: Node<'a>, anchors: &[&str]) -> Option<Node<'a>> {
    let mut current = node;
    loop {
        if anchors.contains(&current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn byte_for_point(source: &str, point: Point) -> usize {
    let mut line = 0;
    for (i, c) in source.char_indices() {
        if line == point.row {
            return i + point.column.min(source.len() - i);
        }
        if c == '\n' {
            line += 1;
        }
    }
    source.len()
}
