//! Connects the Slint `Editor` window to the editing `Session` and the project store.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use slint_doc::Document;
use storage::ProjectStore;

use crate::Editor;
use crate::session::{Corner, Session};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const DEFAULT_PROJECT: &str = "default";
const DEFAULT_FILE: &str = "main.slint";

thread_local! {
    static SAVE_HOOK: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Saves the open file. Safe to call from platform callbacks on the UI thread (e.g. Android
/// pause); does nothing before the editor has started.
pub fn save_now() {
    let hook = SAVE_HOOK.with(|h| h.borrow().clone());
    if let Some(h) = hook {
        h();
    }
}

fn to_model(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    ))
}

fn corner_from(i: i32) -> Corner {
    match i {
        0 => Corner::TopLeft,
        1 => Corner::TopRight,
        2 => Corner::BottomLeft,
        _ => Corner::BottomRight,
    }
}

fn platform_err(e: impl std::fmt::Display) -> slint::PlatformError {
    slint::PlatformError::Other(e.to_string())
}

pub struct Glue {
    editor: slint::Weak<Editor>,
    store: Box<dyn ProjectStore>,
    me: Weak<RefCell<Glue>>,
    project: String,
    file: String,
    session: Session,
    zoom: f32,
}

type Shared = Rc<RefCell<Glue>>;

impl Glue {
    fn editor(&self) -> Editor {
        self.editor.unwrap()
    }

    fn load(
        store: &dyn ProjectStore,
        me: &Weak<RefCell<Glue>>,
        project: &str,
        file: &str,
    ) -> Result<Session, String> {
        let text = store.read(project, file).map_err(|e| format!("{file}: {e}"))?;
        let mut session = Session::new(Document::new(
            PathBuf::from(format!("{project}/{file}")),
            text,
        ));
        let me = me.clone();
        // The container switches to a new build some time after `set_factory`; refresh the
        // selection overlay once that has happened.
        session.set_on_instance(move || {
            let me = me.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(g) = me.upgrade()
                    && let Ok(g) = g.try_borrow()
                {
                    g.update_overlay();
                }
            });
        });
        Ok(session)
    }

    fn pick_file(store: &dyn ProjectStore, project: &str) -> Result<String, String> {
        let files = store.files(project).map_err(|e| e.to_string())?;
        if files.iter().any(|f| f == DEFAULT_FILE) {
            return Ok(DEFAULT_FILE.into());
        }
        files
            .into_iter()
            .next()
            .ok_or_else(|| format!("project {project} has no .slint files"))
    }

    fn set_error(&self, text: &str) {
        self.editor().set_error_text(text.into());
    }

    fn sync_status(&self) {
        let ed = self.editor();
        ed.set_dirty(self.session.doc().is_dirty());
        ed.set_current_project(self.project.as_str().into());
        ed.set_current_file(self.file.as_str().into());
        ed.set_current_component(self.session.component().into());
        ed.set_zoom_level(self.zoom);
    }

    fn refresh_lists(&self) {
        let ed = self.editor();
        ed.set_projects(to_model(self.store.projects().unwrap_or_default()));
        ed.set_files(to_model(self.store.files(&self.project).unwrap_or_default()));
        ed.set_components(to_model(
            self.session.doc().components().into_iter().map(|c| c.name).collect(),
        ));
    }

    pub fn update_overlay(&self) {
        let ed = self.editor();
        match self.session.selection_hit() {
            Some(hit) => {
                ed.set_sel_x(hit.rect.x);
                ed.set_sel_y(hit.rect.y);
                ed.set_sel_w(hit.rect.w);
                ed.set_sel_h(hit.rect.h);
                ed.set_has_selection(true);
            }
            None => ed.set_has_selection(false),
        }
    }

    /// Recompiles and swaps the preview; on a compile error the old preview stays and the
    /// first diagnostic is shown in the error band.
    fn rebuild(&mut self) {
        let factory = self.session.rebuild();
        let ed = self.editor();
        match factory {
            Some(f) => {
                ed.set_error_text("".into());
                ed.set_factory(f);
            }
            None => {
                let first = self.session.diagnostics().first().map(|d| d.message.clone());
                ed.set_error_text(first.unwrap_or_default().into());
            }
        }
        self.update_overlay();
        self.sync_status();
    }

    fn open_file(&mut self, project: &str, file: &str) {
        self.save();
        match Self::load(&*self.store, &self.me, project, file) {
            Ok(session) => {
                self.project = project.to_string();
                self.file = file.to_string();
                self.session = session;
                self.refresh_lists();
                self.editor().set_has_selection(false);
                self.rebuild();
            }
            Err(e) => self.set_error(&e),
        }
    }

    pub fn save(&mut self) {
        let ed = self.editor();
        ed.set_message("".into());
        match self
            .store
            .write(&self.project, &self.file, self.session.doc().text())
        {
            Ok(()) => {
                self.session.mark_saved();
                ed.set_message("Saved".into());
            }
            Err(e) => ed.set_error_text(format!("Save failed: {e}").into()),
        }
        self.sync_status();
    }

    fn after_edit(&mut self, result: Result<(), crate::session::SessionError>) {
        match result {
            Ok(()) => self.rebuild(),
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    fn tapped(&mut self, x: f32, y: f32) {
        self.editor().set_message("".into());
        self.session.tap(x / self.zoom, y / self.zoom);
        self.update_overlay();
    }

    fn drag_finished(&mut self, dx: f32, dy: f32) {
        self.editor().set_message("".into());
        let r = self.session.move_selection(dx / self.zoom, dy / self.zoom);
        self.after_edit(r);
    }

    fn resize_finished(&mut self, corner: i32, dx: f32, dy: f32) {
        self.editor().set_message("".into());
        let r = self
            .session
            .resize_selection(corner_from(corner), dx / self.zoom, dy / self.zoom);
        self.after_edit(r);
    }

    fn history(&mut self, undo: bool) {
        self.editor().set_message("".into());
        let changed = if undo { self.session.undo() } else { self.session.redo() };
        if changed {
            self.rebuild();
        }
    }

    fn zoom_by(&mut self, m: f32) {
        self.zoom = (self.zoom * m).clamp(MIN_ZOOM, MAX_ZOOM);
        self.editor().set_zoom_level(self.zoom);
    }

    fn create_project(&mut self, name: &str) {
        match self.store.create_project(name) {
            Ok(()) => {
                self.refresh_lists();
                self.select_project(name);
            }
            Err(e) => self.set_error(&format!("Cannot create project {name}: {e}")),
        }
    }

    fn select_project(&mut self, name: &str) {
        self.editor()
            .set_files(to_model(self.store.files(name).unwrap_or_default()));
    }

    fn choose_component(&mut self, name: &str) {
        self.session.set_component(name);
        self.editor().set_has_selection(false);
        self.rebuild();
    }
}

fn connect(editor: &Editor, glue: &Shared) {
    macro_rules! on {
        ($setter:ident, |$g:ident $(, $a:ident)*| $body:expr) => {{
            let glue = glue.clone();
            editor.$setter(move |$($a),*| {
                let mut $g = glue.borrow_mut();
                $body
            });
        }};
    }
    on!(on_tapped, |g, x, y| g.tapped(x, y));
    on!(on_drag_finished, |g, dx, dy| g.drag_finished(dx, dy));
    on!(on_resize_finished, |g, c, dx, dy| g.resize_finished(c, dx, dy));
    on!(on_undo, |g| g.history(true));
    on!(on_redo, |g| g.history(false));
    on!(on_save, |g| g.save());
    on!(on_open, |g, p, f| g.open_file(&p, &f));
    on!(on_select_project, |g, p| g.select_project(&p));
    on!(on_create_project, |g, n| g.create_project(&n));
    on!(on_choose_component, |g, c| g.choose_component(&c));
    on!(on_zoom, |g, m| g.zoom_by(m));
}

pub fn start(editor: &Editor, store: Box<dyn ProjectStore>) -> Result<(), slint::PlatformError> {
    let mut projects = store.projects().map_err(platform_err)?;
    if projects.is_empty() {
        store.create_project(DEFAULT_PROJECT).map_err(platform_err)?;
        projects.push(DEFAULT_PROJECT.into());
    }
    let project = if projects.iter().any(|p| p == DEFAULT_PROJECT) {
        DEFAULT_PROJECT.to_string()
    } else {
        projects[0].clone()
    };
    let file = Glue::pick_file(&*store, &project).map_err(platform_err)?;

    let glue: Shared = Rc::new_cyclic(|me| {
        let session = Glue::load(&*store, me, &project, &file)
            .unwrap_or_else(|_| Session::new(Document::new(PathBuf::from("main.slint"), String::new())));
        RefCell::new(Glue {
            editor: editor.as_weak(),
            store,
            me: me.clone(),
            project,
            file,
            session,
            zoom: 1.0,
        })
    });
    connect(editor, &glue);
    {
        let mut g = glue.borrow_mut();
        g.refresh_lists();
        g.rebuild();
    }

    let weak = Rc::downgrade(&glue);
    let save: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(g) = weak.upgrade()
            && let Ok(mut g) = g.try_borrow_mut()
        {
            g.save();
        }
    });
    SAVE_HOOK.with(|h| *h.borrow_mut() = Some(save));
    editor.window().on_close_requested(|| {
        save_now();
        slint::CloseRequestResponse::HideWindow
    });
    Ok(())
}
