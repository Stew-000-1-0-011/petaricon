//! Minimal source edits on the syntax tree: only the touched bindings change,
//! everything else in the file is kept byte-for-byte.

use i_slint_compiler::diagnostics::BuildDiagnostics;
use i_slint_compiler::parser::{SyntaxKind, SyntaxNode, parse};
use std::path::Path;

fn find_element(root: &SyntaxNode, start: u32) -> Option<SyntaxNode> {
    root.descendants()
        .find(|n| n.kind() == SyntaxKind::Element && u32::from(n.text_range().start()) == start)
}

/// Returns the type name of the element's parent element if it is a layout.
pub fn parent_layout(source: &str, path: &Path, element_start: u32) -> Option<String> {
    let mut diag = BuildDiagnostics::default();
    let doc = parse(source.to_string(), Some(path), &mut diag);
    let element = find_element(&doc, element_start)?;
    // Element -> SubElement -> parent Element
    let parent = element.parent()?.parent()?;
    if parent.kind() != SyntaxKind::Element {
        return None;
    }
    let name = parent.child_node(SyntaxKind::QualifiedName)?.text().to_string();
    let name = name.trim().to_string();
    name.ends_with("Layout").then_some(name)
}

/// Sets (or inserts) `x` and `y` bindings on the element starting at `element_start`.
pub fn set_position(source: &str, path: &Path, element_start: u32, x: i32, y: i32) -> Option<String> {
    let mut diag = BuildDiagnostics::default();
    let doc = parse(source.to_string(), Some(path), &mut diag);
    let element = find_element(&doc, element_start)?;

    // (start, end, replacement), applied back to front.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut missing = Vec::new();
    for (name, value) in [("x", x), ("y", y)] {
        let binding = element.children().find(|b| {
            b.kind() == SyntaxKind::Binding
                && b.child_text(SyntaxKind::Identifier).as_deref() == Some(name)
        });
        match binding.and_then(|b| b.child_node(SyntaxKind::BindingExpression)) {
            Some(expr) => {
                let r = expr.text_range();
                edits.push((r.start().into(), r.end().into(), format!("{value}px;")));
            }
            None => missing.push(format!(" {name}: {value}px;")),
        }
    }
    if !missing.is_empty() {
        let lbrace = element.child_token(SyntaxKind::LBrace)?;
        let at: usize = lbrace.text_range().end().into();
        edits.push((at, at, missing.concat()));
    }

    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = source.to_string();
    for (s, e, text) in edits {
        out.replace_range(s..e, &text);
    }
    Some(out)
}
