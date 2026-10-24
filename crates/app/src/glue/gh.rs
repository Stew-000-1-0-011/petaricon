//! The GitHub dialog: token, new project from GitHub, 取り込む, 送る and the conflict list.
//!
//! Network work runs on a worker thread; its outcome comes back over a channel that a timer
//! polls on the UI thread, where it is applied. The token is read from the store when a job
//! starts and is never kept on screen or in a log (only `…abcd`).

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use slint::{ModelRc, SharedString, VecModel};

use super::Glue;
use crate::github::{self, ApiFactory, Job, Outcome};

const POLL: Duration = Duration::from_millis(50);

/// A job in flight.
pub(super) struct Running {
    rx: Receiver<Outcome>,
    /// The project the job works on (the open one, or the one being created).
    project: String,
    /// For redacting texts only.
    token: String,
}

fn to_models(items: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.iter().map(|s| SharedString::from(s.as_str())).collect::<Vec<_>>()))
}

impl Glue {
    fn gh_error(&self, text: &str) {
        let ed = self.editor();
        ed.set_gh_error(text.into());
        ed.set_gh_result("".into());
    }

    fn gh_result(&self, text: &str) {
        let ed = self.editor();
        ed.set_gh_error("".into());
        ed.set_gh_result(text.into());
    }

    /// Token hint, link and status of the open project.
    pub(super) fn gh_refresh(&self) {
        let ed = self.editor();
        ed.set_gh_token_hint(self.store.github_token().map(|t| github::mask_token(&t)).unwrap_or_default().into());
        let view = github::project_dir(&*self.store, &self.project)
            .map(|d| github::view(&d))
            .unwrap_or_default();
        ed.set_gh_linked(view.linked);
        ed.set_gh_link_text(view.link_text.into());
        ed.set_gh_status_text(view.status_text.into());
        ed.set_gh_conflicts(to_models(&view.conflicts));
        ed.set_gh_busy(self.gh_job.is_some());
    }

    pub(super) fn gh_open(&mut self) {
        self.begin_action();
        self.flush_prop();
        let ed = self.editor();
        ed.set_gh_editing("".into());
        ed.set_gh_error("".into());
        ed.set_gh_result("".into());
        self.gh_refresh();
        ed.set_gh_mode(1);
    }

    pub(super) fn gh_open_new(&mut self) {
        self.begin_action();
        let ed = self.editor();
        ed.set_gh_editing("".into());
        ed.set_gh_error("".into());
        ed.set_gh_result("".into());
        self.gh_refresh();
        ed.set_gh_mode(2);
    }

    pub(super) fn gh_closed(&mut self) {
        let ed = self.editor();
        ed.set_gh_error("".into());
        ed.set_gh_result("".into());
    }

    pub(super) fn gh_token_set(&mut self, token: &str) {
        match self.store.set_github_token(token) {
            Ok(()) => {
                self.gh_refresh();
                self.gh_result("トークンを保存しました");
            }
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                self.gh_error("トークンに空白や改行は使えません。コピーし直してください");
            }
            // The error text of the store never contains the token.
            Err(e) => self.gh_error(&format!("トークンを保存できませんでした: {e}")),
        }
    }

    pub(super) fn gh_token_clear(&mut self) {
        match self.store.clear_github_token() {
            Ok(()) => {
                self.gh_refresh();
                self.gh_result("トークンを削除しました");
            }
            Err(e) => self.gh_error(&format!("トークンを削除できませんでした: {e}")),
        }
    }

    /// Starts `job` on `project`: needs a token, saves the open document first.
    fn gh_start(&mut self, job: Job, project: &str) {
        if self.gh_job.is_some() {
            return self.gh_error("通信中です。終わるまでお待ちください");
        }
        let Some(token) = self.store.github_token() else {
            self.gh_error("先に GitHub のトークンを設定してください");
            self.editor().invoke_gh_open_bar("token".into());
            return;
        };
        let dir = match github::project_dir(&*self.store, project) {
            Ok(d) => d,
            Err(e) => return self.gh_error(&e),
        };
        self.flush_prop();
        if self.loaded && !self.save() {
            let e = self.editor().get_error_text().to_string();
            return self.gh_error(&e);
        }
        let (tx, rx) = mpsc::channel();
        let factory: ApiFactory = self.api.clone();
        let (job2, token2) = (job.clone(), token.clone());
        let spawned = std::thread::Builder::new().name("github".into()).spawn(move || {
            let api = factory(&token2);
            let _ = tx.send(github::run(&*api, &dir, &job2));
        });
        if let Err(e) = spawned {
            return self.gh_error(&format!("通信を始められませんでした: {e}"));
        }
        self.gh_job = Some(Running { rx, project: project.to_string(), token });
        let ed = self.editor();
        ed.set_gh_busy(true);
        ed.set_gh_error("".into());
        ed.set_gh_result("".into());
        let me = self.me.clone();
        self.gh_timer.start(slint::TimerMode::Repeated, POLL, move || poll(me.clone()));
    }

    pub(super) fn gh_pull(&mut self) {
        self.begin_action();
        let project = self.project.clone();
        self.gh_start(Job::Pull, &project);
    }

    pub(super) fn gh_push(&mut self, message: &str) {
        self.begin_action();
        let message = match message.trim() {
            "" => github::DEFAULT_MESSAGE,
            m => m,
        };
        let project = self.project.clone();
        self.gh_start(Job::Push { message: message.to_string() }, &project);
    }

    pub(super) fn gh_take_remote(&mut self, path: &str) {
        self.begin_action();
        let project = self.project.clone();
        self.gh_start(Job::TakeRemote(path.to_string()), &project);
    }

    pub(super) fn gh_keep_local(&mut self, path: &str) {
        self.begin_action();
        let project = self.project.clone();
        self.gh_start(Job::KeepLocal(path.to_string()), &project);
    }

    pub(super) fn gh_import(&mut self, repo: &str, branch: &str, folder: &str, name: &str) {
        self.begin_action();
        let input = match github::check_clone_input(repo, branch, folder, name) {
            Ok(i) => i,
            Err(e) => return self.gh_error(&e),
        };
        let job = Job::Clone { repo: input.repo, branch: input.branch, folder: input.folder };
        self.gh_start(job, &input.name);
    }

    /// The job finished: shows what happened and brings the editor up to date.
    fn gh_done(&mut self, run: Running, outcome: Outcome) {
        let ed = self.editor();
        ed.set_gh_busy(false);
        let Running { project, token, .. } = run;
        let show_err = |g: &Self, e: &ghsync::SyncError| {
            g.gh_error(&github::redact(&github::error_text(e), &token));
        };
        match outcome {
            Outcome::Cloned(Ok(state)) => {
                let files = state.files.len();
                ed.set_gh_mode(0);
                ed.set_gh_name("".into());
                ed.set_menu_project(project.as_str().into());
                self.refresh_lists();
                match Glue::pick_file(&*self.store, &project) {
                    Ok(file) => {
                        self.open_file(&project, &file);
                        if self.project == project {
                            self.editor().set_message(
                                format!("{project} を取り込みました({files} ファイル)").into(),
                            );
                        }
                    }
                    Err(e) => self.set_error(&format!(
                        "{project} を取り込みましたが、開けるファイルがありません: {e}"
                    )),
                }
            }
            Outcome::Cloned(Err(e)) => show_err(self, &e),
            Outcome::Pulled(Ok(report)) => {
                let changed: Vec<String> =
                    report.updated.iter().chain(&report.deleted).cloned().collect();
                self.gh_refresh();
                self.gh_result(&github::pull_summary(&report));
                self.gh_reload(&project, &changed);
            }
            Outcome::Pulled(Err(e)) => show_err(self, &e),
            Outcome::Pushed(Ok(report)) => {
                self.gh_refresh();
                self.gh_result(&github::push_summary(&report));
            }
            Outcome::Pushed(Err(e)) => {
                self.gh_refresh();
                show_err(self, &e);
            }
            Outcome::Resolved { path, took_remote, result: Ok(()) } => {
                self.gh_refresh();
                if took_remote {
                    self.gh_result(&format!(
                        "{path} をリモートの版にしました(手元の版は「{path}.local」に残してあります)"
                    ));
                    self.gh_reload(&project, std::slice::from_ref(&path));
                } else {
                    self.gh_result(&format!("{path} は手元の版を残しました(次に送るときに反映されます)"));
                }
            }
            Outcome::Resolved { result: Err(e), .. } => {
                self.gh_refresh();
                show_err(self, &e);
            }
        }
    }

    /// Files of `project` changed on disk: re-reads the open document when it is one of them
    /// (its undo history is cleared only when its text changed) and rebuilds the preview.
    fn gh_reload(&mut self, project: &str, changed: &[String]) {
        if project != self.project || changed.is_empty() {
            return;
        }
        preview::clear_verdicts();
        self.refresh_lists();
        if !changed.contains(&self.file) {
            // An imported file or asset may have changed: recompile.
            self.rebuild();
            return;
        }
        match self.store.read(&self.project, &self.file) {
            Ok(text) if text == self.session.doc().text() => self.rebuild(),
            Ok(_) if self.session.doc().is_dirty() => {
                // Edits made while the transfer ran: keep them (saving them replaces the
                // fetched version, so say so).
                self.gh_error(&format!(
                    "{} は取り込んだ内容と、未保存の編集の両方があります。エディタの内容を残しました(保存すると取り込んだ内容は上書きされます)",
                    self.file
                ));
            }
            Ok(_) => {
                self.discard_source_edits();
                let (p, f) = (self.project.clone(), self.file.clone());
                match Glue::load(&*self.store, &self.me, &p, &f) {
                    Ok(session) => {
                        self.nav.clear();
                        self.show_loaded(&p, &f, session);
                    }
                    Err(e) => self.set_error(&e),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The file was deleted on GitHub: switch to another one of the project.
                let p = self.project.clone();
                match Glue::pick_file(&*self.store, &p) {
                    Ok(f) => {
                        self.editor().set_gh_mode(0);
                        self.open_file(&p, &f);
                        self.editor().set_message(
                            format!("開いていたファイルが GitHub で削除されたので {f} を開きました").into(),
                        );
                    }
                    Err(e) => self.set_error(&e),
                }
            }
            Err(e) => self.set_error(&format!("{}: {e}", self.file)),
        }
    }
}

fn poll(me: Weak<RefCell<Glue>>) {
    let Some(g) = me.upgrade() else { return };
    let Ok(mut g) = g.try_borrow_mut() else { return };
    let Some(run) = g.gh_job.take() else {
        g.gh_timer.stop();
        return;
    };
    match run.rx.try_recv() {
        Ok(outcome) => {
            g.gh_timer.stop();
            g.gh_done(run, outcome);
        }
        Err(TryRecvError::Empty) => g.gh_job = Some(run),
        Err(TryRecvError::Disconnected) => {
            g.gh_timer.stop();
            g.editor().set_gh_busy(false);
            g.gh_error("通信の処理が途中で止まりました。もう一度お試しください");
        }
    }
}
