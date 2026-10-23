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

/// Slint identifier (`[A-Za-z_][A-Za-z0-9_-]*`): the only thing a property name may be.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Why `expr` cannot be inlined as `name: expr;` into the wrapper (it would escape the
/// binding or swallow its neighbours), or `None`. Comments and `\{` interpolation have no use
/// in preview values and are refused.
fn shape_error(expr: &str) -> Option<&'static str> {
    let mut stack: Vec<char> = Vec::new();
    let mut in_str = false;
    let mut chars = expr.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => match chars.next() {
                    Some('{') => return Some("文字列の中の \\{ は使えません"),
                    Some(_) => {}
                    None => return Some("文字列が閉じていません"),
                },
                '"' => in_str = false,
                '\n' | '\r' => return Some("改行は使えません"),
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '/' if matches!(chars.peek(), Some('/' | '*')) => return Some("コメントは使えません"),
            '*' if chars.peek() == Some(&'/') => return Some("コメントは使えません"),
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

/// An error of a build: message and byte offset in the whole compiled source (text + wrapper).
struct RawError {
    message: String,
    full_offset: Option<usize>,
}

thread_local! {
    static COMPILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times the Slint compiler has run on this thread (for measuring).
pub fn compile_count() -> usize {
    COMPILES.with(|c| c.get())
}

/// Byte ranges (in the compiled source) of each binding ` name: expr;`, in order.
type Ranges = Vec<std::ops::Range<usize>>;

fn build_raw(
    text: &str,
    path: &Path,
    component: &str,
    values: &[(String, String)],
) -> (Result<Compiled, Vec<RawError>>, Ranges) {
    let head = format!(
        "{text}\nexport component {WRAPPER_NAME} inherits Rectangle {{ {component} {{ width: 100%; height: 100%;"
    );
    let mut full = head;
    let mut ranges = Vec::new();
    for (n, v) in values {
        let start = full.len();
        full += &format!(" {n}: {v};");
        ranges.push(start..full.len());
    }
    full += " } }\n";
    // Source files are matched by path: `..` and symlinks must not make them differ.
    let path = &path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    COMPILES.with(|c| c.set(c.get() + 1));
    let t0 = Instant::now();
    let compiler = slint_interpreter::Compiler::new();
    let result = spin_on::spin_on(compiler.build_from_source(full.clone(), path.to_path_buf()));
    let compile_ms = t0.elapsed().as_secs_f32() * 1000.0;
    let errors: Vec<RawError> = result
        .diagnostics()
        .filter(|d| d.level() == DiagnosticLevel::Error)
        .map(|d| {
            let (line, col) = d.line_column();
            RawError {
                message: d.message().to_string(),
                full_offset: line_col_to_offset(&full, line, col),
            }
        })
        .collect();
    if !errors.is_empty() {
        return (Err(errors), ranges);
    }
    let r = match result.component(WRAPPER_NORMALIZED) {
        Some(def) => Ok(Compiled {
            compile_ms,
            def,
            path: path.to_path_buf(),
            text_len: text.len() as u32,
        }),
        None => Err(vec![RawError {
            message: format!("component `{component}` not found"),
            full_offset: None,
        }]),
    };
    (r, ranges)
}

fn to_diagnostics(errors: Vec<RawError>, text_len: usize) -> Vec<Diagnostic> {
    errors
        .into_iter()
        .map(|e| Diagnostic {
            message: e.message,
            offset: e.full_offset.filter(|&o| o < text_len).map(|o| o as u32),
        })
        .collect()
}

fn build(
    text: &str,
    path: &Path,
    component: &str,
    values: &[(String, String)],
) -> Result<Compiled, Vec<Diagnostic>> {
    build_raw(text, path, component, values).0.map_err(|e| to_diagnostics(e, text.len()))
}

pub fn compile(text: &str, path: &Path, component: &str) -> Result<Compiled, Vec<Diagnostic>> {
    build(text, path, component, &[])
}

/// The result of `compile_with`: the build, and the values that were left out with the reason.
pub struct Built {
    pub compiled: Compiled,
    pub dropped: Vec<(String, String)>,
}

type Verdict = Result<(), String>;
type VerdictKey = (u64, String, String, String);

thread_local! {
    /// Whether a value compiles for a given source text (hash of text and path), component
    /// and property, so that a value already known good or bad is never compiled again.
    static VERDICTS: RefCell<std::collections::HashMap<VerdictKey, Verdict>> = RefCell::default();
}

fn text_key(text: &str, path: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    path.hash(&mut h);
    h.finish()
}

/// Forgets every remembered verdict (call when any project file changes on disk).
pub fn clear_verdicts() {
    VERDICTS.with(|v| v.borrow_mut().clear());
}

fn verdict_of(key: &VerdictKey) -> Option<Verdict> {
    VERDICTS.with(|v| v.borrow().get(key).cloned())
}

fn remember(key: VerdictKey, v: Verdict) {
    VERDICTS.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 2000 {
            m.clear();
        }
        m.insert(key, v);
    });
}

/// Compiles with `values` (property name, Slint expression) given to the edited component.
/// A value that does not compile is dropped (the others still apply) and reported in
/// `dropped` with the compiler's reason. Fails only when the text itself does not compile.
///
/// One compile in the common case. When it fails, errors located inside a binding are
/// attributed to that value (remembered per source text, so the next build with the same bad
/// value compiles once); only errors that cannot be attributed fall back to compiling each
/// unknown value alone.
pub fn compile_with(
    text: &str,
    path: &Path,
    component: &str,
    values: &[(String, String)],
) -> Result<Built, Vec<Diagnostic>> {
    let tk = text_key(text, path);
    let key = |n: &str, v: &str| (tk, component.to_string(), n.to_string(), v.to_string());
    let mut dropped: Vec<(String, String)> = Vec::new();
    // (name, expr, known good)
    let mut live: Vec<(String, String, bool)> = Vec::new();
    for (n, v) in values {
        if !is_identifier(n) {
            dropped.push((n.clone(), "プロパティ名として使えません".to_string()));
        } else if let Some(r) = shape_error(v) {
            dropped.push((n.clone(), r.to_string()));
        } else {
            match verdict_of(&key(n, v)) {
                Some(Err(why)) => dropped.push((n.clone(), why)),
                Some(Ok(())) => live.push((n.clone(), v.clone(), true)),
                None => live.push((n.clone(), v.clone(), false)),
            }
        }
    }
    let pairs = |l: &[(String, String, bool)]| -> Vec<(String, String)> {
        l.iter().map(|(n, v, _)| (n.clone(), v.clone())).collect()
    };
    let (first, ranges) = build_raw(text, path, component, &pairs(&live));
    let errors = match first {
        Ok(compiled) => {
            for (n, v, _) in &live {
                remember(key(n, v), Ok(()));
            }
            return Ok(Built { compiled, dropped });
        }
        Err(e) => e,
    };
    // Attribute errors to bindings by position.
    let mut bad: Vec<Option<String>> = vec![None; live.len()];
    let mut ambiguous = false;
    for e in &errors {
        let hit = e
            .full_offset
            .and_then(|o| ranges.iter().position(|r| r.contains(&o)));
        match hit {
            Some(i) => {
                bad[i].get_or_insert_with(|| e.message.clone());
            }
            None => ambiguous = true,
        }
    }
    if ambiguous || bad.iter().all(Option::is_none) {
        // The text itself may be broken, or the error cannot be attributed.
        build(text, path, component, &[])?;
        bad = vec![None; live.len()];
        for (i, (n, v, good)) in live.iter().enumerate() {
            if *good {
                continue;
            }
            if let Err(e) = build(text, path, component, &[(n.clone(), v.clone())]) {
                bad[i] = Some(e.first().map(|d| d.message.clone()).unwrap_or_default());
            }
        }
    }
    let mut keep: Vec<(String, String, bool)> = Vec::new();
    for ((n, v, good), why) in live.into_iter().zip(bad) {
        match why {
            Some(why) => {
                remember(key(&n, &v), Err(why.clone()));
                dropped.push((n, why));
            }
            None => keep.push((n, v, good)),
        }
    }
    // Only values that compile alone go into the combined build; verify the unknown ones.
    if keep.iter().any(|k| !k.2) {
        let mut verified = Vec::new();
        for (n, v, good) in keep {
            if good {
                verified.push((n, v, true));
                continue;
            }
            match build(text, path, component, &[(n.clone(), v.clone())]) {
                Ok(_) => {
                    remember(key(&n, &v), Ok(()));
                    verified.push((n, v, true));
                }
                Err(e) => {
                    let why = e.first().map(|d| d.message.clone()).unwrap_or_default();
                    remember(key(&n, &v), Err(why.clone()));
                    dropped.push((n, why));
                }
            }
        }
        keep = verified;
    }
    match build(text, path, component, &pairs(&keep)) {
        Ok(compiled) => Ok(Built { compiled, dropped }),
        Err(e) => {
            // Each value is fine alone but not together (or the text is broken).
            let bare = build(text, path, component, &[])?;
            let why = e.first().map(|d| d.message.clone()).unwrap_or_default();
            dropped.extend(keep.into_iter().map(|(n, _, _)| (n, why.clone())));
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
