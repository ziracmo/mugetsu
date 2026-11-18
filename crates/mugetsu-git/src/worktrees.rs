//! Dead worktrees: linked worktrees left behind once their work is over
//! (folder gone, branch deleted, merged, or its remote branch deleted),
//! with the disk space removing them frees.

use std::path::Path;

use anyhow::Context as _;

use crate::{Git, LocalChanges, RefsSnapshot, Repo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadReason {
    /// The folder no longer exists (`git worktree prune` material).
    Missing,
    /// The checked-out branch was deleted.
    BranchGone,
    /// Its remote branch was deleted (typically a merged pull request).
    UpstreamGone,
    /// Everything it holds is already in the default branch.
    Merged,
}

/// Steps of [`Repo::checkout_from_worktree_with`], in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorktreeCheckoutStep {
    /// Stashing the worktree's uncommitted changes.
    Stash,
    /// Deleting the worktree's folder (the long one: `node_modules`…).
    Remove,
    /// Checking the branch out here.
    Checkout,
    /// Reapplying the worktree's changes here.
    Reapply,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadWorktree {
    pub path: String,
    pub branch: Option<String>,
    pub reason: DeadReason,
    /// Size of the folder on disk (0 when missing).
    pub bytes: u64,
    /// Uncommitted changes or untracked files that removing it would lose.
    pub dirty: bool,
}

impl Repo {
    /// The linked worktrees that look finished. Never the main worktree, the
    /// current one, or a locked one. Walks each candidate's folder for its
    /// size: call it off the UI thread.
    pub fn dead_worktrees(&self, refs: &RefsSnapshot) -> Vec<DeadWorktree> {
        let git = self.git();
        let mut base: Option<String> = None;
        let mut out = Vec::new();
        for w in &refs.worktrees {
            if w.is_main || w.locked || Path::new(&w.path) == self.root() {
                continue;
            }
            let path = Path::new(&w.path);
            let reason = if w.prunable || !path.exists() {
                Some(DeadReason::Missing)
            } else if let Some(name) = &w.branch {
                match refs.branches.iter().find(|b| &b.name == name) {
                    None => Some(DeadReason::BranchGone),
                    Some(b) if b.upstream_gone => Some(DeadReason::UpstreamGone),
                    Some(b) => {
                        let base = base.get_or_insert_with(|| self.default_branch());
                        merged(git, &b.target.to_string(), base, Some(name))
                            .then_some(DeadReason::Merged)
                    }
                }
            } else {
                let base = base.get_or_insert_with(|| self.default_branch());
                w.head
                    .is_some_and(|h| merged(git, &h.to_string(), base, None))
                    .then_some(DeadReason::Merged)
            };
            let Some(reason) = reason else { continue };
            let alive = reason != DeadReason::Missing;
            out.push(DeadWorktree {
                path: w.path.clone(),
                branch: w.branch.clone(),
                reason,
                bytes: if alive { dir_size(path) } else { 0 },
                dirty: alive
                    && Git::new(path)
                        .run(["status", "--porcelain", "--ignore-submodules"])
                        .is_ok_and(|s| !s.trim().is_empty()),
            });
        }
        out
    }

    /// Removes a worktree for good, uncommitted changes included; a missing
    /// folder is pruned from git's bookkeeping instead. The branch stays.
    pub fn worktree_purge(&self, path: &str) -> anyhow::Result<()> {
        if Path::new(path).exists() {
            self.git()
                .run(["worktree", "remove", "--force", path])
                .map(drop)
        } else {
            self.git()
                .run(["worktree", "prune", "--expire", "now"])
                .map(drop)
        }
    }

    /// Brings `branch`, checked out in the linked worktree at `path`, back
    /// into this one: the worktree's uncommitted changes (untracked files
    /// included) are stashed, the worktree is removed (its ignored files,
    /// `node_modules` and such, go with it), `branch` is checked out here
    /// (`changes` says what to do with this worktree's own changes), then the
    /// stash is reapplied on top. The branch, local and remote, stays.
    pub fn checkout_from_worktree(
        &self,
        path: &str,
        branch: &str,
        changes: LocalChanges,
    ) -> anyhow::Result<()> {
        self.checkout_from_worktree_with(path, branch, changes, &|_| {})
    }

    /// [`Self::checkout_from_worktree`], calling `step` as each step starts
    /// (steps with nothing to do are skipped).
    pub fn checkout_from_worktree_with(
        &self,
        path: &str,
        branch: &str,
        changes: LocalChanges,
        step: &dyn Fn(WorktreeCheckoutStep),
    ) -> anyhow::Result<()> {
        let wt = Git::new(Path::new(path));
        let dirty = Path::new(path).exists()
            && !wt
                .run(["status", "--porcelain", "--ignore-submodules"])?
                .trim()
                .is_empty();
        // The stash is shared by every worktree: what is stashed there can
        // be popped here. Remembered by id, other stashes may come on top.
        let stash = if dirty {
            step(WorktreeCheckoutStep::Stash);
            wt.run([
                "stash",
                "push",
                "--include-untracked",
                "-m",
                &format!("Mugetsu: worktree {branch}"),
            ])?;
            Some(
                self.git()
                    .run(["rev-parse", "refs/stash"])?
                    .trim()
                    .to_string(),
            )
        } else {
            None
        };
        step(WorktreeCheckoutStep::Remove);
        if let Err(e) = self.worktree_purge(path) {
            if stash.is_some() {
                let _ = wt.run(["stash", "pop", "--index"]);
            }
            return Err(e);
        }
        let kept = || {
            if stash.is_some() {
                "the worktree's changes are kept in the stash"
            } else {
                "the worktree is removed"
            }
        };
        step(WorktreeCheckoutStep::Checkout);
        self.switch_to(branch, None, changes).with_context(kept)?;
        if let Some(id) = stash {
            step(WorktreeCheckoutStep::Reapply);
            let list = self.git().run(["stash", "list", "--format=%H"])?;
            let index = list
                .lines()
                .position(|l| l.trim() == id)
                .context("the worktree's stash is gone")?;
            self.stash_pop(index).context(
                "checked out, but the worktree's changes conflict: they are kept in the stash",
            )?;
        }
        Ok(())
    }
}

/// `commit` is in `base`, and the branch did some work of its own: one
/// just created from `base` (a single reflog entry, or the very same commit
/// without a reflog) isn't finished, it hasn't started.
fn merged(git: &Git, commit: &str, base: &str, branch: Option<&str>) -> bool {
    let same_as_base = |b: &str| base == b || base.ends_with(&format!("/{b}"));
    if base == "HEAD" || branch.is_some_and(same_as_base) {
        return false;
    }
    if git
        .run(["merge-base", "--is-ancestor", commit, base])
        .is_err()
    {
        return false;
    }
    let reflog = branch.map_or(0, |b| {
        git.run(["reflog", "show", "--format=%H", &format!("refs/heads/{b}")])
            .map_or(0, |s| s.lines().count())
    });
    match reflog {
        0 => git
            .run(["rev-parse", "--verify", "-q", &format!("{base}^{{commit}}")])
            .is_ok_and(|t| t.trim() != commit),
        1 => false,
        _ => true,
    }
}

/// Bytes taken by every file under `path`, without following symlinks.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}
