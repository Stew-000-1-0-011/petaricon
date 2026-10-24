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
    /// Turns the element at `path` into a new component `name` (declared right before the
    /// component that contains it, as `component {name} inherits {type} { ... }`, carrying the
    /// element's body without `x` / `y` / `width` / `height`) and replaces it by a usage that
    /// keeps its id and those geometry bindings. Returns the usage's path.
    ExtractComponent { path: ElementPath, name: String },
    /// Inserts the one-line declaration `text` into the root element of `component`: after its
    /// last property / callback declaration, else at the top of its body.
    AddDeclaration { component: String, text: String },
    /// Turns the binding `property` of the element at `path` into a parameter of its
    /// component: adds `in property <ty> name: <old expression>;` to the component's root
    /// element (placed as `Op::AddDeclaration` does) and rewrites the binding to
    /// `property: root.name;`. Refused (nothing changes) when the property has no value, the
    /// expression mentions `self` or `parent`, it already is `root.<name>`, or `name` is taken.
    Parameterize {
        path: ElementPath,
        property: String,
        name: String,
        ty: String,
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
    #[error("要素が見つかりません: {0:?}")]
    ElementNotFound(ElementPath),
    #[error("部品が見つかりません: {0}")]
    ComponentNotFound(String),
    #[error("{0}")]
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
            return Err(EditError::InvalidTarget("編集範囲が不正です".into()));
        }
    }
    if edits.windows(2).any(|w| w[0].end > w[1].start) {
        return Err(EditError::InvalidTarget("編集が重なっています".into()));
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
                "`{name}` が複数回指定されています"
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
                    EditError::InvalidTarget(format!("`{name}` に式がありません"))
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
            .ok_or_else(|| EditError::InvalidTarget("要素に `{` がありません".into()))?;
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
        // Remove the whole line, trailing whitespace included (but keep a `\r`).
        let rm_end = le - usize::from(le > e && text.as_bytes()[le - 1] == b'\r');
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

/// Whether the last line of `block` ends in a `//` comment (outside string literals).
fn ends_in_line_comment(block: &str) -> bool {
    let last = block.rsplit('\n').next().unwrap_or("");
    let (mut in_str, mut escaped, mut prev) = (false, false, ' ');
    for c in last.chars() {
        if in_str {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '/' && prev == '/' {
            return true;
        }
        prev = c;
    }
    false
}

/// What to put after `block` when text continues at `at` on the same line: a space, or when
/// `block` ends in a comment (which would swallow that text) a line break plus `indent`.
fn block_tail(text: &str, block: &str, at: usize, indent: &str, otherwise: &str) -> String {
    let rest_blank = text[at..line_end(text, at)].trim().is_empty();
    if ends_in_line_comment(block) && !rest_blank {
        format!("\n{indent}")
    } else {
        otherwise.to_string()
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
            "ソースは要素1つだけにしてください".into(),
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
        let lead: String = block
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let tail = block_tail(text, block, at, &lead, "");
        return Ok(TextEdit {
            start: at,
            end: at,
            text: format!("\n{block}{tail}"),
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
                text: format!(
                    "{}{}",
                    block.trim_start(),
                    block_tail(
                        text,
                        block,
                        ext.start,
                        &syntax::line_indent(text, ext.start),
                        " "
                    )
                ),
            }
        });
    }
    let brace = parent
        .child_token(SyntaxKind::RBrace)
        .ok_or_else(|| EditError::InvalidTarget("要素に `}` がありません".into()))?;
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
        .ok_or_else(|| EditError::InvalidTarget("子に要素がありません".into()))?;
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
            "繰り返しや条件付きの子は、レイアウトの出し入れができません".into(),
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
        let tail = block_tail(
            text,
            &block,
            ext.end,
            &syntax::line_indent(text, ext.start),
            "",
        );
        // After a line break the old spacing before the following text is redundant.
        let mut end = ext.end;
        if !tail.is_empty() {
            while matches!(text.as_bytes().get(end), Some(b' ' | b'\t')) {
                end += 1;
            }
        }
        TextEdit {
            start: ext.start,
            end,
            text: format!("{}{tail}", block.trim_start()),
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

const GEOMETRY: [&str; 4] = ["x", "y", "width", "height"];

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Edits for `Op::ExtractComponent`: the new declaration and the usage replacing the target.
fn extract_component(
    text: &str,
    root: &SyntaxNode,
    path: &ElementPath,
    name: &str,
) -> Result<Vec<TextEdit>, EditError> {
    let (wrapper, element) = resolve_in(root, path)?;
    let wrapper = wrapper
        .ok_or_else(|| EditError::InvalidTarget("部品のルート要素は部品にできません".into()))?;
    if wrapper.kind() != SyntaxKind::SubElement {
        return Err(EditError::InvalidTarget(
            "繰り返しや条件付きの要素は部品にできません".into(),
        ));
    }
    if !valid_identifier(name) {
        return Err(EditError::InvalidTarget(format!(
            "`{name}` は部品名に使えません"
        )));
    }
    if syntax::component_node(root, name).is_some() {
        return Err(EditError::InvalidTarget(format!(
            "部品 `{name}` はすでにあります"
        )));
    }
    let type_name = crate::type_name(&element);
    let type_node = element
        .child_node(SyntaxKind::QualifiedName)
        .filter(|_| !type_name.is_empty())
        .ok_or_else(|| EditError::InvalidTarget("要素に型がありません".into()))?;
    let (Some(lb), Some(rb)) = (
        element.child_token(SyntaxKind::LBrace),
        element.child_token(SyntaxKind::RBrace),
    ) else {
        return Err(EditError::InvalidTarget("要素に本体がありません".into()));
    };
    let (body_start, body_end) = (
        usize::from(lb.text_range().end()),
        usize::from(rb.text_range().start()),
    );

    // Geometry bindings that were present, in source order.
    let mut kept = Vec::new();
    let mut removals = Vec::new();
    for b in element
        .children()
        .filter(|b| b.kind() == SyntaxKind::Binding)
    {
        let Some(id) = b.child_text(SyntaxKind::Identifier) else {
            continue;
        };
        if GEOMETRY.contains(&id.as_str()) {
            let t = text[range(&b).0..range(&b).1].trim();
            kept.push(if t.ends_with(';') || t.ends_with('}') {
                t.to_string()
            } else {
                format!("{t};")
            });
            removals.push((id.to_string(), None));
        }
    }
    let strip = set_bindings(text, &element, &removals)?
        .into_iter()
        .map(|e| TextEdit {
            start: e.start - body_start,
            end: e.end - body_start,
            text: e.text,
        })
        .collect();
    let body = apply_edits(&text[body_start..body_end], strip)?;
    let from = format!("{}    ", syntax::element_indent(text, &element));
    let mut lines: Vec<&str> = body.split('\n').collect();
    // Text sharing the line with `{` is the first body line.
    let first = lines.remove(0);
    let mut out: Vec<String> = Vec::new();
    if !first.trim().is_empty() {
        out.push(format!("    {}", first.trim()));
    }
    out.extend(
        reindent(&lines.join("\n"), &from, "    ")
            .split('\n')
            .map(str::to_string),
    );
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.trim().is_empty()) {
        out.remove(0);
    }
    let mut declaration = format!("component {name} inherits {type_name} {{\n");
    for l in &out {
        declaration.push_str(l);
        declaration.push('\n');
    }
    declaration.push_str("}\n\n");

    let component = syntax::component_node(root, &path.component)
        .ok_or_else(|| EditError::ComponentNotFound(path.component.clone()))?;
    let at = line_start(text, range(&component).0);

    // The usage replaces the target's own text; an `id := ` prefix stays.
    let (w_start, w_end) = range(&wrapper);
    let prefix = &text[w_start..range(&type_node).0];
    let usage = if kept.is_empty() {
        format!("{prefix}{name} {{ }}")
    } else {
        format!("{prefix}{name} {{ {} }}", kept.join(" "))
    };
    Ok(vec![
        TextEdit {
            start: at,
            end: at,
            text: declaration,
        },
        TextEdit {
            start: w_start,
            end: w_end,
            text: usage,
        },
    ])
}

/// The edit for `Op::AddDeclaration`.
fn add_declaration(
    text: &str,
    root: &SyntaxNode,
    component: &str,
    decl: &str,
) -> Result<TextEdit, EditError> {
    add_declaration_in(text, root, component, decl, false)
}

/// `add_declaration`; `multiline` allows continuation lines (copied as written).
fn add_declaration_in(
    text: &str,
    root: &SyntaxNode,
    component: &str,
    decl: &str,
    multiline: bool,
) -> Result<TextEdit, EditError> {
    let decl = decl.trim();
    if decl.is_empty() || (!multiline && decl.contains('\n')) {
        return Err(EditError::InvalidTarget(
            "宣言は空でない1行にしてください".into(),
        ));
    }
    let node = syntax::component_node(root, component)
        .ok_or_else(|| EditError::ComponentNotFound(component.to_string()))?;
    let element = syntax::component_element(&node)
        .ok_or_else(|| EditError::InvalidTarget("部品にルート要素がありません".into()))?;
    let last = element
        .children()
        .filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::PropertyDeclaration | SyntaxKind::CallbackDeclaration
            )
        })
        .last();
    let (at, indent) = match &last {
        Some(d) => (range(d).1, syntax::line_indent(text, range(d).0)),
        None => {
            let lb = element
                .child_token(SyntaxKind::LBrace)
                .ok_or_else(|| EditError::InvalidTarget("要素に `{` がありません".into()))?;
            let first_member = element
                .children()
                .find(|n| n.kind() == SyntaxKind::Binding)
                .map(|n| syntax::line_indent(text, range(&n).0));
            (
                usize::from(lb.text_range().end()),
                first_member.unwrap_or_else(|| syntax::child_indent(text, &element)),
            )
        }
    };
    let le = line_end(text, at);
    let rest = text[at..le].trim();
    if rest.is_empty() || rest.starts_with("//") {
        // Own line: after any same-line comment, on a new line.
        let mut end = le;
        let mut eol = "\n";
        if end > at && text.as_bytes()[end - 1] == b'\r' {
            end -= 1;
            eol = "\r\n";
        }
        Ok(TextEdit {
            start: end,
            end,
            text: format!("{eol}{indent}{decl}"),
        })
    } else {
        Ok(TextEdit {
            start: at,
            end: at,
            text: format!(" {decl}"),
        })
    }
}

/// The `Binding` of `property` on `element`.
fn find_binding(element: &SyntaxNode, property: &str) -> Option<SyntaxNode> {
    element.children().find(|b| {
        b.kind() == SyntaxKind::Binding
            && b.child_text(SyntaxKind::Identifier)
                .is_some_and(|n| n == property)
    })
}

/// Names declared (properties and callbacks) on the root element of `component`.
fn declared_names(root: &SyntaxNode, component: &str) -> Vec<String> {
    syntax::component_node(root, component)
        .and_then(|c| syntax::component_element(&c))
        .into_iter()
        .flat_map(|e| e.children())
        .filter(|n| {
            matches!(
                n.kind(),
                SyntaxKind::PropertyDeclaration | SyntaxKind::CallbackDeclaration
            )
        })
        .filter_map(|n| {
            n.child_node(SyntaxKind::DeclaredIdentifier)
                .map(|d| d.text().to_string().trim().to_string())
        })
        .collect()
}

/// Property names every element has (or the usual ones), which a parameter must not shadow.
const BUILTIN_PROPS: &[&str] = &[
    "x", "y", "z", "width", "height", "min-width", "min-height", "max-width", "max-height",
    "preferred-width", "preferred-height", "horizontal-stretch", "vertical-stretch", "opacity",
    "visible", "enabled", "clip", "text", "color", "background", "source", "font-size",
    "font-weight", "font-family", "border-radius", "border-width", "border-color", "spacing",
    "padding", "padding-left", "padding-right", "padding-top", "padding-bottom", "alignment",
    "cross-axis-alignment", "layout-cache", "has-focus", "checked", "value",
];

/// `_` and `-` are the same character in a Slint name.
fn normalize_name(n: &str) -> String {
    n.replace('_', "-")
}

/// Whether `name` is declared on the root of `component`, is a property written on it, or is
/// a builtin property name (all compared with `_` = `-`).
fn name_taken(root: &SyntaxNode, component: &str, name: &str) -> bool {
    let n = normalize_name(name);
    let bindings = syntax::component_node(root, component)
        .and_then(|c| syntax::component_element(&c))
        .into_iter()
        .flat_map(|e| e.children())
        .filter(|b| matches!(b.kind(), SyntaxKind::Binding | SyntaxKind::TwoWayBinding))
        .filter_map(|b| b.child_text(SyntaxKind::Identifier).map(|i| i.to_string()));
    declared_names(root, component)
        .into_iter()
        .chain(bindings)
        .any(|d| normalize_name(&d) == n)
        || BUILTIN_PROPS.contains(&n.as_str())
}

/// The expression of `property` on the element at `path` if it can become a parameter, else
/// the reason (short Japanese text) it cannot.
fn parameter_source(
    root: &SyntaxNode,
    path: &ElementPath,
    property: &str,
) -> Result<(SyntaxNode, String), EditError> {
    let (_, element) = resolve_in(root, path)?;
    let refuse = |m: &str| Err(EditError::InvalidTarget(m.to_string()));
    let named = |n: &SyntaxNode, kind: SyntaxKind| {
        n.kind() == SyntaxKind::PropertyDeclaration
            && n.child_node(kind)
                .is_some_and(|d| d.text().to_string().trim() == property)
    };
    if element
        .children()
        .any(|n| named(&n, SyntaxKind::DeclaredIdentifier))
    {
        return refuse("要素で宣言したプロパティは引数にできません");
    }
    if element.children().any(|n| {
        n.kind() == SyntaxKind::TwoWayBinding
            && n.child_text(SyntaxKind::Identifier)
                .is_some_and(|i| i == property)
    }) {
        return refuse("双方向バインディングは引数にできません");
    }
    let Some(binding) = find_binding(&element, property) else {
        return refuse("先に値を入れてください");
    };
    let Some(expr_node) = binding.child_node(SyntaxKind::BindingExpression) else {
        return refuse("先に値を入れてください");
    };
    let raw = expr_node.text().to_string();
    let expr = raw.trim();
    let expr = expr.strip_suffix(';').unwrap_or(expr).trim().to_string();
    if expr.is_empty() {
        return refuse("先に値を入れてください");
    }
    if expr_node.descendants_with_tokens().any(|e| {
        e.as_token().is_some_and(|t| {
            t.kind() == SyntaxKind::Identifier && matches!(t.text(), "self" | "parent")
        })
    }) {
        return refuse("self や parent を使う値は引数にできません");
    }
    if expr
        .strip_prefix("root.")
        .is_some_and(|r| valid_identifier(r.trim()))
    {
        return refuse("すでに引数です");
    }
    Ok((expr_node, expr))
}

/// The edits for `Op::Parameterize`.
fn parameterize(
    text: &str,
    root: &SyntaxNode,
    path: &ElementPath,
    property: &str,
    name: &str,
    ty: &str,
) -> Result<Vec<TextEdit>, EditError> {
    if !valid_identifier(name) {
        return Err(EditError::InvalidTarget(format!(
            "`{name}` は名前に使えません"
        )));
    }
    if !valid_identifier(ty) {
        return Err(EditError::InvalidTarget(format!("`{ty}` は使えない型です")));
    }
    let (expr_node, expr) = parameter_source(root, path, property)?;
    if name_taken(root, &path.component, name) {
        return Err(EditError::InvalidTarget(format!(
            "`{name}` はすでに宣言されています"
        )));
    }
    let decl = add_declaration_in(
        text,
        root,
        &path.component,
        &format!("in property <{ty}> {name}: {expr};"),
        true,
    )?;
    let (start, end) = range(&expr_node);
    Ok(vec![
        decl,
        TextEdit {
            start,
            end,
            text: format!("root.{name};"),
        },
    ])
}

impl Document {
    /// The expression `property` of the element at `path` would give a parameter as its
    /// default, or why it cannot become one.
    pub fn parameter_source(&self, path: &ElementPath, property: &str) -> Result<String, EditError> {
        parameter_source(&self.parse(), path, property).map(|(_, e)| e)
    }

    /// Whether a parameter `name` would clash on the root of `component`.
    pub fn parameter_name_taken(&self, component: &str, name: &str) -> bool {
        name_taken(&self.parse(), component, name)
    }

    /// Properties and callbacks declared on the root element of `component`.
    pub fn declared_names(&self, component: &str) -> Vec<String> {
        declared_names(&self.parse(), component)
    }

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
                    EditError::InvalidTarget("部品のルート要素は削除できません".into())
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
                    EditError::InvalidTarget("部品のルート要素は移動できません".into())
                })?;
                let (_, dest) = resolve_in(&root, &new_parent)?;
                if new_parent.component != path.component
                    || is_prefix(&path.indices, &new_parent.indices)
                {
                    return Err(EditError::InvalidTarget(
                        "要素を自分自身や別の部品の中へは移動できません".into(),
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
                            "レイアウトは Horizontal か Vertical にしてください".into(),
                        ));
                    }
                };
                let children = syntax::element_children(&element);
                if children.is_empty() {
                    return Err(EditError::InvalidTarget("入れ物に子要素がありません".into()));
                }
                let children = plain_children(&element)?;
                let mut seen = vec![false; children.len()];
                for &i in &layout.order {
                    if i >= children.len() || std::mem::replace(&mut seen[i], true) {
                        return Err(EditError::InvalidTarget(
                            "並び順が子要素と一致しません".into(),
                        ));
                    }
                }
                if layout.order.len() != children.len() {
                    return Err(EditError::InvalidTarget(
                        "並び順が子要素と一致しません".into(),
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
                    EditError::InvalidTarget("部品のルート要素は解除できません".into())
                })?;
                if crate::layout_kind(&crate::type_name(&element)).is_none() {
                    return Err(EditError::InvalidTarget("要素がレイアウトではありません".into()));
                }
                if wrapper.kind() != SyntaxKind::SubElement {
                    return Err(EditError::InvalidTarget(
                        "繰り返しや条件付きのレイアウトは解除できません".into(),
                    ));
                }
                let mut parent_path = layout.clone();
                parent_path.indices.pop();
                let (_, parent) = resolve_in(&root, &parent_path)?;
                if crate::layout_kind(&crate::type_name(&parent)).is_some() {
                    return Err(EditError::InvalidTarget(
                        "親がレイアウトのため、子の x / y は指定できません".into(),
                    ));
                }
                let children = plain_children(&element)?;
                if children.len() != positions.len() {
                    return Err(EditError::InvalidTarget(
                        "位置の数が子要素の数と一致しません".into(),
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
                let edit = replace_extent(&self.text, &wrapper, blocks.join("\n"));
                (apply_edits(&self.text, vec![edit])?, Some(parent_path))
            }
            Op::ExtractComponent { path, name } => {
                let edits = extract_component(&self.text, &root, &path, &name)?;
                (apply_edits(&self.text, edits)?, Some(path))
            }
            Op::AddDeclaration { component, text } => {
                let edit = add_declaration(&self.text, &root, &component, &text)?;
                (apply_edits(&self.text, vec![edit])?, None)
            }
            Op::Parameterize {
                path,
                property,
                name,
                ty,
            } => {
                let edits = parameterize(&self.text, &root, &path, &property, &name, &ty)?;
                (apply_edits(&self.text, edits)?, None)
            }
        };
        self.set_text(new_text);
        Ok(result)
    }
}
