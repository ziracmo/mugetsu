//! One repository tab: sidebar (branches, remotes, tags, worktrees, stashes),
//! history graph with commit details, and the working-copy "Changes" view.

mod card;
mod changes;
mod commit_files;
mod compact;
mod conflicts;
mod drag_rebase;
pub(crate) mod file_view;
pub(crate) mod forms;
mod history;
pub(crate) mod links;
mod local_changes;
mod menus;
mod multi;
mod nav;
mod particles;
mod pr_panel;
mod pr_tip;
mod pr_view;
mod pulse;
mod rebase;
mod restack;
mod search;
mod shimmer;
mod sidebar;
mod stack_panel;
mod staging;
mod stats_panel;
mod svg_preview;
mod toolbar;

use std::collections::{BTreeSet, HashMap, HashSet};
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
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, ScrollStrategy,
    SharedString, Styled as _, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _,
};
use mugetsu_git::{DeadWorktree, FileDiff, RefLabel, Repo};

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
        ShowStats,
        Pull,
        Push,
        NewBranch,
        NewTag,
        Stash,
        RevealHead,
        ToggleCompact,
        ToggleDetailsSide,
        SearchCommits,
        CloseSearch,
        CheckoutBranch,
        ToggleSidebar,
        Commit,
        CommitAndPush,
        StageAll,
        StashChanges,
        CopyGithubLink,
        NextConflict,
        PrevConflict,
        TakeOurs,
        TakeTheirs,
        GoBack,
        GoForward,
        ToggleStage,
        DiscardChange,
        OpenChange,
        RevealChange,
        CopyChangePath,
        NextChange,
        PrevChange
    ]
);

pub fn init(cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.bind_keys([
        KeyBinding::new("up", SelectNewer, Some("History")),
        // Conflict resolver: from conflict to conflict, and a side for the current one.
        KeyBinding::new("alt-down", NextConflict, Some("Changes")),
        KeyBinding::new("alt-up", PrevConflict, Some("Changes")),
        KeyBinding::new("f7", NextConflict, Some("Changes")),
        KeyBinding::new("shift-f7", PrevConflict, Some("Changes")),
        KeyBinding::new("alt-left", TakeOurs, Some("Changes")),
        KeyBinding::new("alt-right", TakeTheirs, Some("Changes")),
        KeyBinding::new("k", SelectNewer, Some("History")),
        KeyBinding::new("down", SelectOlder, Some("History")),
        KeyBinding::new("j", SelectOlder, Some("History")),
        // Plain F: only from the history list, so typing in a field is untouched.
        KeyBinding::new("f", RevealHead, Some("History")),
        KeyBinding::new("secondary-shift-f", Fetch, Some("Repo")),
        KeyBinding::new("secondary-shift-l", Pull, Some("Repo")),
        KeyBinding::new("secondary-shift-p", Push, Some("Repo")),
        KeyBinding::new("secondary-shift-b", NewBranch, Some("Repo")),
        KeyBinding::new("secondary-shift-g", NewTag, Some("Repo")),
        KeyBinding::new("secondary-shift-h", Stash, Some("Repo")),
        // ⌃R fetches the whole window; elsewhere than macOS, where ⌘ is ⌃,
        // refreshing moves to F5.
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-r"
            } else {
                "f5"
            },
            Refresh,
            Some("Repo"),
        ),
        KeyBinding::new("secondary-1", ShowChanges, Some("Repo")),
        KeyBinding::new("secondary-2", ShowHistory, Some("Repo")),
        KeyBinding::new("secondary-3", ShowStats, Some("Repo")),
        KeyBinding::new("secondary-0", RevealHead, Some("Repo")),
        KeyBinding::new("secondary-shift-k", ToggleCompact, Some("Repo")),
        KeyBinding::new("secondary-alt-c", CopyGithubLink, Some("Repo")),
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
        // The file lists (plain keys stay with the filter and the message).
        KeyBinding::new("enter", ToggleStage, Some("ChangeList")),
        KeyBinding::new("space", ToggleStage, Some("ChangeList")),
        KeyBinding::new("delete", DiscardChange, Some("ChangeList")),
        KeyBinding::new("secondary-backspace", DiscardChange, Some("ChangeList")),
        KeyBinding::new("secondary-down", OpenChange, Some("ChangeList")),
        KeyBinding::new("secondary-alt-r", RevealChange, Some("ChangeList")),
        KeyBinding::new("secondary-shift-c", CopyChangePath, Some("ChangeList")),
        KeyBinding::new("down", NextChange, Some("ChangeList")),
        KeyBinding::new("j", NextChange, Some("ChangeList")),
        KeyBinding::new("up", PrevChange, Some("ChangeList")),
        KeyBinding::new("k", PrevChange, Some("ChangeList")),
        // Back / forward, as in browsers and editors (the mouse side buttons too).
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-["
            } else {
                "ctrl-alt-left"
            },
            GoBack,
            Some("Repo"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-]"
            } else {
                "ctrl-alt-right"
            },
            GoForward,
            Some("Repo"),
        ),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    History,
    Changes,
    /// Who works on the default branch, and when (commits only).
    Stats,
    /// A PR stack's progress (see `stack_focus`).
    Stack,
    /// One pull request (see `pr_page`).
    Pr,
    /// One file's history and blame (see `file_page`).
    File,
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
    /// Local branches picked with ⌘/⌃-click, for batch actions.
    picked: BTreeSet<String>,
    /// Last history row ⌘-clicked, where a ⇧-click range starts.
    pick_anchor: Option<ObjectId>,
    history_scroll: UniformListScrollHandle,
    history_focus: FocusHandle,
    changes_focus: FocusHandle,
    /// Stats, Stack and PR views (no list to hold the focus).
    root_focus: FocusHandle,
    commit_diff: Entity<DiffState>,
    commit_message: SharedString,
    commit_files: usize,
    /// File picked in the commit's file tree.
    commit_file: Option<SharedString>,
    commit_files_scroll: ScrollHandle,
    /// Folders folded in the commit's file tree (their paths).
    commit_folded: HashSet<String>,
    /// "Copied" feedback on the SHA (sequence, restarts the burst).
    copied: Option<usize>,
    /// Rows swept by a shimmer (new commits, a branch tip shown), with its sequence.
    shimmer: Option<(Arc<HashSet<ObjectId>>, usize)>,
    /// Long commit bodies start collapsed.
    body_expanded: bool,
    /// Lines picked in the commit diff's gutter (GitHub link pill).
    line_selection: Option<gpui_kit::component::diff::DiffLineRange>,
    /// Bumped when a new selection appears, to replay the pill's entrance.
    line_selection_seq: usize,
    /// "Link copied" flourish: where, and its sequence.
    link_copied: Option<(links::LinkSpot, usize)>,
    _diff_events: Subscription,

    // Changes
    selected_change: Option<SelectedChange>,
    change_diff: Entity<DiffState>,
    change_hunks: Option<FileDiff>,
    /// Rendered before/after of the selected change when it is an SVG.
    change_svg: Option<svg_preview::SvgPreview>,
    message: Entity<TextareaState>,
    amend: bool,
    /// Three-pane resolver on the selected conflicted file.
    conflict: Option<conflicts::ConflictEditor>,
    /// A file was just marked resolved (sequence), for the banner's pill.
    resolved_flash: Option<usize>,
    /// Stage moves shown before git ran them, oldest (the running one) first.
    staging: std::collections::VecDeque<staging::StageOp>,
    staging_running: bool,
    /// Git writes held until `staging` drains.
    after_staging: Vec<staging::AfterStaging>,
    /// Bumped on every stage move: a status read before it is stale.
    status_epoch: usize,
    /// `status_epoch` when the running reload began.
    load_epoch: usize,
    status_loading: bool,
    _stage: Task<()>,
    _status: Task<()>,
    /// Filters both file lists.
    change_filter: Entity<InputState>,
    /// The file lists, where plain keys (space, arrows) act on files.
    change_list_focus: FocusHandle,
    /// Files listed as a folder tree rather than flat.
    change_tree: bool,
    /// Folded folders of the tree, as (staged list, folder path).
    change_folded: HashSet<(bool, String)>,
    /// Files picked with ⌘/Ctrl-click; stage moves act on all of them.
    change_marks: Vec<SelectedChange>,

    // Commit search (⌘F)
    search_open: bool,
    search_input: Entity<InputState>,
    search_matches: Vec<usize>,
    search_hits: HashSet<usize>,
    search_pos: usize,

    /// Merges whose merged branch is unfolded (all others start folded).
    expanded_merges: HashSet<ObjectId>,

    /// Commit row celebrating a checkout (tip, sequence): sweep and particles.
    celebrate: Option<(ObjectId, usize)>,
    celebrate_seq: usize,
    /// A rebase just finished: the next load scrolls to the rebased branch.
    reveal_head_on_load: bool,

    // Sidebar
    sidebar_collapsed: bool,
    collapsed: HashSet<&'static str>,
    /// Folder open state by key; absent means the default (open if it holds HEAD).
    folders: HashMap<String, bool>,
    sidebar_search: Entity<InputState>,
    prs: PrCache,
    /// Linked worktrees that look finished, with the refs state they were found for.
    dead_worktrees: Arc<Vec<DeadWorktree>>,
    dead_key: Option<u64>,
    _dead_scan: Task<()>,
    /// Commits per day over the last month (sidebar foot).
    pulse: pulse::Pulse,
    /// Diffs PR listings into system notifications.
    pr_watcher: crate::notify::Watcher,

    // Fetch
    focused: bool,
    fetching: bool,
    last_fetch: Option<Instant>,
    fetch_error: Option<String>,
    has_updates: bool,
    busy: Option<&'static str>,
    /// Last Fetch / Pull / Push outcome on its button: (label, ok, seq).
    op_flash: Option<(&'static str, bool, usize)>,
    /// The running fetch was asked for (button, ⇧⌘F), not the timer.
    manual_fetch: bool,

    _load: Task<()>,
    _detail: Task<()>,
    /// The PR shown by `Mode::Pr`, and its diff.
    pr_page: Option<pr_view::PrPage>,
    /// Fast clicks on a stack card: (PR, last click, count).
    rage: (u64, Instant, u32),
    rage_hints: usize,
    pr_diff: Entity<DiffState>,
    _pr_task: Task<()>,
    /// The file shown by `Mode::File`, and the diff of its picked commit.
    file_page: Option<file_view::FilePage>,
    file_diff: Entity<DiffState>,
    _file_task: Task<()>,
    _file_diff_task: Task<()>,
    _fetch: Task<()>,
    _op: Task<()>,
    _search: Subscription,
    _change_filter: Subscription,
    _commit_search: Subscription,
    /// History order the data was loaded with (reloads when the setting changes).
    commit_order: crate::settings::CommitSort,
    _settings: Subscription,
    /// Recent PRs of the repository (sidebar PR tab, stacks).
    pr_list: Arc<Vec<mugetsu_forge::PullRequest>>,
    /// History as one row per branch (see `compact`).
    compact: bool,
    compact_seq: usize,
    /// Commit details beside the history instead of below it.
    details_side: bool,
    compact_cache: Option<(Arc<RepoData>, Arc<compact::Summary>)>,
    pub(crate) compact_sort: crate::settings::CompactSort,
    /// Compact view shows only the branches with commits by this email.
    compact_author: Option<String>,
    /// Who commits here (email, name), read once the compact view needs it.
    compact_me: Option<Option<(String, String)>>,
    /// Head of a PR in the stack shown by `Mode::Stack`.
    stack_focus: Option<String>,
    /// Restarts the stack view's entrance animation.
    stack_seq: usize,
    /// Restack running or just finished (stack view card).
    restack: Option<restack::RestackRun>,
    /// Lag of the shown stack behind its remote trunk (its branches, bottom first).
    stack_drift: Option<(Vec<String>, mugetsu_git::StackDrift)>,
    _drift: Task<()>,
    /// GitHub login of the user.
    pr_me: Option<String>,
    sidebar_tab: SidebarTab,
    /// Sidebar / content split, so a shared width can be applied.
    sidebar_split: Entity<ResizableState>,
    /// Apply the shared layout on the next frame (tab just shown).
    adopt_layout: bool,
    pr_filter: pr_panel::PrFilter,
    /// A PR context menu is open: its info card stays hidden.
    menu_open: bool,
    /// Branch whose push just succeeded (name, sequence), for its chip animation.
    pushed: Option<(String, usize)>,
    /// Refs just deleted (branch, remote branch or stash, with a sequence each):
    /// their chips are slashed before the reload drops them.
    slashed: Vec<(RefLabel, usize)>,
    /// Branch chip dropped on another: the rebase popover asks first.
    pending_rebase: Option<drag_rebase::PendingRebase>,
    /// Stats view (computed on demand, kept per tab).
    stats: stats_panel::StatsState,
    /// Where back / forward lead.
    nav: nav::NavHistory,
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
        let change_filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(tr("Filtrer les fichiers", "Filter files"))
        });
        let _change_filter = cx.subscribe(&change_filter, |_, _, event: &InputEvent, cx| {
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
        let _diff_events = cx.subscribe(
            &commit_diff,
            |this, _, event: &gpui_kit::component::diff::DiffEvent, cx| {
                use gpui_kit::component::diff::DiffEvent;
                let range = match event {
                    DiffEvent::SelectionChanged(r) => r.clone(),
                    DiffEvent::SelectionEnded(r) => Some(r.clone()),
                    DiffEvent::ActiveFileChanged(path) => {
                        this.reveal_commit_file(path.as_deref(), cx);
                        cx.notify();
                        return;
                    }
                    _ => return,
                };
                if this.line_selection.is_none() && range.is_some() {
                    this.line_selection_seq += 1;
                }
                this.line_selection = range;
                cx.notify();
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
            picked: BTreeSet::new(),
            pick_anchor: None,
            history_scroll: UniformListScrollHandle::new(),
            history_focus: cx.focus_handle().tab_stop(true),
            changes_focus: cx.focus_handle(),
            root_focus: cx.focus_handle(),
            commit_diff,
            commit_message: SharedString::default(),
            commit_files: 0,
            commit_file: None,
            commit_files_scroll: ScrollHandle::new(),
            commit_folded: HashSet::new(),
            copied: None,
            shimmer: None,
            body_expanded: false,
            line_selection: None,
            line_selection_seq: 0,
            link_copied: None,
            _diff_events,
            selected_change: None,
            change_diff,
            change_hunks: None,
            change_svg: None,
            message,
            amend: false,
            conflict: None,
            resolved_flash: None,
            staging: Default::default(),
            staging_running: false,
            after_staging: Vec::new(),
            status_epoch: 0,
            load_epoch: 0,
            status_loading: false,
            _stage: Task::ready(()),
            _status: Task::ready(()),
            change_filter,
            change_list_focus: cx.focus_handle(),
            change_tree: crate::settings::get().changes_tree,
            change_folded: HashSet::new(),
            change_marks: Vec::new(),
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
            expanded_merges: HashSet::new(),
            celebrate: None,
            celebrate_seq: 0,
            reveal_head_on_load: false,
            sidebar_collapsed: false,
            sidebar_search,
            prs: PrCache::default(),
            dead_worktrees: Arc::default(),
            dead_key: None,
            _dead_scan: Task::ready(()),
            pulse: Default::default(),
            pr_watcher: Default::default(),
            focused: false,
            fetching: false,
            last_fetch: None,
            fetch_error: None,
            has_updates: false,
            busy: None,
            op_flash: None,
            manual_fetch: false,
            _load: Task::ready(()),
            _detail: Task::ready(()),
            pr_page: None,
            rage: (0, Instant::now(), 0),
            rage_hints: 0,
            pr_diff: cx.new(|cx| DiffState::new([], cx).with_mode(DiffMode::Unified)),
            file_page: None,
            file_diff: cx.new(|cx| DiffState::new([], cx).with_mode(DiffMode::Unified)),
            _file_task: Task::ready(()),
            _file_diff_task: Task::ready(()),
            _pr_task: Task::ready(()),
            _fetch: Task::ready(()),
            _op: Task::ready(()),
            _search,
            _change_filter,
            _commit_search,
            commit_order: crate::settings::get().commit_order,
            pr_list: Arc::default(),
            compact: crate::settings::get().history_compact,
            compact_seq: 0,
            details_side: crate::settings::get().details_side,
            compact_cache: None,
            compact_sort: crate::settings::get().compact_sort,
            compact_author: None,
            compact_me: None,
            stack_focus: None,
            stack_seq: 0,
            restack: None,
            stack_drift: None,
            _drift: Task::ready(()),
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
            menu_open: false,
            pushed: None,
            slashed: Vec::new(),
            pending_rebase: None,
            stats: Default::default(),
            nav: Default::default(),
            _settings: cx.observe_global::<crate::settings::Revision>(|this, cx| {
                let order = crate::settings::get().commit_order;
                if order != this.commit_order {
                    this.commit_order = order;
                    this.reload(cx);
                }
                if !cfg!(test) {
                    this.details_side = crate::settings::get().details_side;
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
    /// File lit up in the commit's file tree (the one read in the diff).
    pub fn commit_active_file(&self, cx: &gpui_kit::App) -> Option<SharedString> {
        let diff = self.commit_diff.read(cx);
        diff.active_file().or(self.commit_file.as_ref()).cloned()
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
        self.busy.is_some() || self.is_staging()
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
        self.load_epoch = self.status_epoch;
        self.loading = true;
        let repo = self.repo.clone();
        let expanded = self.expanded_merges.clone();
        self._load = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { RepoData::load(&repo, &expanded) })
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

    fn install(&mut self, mut data: Arc<RepoData>, cx: &mut Context<Self>) {
        self.merge_loaded_status(&mut data);
        self.load_error = None;
        let first_load = self.data.history.is_empty();
        let had_conflicts = self.data.conflicted().next().is_some();
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
        self.prune_picks();
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
        if std::mem::take(&mut self.reveal_head_on_load) {
            self.reveal_head(cx);
        }
        // New conflicts (a merge, rebase, cherry-pick or stash pop stopped on
        // them, here or in a terminal): straight to the resolver.
        if !had_conflicts && self.data.conflicted().next().is_some() {
            self.open_conflicts(None, cx);
        }
        if self
            .conflict
            .as_ref()
            .is_some_and(|c| !self.is_conflicted(&c.path))
        {
            self.conflict = None;
        }
        self.sync_selected_change(cx);
        if self.search_open {
            self.run_search(cx);
        }
        self.refresh_prs(true, cx);
        self.scan_stack_drift(cx);
        self.scan_dead_worktrees(cx);
        self.scan_pulse(cx);
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
        // What a reload re-selected is no step back.
        let place = self.place();
        self.nav.settle(place);
    }

    /// Looks for finished worktrees (and their size) in the background when
    /// the refs or the worktree list changed.
    fn scan_dead_worktrees(&mut self, cx: &mut Context<Self>) {
        use std::hash::{Hash, Hasher};
        let refs = &self.data.refs;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        refs.fingerprint.hash(&mut h);
        for w in &refs.worktrees {
            (&w.path, &w.branch, w.locked, w.prunable).hash(&mut h);
        }
        let key = h.finish();
        if self.dead_key == Some(key) {
            return;
        }
        self.dead_key = Some(key);
        if refs.worktrees.len() < 2 {
            self.dead_worktrees = Arc::default();
            self._dead_scan = Task::ready(());
            return;
        }
        let (repo, refs) = (self.repo.clone(), refs.clone());
        self._dead_scan = cx.spawn(async move |this, cx| {
            let dead = cx
                .background_spawn(async move { repo.dead_worktrees(&refs) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.dead_worktrees = Arc::new(dead);
                cx.notify();
            });
        });
    }

    pub fn dead_worktrees(&self) -> &[DeadWorktree] {
        &self.dead_worktrees
    }

    // ---- History selection -------------------------------------------------

    pub fn select_commit(&mut self, id: ObjectId, scroll: bool, cx: &mut Context<Self>) {
        let Some(row) = self.data.history.row_of(&id) else {
            return;
        };
        self.reveal(row);
        if scroll && let Some(shown) = self.data.folded.shown_row(row) {
            self.history_scroll
                .scroll_to_item(shown, ScrollStrategy::Center);
        }
        if self.selected == Some(id) && !self.commit_diff.read(cx).files().is_empty() {
            cx.notify();
            return;
        }
        self.selected = Some(id);
        self.body_expanded = false;
        self.line_selection = None;
        self.commit_file = None;
        self.commit_folded.clear();
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
                    // A file restored by back / forward before the diff was there.
                    if let Some(path) = this.commit_file.clone() {
                        this.commit_diff
                            .update(cx, |d, cx| d.scroll_to_file(&path, cx));
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let folded = self.data.folded.clone();
        if folded.is_empty() {
            return;
        }
        let row = self
            .selected
            .and_then(|id| self.data.history.row_of(&id))
            .and_then(|r| folded.shown_row(r))
            .unwrap_or(0);
        let next = row.saturating_add_signed(delta).min(folded.len() - 1);
        let id = self.data.history.commits[folded.history_row(next)].id;
        self.select_commit(id, true, cx);
    }

    // ---- Folded merges -----------------------------------------------------

    /// Unfolds the merges hiding history row `row`, if any.
    fn reveal(&mut self, row: usize) {
        // One level per round: a merge folded inside another one shows once its parent opens.
        while let Some(merge) = self.data.folded.owner(row) {
            if !self
                .expanded_merges
                .insert(self.data.history.commits[merge].id)
            {
                break;
            }
            self.data = Arc::new(self.data.refold(&self.expanded_merges));
        }
    }

    /// Folds or unfolds the branch merged by `merge` (the arrow next to it).
    pub fn toggle_merge(&mut self, merge: ObjectId, cx: &mut Context<Self>) {
        if !self.expanded_merges.remove(&merge) {
            self.expanded_merges.insert(merge);
        }
        self.data = Arc::new(self.data.refold(&self.expanded_merges));
        // A selection folded away moves to the merge that hides it.
        let hidden = self
            .selected
            .and_then(|id| self.data.history.row_of(&id))
            .is_some_and(|r| self.data.folded.shown_row(r).is_none());
        if hidden {
            self.select_commit(merge, false, cx);
        }
        cx.notify();
    }

    // ---- Pull requests -----------------------------------------------------

    /// Lists the repository's recent PRs so every branch shows its chip.
    fn refresh_prs(&mut self, focused: bool, cx: &mut Context<Self>) {
        let Some(forge) = self.data.forge.clone() else {
            return;
        };
        let now = Instant::now();
        if !self.prs.needs_bulk(now, focused) {
            return;
        }
        self.prs.mark_bulk(now);
        let repo_name = forge.name.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { pr::recent(&forge) }).await;
            if let Ok(prs) = result {
                let _ = this.update(cx, |this, cx| {
                    let now = Instant::now();
                    let prefs = crate::settings::get().notifications;
                    let notes =
                        this.pr_watcher
                            .diff(&repo_name, &prs.list, prs.me.as_deref(), prefs, now);
                    notes.iter().for_each(crate::notify::show);
                    this.prs.set_bulk(mugetsu_forge::by_branch(&prs.list), now);
                    this.pr_list = Arc::new(prs.list);
                    this.pr_me = prs.me;
                    this.scan_stack_drift(cx);
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
        let Some(forge) = self.data.forge.clone() else {
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
                .background_spawn(async move { pr::lookup(&forge, &key) })
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
        let lookup = self
            .prs
            .get(&pr::forge_branch(branch, upstream, is_remote))?;
        // Repository tab setting: only my PRs / those I review get a chip.
        let filter = crate::settings::get().pr_chips;
        match lookup {
            _ if filter == crate::settings::PrFilter::All => Some(lookup),
            PrLookup::Found(pr) if pr_panel::keep(pr, filter, self.pr_me.as_deref()) => {
                Some(lookup)
            }
            _ => None,
        }
    }

    // ---- Fetch -------------------------------------------------------------

    /// Called every second by the workspace scheduler.
    pub fn tick(&mut self, focused: bool, now: Instant, cx: &mut Context<Self>) {
        // Background tabs keep listing PRs only when notifications want them.
        if !self.loading && (focused || crate::settings::get().notifications.any()) {
            self.refresh_prs(focused, cx);
        }
        if self.fetching || self.loading || self.data.refs.remotes.is_empty() {
            return;
        }
        if fetch_policy::is_due(self.last_fetch, now, focused) {
            self.fetch(cx);
        }
    }

    /// Fetch asked for by the user: its button shows the outcome.
    pub fn fetch_now(&mut self, cx: &mut Context<Self>) {
        if !self.fetching {
            self.manual_fetch = true;
        }
        self.fetch(cx);
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
                if std::mem::take(&mut this.manual_fetch) {
                    let ok = matches!(&result, Ok((Ok(_), _, _)));
                    this.flash_op("Fetch", ok, cx);
                }
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
        if !self.staging.is_empty() {
            self.after_staging.push(Box::new(move |this, window, cx| {
                this.run_op_then(label, op, on_success, window, cx)
            }));
            return;
        }
        self.busy = Some(label);
        let repo = self.repo.clone();
        self._op = cx.spawn_in(window, async move |this, cx| {
            let (result, conflicts) = cx
                .background_spawn(async move {
                    let result = op(&repo);
                    let conflicts = repo.conflicted_paths().map_or(0, |p| p.len());
                    (result, conflicts)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                this.pump_staging(window, cx);
                this.flash_op(label, result.is_ok(), cx);
                let seq = this.celebrate_seq;
                match result {
                    Ok(()) => on_success(this, window, cx),
                    // Done, but the stashed changes conflict on top: the
                    // resolver takes over, they are still in the stash.
                    Err(e) if e.is::<mugetsu_git::StashConflict>() => {
                        on_success(this, window, cx);
                        window.push_notification(
                            gpui_kit::component::notification::Notification::warning(trf!(
                                "{label} effectué, mais vos changements mis de côté sont en \
                                 conflit : {conflicts} fichier(s) à résoudre. Ils restent \
                                 aussi dans le stash.",
                                "{label} done, but your stashed changes conflict: \
                                 {conflicts} file(s) to resolve. They are also kept in the \
                                 stash."
                            )),
                            cx,
                        )
                    }
                    // Stopped on conflicts: the resolver takes over.
                    Err(_) if conflicts > 0 => window.push_notification(
                        gpui_kit::component::notification::Notification::warning(trf!(
                            "{label} : {conflicts} fichier(s) en conflit à résoudre",
                            "{label}: {conflicts} conflicted file(s) to resolve"
                        )),
                        cx,
                    ),
                    Err(e) => window.push_notification(
                        crate::toast::error(trf!("{label} : échec", "{label} failed"), &e, None),
                        cx,
                    ),
                }
                // A slash started by `on_success` reloads once it has played.
                if !this.slashed.iter().any(|(_, s)| *s > seq) {
                    this.reload(cx);
                }
            });
        });
        cx.notify();
    }

    /// Slashes the chips of `refs`, just deleted, then reloads so they go away.
    fn slash(&mut self, refs: Vec<RefLabel>, cx: &mut Context<Self>) {
        if refs.is_empty() {
            self.reload(cx);
            return;
        }
        for r in refs {
            self.celebrate_seq += 1;
            self.slashed.push((r, self.celebrate_seq));
        }
        let last = self.celebrate_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(750))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.slashed.retain(|(_, s)| *s > last);
                this.reload(cx);
            });
        })
        .detach();
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

    /// Call when a rebase finished: the reload that follows scrolls to the
    /// rebased branch's new tip instead of leaving the view where it was.
    pub(crate) fn rebased(&mut self) {
        self.reveal_head_on_load = true;
    }

    fn reveal_head(&mut self, cx: &mut Context<Self>) {
        if let Some(head) = self.data.refs.head {
            self.set_mode(Mode::History, cx);
            self.select_commit(head, true, cx);
            self.flash_commits(HashSet::from([head]), cx);
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

    /// Switches branch (see [`Repo::switch_to`]) and celebrates on the
    /// branch's commit row: a sweep with particles carried along.
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
            // Refolded: the new HEAD may sit in a folded merged branch.
            self.data = Arc::new(next.refold(&self.expanded_merges));
        }
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.celebrate = self
            .data
            .refs
            .branches
            .iter()
            .find(|b| b.name == local)
            .map(|b| (b.target, seq));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1600))
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

    /// Deletes a local branch; on success its chip is slashed in two, then the
    /// view reloads (the chip goes away). `force` deletes it even unmerged; an
    /// unmerged refusal offers that in its toast, behind a confirmation.
    pub fn delete_branch(
        &mut self,
        name: String,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Delete");
        let repo = self.repo.clone();
        let branch = name.clone();
        self._op = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { repo.delete_branch(&branch, force) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(()) => this.slash(vec![RefLabel::Branch { name, head: false }], cx),
                    Err(e) => {
                        this.delete_failed(name, force, &e, window, cx);
                        this.reload(cx);
                    }
                }
            });
        });
        cx.notify();
    }

    /// Drops `stash@{index}`; on success its chip is slashed like a branch's.
    pub fn drop_stash(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.run_op_then(
            tr("Supprimer stash", "Delete stash"),
            move |r| r.stash_drop(index),
            move |this, _, cx| this.slash(vec![RefLabel::Stash(index)], cx),
            window,
            cx,
        );
    }

    /// Toasts a failed branch delete; an unmerged refusal offers to force it.
    fn delete_failed(
        &mut self,
        name: String,
        force: bool,
        e: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let force = (!force && crate::toast::is_unmerged_branch(e)).then(|| {
            let view = cx.entity().downgrade();
            let name = name.clone();
            let action: crate::toast::Action = (
                tr("Forcer la suppression…", "Force delete…").into(),
                std::rc::Rc::new(move |window, cx| {
                    let name = name.clone();
                    let _ = view.update(cx, |this, cx| this.confirm_force_delete(name, window, cx));
                }),
            );
            action
        });
        window.push_notification(
            crate::toast::error(
                trf!(
                    "Impossible de supprimer « {name} »",
                    "Couldn't delete '{name}'"
                ),
                e,
                force,
            ),
            cx,
        );
    }

    /// Asks before `git branch -D`: the branch's unmerged commits are lost.
    pub fn confirm_force_delete(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm(
            trf!(
                "Forcer la suppression de « {name} » ?",
                "Force delete '{name}'?"
            ),
            tr(
                "La branche n'est pas mergée : ses commits qui ne sont sur aucune autre branche seront perdus.",
                "The branch isn't merged: its commits that are on no other branch will be lost.",
            )
            .into(),
            tr("Forcer la suppression", "Force delete"),
            move |this, window, cx| this.delete_branch(name.clone(), true, window, cx),
            window,
            cx,
        );
    }

    /// A local branch's "Delete…": asks first, unless the settings delete
    /// local branches at once, then forced.
    pub fn request_delete_branch(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::settings::get().force_delete.local {
            self.delete_branch(name, true, window, cx);
            return;
        }
        self.confirm(
            trf!("Supprimer la branche '{name}' ?", "Delete branch '{name}'?"),
            tr(
                "La branche locale est supprimée. Git refuse si elle n'est pas mergée.",
                "The local branch is deleted. Git refuses if it isn't merged.",
            )
            .into(),
            tr("Supprimer", "Delete"),
            move |this, window, cx| this.delete_branch(name.clone(), false, window, cx),
            window,
            cx,
        );
    }

    /// The local branch matching `remote/branch`: the one tracking it, else
    /// the one with its name. Not the checked-out one, which can't be deleted.
    pub fn local_twin(&self, remote: &str, branch: &str) -> Option<String> {
        let full = format!("{remote}/{branch}");
        let branches = &self.data.refs.branches;
        branches
            .iter()
            .find(|b| b.upstream.as_deref() == Some(full.as_str()))
            .or_else(|| branches.iter().find(|b| b.name == branch))
            .filter(|b| !b.is_head)
            .map(|b| b.name.clone())
    }

    /// A remote branch's "Delete…": asks first, offering to delete its local
    /// twin too. With remote deletes set to skip the question, it only asks
    /// when there is such a twin.
    pub fn request_delete_remote_branch(
        &mut self,
        remote: String,
        branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = trf!(
            "Supprimer '{branch}' sur '{remote}' ?",
            "Delete '{branch}' on '{remote}'?"
        );
        let description: String = tr(
            "La branche est supprimée sur le serveur pour tout le monde.",
            "The branch is deleted on the server for everyone.",
        )
        .into();
        let Some(local) = self.local_twin(&remote, &branch) else {
            if crate::settings::get().force_delete.remote {
                self.delete_remote_branch(remote, branch, None, window, cx);
                return;
            }
            self.confirm(
                title,
                description,
                tr("Supprimer", "Delete"),
                move |this, window, cx| {
                    this.delete_remote_branch(remote.clone(), branch.clone(), None, window, cx)
                },
                window,
                cx,
            );
            return;
        };
        crate::dialogs::confirm_with_option(
            title,
            description,
            trf!(
                "Supprimer aussi la branche locale '{local}'",
                "Also delete the local branch '{local}'"
            ),
            crate::settings::get().delete_local_too,
            tr("Supprimer", "Delete"),
            move |this, also, window, cx| {
                crate::settings::remember(|s| s.delete_local_too = also);
                let local = also.then(|| local.clone());
                this.delete_remote_branch(remote.clone(), branch.clone(), local, window, cx)
            },
            window,
            cx,
        );
    }

    /// Deletes `branch` on `remote`, then `local` if given (forced when the
    /// settings force local deletes; else an unmerged refusal offers it).
    pub fn delete_remote_branch(
        &mut self,
        remote: String,
        branch: String,
        local: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let force = crate::settings::get().force_delete.local;
        // Both in one operation: the local delete starts once the remote one is done.
        let deleted_local = std::sync::Arc::new(std::sync::Mutex::new(None));
        let outcome = deleted_local.clone();
        let remote_ref = format!("{remote}/{branch}");
        self.run_op_then(
            tr("Supprimer", "Delete"),
            move |r| {
                r.delete_remote_branch(&remote, &branch)?;
                if let Some(local) = local {
                    let result = r.delete_branch(&local, force);
                    *outcome.lock().unwrap() = Some((local, result));
                }
                Ok(())
            },
            move |this, window, cx| {
                let mut gone = vec![RefLabel::Remote(remote_ref)];
                match deleted_local.lock().unwrap().take() {
                    Some((name, Ok(()))) => gone.push(RefLabel::Branch { name, head: false }),
                    Some((name, Err(e))) => this.delete_failed(name, force, &e, window, cx),
                    None => {}
                }
                this.slash(gone, cx);
            },
            window,
            cx,
        );
    }

    #[cfg(test)]
    pub fn slashed_refs(&self) -> Vec<RefLabel> {
        self.slashed.iter().map(|(r, _)| r.clone()).collect()
    }

    /// Plays the "pushed" animation on a branch chip.
    /// Plays an operation's success or failure on its toolbar button.
    fn flash_op(&mut self, label: &'static str, ok: bool, cx: &mut Context<Self>) {
        if !matches!(label, "Fetch" | "Pull" | "Push") {
            return;
        }
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.op_flash = Some((label, ok, seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1600))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.op_flash.is_some_and(|(_, _, s)| s == seq) {
                    this.op_flash = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(test)]
    pub fn details_side(&self) -> bool {
        self.details_side
    }

    #[cfg(test)]
    pub fn op_flash(&self) -> Option<(&'static str, bool)> {
        self.op_flash.map(|(l, ok, _)| (l, ok))
    }

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

    #[cfg(test)]
    pub fn set_pr_list(&mut self, prs: Vec<mugetsu_forge::PullRequest>, cx: &mut Context<Self>) {
        self.pr_list = Arc::new(prs);
        self.scan_stack_drift(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn history_focus_handle(&self) -> FocusHandle {
        self.history_focus.clone()
    }

    /// Where keyboard actions should land: the list of the current view.
    pub fn focus_target(&self) -> FocusHandle {
        match self.mode {
            Mode::Changes => self.change_list_focus.clone(),
            Mode::History => self.history_focus.clone(),
            Mode::Stats | Mode::Stack | Mode::Pr | Mode::File => self.root_focus.clone(),
        }
    }

    /// Local branch names, and whether each is HEAD.
    pub fn local_branches(&self) -> Vec<(String, bool)> {
        self.data
            .refs
            .branches
            .iter()
            .map(|b| (b.name.clone(), b.is_head))
            .collect()
    }

    #[cfg(test)]
    pub fn prs_tab_open(&self) -> bool {
        self.sidebar_tab == SidebarTab::Prs
    }

    #[cfg(test)]
    pub fn copied_feedback(&self) -> bool {
        self.copied.is_some()
    }

    #[cfg(test)]
    pub fn celebrating(&self) -> Option<ObjectId> {
        self.celebrate.map(|(id, _)| id)
    }
}

impl Render for RepoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = window.rem_size();
        if std::mem::take(&mut self.adopt_layout) {
            self.adopt_shared_layout(window, cx);
        }
        self.ensure_stats(cx);
        self.track_place();
        // Keep keyboard shortcuts working in the Changes view (⌘A, ⌘S…).
        if self.focused
            && self.mode == Mode::Changes
            && (window.focused(cx).is_none() || self.history_focus.is_focused(window))
        {
            self.change_list_focus.focus(window, cx);
        }
        // Stats, Stack and PR draw no list: keep keys on the tab.
        if self.focused
            && matches!(self.mode, Mode::Stats | Mode::Stack | Mode::Pr | Mode::File)
            && (window.focused(cx).is_none() || self.history_focus.is_focused(window))
        {
            self.root_focus.focus(window, cx);
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
                Mode::History => {
                    let side = self.details_side;
                    let split = if side {
                        h_resizable("history-split-side")
                    } else {
                        v_resizable("history-split")
                    };
                    split
                        .child(resizable_panel().size_range(rem * 8. ..rem * 400.).child(
                            if self.compact {
                                self.render_compact(cx)
                            } else if self.compact_seq > 0 {
                                // Back from the compact view: the list eases in.
                                let seq = self.compact_seq;
                                div()
                                    .size_full()
                                    .child(self.render_history(window, cx))
                                    .with_animation(
                                        ("history-in", seq),
                                        gpui_kit::Animation::new(std::time::Duration::from_millis(
                                            380,
                                        )),
                                        |el, t| {
                                            let k = 1. - (1. - t).powi(3);
                                            el.opacity(k).mt(gpui_kit::px(12. * (1. - k)))
                                        },
                                    )
                                    .into_any_element()
                            } else {
                                self.render_history(window, cx).into_any_element()
                            },
                        ))
                        .child(
                            resizable_panel()
                                .size(rem * if side { 36. } else { 22. })
                                .size_range(rem * if side { 20. } else { 8. }..rem * 400.)
                                .child(self.render_commit_details(cx)),
                        )
                        .into_any_element()
                }
                Mode::Changes => self.render_changes(window, cx).into_any_element(),
                Mode::Stats | Mode::Stack | Mode::Pr | Mode::File => div()
                    .size_full()
                    .track_focus(&self.root_focus)
                    .child(match self.mode {
                        Mode::Stats => self.render_stats(cx),
                        Mode::Stack => self.render_stack(cx),
                        Mode::File => self.render_file_page(cx),
                        _ => self.render_pr_page(cx),
                    })
                    .into_any_element(),
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
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.fetch_now(cx)))
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
            .on_action(cx.listener(|this, _: &ToggleCompact, _, cx| {
                this.set_mode(Mode::History, cx);
                this.toggle_compact(cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleDetailsSide, _, cx| {
                this.set_mode(Mode::History, cx);
                this.toggle_details_side(cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &SearchCommits, window, cx| {
                // In Changes, ⌘F filters the files instead.
                if this.mode == Mode::Changes {
                    gpui_kit::Focusable::focus_handle(&this.change_filter, cx).focus(window, cx);
                    return;
                }
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
            .on_action(cx.listener(|this, _: &CopyGithubLink, _, cx| this.copy_github_link(cx)))
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.set_mode(Mode::History, cx)))
            .on_action(cx.listener(|this, _: &ShowChanges, _, cx| this.set_mode(Mode::Changes, cx)))
            .on_action(cx.listener(|this, _: &ShowStats, _, cx| this.set_mode(Mode::Stats, cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.navigate(true, cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.navigate(false, cx)))
            .on_mouse_down(
                gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.navigate(true, cx)),
            )
            .on_mouse_down(
                gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Forward),
                cx.listener(|this, _, _, cx| this.navigate(false, cx)),
            )
            .child(self.render_toolbar(cx))
            .children(self.render_rebase_popover(cx))
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
