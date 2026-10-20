//! The editing session: document, live preview instance, selection and geometry edits.

use std::cell::RefCell;
use std::rc::Rc;

use preview::{Diagnostic, Hit, LiveInstance, compile};
use slint_doc::{
    Document, EditError, ElementInfo, ElementKind, ElementPath, LayoutKind, Op, TreeNode,
};

use crate::guides::{self, Guides};
use crate::layout_bridge::layout_spec;

const MIN_SIZE: f32 = 4.0;
const TAP_THRESHOLD: f32 = 8.0;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("nothing is selected")]
    NoSelection,
    #[error("the element is positioned by a layout")]
    InLayout,
    #[error("the element cannot contain children")]
    NotAContainer,
    #[error("the element is not a layout")]
    NotALayout,
    #[error("the element has no geometry in the preview")]
    NoGeometry,
    #[error("compile failed")]
    Compile(Vec<Diagnostic>),
    #[error("the source changed since it was shown; reopen the source tab")]
    StaleSource,
}

/// How a property value is entered and normalised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Length,
    Color,
    Text,
    Other,
}

/// One row of the property list; `expr` is empty when the property is not set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropRow {
    pub name: String,
    pub expr: String,
    pub kind: PropKind,
}

pub fn prop_kind(name: &str) -> PropKind {
    match name {
        "x" | "y" | "width" | "height" | "spacing" | "border-radius" | "border-width"
        | "font-size" => PropKind::Length,
        n if n.starts_with("padding") => PropKind::Length,
        "color" | "background" | "border-color" => PropKind::Color,
        "text" => PropKind::Text,
        _ => PropKind::Other,
    }
}

const GEOMETRY_PROPS: [&str; 4] = ["x", "y", "width", "height"];
const LAYOUT_PROPS: [&str; 8] = [
    "spacing",
    "padding",
    "padding-left",
    "padding-right",
    "padding-top",
    "padding-bottom",
    "alignment",
    "cross-axis-alignment",
];
const TEXT_PROPS: [&str; 3] = ["text", "color", "font-size"];
const RECTANGLE_PROPS: [&str; 4] = ["background", "border-radius", "border-width", "border-color"];

/// A bare (optionally negative or decimal) number.
fn is_bare_number(v: &str) -> bool {
    let v = v.strip_prefix('-').unwrap_or(v);
    let mut dots = 0;
    !v.is_empty()
        && v.chars().any(|c| c.is_ascii_digit())
        && v.chars().all(|c| {
            if c == '.' {
                dots += 1;
            }
            c.is_ascii_digit() || c == '.'
        })
        && dots <= 1
}

fn normalise(kind: PropKind, value: &str) -> String {
    match kind {
        PropKind::Length if is_bare_number(value) => format!("{value}px"),
        PropKind::Text if !value.starts_with('"') => {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        }
        _ => value.to_string(),
    }
}

/// One row of the element tree, in depth-first display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: ElementPath,
    pub depth: usize,
    /// `"id := Type"` or `"Type"`.
    pub label: String,
    pub is_layout: bool,
}

/// Operations of a tree row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeAction {
    /// Reorder within the same parent.
    Up,
    Down,
    /// Become the next sibling of the parent.
    Out,
    /// Become the last child of the previous sibling, which must be a container.
    In,
    Delete,
    /// Append a new `Rectangle` to the element's parent.
    NewContainer,
}

/// Standard palette entries, in display order.
pub const STD_PALETTE: [&str; 13] = [
    "Rectangle",
    "Text",
    "Image",
    "TouchArea",
    "HorizontalLayout",
    "VerticalLayout",
    "Button",
    "CheckBox",
    "Slider",
    "SpinBox",
    "LineEdit",
    "ComboBox",
    "Switch",
];

/// Palette entries that live in `std-widgets.slint` and need an import.
const STD_WIDGETS: [&str; 7] = [
    "Button", "CheckBox", "Slider", "SpinBox", "LineEdit", "ComboBox", "Switch",
];

/// Assumed size of a freshly inserted element (its real size is unknown before compiling).
const NEW_SIZE: f32 = 40.0;
const PLACE_STEP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Callers must call `rebuild()` after each successful edit: geometry (tap, selection, move,
/// resize) comes from the last successful build, resolved against `snapshot`, the text that
/// build compiled.
pub struct Session {
    doc: Document,
    /// The text the live instance was built from. Replaced only when the instance is.
    snapshot: Rc<RefCell<Document>>,
    on_instance: Option<Rc<dyn Fn()>>,
    component: String,
    live: Rc<RefCell<Option<LiveInstance>>>,
    selection: Option<ElementPath>,
    last_tap: Option<(f32, f32)>,
    pub grid: i32,
    diagnostics: Vec<Diagnostic>,
}

fn snap(v: f32, grid: i32) -> i32 {
    if grid <= 0 {
        return v.round() as i32;
    }
    let g = grid as f32;
    ((v / g).round() * g) as i32
}

impl Session {
    pub fn new(doc: Document) -> Self {
        let comps = doc.components();
        let component = comps
            .iter()
            .find(|c| c.exported)
            .or_else(|| comps.first())
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let snapshot = Document::new(doc.path().to_path_buf(), doc.text().to_string());
        Session {
            snapshot: Rc::new(RefCell::new(snapshot)),
            on_instance: None,
            doc,
            component,
            live: Rc::new(RefCell::new(None)),
            selection: None,
            last_tap: None,
            grid: 4,
            diagnostics: Vec::new(),
        }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn set_component(&mut self, name: &str) {
        self.component = name.to_string();
        self.selection = None;
        self.last_tap = None;
    }

    pub fn selection(&self) -> Option<&ElementPath> {
        self.selection.as_ref()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn mark_saved(&mut self) {
        self.doc.mark_saved();
    }

    /// Called whenever a factory from `rebuild()` has created its instance, i.e. when the
    /// geometry queries start answering for the new build.
    pub fn set_on_instance(&mut self, f: impl Fn() + 'static) {
        self.on_instance = Some(Rc::new(f));
    }

    fn snapshot_doc(&self) -> Document {
        Document::new(self.doc.path().to_path_buf(), self.doc.text().to_string())
    }

    /// Compiles the document; on success returns a factory. The new snapshot is installed
    /// together with the instance, when the container runs the factory, so geometry queries
    /// always see an instance and the text it was built from.
    pub fn rebuild(&mut self) -> Option<slint::ComponentFactory> {
        match compile(self.doc.text(), self.doc.path(), &self.component) {
            Ok(c) => {
                self.diagnostics.clear();
                let pending = self.snapshot_doc();
                let snapshot = self.snapshot.clone();
                let hook = self.on_instance.clone();
                Some(c.factory_with(self.live.clone(), move || {
                    *snapshot.borrow_mut() = Document::new(
                        pending.path().to_path_buf(),
                        pending.text().to_string(),
                    );
                    if let Some(h) = &hook {
                        h();
                    }
                }))
            }
            Err(d) => {
                self.diagnostics = d;
                None
            }
        }
    }

    /// Compiles and instantiates standalone (for tests).
    pub fn rebuild_standalone(&mut self, width: f32, height: f32) -> bool {
        match compile(self.doc.text(), self.doc.path(), &self.component) {
            Ok(c) => {
                self.diagnostics.clear();
                *self.snapshot.borrow_mut() = self.snapshot_doc();
                *self.live.borrow_mut() = Some(c.instantiate(width, height));
                true
            }
            Err(d) => {
                self.diagnostics = d;
                false
            }
        }
    }

    pub fn tap(&mut self, x: f32, y: f32) -> Option<ElementPath> {
        let hits = self.live.borrow().as_ref()?.hits_at(x, y);
        let snapshot = self.snapshot.borrow();
        let candidates: Vec<ElementPath> = hits
            .iter()
            .filter_map(|h| snapshot.path_at_offset(h.start))
            .filter(|p| p.component == self.component)
            .collect();
        let near = self.last_tap.is_some_and(|(lx, ly)| {
            (lx - x).abs() <= TAP_THRESHOLD && (ly - y).abs() <= TAP_THRESHOLD
        });
        self.last_tap = Some((x, y));
        let current = self
            .selection
            .as_ref()
            .and_then(|s| candidates.iter().position(|c| c == s));
        let pick = match current {
            Some(i) if near => (i + 1) % candidates.len(),
            _ => 0,
        };
        self.selection = candidates.get(pick).cloned();
        self.selection.clone()
    }

    pub fn selection_hit(&self) -> Option<Hit> {
        let info = self.snapshot.borrow().element(self.selection.as_ref()?)?;
        self.live.borrow().as_ref()?.geometry(info.start)
    }

    /// The selected path and its geometry, refusing the root and layout children.
    fn movable(&self) -> Result<(ElementPath, Hit, bool), SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        if path.indices.is_empty() {
            return Err(EditError::InvalidTarget(
                "the root element cannot be moved or resized".into(),
            )
            .into());
        }
        let mut parent = path.clone();
        parent.indices.pop();
        let parent_info = self
            .doc
            .element(&parent)
            .ok_or_else(|| EditError::ElementNotFound(parent.clone()))?;
        let in_layout = matches!(parent_info.kind, ElementKind::Layout(_));
        let hit = self.selection_hit().ok_or(SessionError::NoGeometry)?;
        Ok((path, hit, in_layout))
    }

    pub fn move_selection(&mut self, dx: f32, dy: f32) -> Result<(), SessionError> {
        let (path, hit, in_layout) = self.movable()?;
        if in_layout {
            return Err(SessionError::InLayout);
        }
        let x = snap(hit.local.x + dx, self.grid);
        let y = snap(hit.local.y + dy, self.grid);
        self.apply(Op::SetBindings {
            path,
            bindings: vec![
                ("x".into(), Some(format!("{x}px"))),
                ("y".into(), Some(format!("{y}px"))),
            ],
        })
    }

    pub fn resize_selection(
        &mut self,
        corner: Corner,
        dx: f32,
        dy: f32,
    ) -> Result<(), SessionError> {
        let (path, hit, in_layout) = self.movable()?;
        let r = hit.local;
        let (left, top, right, bottom) = (r.x, r.y, r.x + r.w, r.y + r.h);
        let g = self.grid;
        let (mut l, mut t, mut rt, mut b) = (left, top, right, bottom);
        let (moves_left, moves_top) = match corner {
            Corner::TopLeft => (true, true),
            Corner::TopRight => (false, true),
            Corner::BottomLeft => (true, false),
            Corner::BottomRight => (false, false),
        };
        if moves_left {
            l = (snap(left + dx, g) as f32).min(right - MIN_SIZE);
        } else {
            rt = (snap(right + dx, g) as f32).max(left + MIN_SIZE);
        }
        if moves_top {
            t = (snap(top + dy, g) as f32).min(bottom - MIN_SIZE);
        } else {
            b = (snap(bottom + dy, g) as f32).max(top + MIN_SIZE);
        }
        let mut bindings = Vec::new();
        if !in_layout {
            bindings.push(("x".into(), Some(format!("{}px", l as i32))));
            bindings.push(("y".into(), Some(format!("{}px", t as i32))));
        }
        bindings.push((
            "width".into(),
            Some(format!("{}px", (rt - l).round() as i32)),
        ));
        bindings.push((
            "height".into(),
            Some(format!("{}px", (b - t).round() as i32)),
        ));
        self.apply(Op::SetBindings { path, bindings })
    }

    /// Applies an edit. While the preview is stale (`diagnostics` non-empty) structural ops
    /// (insert, remove, move, wrap, unwrap, extract) are refused with `Compile`; `SetBindings`
    /// and `AddDeclaration` stay allowed.
    pub fn apply(&mut self, op: Op) -> Result<(), SessionError> {
        let structural = !matches!(op, Op::SetBindings { .. } | Op::AddDeclaration { .. });
        if structural && !self.diagnostics.is_empty() {
            return Err(SessionError::Compile(self.diagnostics.clone()));
        }
        if let Some(p) = self.doc.apply(op)? {
            self.selection = Some(p);
        }
        Ok(())
    }

    fn revalidate(&mut self) {
        if let Some(s) = &self.selection {
            if self.doc.element(s).is_none() {
                self.selection = None;
            }
        }
    }

    /// Property rows of the selection: geometry first, then the ones that suit its kind, then
    /// any other written binding. Empty without a selection.
    pub fn properties(&self) -> Vec<PropRow> {
        let Some(info) = self.selection.as_ref().and_then(|s| self.doc.element(s)) else {
            return Vec::new();
        };
        let mut names: Vec<&str> = GEOMETRY_PROPS.to_vec();
        if matches!(info.kind, ElementKind::Layout(_)) {
            names.extend(LAYOUT_PROPS);
        }
        match info.type_name.as_str() {
            "Text" => names.extend(TEXT_PROPS),
            "Rectangle" => names.extend(RECTANGLE_PROPS),
            _ => {}
        }
        let mut rows: Vec<PropRow> = names
            .iter()
            .map(|n| PropRow {
                name: n.to_string(),
                expr: info
                    .bindings
                    .iter()
                    .find(|b| b.name == *n)
                    .map(|b| b.expr.clone())
                    .unwrap_or_default(),
                kind: prop_kind(n),
            })
            .collect();
        for b in &info.bindings {
            if !names.contains(&b.name.as_str()) {
                rows.push(PropRow {
                    name: b.name.clone(),
                    expr: b.expr.clone(),
                    kind: prop_kind(&b.name),
                });
            }
        }
        rows
    }

    /// Sets (or, for an empty value, removes) a property of the selection.
    pub fn set_property(&mut self, name: &str, value: &str) -> Result<(), SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        // Only the emptiness check and non-text values are trimmed: `"Hi "` keeps its space.
        let kind = prop_kind(name);
        let value = if kind == PropKind::Text && !value.trim().is_empty() {
            value
        } else {
            value.trim()
        };
        let expr = (!value.is_empty()).then(|| normalise(kind, value));
        let current = self
            .properties()
            .into_iter()
            .find(|r| r.name == name)
            .map(|r| r.expr)
            .unwrap_or_default();
        if current == expr.clone().unwrap_or_default() {
            return Ok(());
        }
        self.apply(Op::SetBindings { path, bindings: vec![(name.to_string(), expr)] })
    }

    /// Byte range of the source the Source tab shows: the selected element, else the whole
    /// current component.
    pub fn source_range(&self) -> Option<std::ops::Range<usize>> {
        match &self.selection {
            Some(s) => self.doc.element_range(s),
            None => self.doc.component_range(&self.component),
        }
    }

    /// Byte range of the whole component containing `offset`; the whole file when `offset`
    /// is `None` or lies in no component.
    pub fn component_range_at(&self, offset: Option<u32>) -> std::ops::Range<usize> {
        let whole = 0..self.doc.text().len();
        let Some(offset) = offset else { return whole };
        let offset = offset as usize;
        self.doc
            .components()
            .into_iter()
            .filter_map(|c| self.doc.component_range(&c.name))
            .find(|r| r.contains(&offset) || r.end == offset)
            .unwrap_or(whole)
    }

    /// Replaces only `start..start + original.len()` of the document with `new`, provided the
    /// text there is still `original`; otherwise `StaleSource` and nothing changes.
    pub fn replace_source(
        &mut self,
        start: usize,
        original: &str,
        new: &str,
    ) -> Result<(), SessionError> {
        let text = self.doc.text();
        let end = start + original.len();
        if text.get(start..end) != Some(original) {
            return Err(SessionError::StaleSource);
        }
        let updated = format!("{}{}{}", &text[..start], new, &text[end..]);
        self.replace_text(updated);
        Ok(())
    }

    pub fn replace_text(&mut self, text: String) {
        self.doc.replace_text(text);
        self.revalidate();
    }

    pub fn undo(&mut self) -> bool {
        let r = self.doc.undo();
        self.revalidate();
        r
    }

    pub fn redo(&mut self) -> bool {
        let r = self.doc.redo();
        self.revalidate();
        r
    }

    pub fn select_path(&mut self, path: ElementPath) {
        if self.doc.element(&path).is_some() {
            self.selection = Some(path);
        }
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        fn walk(n: &TreeNode, depth: usize, out: &mut Vec<TreeRow>) {
            let label = match &n.info.id {
                Some(id) => format!("{id} := {}", n.info.type_name),
                None => n.info.type_name.clone(),
            };
            out.push(TreeRow {
                path: n.info.path.clone(),
                depth,
                label,
                is_layout: matches!(n.info.kind, ElementKind::Layout(_)),
            });
            for c in &n.children {
                walk(c, depth + 1, out);
            }
        }
        let mut out = Vec::new();
        if let Some(t) = self.doc.tree(&self.component) {
            walk(&t, 0, &mut out);
        }
        out
    }

    /// The standard parts followed by the other components of the document.
    pub fn palette(&self) -> Vec<String> {
        let mut v: Vec<String> = STD_PALETTE.iter().map(|s| s.to_string()).collect();
        v.extend(
            self.doc
                .components()
                .into_iter()
                .map(|c| c.name)
                .filter(|n| *n != self.component),
        );
        v
    }

    /// A container (palette insertion goes inside it) is a component root, a `Rectangle`,
    /// `TouchArea`, `Window` or any layout; every other element is a leaf.
    fn is_container(info: &ElementInfo) -> bool {
        info.path.indices.is_empty()
            || matches!(info.kind, ElementKind::Layout(_))
            || matches!(info.type_name.as_str(), "Rectangle" | "TouchArea" | "Window")
    }

    fn local_rect_of(&self, path: &ElementPath) -> Option<(f32, f32, f32, f32)> {
        let info = self.snapshot.borrow().element(path)?;
        let hit = self.live.borrow().as_ref()?.geometry(info.start)?;
        Some((hit.local.x, hit.local.y, hit.local.w, hit.local.h))
    }

    /// First 8px-grid position (x first, then y) where an assumed 40x40 element fits inside
    /// `parent` without overlapping or touching its existing children.
    fn free_position(&self, parent: &ElementPath, child_count: usize) -> (i32, i32) {
        let Some((_, _, pw, ph)) = self.local_rect_of(parent) else {
            return (PLACE_STEP as i32, PLACE_STEP as i32);
        };
        let kids: Vec<_> = (0..child_count)
            .filter_map(|i| {
                let mut p = parent.clone();
                p.indices.push(i);
                self.local_rect_of(&p)
            })
            .collect();
        let mut y = PLACE_STEP;
        while y + NEW_SIZE <= ph {
            let mut x = PLACE_STEP;
            while x + NEW_SIZE <= pw {
                let overlaps = kids.iter().any(|&(kx, ky, kw, kh)| {
                    x <= kx + kw && kx <= x + NEW_SIZE && y <= ky + kh && ky <= y + NEW_SIZE
                });
                if !overlaps {
                    return (x as i32, y as i32);
                }
                x += PLACE_STEP;
            }
            y += PLACE_STEP;
        }
        (PLACE_STEP as i32, PLACE_STEP as i32)
    }

    fn refuse_if_stale(&self) -> Result<(), SessionError> {
        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(SessionError::Compile(self.diagnostics.clone()))
        }
    }

    /// Inserts `name` at the end of `parent`, adding the `std-widgets.slint` import when needed,
    /// as a single history step. Selects the new element.
    fn insert_part(&mut self, parent: ElementPath, name: &str) -> Result<(), SessionError> {
        self.refuse_if_stale()?;
        let parent_info = self
            .doc
            .element(&parent)
            .ok_or_else(|| EditError::ElementNotFound(parent.clone()))?;
        let mut props = Vec::new();
        if !matches!(parent_info.kind, ElementKind::Layout(_)) {
            let (x, y) = self.free_position(&parent, parent_info.child_count);
            props.push(format!("x: {x}px;"));
            props.push(format!("y: {y}px;"));
        }
        match name {
            "Rectangle" | "TouchArea" => {
                props.push("width: 40px;".into());
                props.push("height: 40px;".into());
                if name == "Rectangle" {
                    props.push("background: #808890;".into());
                }
            }
            "Text" => props.push("text: \"Text\";".into()),
            "Button" => props.push("text: \"Button\";".into()),
            "CheckBox" => props.push("text: \"CheckBox\";".into()),
            "Switch" => props.push("text: \"Switch\";".into()),
            _ => {}
        }
        let source = if props.is_empty() {
            format!("{name} {{ }}")
        } else {
            format!("{name} {{ {} }}", props.join(" "))
        };
        // Edit a scratch copy so the insert and the import land in one history step.
        let mut scratch = Document::new(self.doc.path().to_path_buf(), self.doc.text().to_string());
        let new_path = scratch
            .apply(Op::InsertElement { parent, index: None, source })?
            .ok_or_else(|| EditError::InvalidTarget("insert returned no path".into()))?;
        let mut text = scratch.text().to_string();
        if STD_WIDGETS.contains(&name) {
            text = with_std_import(&text, name);
        }
        self.doc.replace_text(text);
        self.selection = Some(new_path);
        Ok(())
    }

    /// Adds a part: into the selection when it is a container, else into its parent (the
    /// component root when nothing is selected).
    pub fn add_from_palette(&mut self, name: &str) -> Result<(), SessionError> {
        self.refuse_if_stale()?;
        let target = match &self.selection {
            None => ElementPath { component: self.component.clone(), indices: vec![] },
            Some(sel) => {
                let info = self
                    .doc
                    .element(sel)
                    .ok_or_else(|| EditError::ElementNotFound(sel.clone()))?;
                if Self::is_container(&info) {
                    sel.clone()
                } else {
                    let mut p = sel.clone();
                    p.indices.pop();
                    p
                }
            }
        };
        self.insert_part(target, name)
    }

    fn child_path(parent: &ElementPath, i: usize) -> ElementPath {
        let mut p = parent.clone();
        p.indices.push(i);
        p
    }

    /// Geometry in the live preview of the element at `path`.
    fn hit_of(&self, path: &ElementPath) -> Option<Hit> {
        let info = self.snapshot.borrow().element(path)?;
        self.live.borrow().as_ref()?.geometry(info.start)
    }

    /// Wraps the children of the selected container in an inferred layout and selects it.
    /// The positions come from the live preview; nothing changes when any is missing.
    pub fn tidy_selection(&mut self) -> Result<Vec<tidy::Warning>, SessionError> {
        let path = self.selection.clone().ok_or(SessionError::NoSelection)?;
        let info = self
            .doc
            .element(&path)
            .ok_or_else(|| EditError::ElementNotFound(path.clone()))?;
        if !Self::is_container(&info) || matches!(info.kind, ElementKind::Layout(_)) {
            return Err(SessionError::NotAContainer);
        }
        for i in 0..info.child_count {
            let kind = self.doc.element(&Self::child_path(&path, i)).map(|c| c.kind);
            if matches!(kind, Some(ElementKind::Repeated | ElementKind::Conditional)) {
                return Err(EditError::InvalidTarget(
                    "cannot tidy a container with for / if children".into(),
                )
                .into());
            }
        }
        let container = self.hit_of(&path).ok_or(SessionError::NoGeometry)?;
        let children = (0..info.child_count)
            .map(|i| self.hit_of(&Self::child_path(&path, i)).map(|h| h.local))
            .collect::<Option<Vec<_>>>()
            .ok_or(SessionError::NoGeometry)?;
        let plan = tidy::plan(container.rect.w, container.rect.h, &children, self.grid)
            .ok_or(SessionError::NotAContainer)?;
        self.apply(Op::WrapInLayout { container: path, layout: layout_spec(&plan) })?;
        Ok(plan.warnings)
    }

    /// The one layout that guides and untidy act on: the selection when it is a horizontal or
    /// vertical layout, else the selected container's only horizontal/vertical layout child
    /// (other children may exist). A container with several such layouts is ambiguous: `None`.
    fn guide_layout(&self) -> Option<(ElementPath, ElementInfo, LayoutKind)> {
        let path = self.selection.clone()?;
        let info = self.doc.element(&path)?;
        let stack = |i: &ElementInfo| match i.kind {
            ElementKind::Layout(k @ (LayoutKind::Horizontal | LayoutKind::Vertical)) => Some(k),
            _ => None,
        };
        if let Some(k) = stack(&info) {
            return Some((path, info, k));
        }
        if !Self::is_container(&info) {
            return None;
        }
        let mut found = (0..info.child_count).filter_map(|i| {
            let p = Self::child_path(&path, i);
            let ci = self.doc.element(&p)?;
            stack(&ci).map(|k| (p, ci, k))
        });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Path of the layout the guides currently describe.
    pub fn guide_layout_path(&self) -> Option<ElementPath> {
        self.guide_layout().map(|(p, ..)| p)
    }

    /// Replaces the layout found by the same rule as the guides by its children, keeping
    /// their on-screen positions rounded to the grid. `NotALayout` when there is none.
    pub fn untidy_selection(&mut self) -> Result<(), SessionError> {
        self.selection.as_ref().ok_or(SessionError::NoSelection)?;
        let (layout, layout_info, _) = self.guide_layout().ok_or(SessionError::NotALayout)?;
        let mut parent = layout.clone();
        parent.indices.pop();
        let origin = self.hit_of(&parent).ok_or(SessionError::NoGeometry)?.rect;
        let positions = (0..layout_info.child_count)
            .map(|i| {
                self.hit_of(&Self::child_path(&layout, i)).map(|h| {
                    (snap(h.rect.x - origin.x, self.grid), snap(h.rect.y - origin.y, self.grid))
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(SessionError::NoGeometry)?;
        self.apply(Op::Unwrap { layout, positions })
    }

    /// Guides of the selected layout, or of the layout inside the selected container.
    pub fn guides(&self) -> Option<Guides> {
        let (path, info, kind) = self.guide_layout()?;
        let layout = self.hit_of(&path)?;
        let children: Vec<Hit> = (0..info.child_count)
            .map(|i| self.hit_of(&Self::child_path(&path, i)))
            .collect::<Option<_>>()?;
        Some(guides::compute(&layout, kind, &children, &info.bindings))
    }

    /// Edits a property of the guided layout, selecting the layout first when needed.
    pub fn set_guide_property(&mut self, name: &str, value: &str) -> Result<(), SessionError> {
        let (path, ..) = self.guide_layout().ok_or(SessionError::NotALayout)?;
        let previous = self.selection.replace(path);
        let r = self.set_property(name, value);
        if r.is_err() {
            self.selection = previous;
        }
        r
    }

    pub fn tree_action(&mut self, path: &ElementPath, action: TreeAction) -> Result<(), SessionError> {
        let root_err = || {
            SessionError::Edit(EditError::InvalidTarget(
                "the root element cannot be moved or removed".into(),
            ))
        };
        let Some((&index, parent_indices)) = path.indices.split_last() else {
            return match action {
                TreeAction::NewContainer => self.insert_part(path.clone(), "Rectangle"),
                _ => Err(root_err()),
            };
        };
        let parent = ElementPath { component: path.component.clone(), indices: parent_indices.to_vec() };
        let nothing = |what: &str| {
            SessionError::Edit(EditError::InvalidTarget(format!("cannot move {what}")))
        };
        match action {
            TreeAction::Up => {
                if index == 0 {
                    return Err(nothing("further up"));
                }
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: parent,
                    index: Some(index - 1),
                })
            }
            TreeAction::Down => {
                let count = self.doc.element(&parent).map_or(0, |p| p.child_count);
                if index + 1 >= count {
                    return Err(nothing("further down"));
                }
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: parent,
                    index: Some(index + 1),
                })
            }
            TreeAction::Out => {
                let Some((&parent_index, grand)) = parent.indices.split_last() else {
                    return Err(nothing("out of the root"));
                };
                let grandparent = ElementPath { component: path.component.clone(), indices: grand.to_vec() };
                self.apply(Op::MoveElement {
                    path: path.clone(),
                    new_parent: grandparent,
                    index: Some(parent_index + 1),
                })
            }
            TreeAction::In => {
                if index == 0 {
                    return Err(nothing("in: no previous sibling"));
                }
                let mut sibling = parent;
                sibling.indices.push(index - 1);
                let info = self
                    .doc
                    .element(&sibling)
                    .ok_or_else(|| EditError::ElementNotFound(sibling.clone()))?;
                if !Self::is_container(&info) {
                    return Err(SessionError::NotAContainer);
                }
                self.apply(Op::MoveElement { path: path.clone(), new_parent: sibling, index: None })
            }
            TreeAction::Delete => {
                self.apply(Op::RemoveElement { path: path.clone() })?;
                self.selection = Some(parent);
                self.revalidate();
                Ok(())
            }
            TreeAction::NewContainer => self.insert_part(parent, "Rectangle"),
        }
    }
}

/// Adds `name` to the `import { ... } from "std-widgets.slint";` line, creating the line at the
/// top of the file when there is none. Everything else in the file stays byte-identical:
/// the name is inserted right after the last entry, matching a multi-line layout and a
/// trailing comma when the import uses them. An entry already imported (also under an alias,
/// `Button as B`) is left alone.
fn with_std_import(text: &str, name: &str) -> String {
    const MODULE: &str = "\"std-widgets.slint\"";
    if let Some(from) = text.find(MODULE)
        && let Some(close) = text[..from].rfind('}')
        && let Some(open) = text[..close].rfind('{')
        && text[..open].trim_end().ends_with("import")
    {
        let names = &text[open + 1..close];
        if names.split(',').any(|n| n.split_whitespace().next() == Some(name)) {
            return text.to_string();
        }
        let (at, insert) = match names.rfind(|c: char| !c.is_whitespace()) {
            None if names.is_empty() => (open + 1, format!(" {name} ")),
            None => (close, format!("{name} ")),
            Some(last) => {
                let trailing_comma = names[last..].starts_with(',');
                let at = open + 1 + last + 1;
                let insert = if names.contains('\n') {
                    let line_start = names[..last].rfind('\n').map_or(0, |n| n + 1);
                    let line = &names[line_start..];
                    let indent = &line[..line.len() - line.trim_start().len()];
                    if trailing_comma {
                        format!("\n{indent}{name},")
                    } else {
                        format!(",\n{indent}{name}")
                    }
                } else if trailing_comma {
                    format!(" {name},")
                } else {
                    format!(", {name}")
                };
                (at, insert)
            }
        };
        return format!("{}{insert}{}", &text[..at], &text[at..]);
    }
    format!("import {{ {name} }} from {MODULE};\n{text}")
}
