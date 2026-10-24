//! Read model of a `.slint` document: components, element tree and element info.

mod history;
mod ops;
mod syntax;

pub use ops::{EditError, LayoutSpec, Op};

use i_slint_compiler::parser::{SyntaxKind, SyntaxNode};
use std::path::{Path, PathBuf};

/// Address of an element. Element children are the `SubElement` / `RepeatedElement` /
/// `ConditionalElement` / `MatchElement` nodes of an element in source order; for a
/// `match`, only the first case's element is addressable.
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

/// One `import { A, B as C } from "path";` (a bare `import "font.ttf";` is not listed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportInfo {
    /// The path as written, without quotes.
    pub path: String,
    /// `(name in the other file, name used here)`.
    pub names: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Document {
    path: PathBuf,
    text: String,
    history: history::History,
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
            let expr = b
                .child_node(SyntaxKind::BindingExpression)?
                .text()
                .to_string();
            let expr = expr.trim();
            let expr = expr.strip_suffix(';').unwrap_or(expr).trim_end();
            Some(Binding {
                name: name.to_string(),
                expr: expr.to_string(),
            })
        })
        .collect()
}

fn element_info(
    path: ElementPath,
    child: Option<&SyntaxNode>,
    element: &SyntaxNode,
) -> ElementInfo {
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
            Some(build_tree(
                ElementPath {
                    component: path.component.clone(),
                    indices,
                },
                Some(c),
                &inner,
            ))
        })
        .collect();
    TreeNode { info, children }
}

impl Document {
    pub fn new(path: PathBuf, text: String) -> Self {
        let history = history::History::new(text.clone());
        Self {
            path,
            text,
            history,
        }
    }

    /// Replaces the whole text (one undo step); unchanged text is not recorded.
    pub fn replace_text(&mut self, text: String) {
        self.set_text(text);
    }

    pub(crate) fn set_text(&mut self, text: String) {
        if self.history.push(text.clone()) {
            self.text = text;
        }
    }

    pub fn undo(&mut self) -> bool {
        match self.history.undo() {
            Some(t) => {
                self.text = t.to_string();
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.history.redo() {
            Some(t) => {
                self.text = t.to_string();
                true
            }
            None => false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Points the document at a new path (rename / save as); text and history are kept.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    /// The syntax errors of the current text (empty when it parses cleanly).
    pub fn syntax_errors(&self) -> Vec<String> {
        syntax::syntax_errors(&self.text, &self.path)
    }

    /// What `op` would turn the text into, without touching this document or its history.
    pub fn preview_apply(&self, op: Op) -> Result<String, EditError> {
        let mut scratch = Document::new(self.path.clone(), self.text.clone());
        scratch.apply(op)?;
        Ok(scratch.text)
    }

    fn parse(&self) -> SyntaxNode {
        syntax::parse(&self.text, &self.path)
    }

    /// The `global` declarations (they cannot be previewed, so `components()` leaves them out).
    pub fn global_names(&self) -> Vec<String> {
        let root = self.parse();
        syntax::components(&root)
            .iter()
            .filter(|c| syntax::is_global(c))
            .map(syntax::component_name)
            .collect()
    }

    pub fn imports(&self) -> Vec<ImportInfo> {
        let root = self.parse();
        root.children()
            .filter(|n| n.kind() == SyntaxKind::ImportSpecifier)
            .filter_map(|n| {
                let lit = n
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::StringLiteral)?;
                let path = lit.text().trim_matches('"').to_string();
                let list = n.child_node(SyntaxKind::ImportIdentifierList)?;
                let names = list
                    .children()
                    .filter(|i| i.kind() == SyntaxKind::ImportIdentifier)
                    .filter_map(|i| {
                        let ext = i.child_node(SyntaxKind::ExternalName)?.text().to_string().trim().to_string();
                        let int = i.child_node(SyntaxKind::InternalName).map_or(ext.clone(), |n| n.text().to_string().trim().to_string());
                        Some((ext, int))
                    })
                    .collect();
                Some(ImportInfo { path, names })
            })
            .collect()
    }

    pub fn components(&self) -> Vec<ComponentInfo> {
        let root = self.parse();
        syntax::components(&root)
            .iter()
            .filter(|c| !syntax::is_global(c))
            .map(|c| {
                let element = syntax::component_element(c);
                ComponentInfo {
                    name: syntax::component_name(c),
                    base: element.as_ref().map(type_name).unwrap_or_default(),
                    exported: c
                        .parent()
                        .is_some_and(|p| p.kind() == SyntaxKind::ExportsList),
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
        Some(build_tree(
            ElementPath {
                component: component.to_string(),
                indices: vec![],
            },
            None,
            &element,
        ))
    }

    pub fn element(&self, path: &ElementPath) -> Option<ElementInfo> {
        let root = self.parse();
        let (child, element) = syntax::resolve(&root, path)?;
        Some(element_info(path.clone(), child.as_ref(), &element))
    }

    /// Byte range of the element as the editor extents it: its wrapper (`id :=`, `for`, `if`)
    /// when it has one, trimmed of surrounding whitespace.
    pub fn element_range(&self, path: &ElementPath) -> Option<std::ops::Range<usize>> {
        let root = self.parse();
        let (wrapper, element) = syntax::resolve(&root, path)?;
        Some(trimmed_range(&self.text, wrapper.as_ref().unwrap_or(&element)))
    }

    /// Byte range of a whole component declaration (including a leading `export`).
    pub fn component_range(&self, name: &str) -> Option<std::ops::Range<usize>> {
        let root = self.parse();
        let node = syntax::component_node(&root, name)?;
        let outer = node
            .parent()
            .filter(|p| p.kind() == SyntaxKind::ExportsList)
            .unwrap_or(node);
        Some(trimmed_range(&self.text, &outer))
    }

    pub fn path_at_offset(&self, offset: u32) -> Option<ElementPath> {
        let root = self.parse();
        for component in syntax::components(&root) {
            let name = syntax::component_name(&component);
            let Some(element) = syntax::component_element(&component) else {
                continue;
            };
            if let Some(indices) = find(&element, offset) {
                return Some(ElementPath {
                    component: name,
                    indices,
                });
            }
        }
        None
    }
}

fn trimmed_range(text: &str, node: &SyntaxNode) -> std::ops::Range<usize> {
    let r = node.text_range();
    let (mut a, mut b) = (usize::from(r.start()), usize::from(r.end()).min(text.len()));
    while a < b && text.as_bytes()[a].is_ascii_whitespace() {
        a += 1;
    }
    while b > a && text.as_bytes()[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    a..b
}

fn find(element: &SyntaxNode, offset: u32) -> Option<Vec<usize>> {
    if u32::from(element.text_range().start()) == offset {
        return Some(vec![]);
    }
    for (i, c) in syntax::element_children(element).iter().enumerate() {
        if let Some(inner) = syntax::inner_element(c)
            && let Some(mut rest) = find(&inner, offset)
        {
            rest.insert(0, i);
            return Some(rest);
        }
    }
    None
}

/// The syntax errors in `new` that `old` does not already have (compared as a multiset of
/// messages), i.e. what an edit from `old` to `new` broke.
pub fn new_syntax_errors(old: &str, new: &str, path: &Path) -> Vec<String> {
    let mut before = syntax::syntax_errors(old, path);
    let mut out = Vec::new();
    for e in syntax::syntax_errors(new, path) {
        match before.iter().position(|b| *b == e) {
            Some(i) => {
                before.swap_remove(i);
            }
            None => out.push(e),
        }
    }
    out
}
