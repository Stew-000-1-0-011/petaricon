//! 「中を開く」 / 「戻る」: jumping from a use of a component to its definition and back.

use storage::ProjectStore;

use crate::glue::save_or_recover;
use crate::session::{Session, UseTarget};
use slint_doc::ElementPath;

/// Where to return to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    file: String,
    component: String,
    selection: Option<ElementPath>,
}

/// The stack of places 「戻る」 returns to (file, component, selection), innermost last.
#[derive(Debug, Default)]
pub struct Nav {
    stack: Vec<Frame>,
}

impl Nav {
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }
}

/// Loads a session for a file of the project (its preview is built by the caller).
pub type Loader<'a> = &'a dyn Fn(&str) -> Result<Session, String>;

/// A file name from an `import` path, when it names a file of the (flat) project.
fn project_file(store: &dyn ProjectStore, project: &str, path: &str) -> Result<String, String> {
    let name = path.strip_prefix("./").unwrap_or(path);
    let known = !name.contains('/') && store.files(project).is_ok_and(|f| f.iter().any(|x| x == name));
    if known {
        Ok(name.to_string())
    } else {
        Err(format!("{path} はこのプロジェクトにないため、開けません"))
    }
}

/// Saves unsaved edits of the open file (before it is left for another file).
fn save_open(
    store: &dyn ProjectStore,
    project: &str,
    file: &str,
    session: &mut Session,
    loaded: bool,
) -> Result<(), String> {
    if loaded {
        save_or_recover(store, project, file, session, true)?;
    }
    Ok(())
}

/// Opens the definition of the selected component use: the same file switches the edited
/// component, another file (found through the `import`) is saved-for and opened. The place left
/// is pushed for `go_back`. Nothing changes on an error (unsaved edits may have been saved).
pub fn open_inside(
    store: &dyn ProjectStore,
    project: &str,
    file: &mut String,
    session: &mut Session,
    loaded: bool,
    nav: &mut Nav,
    load: Loader,
) -> Result<(), String> {
    let target = session.selected_use().ok_or_else(|| crate::session::SessionError::NotAUse.to_string())?;
    let frame = Frame {
        file: file.clone(),
        component: session.component().to_string(),
        selection: session.selection().cloned(),
    };
    match target {
        UseTarget::Local(name) => session.set_component(&name),
        UseTarget::Imported { path, component } => {
            let other = project_file(store, project, &path)?;
            save_open(store, project, file, session, loaded)?;
            let mut next = load(&other)?;
            if !next.doc().components().iter().any(|c| c.name == component) {
                return Err(format!("{other} に部品 {component} が見つかりません"));
            }
            next.set_component(&component);
            *session = next;
            *file = other;
        }
    }
    nav.stack.push(frame);
    Ok(())
}

/// Returns to the place `open_inside` left: file, component and selection.
pub fn go_back(
    store: &dyn ProjectStore,
    project: &str,
    file: &mut String,
    session: &mut Session,
    loaded: bool,
    nav: &mut Nav,
    load: Loader,
) -> Result<(), String> {
    let frame = nav.stack.last().cloned().ok_or("戻る先がありません")?;
    if frame.file != *file {
        save_open(store, project, file, session, loaded)?;
        *session = load(&frame.file)?;
        *file = frame.file.clone();
    }
    session.set_component(&frame.component);
    if let Some(sel) = frame.selection {
        session.select_path(sel);
    }
    nav.stack.pop();
    Ok(())
}
