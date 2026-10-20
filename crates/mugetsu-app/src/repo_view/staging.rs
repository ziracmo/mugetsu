//! Stage, unstage, discard and ignore from the Changes view. Files move as
//! soon as they are asked to; git runs behind, one operation at a time, and
//! the status git reports once the queue is empty replaces the prediction
//! (which also rolls back an operation that failed).

use std::sync::Arc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, ClipboardItem, Context, Window};
use mugetsu_git::{Change, FileStatus, Repo, Status};

use super::{RepoView, SelectedChange};
use crate::i18n::tr;
use crate::model::RepoData;

#[derive(Clone, Debug)]
pub(super) enum StageOp {
    Stage(Vec<String>),
    StageAll,
    Unstage(Vec<String>),
    /// Worktree changes thrown away; untracked files are deleted.
    Discard {
        tracked: Vec<String>,
        untracked: Vec<String>,
    },
    /// A `.gitignore` line, and the untracked file it was asked for.
    Ignore {
        pattern: String,
        path: String,
    },
}

impl StageOp {
    fn label(&self) -> &'static str {
        match self {
            Self::Stage(_) | Self::StageAll => "Stage",
            Self::Unstage(_) => "Unstage",
            Self::Discard { .. } => "Discard",
            Self::Ignore { .. } => "Ignore",
        }
    }

    /// Shows the operation's outcome on `status` before git runs it.
    fn predict(&self, status: &mut Status) {
        match self {
            Self::Stage(paths) => status.stage_paths(&refs(paths)),
            Self::StageAll => {
                let all: Vec<String> = status.unstaged().map(|f| f.path.clone()).collect();
                status.stage_paths(&refs(&all));
            }
            Self::Unstage(paths) => status.unstage_paths(&refs(paths)),
            Self::Discard { tracked, untracked } => {
                status.discard_paths(&refs(tracked));
                status.discard_paths(&refs(untracked));
            }
            Self::Ignore { path, .. } => status
                .files
                .retain(|f| !(f.path == *path && f.worktree == Change::Untracked)),
        }
    }

    fn run(&self, repo: &Repo) -> anyhow::Result<()> {
        match self {
            Self::Stage(paths) => repo.stage(&refs(paths)),
            Self::StageAll => repo.stage_all(),
            Self::Unstage(paths) => repo.unstage(&refs(paths)),
            Self::Discard { tracked, untracked } => {
                if !tracked.is_empty() {
                    repo.discard(&refs(tracked))?;
                }
                if !untracked.is_empty() {
                    repo.remove_untracked(&refs(untracked))?;
                }
                Ok(())
            }
            Self::Ignore { pattern, .. } => repo.ignore(pattern),
        }
    }
}

fn refs(paths: &[String]) -> Vec<&str> {
    paths.iter().map(String::as_str).collect()
}

/// A git write waiting for the stage queue to drain (the index takes one writer).
pub(super) type AfterStaging = Box<dyn FnOnce(&mut RepoView, &mut Window, &mut Context<RepoView>)>;

/// What can be ignored for an untracked `path`: the file, its extension, its folder.
pub(super) fn ignore_choices(path: &str) -> Vec<(String, String)> {
    let mut out = vec![(
        trf!("Ignorer « {path} »", "Ignore '{path}'"),
        mugetsu_git::ignore_literal(path),
    )];
    let name = path.rsplit('/').next().unwrap_or(path);
    if let Some((stem, ext)) = name.rsplit_once('.')
        && !stem.is_empty()
        && !ext.is_empty()
        && !ext.contains(['*', '?', '[', '\\', ' '])
    {
        out.push((
            trf!("Ignorer tous les *.{ext}", "Ignore all *.{ext}"),
            format!("*.{ext}"),
        ));
    }
    if let Some((dir, _)) = path.rsplit_once('/') {
        out.push((
            trf!("Ignorer le dossier « {dir}/ »", "Ignore folder '{dir}/'"),
            mugetsu_git::ignore_literal(dir) + "/",
        ));
    }
    out
}

impl RepoView {
    /// Applies `op` to the shown status now and queues it for git.
    pub(super) fn queue_stage(&mut self, op: StageOp, window: &mut Window, cx: &mut Context<Self>) {
        self.status_epoch += 1;
        op.predict(&mut Arc::make_mut(&mut self.data).status);
        self.staging.push_back(op);
        self.follow_selected_change(cx);
        self.pump_staging(window, cx);
        cx.notify();
    }

    /// Starts the next queued operation unless one (or another git write) runs.
    pub(super) fn pump_staging(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.staging_running || self.busy.is_some() {
            return;
        }
        let Some(op) = self.staging.front().cloned() else {
            return;
        };
        self.staging_running = true;
        let repo = self.repo.clone();
        self._stage = cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { op.run(&repo) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.staging_running = false;
                let done = this.staging.pop_front();
                if let (Err(e), Some(op)) = (result, done) {
                    let label = op.label();
                    window.push_notification(
                        crate::toast::error(trf!("{label} : échec", "{label} failed"), &e, None),
                        cx,
                    );
                }
                if this.staging.is_empty() {
                    this.refresh_status(cx);
                    for then in std::mem::take(&mut this.after_staging) {
                        then(this, window, cx);
                    }
                } else {
                    this.pump_staging(window, cx);
                }
            });
        });
    }

    /// Reads the real status again (status only: history did not move).
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let epoch = self.status_epoch;
        let repo = self.repo.clone();
        self.status_loading = true;
        self._status = cx.spawn(async move |this, cx| {
            let status = cx.background_spawn(async move { repo.status() }).await;
            let _ = this.update(cx, |this, cx| {
                this.status_loading = false;
                // A newer move is shown: the queue reads the status again after it.
                if this.status_epoch != epoch || !this.staging.is_empty() {
                    return;
                }
                if let Ok(status) = status {
                    Arc::make_mut(&mut this.data).status = status;
                    this.sync_selected_change(cx);
                }
                cx.notify();
            });
        });
    }

    /// Status from a full reload: kept out when moves were queued since the
    /// reload began, and the moves still running are shown on top of it.
    pub(super) fn merge_loaded_status(&mut self, data: &mut Arc<RepoData>) {
        if self.load_epoch != self.status_epoch {
            Arc::make_mut(data).status = self.data.status.clone();
        } else if !self.staging.is_empty() {
            let status = &mut Arc::make_mut(data).status;
            for op in &self.staging {
                op.predict(status);
            }
        }
    }

    /// The selected file moved to the other list: keep it selected there.
    fn follow_selected_change(&mut self, cx: &mut Context<Self>) {
        let Some(sel) = self.selected_change.clone() else {
            return;
        };
        let on = |staged: bool| {
            self.data
                .status
                .files
                .iter()
                .any(|f| f.path == sel.path && side(f, staged))
        };
        if on(sel.staged) {
            return;
        }
        if on(!sel.staged) {
            // The diff reloads once git has moved it.
            self.selected_change = Some(SelectedChange {
                staged: !sel.staged,
                ..sel
            });
        } else {
            self.select_first_change(cx);
        }
    }

    /// Keeps the selected file if it is still listed, else selects the first one.
    pub(super) fn sync_selected_change(&mut self, cx: &mut Context<Self>) {
        if self.mode != super::Mode::Changes {
            return;
        }
        let still_there = self.selected_change.as_ref().is_some_and(|sel| {
            self.data
                .status
                .files
                .iter()
                .any(|f| f.path == sel.path && side(f, sel.staged))
        });
        if still_there {
            let sel = self.selected_change.clone().unwrap();
            self.load_change(sel, cx);
        } else {
            self.selected_change = None;
            self.select_first_change(cx);
        }
    }

    fn change_status(&self, path: &str) -> Option<&FileStatus> {
        self.data.status.files.iter().find(|f| f.path == path)
    }

    /// Stages an unstaged file, or unstages a staged one; a ⌘/Ctrl-picked
    /// file moves with every other picked one.
    pub(super) fn toggle_stage(
        &mut self,
        sel: &SelectedChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sels = if self.change_marks.contains(sel) {
            self.change_marks.clone()
        } else {
            vec![sel.clone()]
        };
        self.toggle_stage_all(sels, window, cx);
    }

    /// Stages the unstaged files of `sels` and unstages the staged ones.
    pub(super) fn toggle_stage_all(
        &mut self,
        sels: Vec<SelectedChange>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let listed = |sel: &SelectedChange| {
            self.data
                .status
                .files
                .iter()
                .any(|f| f.path == sel.path && side(f, sel.staged))
        };
        let (staged, unstaged): (Vec<_>, Vec<_>) = sels
            .into_iter()
            .filter(|s| !self.is_conflicted(&s.path) && listed(s))
            .partition(|s| s.staged);
        self.change_marks.clear();
        if !unstaged.is_empty() {
            let paths = unstaged.into_iter().map(|s| s.path).collect();
            self.queue_stage(StageOp::Stage(paths), window, cx);
        }
        if !staged.is_empty() {
            let op = self.unstage_op(staged.into_iter().map(|s| s.path).collect());
            self.queue_stage(op, window, cx);
        }
    }

    /// Unstages `paths`, renames with their source (as a whole, like git shows them).
    pub(super) fn unstage_op(&self, mut paths: Vec<String>) -> StageOp {
        let sources: Vec<String> = self
            .data
            .status
            .staged()
            .filter(|f| f.index == Change::Renamed && paths.contains(&f.path))
            .filter_map(|f| f.orig_path.clone())
            .collect();
        paths.extend(sources);
        StageOp::Unstage(paths)
    }

    /// Asks, then throws away the unstaged changes of `path`.
    pub(super) fn confirm_discard(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(f) = self.change_status(&path) else {
            return;
        };
        if !f.is_unstaged() || f.worktree == Change::Conflicted {
            return;
        }
        let untracked = f.worktree == Change::Untracked;
        let description = if untracked {
            tr(
                "Ce fichier non suivi est supprimé du disque. C'est irréversible.",
                "This untracked file is deleted from disk. This cannot be undone.",
            )
        } else {
            tr(
                "Les modifications non indexées de ce fichier sont perdues. C'est irréversible.",
                "This file's unstaged changes are lost. This cannot be undone.",
            )
        };
        self.confirm(
            trf!(
                "Annuler les modifications de « {path} » ?",
                "Discard changes to '{path}'?"
            ),
            description.to_string(),
            tr("Annuler les modifications", "Discard"),
            move |this, window, cx| {
                let (tracked, untracked) = if untracked {
                    (vec![], vec![path.clone()])
                } else {
                    (vec![path.clone()], vec![])
                };
                this.queue_stage(StageOp::Discard { tracked, untracked }, window, cx);
            },
            window,
            cx,
        );
    }

    pub fn ignore_change(
        &mut self,
        path: String,
        pattern: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.queue_stage(StageOp::Ignore { pattern, path }, window, cx);
    }

    pub(super) fn open_change(&self, path: &str, cx: &mut Context<Self>) {
        let full = self.repo.root().join(path);
        if full.exists() {
            cx.open_with_system(&full);
        }
    }

    pub(super) fn reveal_change(&self, path: &str, cx: &mut Context<Self>) {
        let full = self.repo.root().join(path);
        // A deleted file: show the folder it was in.
        let target = if full.exists() {
            full
        } else {
            full.ancestors()
                .find(|p| p.exists())
                .unwrap_or(self.repo.root())
                .to_path_buf()
        };
        cx.reveal_path(&target);
    }

    pub(super) fn copy_change_path(&self, path: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
    }

    #[cfg(test)]
    pub(super) fn is_staging(&self) -> bool {
        !self.staging.is_empty() || self.status_loading
    }
}

fn side(f: &FileStatus, staged: bool) -> bool {
    if staged {
        f.is_staged()
    } else {
        f.is_unstaged()
    }
}

/// Name for the file manager on this platform.
pub(super) fn reveal_label() -> String {
    if cfg!(target_os = "macos") {
        tr("Révéler dans le Finder", "Reveal in Finder").to_string()
    } else if cfg!(target_os = "windows") {
        tr("Afficher dans l'Explorateur", "Show in Explorer").to_string()
    } else {
        tr("Afficher dans le dossier", "Show in folder").to_string()
    }
}
