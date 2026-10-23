//! Thin helpers over the Slint syntax tree, shared by reads and edits.

use crate::ElementPath;
use i_slint_compiler::diagnostics::BuildDiagnostics;
use i_slint_compiler::parser::{SyntaxKind, SyntaxNode};
use std::path::Path;

pub(crate) fn parse(text: &str, path: &Path) -> SyntaxNode {
    let mut diag = BuildDiagnostics::default();
    i_slint_compiler::parser::parse(text.to_string(), Some(path), &mut diag)
}

pub(crate) fn component_name(component: &SyntaxNode) -> String {
    component
        .child_node(SyntaxKind::DeclaredIdentifier)
        .map(|n| n.text().to_string().trim().to_string())
        .unwrap_or_default()
}

/// `global Name { .. }`: the node starts with the keyword token.
pub(crate) fn is_global(component: &SyntaxNode) -> bool {
    component
        .child_token(SyntaxKind::Identifier)
        .is_some_and(|t| t.text() == "global")
}

pub(crate) fn components(root: &SyntaxNode) -> Vec<SyntaxNode> {
    root.descendants()
        .filter(|n| n.kind() == SyntaxKind::Component)
        .collect()
}

pub(crate) fn component_node(root: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    components(root)
        .into_iter()
        .find(|c| component_name(c) == name)
}

pub(crate) fn component_element(component: &SyntaxNode) -> Option<SyntaxNode> {
    component.child_node(SyntaxKind::Element)
}

/// The "children of an element": its Sub/Repeated/Conditional/Match nodes in source order.
pub(crate) fn element_children(element: &SyntaxNode) -> Vec<SyntaxNode> {
    element
        .children()
        .filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::SubElement
                    | SyntaxKind::RepeatedElement
                    | SyntaxKind::ConditionalElement
                    | SyntaxKind::MatchElement
            )
        })
        .collect()
}

/// The `Element` inside a child node (for `MatchElement`, the first element of its first case).
pub(crate) fn inner_element(child: &SyntaxNode) -> Option<SyntaxNode> {
    match child.kind() {
        SyntaxKind::SubElement => child.child_node(SyntaxKind::Element),
        SyntaxKind::RepeatedElement | SyntaxKind::ConditionalElement => child
            .child_node(SyntaxKind::SubElement)?
            .child_node(SyntaxKind::Element),
        _ => child
            .descendants()
            .find(|n| n.kind() == SyntaxKind::Element),
    }
}

/// Resolves a path to its `Element` node and the child node wrapping it
/// (`None` for the component root).
pub(crate) fn resolve(
    root: &SyntaxNode,
    path: &ElementPath,
) -> Option<(Option<SyntaxNode>, SyntaxNode)> {
    let mut element = component_element(&component_node(root, &path.component)?)?;
    let mut wrapper = None;
    for &i in &path.indices {
        let child = element_children(&element).into_iter().nth(i)?;
        element = inner_element(&child)?;
        wrapper = Some(child);
    }
    Some((wrapper, element))
}

/// Leading whitespace of the line containing `pos`.
pub(crate) fn line_indent(text: &str, pos: usize) -> String {
    let ls = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    text[ls..]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect()
}

/// Indentation of the line an element starts on.
pub(crate) fn element_indent(text: &str, element: &SyntaxNode) -> String {
    line_indent(text, usize::from(element.text_range().start()))
}

/// Indentation for children of `element`: that of its first child, or the element's own
/// indentation plus four spaces when it has none.
pub(crate) fn child_indent(text: &str, element: &SyntaxNode) -> String {
    match element_children(element).first() {
        Some(child) => line_indent(text, usize::from(child.text_range().start())),
        None => format!("{}    ", element_indent(text, element)),
    }
}
