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
/// its own element tree through its root element.
fn kids(e: &ElementRc) -> Vec<ElementRc> {
    let elem = e.borrow();
    let mut v: Vec<ElementRc> = elem.children.iter().rev().cloned().collect();
    if let ElementType::Component(c) = &elem.base_type {
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

pub fn compile(text: &str, path: &Path, component: &str) -> Result<Compiled, Vec<Diagnostic>> {
    let full = format!(
        "{text}\nexport component {WRAPPER_NAME} inherits Rectangle {{ {component} {{ width: 100%; height: 100%; }} }}\n"
    );
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
            for c in &kids(e) {
                if let Some(mut chain) = walk(li, c, p, depth + 1) {
                    if depth > 0 {
                        if let Some(h) = li.own_hit(e, p) {
                            chain.push(h);
                        }
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

    /// Geometry of the first instance of the element starting at `start`.
    pub fn geometry(&self, start: u32) -> Option<Hit> {
        fn find(li: &LiveInstance, e: &ElementRc, start: u32) -> Option<Hit> {
            if li.own_start(e) == Some(start) {
                let rect = li.inst.element_positions(e).into_iter().next()?;
                return Some(LiveInstance::hit_for(&rect, start));
            }
            kids(e).iter().find_map(|c| find(li, c, start))
        }
        find(self, &self.root(), start)
    }
}
