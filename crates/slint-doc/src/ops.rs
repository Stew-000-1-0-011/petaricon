//! Text-level edit operations on a document.

use crate::{Document, ElementPath, syntax};
use i_slint_compiler::parser::{SyntaxKind, SyntaxNode};

/// An edit applied through [`Document::apply`].
#[derive(Debug, Clone)]
pub enum Op {
    /// Sets (`Some(expr)`) or removes (`None`) bindings on an element, in the given order.
    SetBindings {
        path: ElementPath,
        bindings: Vec<(String, Option<String>)>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("element not found: {0:?}")]
    ElementNotFound(ElementPath),
    #[error("component not found: {0}")]
    ComponentNotFound(String),
    #[error("invalid target: {0}")]
    InvalidTarget(String),
}

pub(crate) struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Applies non-overlapping edits, back to front so offsets stay valid.
pub(crate) fn apply_edits(text: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|e| (e.start, e.end));
    let mut out = text.to_string();
    for e in edits.into_iter().rev() {
        out.replace_range(e.start..e.end, &e.text);
    }
    out
}

fn range(node: &SyntaxNode) -> (usize, usize) {
    let r = node.text_range();
    (usize::from(r.start()), usize::from(r.end()))
}

fn set_bindings(
    text: &str,
    element: &SyntaxNode,
    bindings: &[(String, Option<String>)],
) -> Result<Vec<TextEdit>, EditError> {
    let mut edits = Vec::new();
    let mut inserted = String::new();
    for (name, value) in bindings {
        let existing = element.children().find(|b| {
            b.kind() == SyntaxKind::Binding
                && b.child_text(SyntaxKind::Identifier)
                    .is_some_and(|n| n == name.as_str())
        });
        match (existing, value) {
            (Some(b), Some(expr)) => {
                let e = b.child_node(SyntaxKind::BindingExpression).ok_or_else(|| {
                    EditError::InvalidTarget(format!("binding `{name}` has no expression"))
                })?;
                let (start, end) = range(&e);
                edits.push(TextEdit {
                    start,
                    end,
                    text: format!("{expr};"),
                });
            }
            (Some(b), None) => {
                let (mut start, end) = range(&b);
                let bytes = text.as_bytes();
                while start > 0 && bytes[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
                edits.push(TextEdit {
                    start,
                    end,
                    text: String::new(),
                });
            }
            (None, Some(expr)) => inserted.push_str(&format!(" {name}: {expr};")),
            (None, None) => {}
        }
    }
    if !inserted.is_empty() {
        let brace = element
            .child_token(SyntaxKind::LBrace)
            .ok_or_else(|| EditError::InvalidTarget("element has no `{`".into()))?;
        let at = usize::from(brace.text_range().end());
        edits.push(TextEdit {
            start: at,
            end: at,
            text: inserted,
        });
    }
    Ok(edits)
}

impl Document {
    /// Applies `op`. Returns the (post-edit) path of the edited element.
    pub fn apply(&mut self, op: Op) -> Result<Option<ElementPath>, EditError> {
        let root = self.parse();
        let (new_text, result) = match op {
            Op::SetBindings { path, bindings } => {
                if syntax::component_node(&root, &path.component).is_none() {
                    return Err(EditError::ComponentNotFound(path.component));
                }
                let Some((_, element)) = syntax::resolve(&root, &path) else {
                    return Err(EditError::ElementNotFound(path));
                };
                let edits = set_bindings(&self.text, &element, &bindings)?;
                (apply_edits(&self.text, edits), path)
            }
        };
        self.set_text(new_text);
        Ok(Some(result))
    }
}
