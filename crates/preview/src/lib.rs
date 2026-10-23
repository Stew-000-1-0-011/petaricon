//! Wraps slint-interpreter: compiles the text being edited, embeds it, and answers
//! hit-test / geometry queries by source offset.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use i_slint_compiler::langtype::ElementType;
use i_slint_compiler::object_tree::ElementRc;
use i_slint_compiler::parser::SyntaxKind;
use slint::ComponentHandle;
use slint_interpreter::highlight::HighlightedRect;
use slint_interpreter::{ComponentDefinition, ComponentInstance, DiagnosticLevel};
use tidy::Rect;

const WRAPPER_NAME: &str = "__PetariconPreviewRoot";
/// `WRAPPER_NAME` as Slint normalizes it (`__` becomes `_-`).
const WRAPPER_NORMALIZED: &str = "_-PetariconPreviewRoot";

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub message: String,
    /// Byte offset in the edited text; `None` for errors located in the appended wrapper.
    pub offset: Option<u32>,
}

pub struct Compiled {
    pub compile_ms: f32,
    def: ComponentDefinition,
    path: PathBuf,
    text_len: u32,
}

#[derive(Clone)]
pub struct LiveInstance {
    inst: Rc<ComponentInstance>,
    path: PathBuf,
    text_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub start: u32,
    /// In the preview area's coordinates.
    pub rect: Rect,
    /// Relative to the parent; matches the source `x` / `y`.
    pub local: Rect,
}

/// Child elements to visit, topmost first. A used (non-inlined) component also exposes
/// its own element tree through its root element, but only when `enter` is set: that is
/// wanted for the wrapper's use of the edited component, never for uses inside it. A use is
/// one node; what lies inside it belongs to another definition.
fn kids(e: &ElementRc, enter: bool) -> Vec<ElementRc> {
    let elem = e.borrow();
    let mut v: Vec<ElementRc> = elem.children.iter().rev().cloned().collect();
    if enter && let ElementType::Component(c) = &elem.base_type {
        v.push(c.root_element.clone());
    }
    v
}

fn to_rect(r: &i_slint_core::lengths::LogicalRect) -> Rect {
    Rect {
        x: r.origin.x,
        y: r.origin.y,
        w: r.size.width,
        h: r.size.height,
    }
}

fn line_col_to_offset(text: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut line_start = 0;
    for _ in 1..line {
        line_start += text[line_start..].find('\n')? + 1;
    }
    let rest = &text[line_start..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line_text = &rest[..line_end];
    // `column` is a 1-based byte column.
    let mut col = column.saturating_sub(1).min(line_text.len());
    while !line_text.is_char_boundary(col) {
        col -= 1;
    }
    Some(line_start + col)
}

/// Why `expr` cannot be inlined as `name: expr;` into the wrapper (it would escape the
/// binding), or `None`.
fn shape_error(expr: &str) -> Option<&'static str> {
    let mut stack: Vec<char> = Vec::new();
    let mut in_str = false;
    let mut chars = expr.chars();
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let open = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if stack.pop() != Some(open) {
                    return Some("括弧が対応していません");
                }
            }
            ';' if stack.is_empty() => return Some("`;` は使えません"),
            '\n' | '\r' => return Some("改行は使えません"),
            _ => {}
        }
    }
    if in_str {
        return Some("文字列が閉じていません");
    }
    if !stack.is_empty() {
        return Some("括弧が閉じていません");
    }
    None
}

fn build(
    text: &str,
    path: &Path,
    component: &str,
    values: &[(String, String)],
) -> Result<Compiled, Vec<Diagnostic>> {
    let bindings: String = values.iter().map(|(n, v)| format!(" {n}: {v};")).collect();
    let full = format!(
        "{text}\nexport component {WRAPPER_NAME} inherits Rectangle {{ {component} {{ width: 100%; height: 100%;{bindings} }} }}\n"
    );
    // Source files are matched by path: `..` and symlinks must not make them differ.
    let path = &path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let t0 = Instant::now();
    let compiler = slint_interpreter::Compiler::new();
    let result = spin_on::spin_on(compiler.build_from_source(full.clone(), path.to_path_buf()));
    let compile_ms = t0.elapsed().as_secs_f32() * 1000.0;
    let errors: Vec<Diagnostic> = result
        .diagnostics()
        .filter(|d| d.level() == DiagnosticLevel::Error)
        .map(|d| {
            let (line, col) = d.line_column();
            let offset = line_col_to_offset(&full, line, col)
                .filter(|&o| o < text.len())
                .map(|o| o as u32);
            Diagnostic {
                message: d.message().to_string(),
                offset,
            }
        })
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    match result.component(WRAPPER_NORMALIZED) {
        Some(def) => Ok(Compiled {
            compile_ms,
            def,
            path: path.to_path_buf(),
            text_len: text.len() as u32,
        }),
        None => Err(vec![Diagnostic {
            message: format!("component `{component}` not found"),
            offset: None,
        }]),
    }
}

pub fn compile(text: &str, path: &Path, component: &str) -> Result<Compiled, Vec<Diagnostic>> {
    build(text, path, component, &[])
}

/// The result of `compile_with`: the build, and the values that were left out with the reason.
pub struct Built {
    pub compiled: Compiled,
    pub dropped: Vec<(String, String)>,
}

/// Compiles with `values` (property name, Slint expression) given to the edited component.
/// A value that does not compile is dropped (the others still apply) and reported in
/// `dropped` with the compiler's reason. Fails only when the text itself does not compile.
pub fn compile_with(
    text: &str,
    path: &Path,
    component: &str,
    values: &[(String, String)],
) -> Result<Built, Vec<Diagnostic>> {
    let mut dropped: Vec<(String, String)> = Vec::new();
    let mut values_ok: Vec<(String, String)> = Vec::new();
    for (n, v) in values {
        match shape_error(v) {
            Some(r) => dropped.push((n.clone(), r.to_string())),
            None => values_ok.push((n.clone(), v.clone())),
        }
    }
    if let Ok(compiled) = build(text, path, component, &values_ok) {
        return Ok(Built { compiled, dropped });
    }
    let bare = build(text, path, component, &[])?;
    let mut keep = Vec::new();
    for (name, expr) in &values_ok {
        let one = [(name.clone(), expr.clone())];
        match build(text, path, component, &one) {
            Err(e) => dropped.push((name.clone(), e.first().map(|d| d.message.clone()).unwrap_or_default())),
            Ok(_) => keep.push((name.clone(), expr.clone())),
        }
    }
    if keep.is_empty() {
        return Ok(Built { compiled: bare, dropped });
    }
    match build(text, path, component, &keep) {
        Ok(compiled) => Ok(Built { compiled, dropped }),
        Err(e) => {
            // Each value is fine alone but not together.
            let why = e.first().map(|d| d.message.clone()).unwrap_or_default();
            dropped.extend(keep.into_iter().map(|(n, _)| (n, why.clone())));
            Ok(Built { compiled: bare, dropped })
        }
    }
}

impl Compiled {
    fn live(&self, inst: ComponentInstance) -> LiveInstance {
        LiveInstance {
            inst: Rc::new(inst),
            path: self.path.clone(),
            text_len: self.text_len,
        }
    }

    /// A factory for a `ComponentContainer`; each created instance is stored in `slot`.
    pub fn factory(&self, slot: Rc<RefCell<Option<LiveInstance>>>) -> slint::ComponentFactory {
        self.factory_with(slot, || {})
    }

    /// Like `factory`, and runs `on_created` right after the new instance is stored in `slot`
    /// (that is when the container actually switches over to this build).
    pub fn factory_with(
        &self,
        slot: Rc<RefCell<Option<LiveInstance>>>,
        on_created: impl Fn() + 'static,
    ) -> slint::ComponentFactory {
        let def = self.def.clone();
        let path = self.path.clone();
        let text_len = self.text_len;
        slint::ComponentFactory::new(move |ctx| {
            let inst = def.create_embedded(ctx).ok()?;
            *slot.borrow_mut() = Some(LiveInstance {
                inst: Rc::new(inst.clone_strong()),
                path: path.clone(),
                text_len,
            });
            on_created();
            Some(inst)
        })
    }

    /// A standalone instance sized `width` x `height` (for tests).
    pub fn instantiate(&self, width: f32, height: f32) -> LiveInstance {
        let inst = self.def.create().expect("create preview instance");
        inst.window()
            .set_size(slint::LogicalSize::new(width, height));
        self.live(inst)
    }
}

impl LiveInstance {
    /// The element the wrapper appended after the edited text uses to place the edited
    /// component: the only use whose inside is entered.
    fn is_wrapper_use(&self, e: &ElementRc) -> bool {
        e.borrow().debug.iter().any(|d| {
            d.node.source_file.path() == self.path
                && u32::from(d.node.text_range().start()) >= self.text_len
        })
    }

    fn own_start(&self, e: &ElementRc) -> Option<u32> {
        let elem = e.borrow();
        let dbg = elem.debug.iter().find(|d| {
            d.node.source_file.path() == self.path
                && d.node
                    .parent()
                    .is_some_and(|p| p.kind() != SyntaxKind::Component)
                && u32::from(d.node.text_range().start()) < self.text_len
        })?;
        Some(u32::from(dbg.node.text_range().start()))
    }

    fn hit_for(rect: &HighlightedRect, start: u32) -> Hit {
        Hit {
            start,
            rect: to_rect(&rect.rect),
            local: to_rect(&rect.local_rect),
        }
    }

    /// The text of every `Text` item that is currently visible in the window (not hidden by
    /// `visible: false`, a zero-size clip or an ancestor's clip), in tree order. Shows what the
    /// preview really displays.
    pub fn visible_texts(&self) -> Vec<String> {
        use i_slint_core::items::{ComplexText, ItemRc, SimpleText};
        let window = i_slint_core::window::WindowInner::from_pub(self.inst.window());
        let component = window.component();
        let mut out = Vec::new();
        let mut stack = vec![ItemRc::new_root(component)];
        while let Some(item) = stack.pop() {
            if let Some(next) = item.next_sibling() {
                stack.push(next);
            }
            if item.is_visible() {
                if let Some(t) = item.downcast::<SimpleText>() {
                    out.push(SimpleText::FIELD_OFFSETS.text().apply_pin(t.as_pin_ref()).get().to_string());
                } else if let Some(t) = item.downcast::<ComplexText>() {
                    out.push(ComplexText::FIELD_OFFSETS.text().apply_pin(t.as_pin_ref()).get().to_string());
                }
            }
            if let Some(child) = item.first_child() {
                stack.push(child);
            }
        }
        out
    }

    fn root(&self) -> ElementRc {
        self.inst.definition().root_component().root_element.clone()
    }

    pub fn hits_at(&self, x: f32, y: f32) -> Vec<Hit> {
        let p = i_slint_core::lengths::LogicalPoint::new(x, y);
        fn walk(
            li: &LiveInstance,
            e: &ElementRc,
            p: i_slint_core::lengths::LogicalPoint,
            depth: usize,
        ) -> Option<Vec<Hit>> {
            for c in &kids(e, li.is_wrapper_use(e)) {
                if let Some(mut chain) = walk(li, c, p, depth + 1) {
                    if depth > 0
                        && let Some(h) = li.own_hit(e, p)
                    {
                        chain.push(h);
                    }
                    return Some(chain);
                }
            }
            if depth == 0 {
                return None;
            }
            li.own_hit(e, p).map(|h| vec![h])
        }
        let mut out: Vec<Hit> = Vec::new();
        for h in walk(self, &self.root(), p, 0).unwrap_or_default() {
            if !out.iter().any(|o| o.start == h.start) {
                out.push(h);
            }
        }
        out
    }

    fn own_hit(&self, e: &ElementRc, p: i_slint_core::lengths::LogicalPoint) -> Option<Hit> {
        let start = self.own_start(e)?;
        let rect = self
            .inst
            .element_positions(e)
            .into_iter()
            .find(|r| r.contains(p))?;
        Some(Self::hit_for(&rect, start))
    }

    /// Geometry of the edited component's root element (which has no source start of its own
    /// that `geometry` could look up): the wrapper's single child, the use site of the
    /// component. `Hit::start` is 0.
    pub fn root_geometry(&self) -> Option<Hit> {
        let used = kids(&self.root(), false).into_iter().next()?;
        let rect = self.inst.element_positions(&used).into_iter().next()?;
        Some(Self::hit_for(&rect, 0))
    }

    /// Geometry of the first instance of the element starting at `start`.
    pub fn geometry(&self, start: u32) -> Option<Hit> {
        fn find(li: &LiveInstance, e: &ElementRc, start: u32) -> Option<Hit> {
            if li.own_start(e) == Some(start) {
                let rect = li.inst.element_positions(e).into_iter().next()?;
                return Some(LiveInstance::hit_for(&rect, start));
            }
            kids(e, li.is_wrapper_use(e)).iter().find_map(|c| find(li, c, start))
        }
        find(self, &self.root(), start)
    }
}
