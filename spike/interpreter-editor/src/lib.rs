//! Throwaway spike: can slint-interpreter drive a touch editor on Android?
//! Checks (1) runtime compile + embed, (2) tap-to-element hit testing,
//! (3) drag to move by rewriting the source and recompiling, with timings.

mod edit;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use i_slint_compiler::object_tree::ElementRc;
use i_slint_compiler::parser::SyntaxKind;
use slint::ComponentHandle;
use slint_interpreter::{ComponentDefinition, ComponentInstance};

slint::include_modules!();

const INITIAL_SOURCE: &str = include_str!("../design/design.slint");
const DESIGN_PATH: &str = "/spike/design.slint";

struct Selection {
    /// Start offset of the element's syntax node; stable across edits inside it.
    element_start: u32,
    type_name: String,
    local_x: f32,
    local_y: f32,
}

struct State {
    source: String,
    instance: Rc<RefCell<Option<ComponentInstance>>>,
    selection: Option<Selection>,
    last_compile_ms: f32,
    last_instantiate_ms: Rc<RefCell<f32>>,
}

fn path() -> PathBuf {
    PathBuf::from(DESIGN_PATH)
}

fn compile(source: &str) -> Result<(ComponentDefinition, f32), String> {
    let t0 = Instant::now();
    let compiler = slint_interpreter::Compiler::new();
    let result = spin_on::spin_on(compiler.build_from_source(source.to_string(), path()));
    let ms = t0.elapsed().as_secs_f32() * 1000.0;
    let errors: Vec<String> = result
        .diagnostics()
        .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
        .map(|d| d.message().to_string())
        .collect();
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    result.component("Design").map(|d| (d, ms)).ok_or_else(|| "no Design component".into())
}

fn install(editor: &Editor, st: &mut State, def: ComponentDefinition, compile_ms: f32) {
    st.last_compile_ms = compile_ms;
    let slot = st.instance.clone();
    let inst_ms = st.last_instantiate_ms.clone();
    editor.set_factory(slint::ComponentFactory::new(move |ctx| {
        let t0 = Instant::now();
        let inst = def.create_embedded(ctx).ok()?;
        *inst_ms.borrow_mut() = t0.elapsed().as_secs_f32() * 1000.0;
        *slot.borrow_mut() = Some(inst.clone_strong());
        Some(inst)
    }));
}

/// Walks the compiled element tree depth-first, topmost child first, and returns the
/// first element of the design file (excluding the root) that covers `pos`.
fn hit_test(
    instance: &ComponentInstance,
    design_start: u32,
    pos: slint::LogicalPosition,
) -> Option<(u32, String, slint_interpreter::highlight::HighlightedRect)> {
    fn walk(
        instance: &ComponentInstance,
        e: &ElementRc,
        design_start: u32,
        pos: slint::LogicalPosition,
        depth: usize,
    ) -> Option<(u32, String, slint_interpreter::highlight::HighlightedRect)> {
        for c in e.borrow().children.iter().rev() {
            if let Some(hit) = walk(instance, c, design_start, pos, depth + 1) {
                return Some(hit);
            }
        }
        if depth == 0 {
            return None;
        }
        let p = i_slint_core_point(pos);
        let rect = instance.element_positions(e).into_iter().find(|r| r.contains(p))?;
        let elem = e.borrow();
        // Inlined sub-components carry several debug nodes; pick the one written in `Design`.
        let dbg = elem.debug.iter().find(|d| {
            d.node.source_file.path() == Path::new(DESIGN_PATH)
                && u32::from(d.node.text_range().start()) > design_start
                // The component's own root element is not selectable.
                && d.node.parent().is_some_and(|p| p.kind() != SyntaxKind::Component)
        })?;
        Some((u32::from(dbg.node.text_range().start()), dbg.type_name.clone(), rect))
    }
    let root = instance.definition().root_component().root_element.clone();
    walk(instance, &root, design_start, pos, 0)
}

fn i_slint_core_point(p: slint::LogicalPosition) -> i_slint_core::lengths::LogicalPoint {
    i_slint_core::lengths::LogicalPoint::new(p.x, p.y)
}

fn element_first_line(source: &str, start: u32) -> String {
    source[start as usize..].lines().next().unwrap_or_default().trim().to_string()
}

fn status(editor: &Editor, st: &State, extra: &str) {
    editor.set_status(
        format!(
            "compile {:.1} ms | instantiate {:.1} ms {extra}",
            st.last_compile_ms,
            *st.last_instantiate_ms.borrow()
        )
        .into(),
    );
}

pub fn run() -> Result<(), slint::PlatformError> {
    let editor = Editor::new()?;
    let state = Rc::new(RefCell::new(State {
        source: INITIAL_SOURCE.to_string(),
        instance: Default::default(),
        selection: None,
        last_compile_ms: 0.0,
        last_instantiate_ms: Default::default(),
    }));

    let reload = {
        let editor = editor.as_weak();
        let state = state.clone();
        move || {
            let editor = editor.unwrap();
            let mut st = state.borrow_mut();
            st.source = INITIAL_SOURCE.to_string();
            st.selection = None;
            editor.set_has_selection(false);
            match compile(&st.source) {
                Ok((def, ms)) => install(&editor, &mut st, def, ms),
                Err(e) => editor.set_status(format!("compile error: {e}").into()),
            }
            status(&editor, &st, "");
        }
    };
    reload();
    editor.on_reload(reload);

    editor.on_tapped({
        let editor = editor.as_weak();
        let state = state.clone();
        move |mx, my| {
            let editor = editor.unwrap();
            let mut st = state.borrow_mut();
            let Some(instance) = st.instance.borrow().as_ref().map(|i| i.clone_strong()) else {
                return;
            };
            let design_start = st.source.find("export component Design").unwrap_or(0) as u32;
            // Highlight geometry is relative to the embedded component's root, i.e. the design area.
            let pos = slint::LogicalPosition::new(mx, my);
            let t0 = Instant::now();
            let hit = hit_test(&instance, design_start, pos);
            let hit_ms = t0.elapsed().as_secs_f32() * 1000.0;
            match hit {
                Some((start, type_name, rect)) => {
                    editor.set_sel_x(rect.rect.origin.x);
                    editor.set_sel_y(rect.rect.origin.y);
                    editor.set_sel_w(rect.rect.size.width);
                    editor.set_sel_h(rect.rect.size.height);
                    editor.set_has_selection(true);
                    let layout = edit::parent_layout(&st.source, &path(), start);
                    editor.set_selection_text(
                        format!(
                            "{type_name} @ ({:.0}, {:.0}) {:.0}x{:.0}{}",
                            rect.local_rect.origin.x,
                            rect.local_rect.origin.y,
                            rect.local_rect.size.width,
                            rect.local_rect.size.height,
                            layout.map(|l| format!(" | in {l}: not movable")).unwrap_or_default()
                        )
                        .into(),
                    );
                    st.selection = Some(Selection {
                        element_start: start,
                        type_name,
                        local_x: rect.local_rect.origin.x,
                        local_y: rect.local_rect.origin.y,
                    });
                }
                None => {
                    editor.set_has_selection(false);
                    editor.set_selection_text("nothing here".into());
                    st.selection = None;
                }
            }
            status(&editor, &st, &format!("| hit-test {hit_ms:.2} ms"));
        }
    });

    editor.on_drag_finished({
        let editor = editor.as_weak();
        let state = state.clone();
        move |dx, dy| {
            let editor = editor.unwrap();
            let mut st = state.borrow_mut();
            let Some(sel) = st.selection.as_ref() else { return };
            if edit::parent_layout(&st.source, &path(), sel.element_start).is_some() {
                editor.set_selection_text(format!("{}: inside a layout, not moved", sel.type_name).into());
                return;
            }
            let (nx, ny) = ((sel.local_x + dx).round() as i32, (sel.local_y + dy).round() as i32);
            let t0 = Instant::now();
            let Some(new_source) = edit::set_position(&st.source, &path(), sel.element_start, nx, ny)
            else {
                editor.set_selection_text("edit failed: element not found".into());
                return;
            };
            let edit_ms = t0.elapsed().as_secs_f32() * 1000.0;
            match compile(&new_source) {
                Ok((def, ms)) => {
                    let new_line_source = new_source.clone();
                    st.source = new_source;
                    install(&editor, &mut st, def, ms);
                    editor.set_sel_x(editor.get_sel_x() + dx);
                    editor.set_sel_y(editor.get_sel_y() + dy);
                    let sel = st.selection.as_mut().unwrap();
                    sel.local_x = nx as f32;
                    sel.local_y = ny as f32;
                    let line = element_first_line(&new_line_source, sel.element_start);
                    editor.set_selection_text(format!("moved -> {line}").into());
                }
                Err(e) => editor.set_selection_text(format!("compile error, reverted: {e}").into()),
            }
            status(&editor, &st, &format!("| edit {edit_ms:.2} ms"));
        }
    });

    editor.on_bench({
        let editor = editor.as_weak();
        let state = state.clone();
        move || {
            let editor = editor.unwrap();
            let st = state.borrow();
            let mut times = Vec::new();
            for _ in 0..10 {
                match compile(&st.source) {
                    Ok((_, ms)) => times.push(ms),
                    Err(e) => {
                        editor.set_status(format!("compile error: {e}").into());
                        return;
                    }
                }
            }
            times.sort_by(f32::total_cmp);
            editor.set_status(
                format!(
                    "compile x10: min {:.1} / median {:.1} / max {:.1} ms",
                    times[0], times[5], times[9]
                )
                .into(),
            );
        }
    });

    editor.run()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: slint::android::AndroidApp) {
    slint::android::init(android_app).unwrap();
    run().unwrap();
}
