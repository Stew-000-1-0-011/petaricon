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

pub(crate) fn components(root: &SyntaxNode) -> Vec<SyntaxNode> {
    root.descendants().filter(|n| n.kind() == SyntaxKind::Component).collect()
}

pub(crate) fn component_node(root: &SyntaxNode, name: &str) -> Option<SyntaxNode> {
    components(root).into_iter().find(|c| component_name(c) == name)
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
        SyntaxKind::RepeatedElement | SyntaxKind::ConditionalElement => {
            child.child_node(SyntaxKind::SubElement)?.child_node(SyntaxKind::Element)
        }
        _ => child.descendants().find(|n| n.kind() == SyntaxKind::Element),
    }
}

/// Resolves a path to its `Element` node.
pub(crate) fn resolve(root: &SyntaxNode, path: &ElementPath) -> Option<SyntaxNode> {
    let mut element = component_element(&component_node(root, &path.component)?)?;
    for &i in &path.indices {
        let child = element_children(&element).into_iter().nth(i)?;
        element = inner_element(&child)?;
    }
    Some(element)
}
