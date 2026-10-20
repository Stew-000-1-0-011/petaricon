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
    /// Inserts one element (`source`, e.g. `Button { text: "OK"; }`) into `parent` at `index`
    /// (`None` appends). Returns the new element's path.
    InsertElement {
        parent: ElementPath,
        index: Option<usize>,
        source: String,
    },
    /// Removes an element together with its attached leading `//` comment lines.
    RemoveElement { path: ElementPath },
    /// Moves an element (with its id and attached comments) under `new_parent`. `index` counts
    /// the children of `new_parent` after the element has been taken out. `x` / `y` are dropped
    /// when the new parent is a layout. Returns the element's post-edit path.
    MoveElement {
        path: ElementPath,
        new_parent: ElementPath,
        index: Option<usize>,
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
/// Overlapping edits are rejected with `InvalidTarget` and nothing is changed.
/// A zero-width insert may share an offset with the start or end of another edit.
pub(crate) fn apply_edits(text: &str, mut edits: Vec<TextEdit>) -> Result<String, EditError> {
    edits.sort_by_key(|e| (e.start, e.end));
    for e in &edits {
        if e.start > e.end
            || e.end > text.len()
            || !text.is_char_boundary(e.start)
            || !text.is_char_boundary(e.end)
        {
            return Err(EditError::InvalidTarget("edit range is invalid".into()));
        }
    }
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err(EditError::InvalidTarget("overlapping edits".into()));
    }
    let mut out = text.to_string();
    for e in edits.into_iter().rev() {
        out.replace_range(e.start..e.end, &e.text);
    }
    Ok(out)
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
    for (i, (name, _)) in bindings.iter().enumerate() {
        if bindings[..i].iter().any(|(n, _)| n == name) {
            return Err(EditError::InvalidTarget(format!(
                "binding `{name}` given more than once"
            )));
        }
    }
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
                // Same-line binding: drop the spaces before it. Own-line binding: drop its
                // indentation and exactly one preceding line break.
                while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
                    start -= 1;
                }
                if start > 0 && bytes[start - 1] == b'\n' {
                    start -= 1;
                    if start > 0 && bytes[start - 1] == b'\r' {
                        start -= 1;
                    }
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

/// Where an element child sits in the text, and what removing it takes with it.
struct Extent {
    /// Start of the element together with its attached comment lines (and its indentation).
    start: usize,
    end: usize,
    /// Range deleted when removing the element.
    remove: (usize, usize),
    /// The element owns its line(s), so whole lines are moved / removed.
    line_mode: bool,
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i)
}

fn extent(text: &str, wrapper: &SyntaxNode) -> Extent {
    let (s, e) = range(wrapper);
    let ls = line_start(text, s);
    let le = line_end(text, e);
    let owns_line = text[ls..s].trim().is_empty() && text[e..le].trim().is_empty();
    if owns_line {
        let mut start = ls;
        while start > 0 {
            let prev = line_start(text, start - 1);
            if text[prev..start - 1].trim_start().starts_with("//") {
                start = prev;
            } else {
                break;
            }
        }
        let mut rm_start = start;
        if rm_start > 0 {
            rm_start -= 1;
            if rm_start > 0 && text.as_bytes()[rm_start - 1] == b'\r' {
                rm_start -= 1;
            }
        }
        // Keep a trailing `\r` of this line out of the removed range.
        let rm_end = e + text[e..le].trim_end_matches('\r').len();
        return Extent {
            start,
            end: e,
            remove: (rm_start, rm_end),
            line_mode: true,
        };
    }
    let mut rm_start = s;
    while rm_start > 0 && matches!(text.as_bytes()[rm_start - 1], b' ' | b'\t') {
        rm_start -= 1;
    }
    Extent {
        start: s,
        end: e,
        remove: (rm_start, e),
        line_mode: false,
    }
}

/// Re-indents `block`: strips `from` off every line and prefixes `to` to non-empty ones.
fn reindent(block: &str, from: &str, to: &str) -> String {
    block
        .split('\n')
        .map(|l| {
            let l = l.strip_prefix(from).unwrap_or(l);
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("{to}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn check_source(source: &str) -> Result<(), EditError> {
    let wrapped = format!("component Probe__ inherits Rectangle {{\n{source}\n}}\n");
    let mut diag = i_slint_compiler::diagnostics::BuildDiagnostics::default();
    let root = i_slint_compiler::parser::parse(wrapped, None, &mut diag);
    let one = syntax::component_node(&root, "Probe__")
        .and_then(|c| syntax::component_element(&c))
        .is_some_and(|e| syntax::element_children(&e).len() == 1);
    if diag.has_errors() || !one {
        return Err(EditError::InvalidTarget(
            "source is not exactly one element".into(),
        ));
    }
    Ok(())
}

/// The edit that puts `block` (already indented) at `index` among `siblings` of `parent`.
fn insert_edit(
    text: &str,
    parent: &SyntaxNode,
    siblings: &[SyntaxNode],
    index: usize,
    block: &str,
) -> Result<TextEdit, EditError> {
    if index > 0 {
        let mut at = range(&siblings[index - 1]).1;
        let le = line_end(text, at);
        if text[at..le].trim_start().starts_with("//") {
            at = le;
            if text.as_bytes()[at - 1] == b'\r' {
                at -= 1;
            }
        }
        return Ok(TextEdit {
            start: at,
            end: at,
            text: format!("\n{block}"),
        });
    }
    if let Some(next) = siblings.first() {
        let ext = extent(text, next);
        return Ok(if ext.line_mode {
            TextEdit {
                start: ext.start,
                end: ext.start,
                text: format!("{block}\n"),
            }
        } else {
            TextEdit {
                start: ext.start,
                end: ext.start,
                text: format!("{} ", block.trim_start()),
            }
        });
    }
    let brace = parent
        .child_token(SyntaxKind::RBrace)
        .ok_or_else(|| EditError::InvalidTarget("element has no `}`".into()))?;
    let b = usize::from(brace.text_range().start());
    let ls = line_start(text, b);
    if text[ls..b].trim().is_empty() {
        return Ok(TextEdit {
            start: ls,
            end: ls,
            text: format!("{block}\n"),
        });
    }
    let mut ws = b;
    while ws > 0 && matches!(text.as_bytes()[ws - 1], b' ' | b'\t') {
        ws -= 1;
    }
    Ok(TextEdit {
        start: ws,
        end: b,
        text: format!("\n{block}\n{}", syntax::element_indent(text, parent)),
    })
}

fn resolve_in(
    root: &SyntaxNode,
    path: &ElementPath,
) -> Result<(Option<SyntaxNode>, SyntaxNode), EditError> {
    if syntax::component_node(root, &path.component).is_none() {
        return Err(EditError::ComponentNotFound(path.component.clone()));
    }
    syntax::resolve(root, path).ok_or_else(|| EditError::ElementNotFound(path.clone()))
}

fn is_prefix(a: &[usize], b: &[usize]) -> bool {
    b.len() >= a.len() && b[..a.len()] == *a
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
                (apply_edits(&self.text, edits)?, Some(path))
            }
            Op::InsertElement {
                parent,
                index,
                source,
            } => {
                let (_, parent_el) = resolve_in(&root, &parent)?;
                check_source(&source)?;
                let siblings = syntax::element_children(&parent_el);
                let index = index.unwrap_or(siblings.len()).min(siblings.len());
                let indent = syntax::child_indent(&self.text, &parent_el);
                let block = reindent(source.trim(), "", &indent);
                let edit = insert_edit(&self.text, &parent_el, &siblings, index, &block)?;
                let mut indices = parent.indices.clone();
                indices.push(index);
                (
                    apply_edits(&self.text, vec![edit])?,
                    Some(ElementPath {
                        component: parent.component,
                        indices,
                    }),
                )
            }
            Op::RemoveElement { path } => {
                let (wrapper, _) = resolve_in(&root, &path)?;
                let wrapper = wrapper.ok_or_else(|| {
                    EditError::InvalidTarget("the component root cannot be removed".into())
                })?;
                let (start, end) = extent(&self.text, &wrapper).remove;
                let edit = TextEdit {
                    start,
                    end,
                    text: String::new(),
                };
                (apply_edits(&self.text, vec![edit])?, None)
            }
            Op::MoveElement {
                path,
                new_parent,
                index,
            } => {
                let (wrapper, element) = resolve_in(&root, &path)?;
                let wrapper = wrapper.ok_or_else(|| {
                    EditError::InvalidTarget("the component root cannot be moved".into())
                })?;
                let (_, dest) = resolve_in(&root, &new_parent)?;
                if new_parent.component != path.component
                    || is_prefix(&path.indices, &new_parent.indices)
                {
                    return Err(EditError::InvalidTarget(
                        "cannot move an element into itself or another component".into(),
                    ));
                }
                let ext = extent(&self.text, &wrapper);
                let from_indent = if ext.line_mode {
                    syntax::line_indent(&self.text, ext.end)
                } else {
                    String::new()
                };
                let all = syntax::element_children(&dest);
                let to_indent = syntax::child_indent(&self.text, &dest);
                let siblings: Vec<_> = all
                    .iter()
                    .filter(|c| range(c) != range(&wrapper))
                    .cloned()
                    .collect();
                let index = index.unwrap_or(siblings.len()).min(siblings.len());
                // Moved text, with x / y dropped under a layout.
                let mut moved = self.text[ext.start..ext.end].to_string();
                if crate::layout_kind(&crate::type_name(&dest)).is_some() {
                    let strip = set_bindings(
                        &self.text,
                        &element,
                        &[("x".into(), None), ("y".into(), None)],
                    )?
                    .into_iter()
                    .map(|e| TextEdit {
                        start: e.start - ext.start,
                        end: e.end - ext.start,
                        text: e.text,
                    })
                    .collect();
                    moved = apply_edits(&moved, strip)?;
                }
                let block = reindent(&moved, &from_indent, &to_indent);
                let insert = insert_edit(&self.text, &dest, &siblings, index, &block)?;
                let remove = TextEdit {
                    start: ext.remove.0,
                    end: ext.remove.1,
                    text: String::new(),
                };
                // Post-edit path: siblings after the removed one shift down by one.
                let mut indices = new_parent.indices.clone();
                let depth = path.indices.len() - 1;
                if indices.len() > depth
                    && indices[..depth] == path.indices[..depth]
                    && indices[depth] > path.indices[depth]
                {
                    indices[depth] -= 1;
                }
                indices.push(index);
                (
                    apply_edits(&self.text, vec![remove, insert])?,
                    Some(ElementPath {
                        component: path.component,
                        indices,
                    }),
                )
            }
        };
        self.set_text(new_text);
        Ok(result)
    }
}
