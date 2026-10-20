//! The editing session: document, live preview instance, selection and geometry edits.

use std::cell::RefCell;
use std::rc::Rc;

use preview::{Diagnostic, Hit, LiveInstance, compile};
use slint_doc::{Document, EditError, ElementKind, ElementPath, Op};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub struct Session {
    doc: Document,
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
        Session {
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

    /// Compiles the document; on success returns a factory whose created instance lands in `live`.
    pub fn rebuild(&mut self) -> Option<slint::ComponentFactory> {
        match compile(self.doc.text(), self.doc.path(), &self.component) {
            Ok(c) => {
                self.diagnostics.clear();
                Some(c.factory(self.live.clone()))
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
        let candidates: Vec<ElementPath> = hits
            .iter()
            .filter_map(|h| self.doc.path_at_offset(h.start))
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
        let info = self.doc.element(self.selection.as_ref()?)?;
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
        bindings.push(("width".into(), Some(format!("{}px", (rt - l) as i32))));
        bindings.push(("height".into(), Some(format!("{}px", (b - t) as i32))));
        self.apply(Op::SetBindings { path, bindings })
    }

    pub fn apply(&mut self, op: Op) -> Result<(), SessionError> {
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
}
