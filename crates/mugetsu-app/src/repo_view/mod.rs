//! One repository tab: sidebar (branches, remotes, tags, worktrees, stashes),
//! history graph with commit details, and the working-copy "Changes" view.

mod changes;
mod commit_files;
mod forms;
mod history;
mod menus;
mod particles;
mod pr_panel;
mod search;
mod shimmer;
mod sidebar;
mod toolbar;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gix::ObjectId;
use gpui_kit::component::diff::{DiffFile as UiDiffFile, DiffMode, DiffState};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::{
    AnimationExt as _, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollStrategy, SharedString,
    Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use mugetsu_git::{FileDiff, Repo};

use crate::fetch_policy;
use crate::i18n::tr;
use crate::model::RepoData;
use crate::pr::{self, PrCache, PrLookup};

gpui_kit::actions!(
    repo,
    [
        SelectNewer,
        SelectOlder,
        Fetch,
        Refresh,
        ShowHistory,
        ShowChanges,
        Pull,
        Push,
        NewBranch,
        NewTag,
        Stash,
        RevealHead,
        SearchCommits,
        CloseSearch,
        CheckoutBranch,
        ToggleSidebar,
        Commit,
        CommitAndPush,
        StageAll,
        StashChanges
    ]
);

pub fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("up", SelectNewer, Some("History")),
        KeyBinding::new("k", SelectNewer, Some("History")),
        KeyBinding::new("down", SelectOlder, Some("History")),
        KeyBinding::new("j", SelectOlder, Some("History")),
        // Same shortcuts as Fork on macOS.
        KeyBinding::new("secondary-shift-f", Fetch, Some("Repo")),
        KeyBinding::new("secondary-shift-l", Pull, Some("Repo")),
        KeyBinding::new("secondary-shift-p", Push, Some("Repo")),
        KeyBinding::new("secondary-shift-b", NewBranch, Some("Repo")),
        KeyBinding::new("secondary-shift-g", NewTag, Some("Repo")),
        KeyBinding::new("secondary-shift-h", Stash, Some("Repo")),
        KeyBinding::new("secondary-r", Refresh, Some("Repo")),
        KeyBinding::new("secondary-1", ShowChanges, Some("Repo")),
        KeyBinding::new("secondary-2", ShowHistory, Some("Repo")),
        KeyBinding::new("secondary-0", RevealHead, Some("Repo")),
        KeyBinding::new("secondary-f", SearchCommits, Some("Repo")),
        KeyBinding::new("ctrl-f", SearchCommits, Some("Repo")),
        KeyBinding::new("secondary-b", CheckoutBranch, Some("Repo")),
        KeyBinding::new("secondary-ctrl-s", ToggleSidebar, Some("Repo")),
        KeyBinding::new("secondary-enter", Commit, Some("Repo")),
        KeyBinding::new("secondary-shift-enter", CommitAndPush, Some("Repo")),
        KeyBinding::new("escape", CloseSearch, Some("CommitSearch")),
        KeyBinding::new("secondary-a", StageAll, Some("Changes")),
        KeyBinding::new("ctrl-a", StageAll, Some("Changes")),
        KeyBinding::new("secondary-s", StashChanges, Some("Changes")),
        KeyBinding::new("ctrl-s", StashChanges, Some("Changes")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    History,
    Changes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoEvent {
    /// Unseen remote changes appeared or were acknowledged (tab indicator).
    UpdatesChanged,
    OpenWorktree(PathBuf),
}

/// Which working-copy file is shown in the Changes view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedChange {
    pub path: String,
    pub staged: bool,
}

pub struct RepoView {
    repo: Repo,
    data: Arc<RepoData>,
    loading: bool,
    load_error: Option<String>,
    /// Bumped on every reload; also keys the fade-in animation.
    generation: usize,
    mode: Mode,

    // History
    selected: Option<ObjectId>,
    selected_ref: Option<SharedString>,
    history_scroll: UniformListScrollHandle,
    history_focus: FocusHandle,
    changes_focus: FocusHandle,
    commit_diff: Entity<DiffState>,
    commit_message: SharedString,
    commit_files: usize,
    /// File picked in the commit's file tree.
    commit_file: Option<SharedString>,
    /// "Copied" feedback on the SHA (sequence, restarts the burst).
    copied: Option<usize>,
    /// Rows swept by a shimmer (new commits, a branch tip shown), with its sequence.
    shimmer: Option<(Arc<HashSet<ObjectId>>, usize)>,
    /// Long commit bodies start collapsed.
    body_expanded: bool,

    // Changes
    selected_change: Option<SelectedChange>,
    change_diff: Entity<DiffState>,
    change_hunks: Option<FileDiff>,
    message: Entity<TextareaState>,
    amend: bool,

    // Commit search (⌘F)
    search_open: bool,
    search_input: Entity<InputState>,
    search_matches: Vec<usize>,
    search_hits: HashSet<usize>,
    search_pos: usize,

    /// Branch that was just checked out, for the particle burst (name, sequence).
    celebrate: Option<(String, usize)>,
    celebrate_seq: usize,

    // Sidebar
    sidebar_collapsed: bool,
    collapsed: HashSet<&'static str>,
    /// Folder open state by key; absent means the default (open if it holds HEAD).
    folders: HashMap<String, bool>,
    sidebar_search: Entity<InputState>,
    prs: PrCache,

    // Fetch
    focused: bool,
    fetching: bool,
    last_fetch: Option<Instant>,
    fetch_error: Option<String>,
    has_updates: bool,
    busy: Option<&'static str>,

    _load: Task<()>,
    _detail: Task<()>,
    _fetch: Task<()>,
    _op: Task<()>,
    _search: Subscription,
    _commit_search: Subscription,
    /// History order the data was loaded with (reloads when the setting changes).
    commit_order: crate::settings::CommitSort,
    _settings: Subscription,
    /// Recent PRs of the repository (sidebar PR tab, stacks).
    pr_list: Arc<Vec<mugetsu_forge::PullRequest>>,
    /// GitHub login of the user.
    pr_me: Option<String>,
    sidebar_tab: SidebarTab,
    /// Sidebar / content split, so a shared width can be applied.
    sidebar_split: Entity<ResizableState>,
    /// Apply the shared layout on the next frame (tab just shown).
    adopt_layout: bool,
    pr_filter: pr_panel::PrFilter,
    /// A PR context menu is open: its info card stays hidden.
    pr_menu_open: bool,
    /// Branch whose push just succeeded (name, sequence), for its chip animation.
    pushed: Option<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarTab {
    Repo,
    Prs,
}

impl EventEmitter<RepoEvent> for RepoView {}

impl RepoView {
    pub fn new(repo: Repo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let commit_diff = cx.new(|cx| DiffState::new([], cx).with_mode(DiffMode::Unified));
        let change_diff = cx.new(|cx| DiffState::new([], cx).with_mode(DiffMode::Unified));
        let message = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder(tr("Message de commit", "Commit message"));
            state.set_auto_grow(3, 8, cx);
            state
        });
        let sidebar_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("Filtrer", "Filter")));
        let _search = cx.subscribe(&sidebar_search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr(
                "Rechercher un commit, un auteur ou un SHA",
                "Search a commit, author or SHA",
            ))
        });
        let _commit_search =
            cx.subscribe(
                &search_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.run_search(cx),
                    InputEvent::PressEnter { shift, .. } => this.step_search(*shift, cx),
                    _ => {}
                },
            );
        if let Some(avatars) = crate::avatars::store(cx) {
            cx.observe(&avatars, |_, _, cx| cx.notify()).detach();
        }
        let mut this = Self {
            repo,
            data: Arc::default(),
            loading: true,
            load_error: None,
            generation: 0,
            mode: Mode::History,
            selected: None,
            selected_ref: None,
            history_scroll: UniformListScrollHandle::new(),
            history_focus: cx.focus_handle().tab_stop(true),
            changes_focus: cx.focus_handle(),
            commit_diff,
            commit_message: SharedString::default(),
            commit_files: 0,
            commit_file: None,
            copied: None,
            shimmer: None,
            body_expanded: false,
            selected_change: None,
            change_diff,
            change_hunks: None,
            message,
            amend: false,
            collapsed: crate::settings::get()
                .collapsed_sections
                .iter()
                .filter_map(|k| sidebar::SECTION_KEYS.iter().find(|s| **s == k.as_str()))
                .copied()
                .collect(),
            folders: HashMap::new(),
            search_open: false,
            search_input,
            search_matches: Vec::new(),
            search_hits: HashSet::new(),
            search_pos: 0,
            celebrate: None,
            celebrate_seq: 0,
            sidebar_collapsed: false,
            sidebar_search,
            prs: PrCache::default(),
            focused: false,
            fetching: false,
            last_fetch: None,
            fetch_error: None,
            has_updates: false,
            busy: None,
            _load: Task::ready(()),
            _detail: Task::ready(()),
            _fetch: Task::ready(()),
            _op: Task::ready(()),
            _search,
            _commit_search,
            commit_order: crate::settings::get().commit_order,
            pr_list: Arc::default(),
            pr_me: None,
            sidebar_tab: if crate::settings::get().shared_layout
                && crate::settings::get().sidebar_prs
            {
                SidebarTab::Prs
            } else {
                SidebarTab::Repo
            },
            sidebar_split: cx.new(|_| ResizableState::default()),
            adopt_layout: true,
            pr_filter: crate::settings::get().pr_filter,
            pr_menu_open: false,
            pushed: None,
            _settings: cx.observe_global::<crate::settings::Revision>(|this, cx| {
                let order = crate::settings::get().commit_order;
                if order != this.commit_order {
                    this.commit_order = order;
                    this.reload(cx);
                }
                cx.notify();
            }),
        };
        this.reload(cx);
        this
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub fn name(&self) -> String {
        self.repo.name()
    }

    #[cfg(test)]
    pub fn data(&self) -> &Arc<RepoData> {
        &self.data
    }

    /// Current branch, or the short SHA when detached.
    pub fn head_label(&self) -> String {
        self.data.refs.head_branch.clone().unwrap_or_else(|| {
            self.data
                .refs
                .head
                .map(|h| h.to_hex_with_len(7).to_string())
                .unwrap_or_default()
        })
    }

    /// Files with uncommitted changes.
    pub fn uncommitted(&self) -> usize {
        self.data.uncommitted()
    }

    pub fn has_updates(&self) -> bool {
        self.has_updates
    }

    #[cfg(test)]
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn is_fetching(&self) -> bool {
        self.fetching
    }

    #[cfg(test)]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    #[cfg(test)]
    pub fn selected_commit(&self) -> Option<ObjectId> {
        self.selected
    }

    #[cfg(test)]
    pub fn selected_ref(&self) -> Option<&SharedString> {
        self.selected_ref.as_ref()
    }

    #[cfg(test)]
    pub fn body_expanded(&self) -> bool {
        self.body_expanded
    }

    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// Called by the workspace when the tab gains or loses focus.
    pub fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.focused == focused {
            return;
        }
        if focused {
            self.adopt_layout = true;
        }
        self.focused = focused;
        if focused && self.has_updates {
            self.has_updates = false;
            cx.emit(RepoEvent::UpdatesChanged);
            self.reload(cx);
        }
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == Mode::Changes && self.selected_change.is_none() {
            self.select_first_change(cx);
        }
        cx.notify();
    }

    /// Reloads refs, history, graph and status in the background.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let repo = self.repo.clone();
        self._load = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { RepoData::load(&repo) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(data) => this.install(data, cx),
                    Err(e) => this.load_error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn install(&mut self, data: Arc<RepoData>, cx: &mut Context<Self>) {
        self.load_error = None;
        let first_load = self.data.history.is_empty();
        let fresh: HashSet<ObjectId> = if first_load {
            HashSet::new()
        } else {
            data.history
                .commits
                .iter()
                .take(200)
                .map(|c| c.id)
                .filter(|id| self.data.history.row_of(id).is_none())
                .collect()
        };
        self.data = data;
        if !fresh.is_empty() {
            self.flash_commits(fresh, cx);
        }
        // Keep the selected commit if it still exists, else select HEAD.
        let keep = self
            .selected
            .filter(|id| self.data.history.row_of(id).is_some());
        match keep.or(self.data.refs.head) {
            Some(id) if keep.is_none() || first_load => self.select_commit(id, true, cx),
            _ => {}
        }
        if self.mode == Mode::Changes {
            let still_there = self.selected_change.as_ref().is_some_and(|sel| {
                self.data.status.files.iter().any(|f| {
                    f.path == sel.path
                        && if sel.staged {
                            f.is_staged()
                        } else {
                            f.is_unstaged()
                        }
                })
            });
            if still_there {
                let sel = self.selected_change.clone().unwrap();
                self.load_change(sel, cx);
            } else {
                self.selected_change = None;
                self.select_first_change(cx);
            }
        }
        if self.search_open {
            self.run_search(cx);
        }
        self.refresh_prs(cx);
        if let Some(head) = self.data.refs.head_branch.clone() {
            let upstream = self
                .data
                .refs
                .branches
                .iter()
                .find(|b| b.is_head)
                .and_then(|b| b.upstream.clone());
            self.lookup_pr(&head, upstream.as_deref(), false, cx);
        }
    }

    // ---- History selection -------------------------------------------------

    pub fn select_commit(&mut self, id: ObjectId, scroll: bool, cx: &mut Context<Self>) {
        let Some(row) = self.data.history.row_of(&id) else {
            return;
        };
        if scroll {
            self.history_scroll
                .scroll_to_item(row, ScrollStrategy::Center);
        }
        if self.selected == Some(id) && !self.commit_diff.read(cx).files().is_empty() {
            cx.notify();
            return;
        }
        self.selected = Some(id);
        self.body_expanded = false;
        self.commit_file = None;
        self.commit_message = SharedString::default();
        let repo = self.repo.clone();
        let rev = id.to_string();
        self._detail = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let message = repo.git().run(["log", "-1", "--format=%B", &rev])?;
                    let patch = repo.git().run([
                        "show",
                        "--format=",
                        "--no-ext-diff",
                        "--patch",
                        "--find-renames",
                        "--first-parent",
                        &rev,
                    ])?;
                    let files = UiDiffFile::parse(&patch).unwrap_or_default();
                    anyhow::Ok((message.trim().to_string(), files))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.selected != Some(id) {
                    return;
                }
                if let Ok((message, files)) = result {
                    this.commit_message = message.into();
                    this.commit_files = files.len();
                    this.commit_diff.update(cx, |d, cx| d.set_files(files, cx));
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.data.history.len();
        if len == 0 {
            return;
        }
        let row = self
            .selected
            .and_then(|id| self.data.history.row_of(&id))
            .unwrap_or(0);
        let next = row.saturating_add_signed(delta).min(len - 1);
        let id = self.data.history.commits[next].id;
        self.select_commit(id, true, cx);
    }

    // ---- Pull requests -----------------------------------------------------

    /// Lists the repository's recent PRs so every branch shows its chip.
    fn refresh_prs(&mut self, cx: &mut Context<Self>) {
        let Some(gh) = self.data.github.clone() else {
            return;
        };
        let now = Instant::now();
        if !self.prs.needs_bulk(now) {
            return;
        }
        self.prs.mark_bulk(now);
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { pr::recent(&gh) }).await;
            if let Ok(prs) = result {
                let _ = this.update(cx, |this, cx| {
                    this.prs
                        .set_bulk(mugetsu_forge::by_branch(&prs.list), Instant::now());
                    this.pr_list = Arc::new(prs.list);
                    this.pr_me = prs.me;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn lookup_pr(
        &mut self,
        branch: &str,
        upstream: Option<&str>,
        is_remote: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(gh) = self.data.github.clone() else {
            return;
        };
        let forge_branch = pr::forge_branch(branch, upstream, is_remote);
        let now = Instant::now();
        if !self.prs.needs_lookup(&forge_branch, now) {
            return;
        }
        self.prs.set(forge_branch.clone(), PrLookup::Loading, now);
        cx.spawn(async move |this, cx| {
            let key = forge_branch.clone();
            let result = cx
                .background_spawn(async move { pr::lookup(&gh, &key) })
                .await;
            let lookup = match result {
                Ok(Some(found)) => PrLookup::Found(Box::new(found)),
                Ok(None) => PrLookup::None,
                Err(e) => PrLookup::Failed(e),
            };
            let _ = this.update(cx, |this, cx| {
                this.prs.set(forge_branch, lookup, Instant::now());
                cx.notify();
            });
        })
        .detach();
    }

    pub fn pr_for(
        &self,
        branch: &str,
        upstream: Option<&str>,
        is_remote: bool,
    ) -> Option<&PrLookup> {
        self.prs.get(&pr::forge_branch(branch, upstream, is_remote))
    }

    // ---- Fetch -------------------------------------------------------------

    /// Called every second by the workspace scheduler.
    pub fn tick(&mut self, focused: bool, now: Instant, cx: &mut Context<Self>) {
        if focused && !self.loading {
            self.refresh_prs(cx);
        }
        if self.fetching || self.loading || self.data.refs.remotes.is_empty() {
            return;
        }
        if fetch_policy::is_due(self.last_fetch, now, focused) {
            self.fetch(cx);
        }
    }

    pub fn fetch(&mut self, cx: &mut Context<Self>) {
        if self.fetching {
            return;
        }
        self.fetching = true;
        self.last_fetch = Some(Instant::now());
        let repo = self.repo.clone();
        let before = self.data.refs.fingerprint;
        let status_before = self.data.status.clone();
        self._fetch = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let fetched = repo.fetch_all();
                    let refs = repo.refs()?;
                    let status = repo.status()?;
                    anyhow::Ok((fetched, refs.fingerprint != before, status != status_before))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.fetching = false;
                match result {
                    Ok((fetched, refs_moved, status_changed)) => {
                        this.fetch_error = fetched.err().map(|e| e.to_string());
                        if refs_moved || status_changed {
                            if this.focused {
                                this.reload(cx);
                            } else if refs_moved && !this.has_updates {
                                this.has_updates = true;
                                cx.emit(RepoEvent::UpdatesChanged);
                            }
                        }
                    }
                    Err(e) => this.fetch_error = Some(e.to_string()),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    // ---- Git operations ----------------------------------------------------

    /// Runs a write operation in the background, then reloads; errors become notifications.
    pub fn run_op(
        &mut self,
        label: &'static str,
        op: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_op_then(label, op, |_, _, _| {}, window, cx);
    }

    /// Like [`Self::run_op`], with `on_success` run on the UI thread when the operation succeeds.
    pub fn run_op_then(
        &mut self,
        label: &'static str,
        op: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        on_success: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label);
        let repo = self.repo.clone();
        self._op = cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { op(&repo) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(()) => on_success(this, window, cx),
                    Err(e) => window.push_notification(
                        gpui_kit::component::notification::Notification::error(trf!(
                            "{label} : {e:#}",
                            "{label}: {e:#}"
                        )),
                        cx,
                    ),
                }
                this.reload(cx);
            });
        });
        cx.notify();
    }
}

impl RepoView {
    // ---- Shortcut actions ------------------------------------------------

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    #[cfg(test)]
    pub fn sidebar_collapsed(&self) -> bool {
        self.sidebar_collapsed
    }

    pub fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op("Pull", |r| r.pull(), window, cx);
    }

    /// Pushes HEAD, setting its upstream on the default remote the first time.
    pub fn push_head(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let upstream = self
            .data
            .refs
            .branches
            .iter()
            .find(|b| b.is_head)
            .map(|b| (b.name.clone(), b.upstream.is_some() && !b.upstream_gone));
        let remote = sidebar::default_remote(&self.data.refs);
        self.run_op(
            "Push",
            move |r| match (upstream, remote) {
                (Some((branch, false)), Some(remote)) => r.push_branch(&remote, &branch, true),
                _ => r.push(None),
            },
            window,
            cx,
        );
    }

    pub fn stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data.uncommitted() > 0 {
            self.run_op("Stash", |r| r.stash_push(None), window, cx);
        }
    }

    fn reveal_head(&mut self, cx: &mut Context<Self>) {
        if let Some(head) = self.data.refs.head {
            self.set_mode(Mode::History, cx);
            self.select_commit(head, true, cx);
        }
    }

    fn new_branch_at_head(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt(
            tr("Nouvelle branche depuis HEAD", "New branch from HEAD").into(),
            tr("Nom de la branche", "Branch name"),
            String::new(),
            tr("Créer et checkout", "Create and checkout"),
            |this, name, window, cx| {
                this.run_op(
                    tr("Nouvelle branche", "New branch"),
                    move |r| r.create_branch(&name, None, true),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    fn new_tag_at_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.selected.or(self.data.refs.head) else {
            return;
        };
        let sha = target.to_string();
        self.prompt(
            trf!("Nouveau tag sur {}", "New tag on {}", &sha[..7]),
            tr("Nom du tag", "Tag name"),
            String::new(),
            tr("Créer", "Create"),
            move |this, name, window, cx| {
                let sha = sha.clone();
                this.run_op(
                    tr("Nouveau tag", "New tag"),
                    move |r| r.create_tag(&name, &sha, None),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    fn checkout_branch_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self
            .data
            .refs
            .branches
            .iter()
            .map(|b| b.name.clone())
            .collect();
        self.prompt(
            tr("Checkout d'une branche", "Checkout branch").into(),
            tr("Nom exact de la branche locale", "Exact local branch name"),
            String::new(),
            "Checkout",
            move |this, name, window, cx| {
                if names.contains(&name) {
                    this.request_checkout(name, false, window, cx);
                } else {
                    window.push_notification(
                        gpui_kit::component::notification::Notification::error(trf!(
                            "Branche introuvable : {name}",
                            "Branch not found: {name}"
                        )),
                        cx,
                    );
                }
            },
            window,
            cx,
        );
    }

    /// Checks out a branch, then bursts particles around its label.
    /// Switches branch (see [`Repo::switch_to`]) and celebrates with a particle
    /// burst on the branch.
    pub fn checkout_branch(
        &mut self,
        local: String,
        track: Option<String>,
        changes: mugetsu_git::LocalChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        // Optimistic: show the new HEAD right away; the reload after git
        // finishes (or fails) brings back the real state.
        if let Some(next) = self.data.with_checkout(&local, track.as_deref()) {
            self.data = Arc::new(next);
        }
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.celebrate = Some((local.clone(), seq));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1400))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.celebrate.as_ref().is_some_and(|(_, s)| *s == seq) {
                    this.celebrate = None;
                    cx.notify();
                }
            });
        })
        .detach();
        let branch = local;
        self.run_op(
            "Checkout",
            move |r| r.switch_to(&branch, track.as_deref(), changes),
            window,
            cx,
        );
    }

    /// Particle burst for a branch label, while its checkout is being celebrated.
    /// Shared layout: the sidebar tab and width other repositories left.
    fn adopt_shared_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = crate::settings::get();
        self.collapsed = settings
            .collapsed_sections
            .iter()
            .filter_map(|k| sidebar::SECTION_KEYS.iter().find(|s| **s == k.as_str()))
            .copied()
            .collect();
        self.pr_filter = settings.pr_filter;
        if !settings.shared_layout {
            return;
        }
        self.sidebar_tab = if settings.sidebar_prs {
            SidebarTab::Prs
        } else {
            SidebarTab::Repo
        };
        if let Some(w) = settings.sidebar_width {
            let current = self.sidebar_split.read(cx).sizes().first().copied();
            let w = w as f32;
            if current.is_some_and(|c| (f32::from(c) - w).abs() > 1.) {
                self.sidebar_split
                    .update(cx, |s, cx| s.resize_panel(0, gpui_kit::px(w), window, cx));
            }
        }
    }

    /// Plays the "pushed" animation on a branch chip.
    pub(super) fn celebrate_push(&mut self, branch: String, cx: &mut Context<Self>) {
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.pushed = Some((branch, seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(2800))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pushed.as_ref().is_some_and(|(_, s)| *s == seq) {
                    this.pushed = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub fn pushed_branch(&self) -> Option<String> {
        self.pushed.as_ref().map(|(b, _)| b.clone())
    }

    /// Sweeps a shimmer over these commits' rows.
    pub(super) fn flash_commits(&mut self, ids: HashSet<ObjectId>, cx: &mut Context<Self>) {
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.shimmer = Some((Arc::new(ids), seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1300))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.shimmer.as_ref().is_some_and(|(_, s)| *s == seq) {
                    this.shimmer = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub fn shimmering(&self) -> usize {
        self.shimmer.as_ref().map_or(0, |(ids, _)| ids.len())
    }

    /// Branches checked out in another worktree of the repository → its path.
    pub fn other_worktrees(&self) -> HashMap<String, PathBuf> {
        self.data
            .refs
            .worktrees
            .iter()
            .filter(|w| std::path::Path::new(&w.path) != self.repo.root())
            .filter_map(|w| Some((w.branch.clone()?, PathBuf::from(&w.path))))
            .collect()
    }

    pub fn history_focus_handle(&self) -> FocusHandle {
        self.history_focus.clone()
    }

    #[cfg(test)]
    pub fn prs_tab_open(&self) -> bool {
        self.sidebar_tab == SidebarTab::Prs
    }

    #[cfg(test)]
    pub fn copied_feedback(&self) -> bool {
        self.copied.is_some()
    }

    pub(super) fn burst_for(&self, branch: &str) -> Option<usize> {
        self.celebrate
            .as_ref()
            .filter(|(name, _)| name == branch)
            .map(|(_, seq)| *seq)
    }
}

impl Render for RepoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        if std::mem::take(&mut self.adopt_layout) {
            self.adopt_shared_layout(window, cx);
        }
        // Keep keyboard shortcuts working in the Changes view (⌘A, ⌘S…).
        if self.focused
            && self.mode == Mode::Changes
            && (window.focused(cx).is_none() || self.history_focus.is_focused(window))
        {
            self.changes_focus.focus(window, cx);
        }
        let body = if let Some(err) = &self.load_error {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_color(crate::theme::red()).child(tr(
                    "Impossible de lire le dépôt",
                    "Can't read the repository",
                )))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(err.clone()),
                )
                .into_any_element()
        } else {
            let main = match self.mode {
                Mode::History => v_resizable("history-split")
                    .child(
                        resizable_panel()
                            .size_range(rem * 8. ..rem * 400.)
                            .child(self.render_history(window, cx)),
                    )
                    .child(
                        resizable_panel()
                            .size(rem * 22.)
                            .size_range(rem * 8. ..rem * 400.)
                            .child(self.render_commit_details(cx)),
                    )
                    .into_any_element(),
                Mode::Changes => self.render_changes(window, cx).into_any_element(),
            };
            // The content floats as a frosted card over the window glass.
            let card = div()
                .id("content-card")
                .size_full()
                .p_1p5()
                .when(!self.sidebar_collapsed, |s| s.pl_0())
                .child(
                    div()
                        .relative()
                        .size_full()
                        .overflow_hidden()
                        .rounded_xl()
                        .border_1()
                        .border_color(crate::theme::hairline())
                        .bg(crate::theme::card())
                        .shadow_xl()
                        // Light catching the top rim of the glass.
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_4()
                                .right_4()
                                .h(gpui_kit::px(1.))
                                .bg(crate::theme::glass_edge()),
                        )
                        .child(main)
                        .children(self.render_search(cx)),
                );
            if self.sidebar_collapsed {
                card.into_any_element()
            } else {
                h_resizable("repo-split")
                    .with_state(&self.sidebar_split)
                    .on_resize(|state, _, cx| {
                        if let Some(w) = state.read(cx).sizes().first() {
                            let w = f32::from(*w).round() as u32;
                            crate::settings::remember(|s| s.sidebar_width = Some(w));
                        }
                    })
                    .child(
                        resizable_panel()
                            .size(
                                crate::settings::get()
                                    .sidebar_width
                                    .filter(|_| crate::settings::get().shared_layout)
                                    .map_or(rem * 17., |w| gpui_kit::px(w as f32)),
                            )
                            .size_range(rem * 12. ..rem * 32.)
                            .child(self.render_sidebar(cx)),
                    )
                    .child(
                        resizable_panel()
                            .size_range(rem * 30. ..rem * 400.)
                            .child(card),
                    )
                    .into_any_element()
            }
        };
        let generation = self.generation.min(1);
        v_flex()
            .id("repo-view")
            .key_context("Repo")
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.fetch(cx)))
            .on_action(cx.listener(|this, _: &Pull, window, cx| this.pull(window, cx)))
            .on_action(
                cx.listener(|this, _: &Push, window, cx| this.open_push_dialog(None, window, cx)),
            )
            .on_action(cx.listener(|this, _: &Stash, window, cx| this.stash(window, cx)))
            .on_action(
                cx.listener(|this, _: &NewBranch, window, cx| this.new_branch_at_head(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewTag, window, cx| this.new_tag_at_selection(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CheckoutBranch, window, cx| {
                this.checkout_branch_prompt(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RevealHead, _, cx| this.reveal_head(cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &SearchCommits, window, cx| {
                this.set_mode(Mode::History, cx);
                this.open_search(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Commit, window, cx| {
                if this.mode == Mode::Changes {
                    this.commit(false, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &CommitAndPush, window, cx| {
                if this.mode == Mode::Changes {
                    this.commit(true, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.reload(cx)))
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.set_mode(Mode::History, cx)))
            .on_action(cx.listener(|this, _: &ShowChanges, _, cx| this.set_mode(Mode::Changes, cx)))
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    // Content fades in when the tab first gets data.
                    .with_animation(
                        ("repo-fade", generation),
                        gpui_kit::Animation::new(std::time::Duration::from_millis(220))
                            .with_easing(gpui_kit::ease_out_quint()),
                        |el, t| el.opacity(0.4 + 0.6 * t),
                    ),
            )
    }
}
