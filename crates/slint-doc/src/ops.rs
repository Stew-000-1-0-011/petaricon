//! Text-level edit operations on a document.

use crate::{Document, ElementPath, LayoutKind, syntax};
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
    /// Moves all children of `container` into a new layout element, in `layout.order`, dropping
    /// their `x` / `y`. The layout takes the place of the first child. Returns the layout's path.
    WrapInLayout {
        container: ElementPath,
        layout: LayoutSpec,
    },
    /// Replaces a layout by its children, in order, giving each the `x` / `y` from `positions`.
    /// Returns the parent's path.
    Unwrap {
        layout: ElementPath,
        positions: Vec<(i32, i32)>,
    },
}

/// Description of the layout created by [`Op::WrapInLayout`].
#[derive(Debug, Clone)]
pub struct LayoutSpec {
    /// Only `Horizontal` and `Vertical` are valid.
    pub kind: LayoutKind,
    /// A permutation of all child indices of the container.
    pub order: Vec<usize>,
    /// Bindings written on the layout, one line each, in this order.
    pub bindings: Vec<(String, String)>,
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
    // A same-line trailing `//` comment belongs to the element.
    let trailing = text[e..le].trim_end();
    let owns_line = text[ls..s].trim().is_empty()
        && (trailing.trim().is_empty() || trailing.trim_start().starts_with("//"));
    if owns_line {
        let e = e + trailing.len();
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
        let rm_end = e;
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
    // A trailing `//` comment runs to the end of the line, so it belongs to the element.
    let e = if trailing.trim_start().starts_with("//") {
        e + trailing.len()
    } else {
        e
    };
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

/// Text of a child (with attached comments) re-indented to `to_indent`, after applying
/// `bindings` to its element.
fn child_block(
    text: &str,
    wrapper: &SyntaxNode,
    bindings: &[(String, Option<String>)],
    to_indent: &str,
) -> Result<String, EditError> {
    let element = syntax::inner_element(wrapper)
        .ok_or_else(|| EditError::InvalidTarget("child has no element".into()))?;
    let ext = extent(text, wrapper);
    let from_indent = if ext.line_mode {
        syntax::line_indent(text, ext.start)
    } else {
        String::new()
    };
    let edits = set_bindings(text, &element, bindings)?
        .into_iter()
        .map(|e| TextEdit {
            start: e.start - ext.start,
            end: e.end - ext.start,
            text: e.text,
        })
        .collect();
    let moved = apply_edits(&text[ext.start..ext.end], edits)?;
    Ok(reindent(&moved, &from_indent, to_indent))
}

fn plain_children(element: &SyntaxNode) -> Result<Vec<SyntaxNode>, EditError> {
    let children = syntax::element_children(element);
    if children.iter().any(|c| c.kind() != SyntaxKind::SubElement) {
        return Err(EditError::InvalidTarget(
            "repeated or conditional children cannot be moved into or out of a layout".into(),
        ));
    }
    Ok(children)
}

/// The edit that replaces `first` (see [`Extent`]) by `block`. Text that is not part of the
/// element stays where it was; an element sharing its line is replaced in place.
fn replace_extent(text: &str, first: &SyntaxNode, block: String) -> TextEdit {
    let ext = extent(text, first);
    if ext.line_mode {
        TextEdit {
            start: ext.remove.0,
            end: ext.remove.1,
            text: if block.is_empty() {
                String::new()
            } else {
                format!("\n{block}")
            },
        }
    } else {
        TextEdit {
            start: ext.start,
            end: ext.end,
            text: block.trim_start().to_string(),
        }
    }
}

/// `//` comment lines of a layout that no child owns: the layout's own trailing comment and
/// comment lines between its children.
fn loose_comments(
    text: &str,
    wrapper: &SyntaxNode,
    layout: &SyntaxNode,
    children: &[SyntaxNode],
) -> Vec<String> {
    let mut found = Vec::new();
    let mut scan = |seg: &str| {
        found.extend(
            seg.lines()
                .map(str::trim)
                .filter(|l| l.starts_with("//"))
                .map(str::to_string),
        );
    };
    let (Some(lb), Some(rb)) = (
        layout.child_token(SyntaxKind::LBrace),
        layout.child_token(SyntaxKind::RBrace),
    ) else {
        return found;
    };
    let mut cursor = usize::from(lb.text_range().end());
    for c in children {
        let ext = extent(text, c);
        scan(&text[cursor..ext.start.max(cursor)]);
        cursor = cursor.max(ext.end);
    }
    scan(&text[cursor..usize::from(rb.text_range().start())]);
    let (_, wrapper_end) = range(wrapper);
    scan(&text[wrapper_end..extent(text, wrapper).end]);
    found
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
                    syntax::line_indent(&self.text, ext.start)
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
            Op::WrapInLayout { container, layout } => {
                let (_, element) = resolve_in(&root, &container)?;
                let type_name = match layout.kind {
                    LayoutKind::Horizontal => "HorizontalLayout",
                    LayoutKind::Vertical => "VerticalLayout",
                    _ => {
                        return Err(EditError::InvalidTarget(
                            "layout kind must be Horizontal or Vertical".into(),
                        ));
                    }
                };
                let children = syntax::element_children(&element);
                if children.is_empty() {
                    return Err(EditError::InvalidTarget("container has no children".into()));
                }
                let children = plain_children(&element)?;
                let mut seen = vec![false; children.len()];
                for &i in &layout.order {
                    if i >= children.len() || std::mem::replace(&mut seen[i], true) {
                        return Err(EditError::InvalidTarget(
                            "order is not a permutation of the children".into(),
                        ));
                    }
                }
                if layout.order.len() != children.len() {
                    return Err(EditError::InvalidTarget(
                        "order is not a permutation of the children".into(),
                    ));
                }
                let indent = syntax::child_indent(&self.text, &element);
                let inner = format!("{indent}    ");
                let strip = [("x".to_string(), None), ("y".to_string(), None)];
                let mut lines = vec![format!("{indent}{type_name} {{")];
                for (name, value) in &layout.bindings {
                    lines.push(format!("{inner}{name}: {value};"));
                }
                for &i in &layout.order {
                    lines.push(child_block(&self.text, &children[i], &strip, &inner)?);
                }
                lines.push(format!("{indent}}}"));
                let mut edits = vec![replace_extent(&self.text, &children[0], lines.join("\n"))];
                for c in &children[1..] {
                    let (start, end) = extent(&self.text, c).remove;
                    edits.push(TextEdit {
                        start,
                        end,
                        text: String::new(),
                    });
                }
                let mut indices = container.indices.clone();
                indices.push(0);
                (
                    apply_edits(&self.text, edits)?,
                    Some(ElementPath {
                        component: container.component,
                        indices,
                    }),
                )
            }
            Op::Unwrap { layout, positions } => {
                let (wrapper, element) = resolve_in(&root, &layout)?;
                let wrapper = wrapper.ok_or_else(|| {
                    EditError::InvalidTarget("the component root cannot be unwrapped".into())
                })?;
                if crate::layout_kind(&crate::type_name(&element)).is_none() {
                    return Err(EditError::InvalidTarget("element is not a layout".into()));
                }
                if wrapper.kind() != SyntaxKind::SubElement {
                    return Err(EditError::InvalidTarget(
                        "repeated or conditional layouts cannot be unwrapped".into(),
                    ));
                }
                let mut parent_path = layout.clone();
                parent_path.indices.pop();
                let (_, parent) = resolve_in(&root, &parent_path)?;
                if crate::layout_kind(&crate::type_name(&parent)).is_some() {
                    return Err(EditError::InvalidTarget(
                        "the parent is a layout; its children cannot take x / y".into(),
                    ));
                }
                let children = plain_children(&element)?;
                if children.len() != positions.len() {
                    return Err(EditError::InvalidTarget(
                        "positions must match the number of children".into(),
                    ));
                }
                let indent = syntax::child_indent(&self.text, &parent);
                let ext = extent(&self.text, &wrapper);
                // Comments attached above the layout lead the restored children; loose ones
                // inside it follow them.
                let mut blocks: Vec<String> = Vec::new();
                if ext.line_mode {
                    let above = &self.text[ext.start..line_start(&self.text, range(&wrapper).0)];
                    blocks.extend(
                        above
                            .lines()
                            .map(|l| format!("{indent}{}", l.trim()))
                            .collect::<Vec<_>>(),
                    );
                }
                for (c, (px, py)) in children.iter().zip(&positions) {
                    let set = [
                        ("x".to_string(), Some(format!("{px}px"))),
                        ("y".to_string(), Some(format!("{py}px"))),
                    ];
                    blocks.push(child_block(&self.text, c, &set, &indent)?);
                }
                let loose = loose_comments(&self.text, &wrapper, &element, &children);
                for l in &loose {
                    blocks.push(format!("{indent}{l}"));
                }
                let loose_last = !loose.is_empty();
                let mut edit = replace_extent(&self.text, &wrapper, blocks.join("\n"));
                // A comment must not swallow code that follows it on the same line.
                let after = &self.text[ext.end..line_end(&self.text, ext.end)];
                if !ext.line_mode && loose_last && !after.trim().is_empty() {
                    edit.text.push_str(&format!("\n{indent}"));
                }
                (apply_edits(&self.text, vec![edit])?, Some(parent_path))
            }
        };
        self.set_text(new_text);
        Ok(result)
    }
}
