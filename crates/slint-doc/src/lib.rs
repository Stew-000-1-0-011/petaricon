//! Read model of a `.slint` document: components, element tree and element info.

mod syntax;

use i_slint_compiler::parser::{SyntaxKind, SyntaxNode};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ElementPath {
    pub component: String,
    /// Indices into "element children"; empty means the component root.
    pub indices: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    Horizontal,
    Vertical,
    Grid,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementKind {
    Plain,
    Layout(LayoutKind),
    Repeated,
    Conditional,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub expr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementInfo {
    pub path: ElementPath,
    pub type_name: String,
    pub id: Option<String>,
    pub kind: ElementKind,
    pub bindings: Vec<Binding>,
    pub child_count: usize,
    pub start: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub info: ElementInfo,
    pub children: Vec<TreeNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentInfo {
    pub name: String,
    pub base: String,
    pub exported: bool,
    pub declarations: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Document {
    path: PathBuf,
    text: String,
}

fn type_name(element: &SyntaxNode) -> String {
    element
        .child_node(SyntaxKind::QualifiedName)
        .map(|n| n.text().to_string().trim().to_string())
        .unwrap_or_default()
}

fn layout_kind(type_name: &str) -> Option<LayoutKind> {
    let base = type_name.rsplit('.').next().unwrap_or(type_name);
    match base {
        "HorizontalLayout" | "HorizontalBox" => Some(LayoutKind::Horizontal),
        "VerticalLayout" | "VerticalBox" => Some(LayoutKind::Vertical),
        "GridLayout" | "GridBox" => Some(LayoutKind::Grid),
        _ if base.ends_with("Layout") => Some(LayoutKind::Other),
        _ => None,
    }
}

fn binding_infos(element: &SyntaxNode) -> Vec<Binding> {
    element
        .children()
        .filter(|b| b.kind() == SyntaxKind::Binding)
        .filter_map(|b| {
            let name = b.child_text(SyntaxKind::Identifier)?;
            let expr = b.child_node(SyntaxKind::BindingExpression)?.text().to_string();
            let expr = expr.trim();
            let expr = expr.strip_suffix(';').unwrap_or(expr).trim_end();
            Some(Binding { name: name.to_string(), expr: expr.to_string() })
        })
        .collect()
}

fn element_info(path: ElementPath, child: Option<&SyntaxNode>, element: &SyntaxNode) -> ElementInfo {
    let type_name = type_name(element);
    let kind = match child.map(|c| c.kind()) {
        Some(SyntaxKind::RepeatedElement) => ElementKind::Repeated,
        Some(SyntaxKind::ConditionalElement | SyntaxKind::MatchElement) => ElementKind::Conditional,
        _ => layout_kind(&type_name).map_or(ElementKind::Plain, ElementKind::Layout),
    };
    let id = element
        .parent()
        .filter(|p| p.kind() == SyntaxKind::SubElement)
        .and_then(|p| p.child_text(SyntaxKind::Identifier))
        .map(|s| s.to_string());
    ElementInfo {
        path,
        type_name,
        id,
        kind,
        bindings: binding_infos(element),
        child_count: syntax::element_children(element).len(),
        start: element.text_range().start().into(),
    }
}

fn build_tree(path: ElementPath, child: Option<&SyntaxNode>, element: &SyntaxNode) -> TreeNode {
    let info = element_info(path.clone(), child, element);
    let children = syntax::element_children(element)
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let inner = syntax::inner_element(c)?;
            let mut indices = path.indices.clone();
            indices.push(i);
            Some(build_tree(ElementPath { component: path.component.clone(), indices }, Some(c), &inner))
        })
        .collect();
    TreeNode { info, children }
}

impl Document {
    pub fn new(path: PathBuf, text: String) -> Self {
        Self { path, text }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn parse(&self) -> SyntaxNode {
        syntax::parse(&self.text, &self.path)
    }

    pub fn components(&self) -> Vec<ComponentInfo> {
        let root = self.parse();
        syntax::components(&root)
            .iter()
            .map(|c| {
                let element = syntax::component_element(c);
                ComponentInfo {
                    name: syntax::component_name(c),
                    base: element.as_ref().map(type_name).unwrap_or_default(),
                    exported: c.parent().is_some_and(|p| p.kind() == SyntaxKind::ExportsList),
                    declarations: element
                        .iter()
                        .flat_map(|e| e.children())
                        .filter(|n| {
                            matches!(
                                n.kind(),
                                SyntaxKind::PropertyDeclaration | SyntaxKind::CallbackDeclaration
                            )
                        })
                        .map(|n| n.text().to_string().trim().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    pub fn tree(&self, component: &str) -> Option<TreeNode> {
        let root = self.parse();
        let element = syntax::component_element(&syntax::component_node(&root, component)?)?;
        Some(build_tree(ElementPath { component: component.to_string(), indices: vec![] }, None, &element))
    }

    pub fn element(&self, path: &ElementPath) -> Option<ElementInfo> {
        let root = self.parse();
        let element = syntax::resolve(&root, path)?;
        let child = element.parent().filter(|p| p.kind() != SyntaxKind::Component);
        // For Repeated/Conditional the Element sits in a SubElement inside the wrapper.
        let child = child.map(|p| match p.parent() {
            Some(w) if matches!(w.kind(), SyntaxKind::RepeatedElement | SyntaxKind::ConditionalElement) => w,
            _ => p,
        });
        Some(element_info(path.clone(), child.as_ref(), &element))
    }

    pub fn path_at_offset(&self, offset: u32) -> Option<ElementPath> {
        let root = self.parse();
        for component in syntax::components(&root) {
            let name = syntax::component_name(&component);
            let element = syntax::component_element(&component)?;
            if let Some(indices) = find(&element, offset) {
                return Some(ElementPath { component: name, indices });
            }
        }
        None
    }
}

fn find(element: &SyntaxNode, offset: u32) -> Option<Vec<usize>> {
    if u32::from(element.text_range().start()) == offset {
        return Some(vec![]);
    }
    for (i, c) in syntax::element_children(element).iter().enumerate() {
        if let Some(inner) = syntax::inner_element(c) {
            if let Some(mut rest) = find(&inner, offset) {
                rest.insert(0, i);
                return Some(rest);
            }
        }
    }
    None
}
