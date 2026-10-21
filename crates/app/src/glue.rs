//! Connects the Slint `Editor` window to the editing `Session` and the project store.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use slint_doc::Document;
use storage::ProjectStore;

use crate::session::{Corner, Session, TreeAction, TreeRow};
use crate::{Editor, GuideBand, PropItem, TreeRowItem};

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const DEFAULT_PROJECT: &str = "default";
const DEFAULT_FILE: &str = "main.slint";
const PROP_DEBOUNCE: Duration = Duration::from_millis(300);
const SOURCE_TAB: i32 = 3;
const RECOVERY_INTERVAL: Duration = Duration::from_secs(5);
const TOAST_TIME: Duration = Duration::from_secs(3);

thread_local! {
    static SAVE_HOOK: RefCell<Option<Rc<dyn Fn() -> bool>>> = const { RefCell::new(None) };
}

/// Saves the open file. Safe to call from platform callbacks on the UI thread (e.g. Android
/// pause); does nothing before the editor has started. Returns false when the document could
/// not be saved (the error is shown and a recovery copy was attempted).
pub fn save_now() -> bool {
    let hook = SAVE_HOOK.with(|h| h.borrow().clone());
    hook.is_none_or(|h| h())
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

/// Writes the document only when it has unsaved edits (every write rotates the single `.bak`),
/// then marks it saved. `Ok(false)` means there was nothing to write. A session that never
/// loaded its file is never written.
pub fn save_if_dirty(
    store: &dyn ProjectStore,
    project: &str,
    file: &str,
    session: &mut Session,
    loaded: bool,
) -> Result<bool, String> {
    if !loaded {
        return Err(format!("{file} was not loaded; refusing to overwrite it"));
    }
    if !session.doc().is_dirty() {
        return Ok(false);
    }
    store
        .write(project, file, session.doc().text())
        .map_err(|e| format!("Save failed: {e}"))?;
    session.mark_saved();
    Ok(true)
}

/// Like `save_if_dirty`, but a failed write also forces a recovery copy of the current text,
/// so closing the window after a failed save loses nothing.
pub fn save_or_recover(
    store: &dyn ProjectStore,
    project: &str,
    file: &str,
    session: &mut Session,
    loaded: bool,
) -> Result<bool, String> {
    let r = save_if_dirty(store, project, file, session, loaded);
    if let Err(e) = &r
        && loaded
        && let Err(re) = store.write_recovery(project, file, session.doc().text())
    {
        return Err(format!("{e}; recovery copy not written: {re}"));
    }
    r
}

/// The project and file to open at startup: the first file that has a recovery copy
/// differing from it (so the recovery prompt shows), else the last open file that still
/// exists, else `default/main.slint` (or the first project and file). Creates the default
/// project when there are none.
pub fn startup_target(store: &dyn ProjectStore) -> Result<(String, String), String> {
    let mut projects = store.projects().map_err(|e| e.to_string())?;
    if projects.is_empty() {
        store.create_project(DEFAULT_PROJECT).map_err(|e| e.to_string())?;
        projects.push(DEFAULT_PROJECT.into());
    }
    for p in &projects {
        for f in store.files(p).unwrap_or_default() {
            if let Ok(text) = store.read(p, &f)
                && let Ok(Some(_)) = pending_recovery(store, p, &f, &text)
            {
                return Ok((p.clone(), f));
            }
        }
    }
    if let Ok(Some((p, f))) = store.last_open()
        && projects.contains(&p)
        && store.files(&p).is_ok_and(|fs| fs.contains(&f))
    {
        return Ok((p, f));
    }
    let project = if projects.iter().any(|p| p == DEFAULT_PROJECT) {
        DEFAULT_PROJECT.to_string()
    } else {
        projects[0].clone()
    };
    let file = Glue::pick_file(store, &project)?;
    Ok((project, file))
}

/// One recovery-timer tick. A dirty, loaded document whose text differs from the last
/// recovery write is written to the recovery copy; a document back at its saved state drops
/// the copy this session wrote. `Ok(true)` means the recovery copy was written.
pub fn recovery_tick(
    store: &dyn ProjectStore,
    project: &str,
    file: &str,
    session: &Session,
    loaded: bool,
    last: &mut Option<String>,
) -> Result<bool, String> {
    if !loaded {
        return Ok(false);
    }
    if !session.doc().is_dirty() {
        if last.is_some() {
            store
                .clear_recovery(project, file)
                .map_err(|e| format!("Recovery copy not removed: {e}"))?;
            *last = None;
        }
        return Ok(false);
    }
    let text = session.doc().text();
    if last.as_deref() == Some(text) {
        return Ok(false);
    }
    store
        .write_recovery(project, file, text)
        .map_err(|e| format!("Recovery copy not written: {e}"))?;
    *last = Some(text.to_string());
    Ok(true)
}

/// The recovery copy of `file`, when one exists and differs from the file's text. A copy
/// equal to the file's text is removed.
pub fn pending_recovery(
    store: &dyn ProjectStore,
    project: &str,
    file: &str,
    file_text: &str,
) -> Result<Option<String>, String> {
    match store.take_recovery(project, file) {
        Ok(Some(r)) if r != file_text => Ok(Some(r)),
        Ok(Some(_)) => store
            .clear_recovery(project, file)
            .map(|()| None)
            .map_err(|e| format!("Recovery copy not removed: {e}")),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("Recovery copy not read: {e}")),
    }
}

pub struct Glue {
    editor: slint::Weak<Editor>,
    store: Box<dyn ProjectStore>,
    me: Weak<RefCell<Glue>>,
    project: String,
    file: String,
    session: Session,
    zoom: f32,
    /// False when the file could not be read; saving is refused then.
    loaded: bool,
    /// Start and text of the range the Source tab shows, as it is in the document.
    source_view: Option<(usize, String)>,
    /// A property edit waiting for its debounce timer.
    pending_prop: Option<PendingProp>,
    /// The Source tab text has edits that were not applied; refreshes leave it alone until
    /// the user applies, jumps to the error, or deliberately changes selection/file/component.
    source_dirty: bool,
    prop_timer: slint::Timer,
    toast_timer: slint::Timer,
    recovery_timer: slint::Timer,
    /// Text of the last recovery write (None: nothing written for this document yet).
    last_recovery: Option<String>,
    /// A recovery write failed and was reported; stay quiet until one succeeds.
    recovery_failed: bool,
    /// `(project, file, text)` of a recovery copy waiting for the user's answer.
    recovered: Option<(String, String, String)>,
    /// Name of the component just extracted; declarations are added to it.
    extracted: String,
    guide_target: RefCell<Option<slint_doc::ElementPath>>,
}

/// A property edit with the element and document it was typed for.
struct PendingProp {
    doc: (String, String),
    path: slint_doc::ElementPath,
    name: String,
    value: String,
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

    fn refresh_tree(&self) {
        let ed = self.editor();
        let sel = self.session.selection();
        let rows: Vec<TreeRowItem> = self
            .session
            .tree_rows()
            .into_iter()
            .map(|r: TreeRow| TreeRowItem {
                selected: sel == Some(&r.path),
                label: r.label.into(),
                depth: r.depth as i32,
                is_layout: r.is_layout,
            })
            .collect();
        ed.set_tree_rows(ModelRc::new(VecModel::from(rows)));
        ed.set_palette(to_model(self.session.palette()));
    }

    fn refresh_props(&self) {
        let rows: Vec<PropItem> = self
            .session
            .properties()
            .into_iter()
            .map(|r| PropItem { name: r.name.into(), value: r.expr.into() })
            .collect();
        let ed = self.editor();
        ed.set_props(ModelRc::new(VecModel::from(rows)));
        ed.set_props_rev(ed.get_props_rev().wrapping_add(1));
    }

    fn show_source(&mut self, range: Option<std::ops::Range<usize>>, title: &str) {
        let ed = self.editor();
        let text = range.as_ref().and_then(|r| self.session.doc().text().get(r.clone()));
        match (range.as_ref(), text) {
            (Some(r), Some(t)) => {
                self.source_dirty = false;
                ed.set_source_text(t.into());
                self.source_view = Some((r.start, t.to_string()));
                ed.set_source_title(title.into());
            }
            _ => {
                self.source_dirty = false;
                ed.set_source_text("".into());
                self.source_view = None;
                ed.set_source_title("".into());
            }
        }
    }

    /// Shows the selected element (else the current component) in the Source tab.
    fn refresh_source(&mut self) {
        if self.source_dirty {
            return;
        }
        let title = match self.session.selection() {
            Some(p) => self
                .session
                .tree_rows()
                .into_iter()
                .find(|r| &r.path == p)
                .map_or_else(String::new, |r| r.label),
            None => self.session.component().to_string(),
        };
        let range = self.session.source_range();
        self.show_source(range, &title);
    }

    /// A deliberate context change (selection, file, component, undo): typed source is dropped.
    fn discard_source_edits(&mut self) {
        self.source_dirty = false;
    }

    fn refresh_panels(&mut self) {
        self.refresh_props();
        self.refresh_source();
    }

    pub fn update_overlay(&self) {
        self.refresh_tree();
        let ed = self.editor();
        match self.session.selection_hit() {
            Some(hit) => {
                ed.set_sel_x(hit.rect.x);
                ed.set_sel_y(hit.rect.y);
                ed.set_sel_w(hit.rect.w);
                ed.set_sel_h(hit.rect.h);
                ed.set_sel_movable(self.session.selection_movable());
                ed.set_has_selection(true);
            }
            None => ed.set_has_selection(false),
        }
        self.update_guides();
    }

    fn update_guides(&self) {
        let ed = self.editor();
        let target = self.session.guide_layout_path();
        if *self.guide_target.borrow() != target {
            // A different layout: an open number editor must not commit into it.
            ed.set_guides_rev(ed.get_guides_rev().wrapping_add(1));
            *self.guide_target.borrow_mut() = target;
        }
        let Some(g) = self.session.guides() else {
            ed.set_has_guides(false);
            ed.set_bands(ModelRc::new(VecModel::from(Vec::<GuideBand>::new())));
            ed.set_has_align(false);
            return;
        };
        ed.set_has_guides(true);
        ed.set_g_x(g.outline.x);
        ed.set_g_y(g.outline.y);
        ed.set_g_w(g.outline.w);
        ed.set_g_h(g.outline.h);
        ed.set_g_vertical(g.direction == tidy::Direction::Vertical);
        let band = |l: &crate::guides::GuideLabel, is_gap: bool| GuideBand {
            x: l.area.x,
            y: l.area.y,
            w: l.area.w,
            h: l.area.h,
            value: l.value,
            property: l.property.as_str().into(),
            is_gap,
        };
        let bands: Vec<GuideBand> = g
            .padding
            .iter()
            .map(|l| band(l, false))
            .chain(g.gaps.iter().map(|l| band(l, true)))
            // Zero or degenerate bands get neither fill nor tap target.
            .filter(|b| b.value != 0 && b.w >= 1.0 && b.h >= 1.0)
            .collect();
        ed.set_bands(ModelRc::new(VecModel::from(bands)));
        match g.align_line {
            Some((x1, y1, x2, y2)) => {
                ed.set_has_align(true);
                ed.set_a_x(x1.min(x2));
                ed.set_a_y(y1.min(y2));
                ed.set_a_w((x2 - x1).abs());
                ed.set_a_h((y2 - y1).abs());
            }
            None => ed.set_has_align(false),
        }
    }

    fn toast(&mut self, text: &str) {
        self.editor().set_toast_text(text.into());
        let me = self.me.clone();
        self.toast_timer.start(slint::TimerMode::SingleShot, TOAST_TIME, move || {
            if let Some(g) = me.upgrade()
                && let Ok(g) = g.try_borrow()
            {
                g.editor().set_toast_text("".into());
            }
        });
    }

    fn stack(&mut self) {
        self.flush_prop();
        self.editor().set_message("".into());
        match self.session.tidy_selection() {
            Ok(warnings) => {
                self.rebuild();
                if !warnings.is_empty() {
                    let text: Vec<&str> = warnings
                        .iter()
                        .map(|w| match w {
                            tidy::Warning::AmbiguousDirection => {
                                "先に階層を分けると、きれいに整形できます"
                            }
                            tidy::Warning::Overlap => "重なっている要素があります",
                        })
                        .collect();
                    self.toast(&text.join("\n"));
                }
            }
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    fn unstack(&mut self) {
        self.flush_prop();
        self.editor().set_message("".into());
        let r = self.session.untidy_selection();
        self.after_edit(r);
    }

    fn guide_edited(&mut self, property: &str, value: &str) {
        self.flush_prop();
        self.editor().set_message("".into());
        let r = self.session.set_guide_property(property, value);
        self.after_edit(r);
    }

    /// Recompiles and swaps the preview; on a compile error the old preview stays and the
    /// first diagnostic is shown in the error band.
    fn rebuild(&mut self) {
        self.rebuild_with(true, true);
    }

    fn rebuild_with(&mut self, props: bool, source: bool) {
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
        if props {
            self.refresh_props();
        }
        if source {
            self.refresh_source();
        }
        self.sync_status();
    }

    fn open_file(&mut self, project: &str, file: &str) {
        self.flush_prop();
        self.discard_source_edits();
        // Keep the current file open (and its error visible) if its edits cannot be saved.
        if self.loaded && !self.save() {
            return;
        }
        match Self::load(&*self.store, &self.me, project, file) {
            Ok(session) => {
                self.project = project.to_string();
                self.file = file.to_string();
                self.session = session;
                self.loaded = true;
                self.last_recovery = None;
                let _ = self.store.set_last_open(project, file);
                self.refresh_lists();
                self.editor().set_has_selection(false);
                self.rebuild();
                self.check_recovery();
            }
            Err(e) => self.set_error(&e),
        }
    }

    /// Saves if there are unsaved edits. Returns false when a save was needed and failed.
    pub fn save(&mut self) -> bool {
        self.flush_prop();
        let ed = self.editor();
        ed.set_message("".into());
        let r = save_or_recover(
            &*self.store,
            &self.project,
            &self.file,
            &mut self.session,
            self.loaded,
        );
        let ok = match r {
            Ok(written) => {
                if written {
                    self.last_recovery = None;
                }
                ed.set_message(if written { "Saved" } else { "No changes" }.into());
                true
            }
            Err(e) => {
                ed.set_error_text(e.into());
                false
            }
        };
        self.sync_status();
        ok
    }

    fn after_edit(&mut self, result: Result<(), crate::session::SessionError>) {
        match result {
            Ok(()) => self.rebuild(),
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    fn tree_selected(&mut self, row: i32) {
        self.flush_prop();
        self.discard_source_edits();
        self.editor().set_message("".into());
        if let Some(r) = self.session.tree_rows().into_iter().nth(row as usize) {
            self.session.select_path(r.path);
        }
        self.update_overlay();
        self.refresh_panels();
    }

    fn tree_action(&mut self, action: i32) {
        self.flush_prop();
        self.editor().set_message("".into());
        let action = match action {
            0 => TreeAction::Up,
            1 => TreeAction::Down,
            2 => TreeAction::Out,
            3 => TreeAction::In,
            4 => TreeAction::Delete,
            _ => TreeAction::NewContainer,
        };
        let r = match self.session.selection().cloned() {
            Some(p) => self.session.tree_action(&p, action),
            None => Err(crate::session::SessionError::NoSelection),
        };
        self.after_edit(r);
    }

    fn palette_picked(&mut self, name: &str) {
        self.flush_prop();
        self.editor().set_message("".into());
        let ed = self.editor();
        let hint = (
            -ed.get_canvas_pan_x() / self.zoom,
            -ed.get_canvas_pan_y() / self.zoom,
        );
        let r = self.session.add_from_palette(name, Some(hint));
        self.after_edit(r);
    }

    fn tapped(&mut self, x: f32, y: f32) {
        self.flush_prop();
        self.discard_source_edits();
        self.editor().set_message("".into());
        self.session.tap(x / self.zoom, y / self.zoom);
        self.update_overlay();
        self.refresh_panels();
    }

    fn drag_finished(&mut self, dx: f32, dy: f32) {
        self.flush_prop();
        self.editor().set_message("".into());
        let r = self.session.move_selection(dx / self.zoom, dy / self.zoom);
        self.after_edit(r);
    }

    fn resize_finished(&mut self, corner: i32, dx: f32, dy: f32) {
        self.flush_prop();
        self.editor().set_message("".into());
        let r = self
            .session
            .resize_selection(corner_from(corner), dx / self.zoom, dy / self.zoom);
        self.after_edit(r);
    }

    fn history(&mut self, undo: bool) {
        self.flush_prop();
        self.discard_source_edits();
        self.editor().set_message("".into());
        let changed = if undo { self.session.undo() } else { self.session.redo() };
        if changed {
            self.rebuild();
        }
    }

    fn queue_prop(&mut self, name: &str, value: &str) -> bool {
        let Some(path) = self.session.selection().cloned() else {
            self.set_error(&crate::session::SessionError::NoSelection.to_string());
            return false;
        };
        self.pending_prop = Some(PendingProp {
            doc: (self.project.clone(), self.file.clone()),
            path,
            name: name.to_string(),
            value: value.to_string(),
        });
        true
    }

    /// Queues a property edit; it is applied 300 ms after the last keystroke.
    fn prop_edited(&mut self, name: &str, value: &str) {
        if !self.queue_prop(name, value) {
            return;
        }
        let me = self.me.clone();
        self.prop_timer.start(slint::TimerMode::SingleShot, PROP_DEBOUNCE, move || {
            // The edit field keeps its text and focus: only the preview is rebuilt.
            commit_when_free(me.clone());
        });
    }

    fn prop_accepted(&mut self, name: &str, value: &str) {
        if self.queue_prop(name, value) {
            self.commit_prop(true);
        }
    }

    fn flush_prop(&mut self) {
        if self.pending_prop.is_some() {
            self.commit_prop(true);
        }
    }

    /// Applies the pending edit unless the selection or the document changed since it was typed.
    fn commit_prop(&mut self, refresh_props: bool) {
        self.prop_timer.stop();
        let Some(p) = self.pending_prop.take() else { return };
        if p.doc != (self.project.clone(), self.file.clone())
            || self.session.selection() != Some(&p.path)
        {
            return;
        }
        self.editor().set_message("".into());
        match self.session.set_property(&p.name, &p.value) {
            Ok(()) => self.rebuild_with(refresh_props, true),
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    /// Replaces the shown source range of the document with the edited text.
    fn source_apply(&mut self, text: &str) {
        self.flush_prop();
        let Some((start, original)) = self.source_view.clone() else {
            self.set_error("Nothing to apply: the source is not shown");
            return;
        };
        match self.session.replace_source(start, &original, text) {
            Ok(()) => {
                self.source_dirty = false;
                self.source_view = Some((start, text.to_string()));
                // Keep the typed text visible, also when it does not compile.
                self.rebuild_with(true, false);
            }
            Err(e) => self.set_error(&e.to_string()),
        }
    }

    /// Error band tap: shows the component containing the first diagnostic in the Source tab.
    fn error_tapped(&mut self) {
        let Some(diag) = self.session.diagnostics().first().cloned() else { return };
        let range = self.session.component_range_at(diag.offset);
        self.show_source(Some(range), "Error location");
        self.editor().set_panel_tab(SOURCE_TAB);
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

    fn dialog_error(&self, text: &str) {
        self.editor().set_dialog_error(text.into());
    }

    /// Flushes the pending property edit and saves unsaved changes. On failure the error is
    /// shown in the dialog (and the error band) and false is returned.
    fn save_before_op(&mut self) -> bool {
        self.flush_prop();
        if self.loaded && !self.save() {
            let e = self.editor().get_error_text();
            self.dialog_error(&e);
            return false;
        }
        true
    }

    fn op_done(&mut self, menu_project: &str) {
        let ed = self.editor();
        ed.set_dialog_error("".into());
        ed.set_file_op(0);
        ed.set_menu_project(menu_project.into());
    }

    fn document_moved(&mut self, project: &str, file: &str) {
        self.project = project.to_string();
        self.file = file.to_string();
        self.session.set_path(PathBuf::from(format!("{project}/{file}")));
        let _ = self.store.set_last_open(project, file);
    }

    /// The File menu dialog was confirmed. `mode`: 1 new file, 2 rename the open file,
    /// 3 rename `project`, 4 save as `name` in `project`. Errors stay in the dialog and
    /// change nothing.
    fn file_op_run(&mut self, mode: i32, name: &str, project: &str) {
        self.editor().set_message("".into());
        self.dialog_error("");
        let project = if mode == 2 { self.project.clone() } else { project.to_string() };
        let project = project.as_str();
        if (1..=4).contains(&mode) && !self.name_free(mode, project, name) {
            return;
        }
        match mode {
            1 => self.new_file(project, name),
            2 => self.rename_open_file(name),
            3 => self.rename_project(project, name),
            4 => self.save_as(project, name),
            _ => {}
        }
    }

    /// Refuses a bad or taken name before anything (a save included) happens.
    fn name_free(&self, mode: i32, project: &str, name: &str) -> bool {
        let err = storage::check_name(name).err().or_else(|| {
            let taken = if mode == 3 {
                self.store.projects().unwrap_or_default().iter().any(|p| p == name)
            } else {
                let file = if name.ends_with(".slint") {
                    name.to_string()
                } else {
                    format!("{name}.slint")
                };
                self.store.files(project).unwrap_or_default().contains(&file)
                    && !(mode == 2 && file == self.file)
            };
            taken.then(|| std::io::ErrorKind::AlreadyExists.into())
        });
        match err {
            Some(e) => {
                self.dialog_error(&op_error(&e, name));
                false
            }
            None => true,
        }
    }

    fn new_file(&mut self, project: &str, name: &str) {
        if !self.save_before_op() {
            return;
        }
        match self.store.create_file(project, name) {
            Ok(file) => {
                self.open_file(project, &file);
                if (self.project.as_str(), self.file.as_str()) == (project, file.as_str()) {
                    self.op_done(project);
                } else {
                    // Opening failed; the error band says why.
                    self.op_done(&self.project.clone());
                }
            }
            Err(e) => self.dialog_error(&op_error(&e, name)),
        }
    }

    fn rename_open_file(&mut self, name: &str) {
        if !self.save_before_op() {
            return;
        }
        let (project, old) = (self.project.clone(), self.file.clone());
        match self.store.rename_file(&project, &old, name) {
            Ok(new) => {
                if let Some(r) = &mut self.recovered
                    && r.0 == project
                    && r.1 == old
                {
                    r.1 = new.clone();
                }
                self.document_moved(&project, &new);
                self.refresh_lists();
                self.rebuild();
                self.op_done(&project);
                self.editor().set_message(format!("Renamed to {new}").into());
            }
            Err(e) => self.dialog_error(&op_error(&e, name)),
        }
    }

    fn rename_project(&mut self, project: &str, name: &str) {
        let is_open = project == self.project;
        if is_open && !self.save_before_op() {
            return;
        }
        match self.store.rename_project(project, name) {
            Ok(()) => {
                if is_open {
                    if let Some(r) = &mut self.recovered
                        && r.0 == project
                    {
                        r.0 = name.to_string();
                    }
                    let file = self.file.clone();
                    self.document_moved(name, &file);
                    self.refresh_lists();
                    self.rebuild();
                } else {
                    self.refresh_lists();
                    self.select_project(name);
                }
                self.op_done(name);
                self.editor().set_message(format!("Renamed to {name}").into());
            }
            Err(e) => self.dialog_error(&op_error(&e, name)),
        }
    }

    /// Writes the editor text to a new file and moves the session there. The original file
    /// is not written, even when the document had unsaved changes.
    fn save_as(&mut self, project: &str, name: &str) {
        self.flush_prop();
        if !self.loaded {
            self.dialog_error(&format!("{} was not loaded; nothing to save", self.file));
            return;
        }
        let text = self.session.doc().text().to_string();
        let file = match self.store.create_file_with(project, name, &text) {
            Ok(f) => f,
            Err(e) => return self.dialog_error(&op_error(&e, name)),
        };
        // The edits now live in the new file; the original's draft must not resurface.
        let cleared = self.store.clear_recovery(&self.project, &self.file);
        self.document_moved(project, &file);
        self.session.mark_saved();
        self.last_recovery = None;
        self.refresh_lists();
        self.rebuild();
        self.op_done(project);
        self.editor().set_message(format!("Saved as {file}").into());
        if let Err(e) = cleared {
            self.set_error(&format!("Recovery copy of the original not removed: {e}"));
        }
    }

    fn select_project(&mut self, name: &str) {
        self.editor()
            .set_files(to_model(self.store.files(name).unwrap_or_default()));
    }

    /// Offers the recovery copy of the open file, if there is a different one.
    fn check_recovery(&mut self) {
        if !self.loaded {
            return;
        }
        self.recovered = match pending_recovery(
            &*self.store,
            &self.project,
            &self.file,
            self.session.doc().text(),
        ) {
            Ok(r) => r.map(|t| (self.project.clone(), self.file.clone(), t)),
            Err(e) => {
                self.set_error(&e);
                None
            }
        };
        self.editor().set_recovery_open(self.recovered.is_some());
    }

    fn recovery_answered(&mut self, restore: bool) {
        self.editor().set_recovery_open(false);
        let Some((project, file, text)) = self.recovered.take() else { return };
        if restore {
            if (project.as_str(), file.as_str()) != (self.project.as_str(), self.file.as_str()) {
                return;
            }
            self.flush_prop();
            self.discard_source_edits();
            self.session.replace_text(text.clone());
            self.last_recovery = Some(text);
            self.refresh_lists();
            self.editor().set_has_selection(false);
            self.rebuild();
        } else if let Err(e) = self.store.clear_recovery(&project, &file) {
            self.set_error(&format!("Recovery copy not removed: {e}"));
        }
    }

    fn recovery_tick(&mut self) {
        match recovery_tick(
            &*self.store,
            &self.project,
            &self.file,
            &self.session,
            self.loaded,
            &mut self.last_recovery,
        ) {
            Ok(_) => self.recovery_failed = false,
            Err(e) => {
                if !self.recovery_failed {
                    self.recovery_failed = true;
                    self.set_error(&e);
                }
            }
        }
    }

    fn extract_open(&mut self) {
        self.flush_prop();
        self.editor().set_message("".into());
        if self.session.selection().is_none() {
            self.set_error("部品にする要素を選んでください");
            return;
        }
        self.editor().set_dialog_error("".into());
        self.editor().set_extract_step(1);
    }

    fn extract_name_changed(&mut self, step: i32, name: &str) {
        let err = if name.is_empty() {
            None
        } else if step == 1 {
            self.session.component_name_error(name)
        } else if crate::session::is_identifier(name) {
            None
        } else {
            Some("名前に使えるのは英数字・_・- だけです(先頭は英字か _)".to_string())
        };
        self.editor().set_dialog_error(err.unwrap_or_default().into());
    }

    fn extract_run(&mut self, name: &str) {
        self.editor().set_message("".into());
        match self.session.extract_selection(name) {
            Ok(()) => {
                self.discard_source_edits();
                self.extracted = name.to_string();
                self.refresh_lists();
                self.rebuild();
                self.editor().set_dialog_error("".into());
                self.editor().set_extract_step(2);
            }
            Err(e) => self.editor().set_dialog_error(e.to_string().into()),
        }
    }

    fn declaration_add(&mut self, kind: i32, ty: &str, name: &str) {
        let Some(kind) = crate::session::DeclKind::from_index(kind) else { return };
        let component = self.extracted.clone();
        match self.session.add_declaration(&component, kind, ty, name) {
            Ok(()) => {
                self.rebuild();
                self.editor().set_dialog_error("".into());
                self.toast(&format!("{name} を追加しました"));
            }
            Err(e) => self.editor().set_dialog_error(e.to_string().into()),
        }
    }

    fn choose_component(&mut self, name: &str) {
        self.flush_prop();
        self.discard_source_edits();
        self.session.set_component(name);
        self.editor().set_has_selection(false);
        self.rebuild();
    }
}

fn op_error(e: &std::io::Error, name: &str) -> String {
    match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{name} already exists"),
        std::io::ErrorKind::InvalidInput => format!(
            "{name:?} is not a valid name (no /, \\ or .. and no leading dot)"
        ),
        _ => format!("{name}: {e}"),
    }
}

/// Commits the pending property edit; when the glue is borrowed right now (re-entrant
/// callback), tries again shortly instead of dropping the edit.
fn commit_when_free(me: Weak<RefCell<Glue>>) {
    let Some(g) = me.upgrade() else { return };
    match g.try_borrow_mut() {
        Ok(mut g) => g.commit_prop(false),
        Err(_) => slint::Timer::single_shot(Duration::from_millis(50), move || {
            commit_when_free(me.clone())
        }),
    }
}

#[cfg(debug_assertions)]
/// Debug hook: `PETARICON_FAKE_KEYBOARD=<height>` pretends an on-screen keyboard of that many
/// logical pixels is up, so the input bar can be tried on a desktop.
fn fake_keyboard_height() -> Option<f32> {
    let h: f32 = std::env::var("PETARICON_FAKE_KEYBOARD").ok()?.trim().parse().ok()?;
    (h > 0.0).then_some(h)
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
    on!(on_save, |g| { g.save(); });
    on!(on_open, |g, p, f| g.open_file(&p, &f));
    on!(on_select_project, |g, p| g.select_project(&p));
    on!(on_create_project, |g, n| g.create_project(&n));
    on!(on_choose_component, |g, c| g.choose_component(&c));
    on!(on_tree_selected, |g, i| g.tree_selected(i));
    on!(on_tree_action, |g, a| g.tree_action(a));
    on!(on_palette_picked, |g, n| g.palette_picked(&n));
    on!(on_prop_edited, |g, n, v| g.prop_edited(&n, &v));
    on!(on_prop_accepted, |g, n, v| g.prop_accepted(&n, &v));
    on!(on_prop_flush, |g| g.flush_prop());
    on!(on_source_apply, |g, t| g.source_apply(&t));
    on!(on_source_edited, |g| g.source_dirty = true);
    on!(on_error_tapped, |g| g.error_tapped());
    on!(on_zoom, |g, m| g.zoom_by(m));
    on!(on_stack, |g| g.stack());
    on!(on_unstack, |g| g.unstack());
    on!(on_extract_open, |g| g.extract_open());
    on!(on_name_changed, |g, s, n| g.extract_name_changed(s, &n));
    on!(on_extract_run, |g, n| g.extract_run(&n));
    on!(on_declaration_add, |g, k, t, n| g.declaration_add(k, &t, &n));
    on!(on_recovery_answered, |g, r| g.recovery_answered(r));
    on!(on_file_op_run, |g, m, n, p| g.file_op_run(m, &n, &p));
    on!(on_guide_edited, |g, p, v| g.guide_edited(&p, &v));
}

pub fn start(editor: &Editor, store: Box<dyn ProjectStore>) -> Result<(), slint::PlatformError> {
    let (project, file) = startup_target(&*store).map_err(platform_err)?;

    let mut load_error = None;
    let glue: Shared = Rc::new_cyclic(|me| {
        let loaded = Glue::load(&*store, me, &project, &file);
        let ok = loaded.is_ok();
        let session = loaded.unwrap_or_else(|e| {
            load_error = Some(e);
            Session::new(Document::new(PathBuf::from(&file), String::new()))
        });
        RefCell::new(Glue {
            loaded: ok,
            editor: editor.as_weak(),
            store,
            me: me.clone(),
            project,
            file,
            session,
            zoom: 1.0,
            source_view: None,
            pending_prop: None,
            source_dirty: false,
            prop_timer: slint::Timer::default(),
            toast_timer: slint::Timer::default(),
            recovery_timer: slint::Timer::default(),
            last_recovery: None,
            recovery_failed: false,
            recovered: None,
            extracted: String::new(),
            guide_target: RefCell::new(None),
        })
    });
    connect(editor, &glue);
    #[cfg(target_os = "android")]
    editor.set_touch_input(true);
    #[cfg(debug_assertions)]
    if let Some(h) = fake_keyboard_height() {
        editor.set_fake_keyboard(h);
        editor.set_touch_input(true);
    }
    {
        let mut g = glue.borrow_mut();
        g.refresh_lists();
        match load_error {
            None => {
                let _ = g.store.set_last_open(&g.project, &g.file);
                g.rebuild();
                g.check_recovery();
            }
            Some(e) => g.set_error(&format!("Cannot open {e}")),
        }
    }

    let weak = Rc::downgrade(&glue);
    glue.borrow().recovery_timer.start(
        slint::TimerMode::Repeated,
        RECOVERY_INTERVAL,
        move || {
            if let Some(g) = weak.upgrade()
                && let Ok(mut g) = g.try_borrow_mut()
            {
                g.recovery_tick();
            }
        },
    );

    let weak = Rc::downgrade(&glue);
    let save: Rc<dyn Fn() -> bool> = Rc::new(move || {
        match weak.upgrade() {
            Some(g) => g.try_borrow_mut().is_ok_and(|mut g| g.save()),
            None => true,
        }
    });
    SAVE_HOOK.with(|h| *h.borrow_mut() = Some(save));
    editor.window().on_close_requested(|| {
        // A failed save keeps the window (and the unsaved edits) open with the error shown.
        if save_now() {
            slint::CloseRequestResponse::HideWindow
        } else {
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    Ok(())
}
