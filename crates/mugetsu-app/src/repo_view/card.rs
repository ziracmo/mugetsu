//! What the tab tells the workspace overview, and how a card click lands here.

use gpui_kit::Context;
use mugetsu_forge::{CheckState, PrState, ReviewDecision};

use super::{Mode, RepoView, SidebarTab, pr_panel};
use crate::overview::{PrLine, PrMark, RepoCard, Target};

impl RepoView {
    /// This repository's card in the overview, from what the tab holds.
    pub fn card(&self) -> RepoCard {
        let data = &self.data;
        let refs = &data.refs;
        let head = refs.branches.iter().find(|b| b.is_head);
        let me = self.pr_me.as_deref();
        let open = || self.pr_list.iter().filter(|p| p.state == PrState::Open);
        let mine: Vec<_> = open()
            .filter(|p| me.is_some_and(|me| p.author == me))
            .collect();
        let status = |p: &&&mugetsu_forge::PullRequest| p.status.clone().unwrap_or_default();
        let line = |p: &mugetsu_forge::PullRequest, mark| PrLine {
            number: p.number,
            title: p.title.clone(),
            mark,
        };
        let mut my_lines: Vec<PrLine> = self
            .pr_list
            .iter()
            .filter(|p| matches!(p.state, PrState::Open | PrState::Draft))
            .filter(|p| me.is_some_and(|me| p.author == me))
            .map(|p| {
                let s = p.status.clone().unwrap_or_default();
                let mark = if p.state == PrState::Draft {
                    PrMark::Draft
                } else if s.checks == Some(CheckState::Failing) {
                    PrMark::Failing
                } else if s.review == Some(ReviewDecision::ChangesRequested) {
                    PrMark::ChangesRequested
                } else if s.checks == Some(CheckState::Pending) {
                    PrMark::Pending
                } else if s.review == Some(ReviewDecision::Approved) {
                    PrMark::Ready
                } else {
                    PrMark::Open
                };
                line(p, mark)
            })
            .collect();
        my_lines.sort_by_key(|l| l.mark);
        RepoCard {
            root: self.repo.root().to_path_buf(),
            name: self.name(),
            branch: self.head_label(),
            dirty: data.uncommitted(),
            conflicts: data.conflicted().count(),
            operation: data.operation.as_ref().map(|o| o.op.name()),
            ahead: head.map_or(0, |b| b.ahead),
            behind: head.map_or(0, |b| b.behind),
            gone: refs.branches.iter().filter(|b| b.upstream_gone).count(),
            stashes: refs.stashes.len(),
            dead_worktrees: self.dead_worktrees.len(),
            updates: self.has_updates,
            reviews: open().filter(|p| pr_panel::awaits(p, me)).count(),
            failing: mine
                .iter()
                .filter(|p| status(p).checks == Some(CheckState::Failing))
                .count(),
            changes_requested: mine
                .iter()
                .filter(|p| status(p).review == Some(ReviewDecision::ChangesRequested))
                .count(),
            ready: mine
                .iter()
                .filter(|p| {
                    let s = status(p);
                    s.review == Some(ReviewDecision::Approved)
                        && s.checks != Some(CheckState::Failing)
                        && s.checks != Some(CheckState::Pending)
                })
                .count(),
            my_prs: mine.len(),
            pulse: self.pulse_days(),
            mine: my_lines,
            to_review: open()
                .filter(|p| pr_panel::awaits(p, me))
                .map(|p| line(p, PrMark::Open))
                .collect(),
        }
    }

    /// Opens the part of the tab a signal is about.
    pub fn show_target(&mut self, target: Target, cx: &mut Context<Self>) {
        match target {
            Target::History => self.set_mode(Mode::History, cx),
            Target::Changes => self.set_mode(Mode::Changes, cx),
            Target::Prs => {
                self.sidebar_tab = SidebarTab::Prs;
                self.sidebar_collapsed = false;
                self.set_mode(Mode::History, cx);
            }
        }
    }
}

impl RepoView {
    /// What the weekly recap reads from this tab.
    pub fn recap_source(&self) -> crate::recap::Source {
        let me = self.pr_me.as_deref();
        let merged = self
            .pr_list
            .iter()
            .filter(|p| p.state == PrState::Merged)
            .filter_map(|p| {
                let at = chrono::DateTime::parse_from_rfc3339(p.merged_at.as_deref()?).ok()?;
                Some(crate::recap::MergedPr {
                    at: at.timestamp(),
                    mine: me.is_some_and(|me| p.author == me),
                })
            })
            .collect();
        crate::recap::Source {
            name: self.name(),
            repo: self.repo.clone(),
            merged,
            login: self.pr_me.clone(),
        }
    }
}
