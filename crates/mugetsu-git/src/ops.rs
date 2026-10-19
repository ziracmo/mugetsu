//! Write and network operations. All go through the git CLI.

use crate::{DiffFile, Repo};

/// What to do with uncommitted changes when switching branches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalChanges {
    /// Let git carry them over (fails if they conflict).
    Keep,
    /// Stash before, reapply after.
    Stash,
    /// Throw tracked changes away (untracked files stay).
    Discard,
}

/// The operation went through, but reapplying the stashed changes on top
/// conflicted: the conflicts are in the working tree, the changes are kept
/// in the stash.
#[derive(Debug)]
pub struct StashConflict;

impl std::fmt::Display for StashConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the stashed changes conflict: they are kept in the stash")
    }
}

impl std::error::Error for StashConflict {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PushOptions {
    pub set_upstream: bool,
    pub force_with_lease: bool,
    pub tags: bool,
}

/// A `.gitignore` line matching exactly `path` (relative to the root), with
/// the characters git would read as a pattern escaped.
pub fn ignore_literal(path: &str) -> String {
    let mut out = String::from("/");
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | '\\' | '!' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    // Trailing spaces are dropped unless escaped.
    let trailing = out.len() - out.trim_end_matches(' ').len();
    if trailing > 0 {
        out.truncate(out.len() - trailing);
        out.push_str(&"\\ ".repeat(trailing));
    }
    out
}

impl Repo {
    pub fn stage(&self, paths: &[&str]) -> anyhow::Result<()> {
        let mut args = vec!["add", "--all", "--"];
        args.extend_from_slice(paths);
        self.git().run(args).map(drop)
    }

    pub fn unstage(&self, paths: &[&str]) -> anyhow::Result<()> {
        let mut args = vec!["restore", "--staged", "--"];
        args.extend_from_slice(paths);
        // On an unborn branch there is no HEAD to restore from.
        self.git().run(&args).or_else(|_| {
            let mut args = vec!["rm", "--cached", "-r", "-q", "--"];
            args.extend_from_slice(paths);
            self.git().run(args)
        })?;
        Ok(())
    }

    pub fn stage_all(&self) -> anyhow::Result<()> {
        self.git().run(["add", "--all"]).map(drop)
    }

    /// Discards worktree changes for tracked paths (irreversible).
    pub fn discard(&self, paths: &[&str]) -> anyhow::Result<()> {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend_from_slice(paths);
        self.git().run(args).map(drop)
    }

    /// Deletes untracked files (irreversible); ignored files are left alone.
    pub fn remove_untracked(&self, paths: &[&str]) -> anyhow::Result<()> {
        let mut args = vec!["clean", "-f", "-q", "--"];
        args.extend_from_slice(paths);
        self.git().run(args).map(drop)
    }

    /// Appends `pattern` to the repository's root `.gitignore`.
    pub fn ignore(&self, pattern: &str) -> anyhow::Result<()> {
        use std::io::Write as _;
        let path = self.root().join(".gitignore");
        let current = std::fs::read(&path).unwrap_or_default();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        if current.last().is_some_and(|b| *b != b'\n') {
            file.write_all(b"\n")?;
        }
        writeln!(file, "{pattern}")?;
        Ok(())
    }

    /// Stages (or unstages with `reverse`) a single hunk of `file`.
    pub fn apply_hunk(&self, file: &DiffFile, hunk: usize, reverse: bool) -> anyhow::Result<()> {
        let patch = file
            .hunk_patch(hunk)
            .ok_or_else(|| anyhow::anyhow!("no hunk {hunk}"))?;
        let mut args = vec!["apply", "--cached", "--whitespace=nowarn"];
        if reverse {
            args.push("--reverse");
        }
        args.push("-");
        self.git().run_with_stdin(args, patch.as_bytes()).map(drop)
    }

    pub fn commit(&self, message: &str, amend: bool) -> anyhow::Result<()> {
        let mut args = vec!["commit", "--cleanup=strip", "-F", "-"];
        if amend {
            args.push("--amend");
        }
        self.git()
            .run_with_stdin(args, message.as_bytes())
            .map(drop)
    }

    pub fn checkout(&self, branch: &str) -> anyhow::Result<()> {
        self.git().run(["switch", branch]).map(drop)
    }

    /// Checks out a remote branch: its local counterpart when one exists,
    /// otherwise a new local branch tracking it.
    pub fn checkout_remote(&self, remote_branch: &str) -> anyhow::Result<()> {
        let local = remote_branch
            .split_once('/')
            .map_or(remote_branch, |(_, b)| b);
        self.switch_to(local, Some(remote_branch), LocalChanges::Keep)
    }

    pub fn has_local_branch(&self, name: &str) -> bool {
        self.git()
            .run([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ])
            .is_ok()
    }

    /// Uncommitted changes, untracked files included.
    pub fn is_dirty(&self) -> anyhow::Result<bool> {
        Ok(!self.git().run(["status", "--porcelain"])?.trim().is_empty())
    }

    /// Switches to `branch` (created from `track` and tracking it when it
    /// doesn't exist yet), handling uncommitted changes.
    pub fn switch_to(
        &self,
        branch: &str,
        track: Option<&str>,
        changes: LocalChanges,
    ) -> anyhow::Result<()> {
        let stashed = changes == LocalChanges::Stash && self.is_dirty()?;
        if stashed {
            self.stash_push(Some(&format!("Mugetsu: checkout {branch}")))?;
        }
        if changes == LocalChanges::Discard {
            self.git().run(["reset", "--hard", "--quiet"])?;
        }
        let switched = match track {
            Some(upstream) if !self.has_local_branch(branch) => self
                .git()
                .run(["switch", "--track", "-c", branch, upstream]),
            _ => self.git().run(["switch", branch]),
        };
        if let Err(e) = switched {
            if stashed {
                let _ = self.git().run(["stash", "pop"]);
            }
            return Err(e);
        }
        if stashed {
            self.git().run(["stash", "pop"]).map_err(|e| {
                e.context(
                    "checked out, but the stashed changes conflict: they are kept in the stash",
                )
            })?;
        }
        Ok(())
    }

    /// Uncommitted changes to tracked files: what gets in the way of a rebase.
    pub fn has_tracked_changes(&self) -> anyhow::Result<bool> {
        Ok(!self
            .git()
            .run(["status", "--porcelain", "--untracked-files=no"])?
            .trim()
            .is_empty())
    }

    /// Throws tracked changes away for [`LocalChanges::Discard`].
    pub(crate) fn discard_for(&self, changes: LocalChanges) -> anyhow::Result<()> {
        if changes == LocalChanges::Discard {
            self.git().run(["reset", "--hard", "--quiet"])?;
        }
        Ok(())
    }

    /// After a rebase that stashed the changes and reapplied them
    /// (`--autostash`): conflicts left once it is done come from them.
    pub(crate) fn check_autostash(&self, done: bool) -> anyhow::Result<bool> {
        if done && !self.conflicted_paths()?.is_empty() {
            return Err(StashConflict.into());
        }
        Ok(done)
    }

    /// Moves `branch` onto `target` (e.g. its upstream). For the checked-out
    /// branch this is a hard reset: uncommitted changes are lost.
    pub fn reset_branch(&self, branch: &str, target: &str, is_head: bool) -> anyhow::Result<()> {
        if is_head {
            self.git().run(["reset", "--hard", "--quiet", target])
        } else {
            self.git().run(["branch", "--force", branch, target])
        }
        .map(drop)
    }

    /// `git push` with the push dialog's options; `remote_branch` may differ
    /// from the local name.
    pub fn push_to(
        &self,
        remote: &str,
        local: &str,
        remote_branch: &str,
        options: PushOptions,
    ) -> anyhow::Result<()> {
        let mut args = vec!["push".to_string(), "--quiet".to_string()];
        if options.set_upstream {
            args.push("-u".into());
        }
        if options.force_with_lease {
            args.push("--force-with-lease".into());
        }
        if options.tags {
            args.push("--tags".into());
        }
        args.push(remote.into());
        args.push(format!("refs/heads/{local}:refs/heads/{remote_branch}"));
        self.git().run(args).map(drop)
    }

    pub fn create_branch(
        &self,
        name: &str,
        start: Option<&str>,
        checkout: bool,
    ) -> anyhow::Result<()> {
        let mut args = if checkout {
            vec!["switch", "-c", name]
        } else {
            vec!["branch", name]
        };
        if let Some(s) = start {
            args.push(s);
        }
        self.git().run(args).map(drop)
    }

    pub fn delete_branch(&self, name: &str, force: bool) -> anyhow::Result<()> {
        self.git()
            .run(["branch", if force { "-D" } else { "-d" }, name])
            .map(drop)
    }

    pub fn create_tag(
        &self,
        name: &str,
        target: &str,
        message: Option<&str>,
    ) -> anyhow::Result<()> {
        match message {
            Some(m) => self.git().run(["tag", "-a", name, target, "-m", m]),
            None => self.git().run(["tag", name, target]),
        }
        .map(drop)
    }

    pub fn delete_tag(&self, name: &str) -> anyhow::Result<()> {
        self.git().run(["tag", "-d", name]).map(drop)
    }

    pub fn fetch_all(&self) -> anyhow::Result<()> {
        self.git()
            .run(["fetch", "--all", "--prune", "--quiet"])
            .map(drop)
    }

    pub fn pull(&self) -> anyhow::Result<()> {
        self.git().run(["pull", "--ff-only", "--quiet"]).map(drop)
    }

    pub fn push(&self, set_upstream: Option<(&str, &str)>) -> anyhow::Result<()> {
        match set_upstream {
            Some((remote, branch)) => self.git().run(["push", "--quiet", "-u", remote, branch]),
            None => self.git().run(["push", "--quiet"]),
        }
        .map(drop)
    }

    pub fn stash_push(&self, message: Option<&str>) -> anyhow::Result<()> {
        match message {
            Some(m) => self
                .git()
                .run(["stash", "push", "--include-untracked", "-m", m]),
            None => self.git().run(["stash", "push", "--include-untracked"]),
        }
        .map(drop)
    }

    /// Drops every stash.
    pub fn stash_clear(&self) -> anyhow::Result<()> {
        self.git().run(["stash", "clear"]).map(drop)
    }

    pub fn stash_pop(&self, index: usize) -> anyhow::Result<()> {
        self.git()
            .run(["stash", "pop", &format!("stash@{{{index}}}")])
            .map(drop)
    }

    pub fn stash_drop(&self, index: usize) -> anyhow::Result<()> {
        self.git()
            .run(["stash", "drop", &format!("stash@{{{index}}}")])
            .map(drop)
    }

    pub fn worktree_add(&self, path: &str, branch: &str, create: bool) -> anyhow::Result<()> {
        if create {
            self.git().run(["worktree", "add", "-b", branch, path])
        } else {
            self.git().run(["worktree", "add", path, branch])
        }
        .map(drop)
    }

    pub fn worktree_remove(&self, path: &str) -> anyhow::Result<()> {
        self.git().run(["worktree", "remove", path]).map(drop)
    }

    pub fn remote_add(&self, name: &str, url: &str) -> anyhow::Result<()> {
        self.git().run(["remote", "add", name, url]).map(drop)
    }

    pub fn remote_remove(&self, name: &str) -> anyhow::Result<()> {
        self.git().run(["remote", "remove", name]).map(drop)
    }

    pub fn rename_branch(&self, old: &str, new: &str) -> anyhow::Result<()> {
        self.git().run(["branch", "-m", old, new]).map(drop)
    }

    /// Fast-forwards `branch` to `upstream`, refusing anything that is not a fast-forward.
    pub fn fast_forward(&self, branch: &str, upstream: &str, is_head: bool) -> anyhow::Result<()> {
        if is_head {
            self.git().run(["merge", "--ff-only", "--quiet", upstream])
        } else {
            // `fetch .` updates a non-checked-out branch, fast-forward only.
            self.git()
                .run(["fetch", "--quiet", ".", &format!("{upstream}:{branch}")])
        }
        .map(drop)
    }

    pub fn push_branch(
        &self,
        remote: &str,
        branch: &str,
        set_upstream: bool,
    ) -> anyhow::Result<()> {
        let mut args = vec!["push", "--quiet"];
        if set_upstream {
            args.push("-u");
        }
        args.extend([remote, branch]);
        self.git().run(args).map(drop)
    }

    /// Deletes `remote/branch` on the server.
    pub fn delete_remote_branch(&self, remote: &str, branch: &str) -> anyhow::Result<()> {
        self.git()
            .run(["push", "--quiet", remote, "--delete", branch])
            .map(drop)
    }

    pub fn push_tag(&self, remote: &str, tag: &str) -> anyhow::Result<()> {
        self.git()
            .run(["push", "--quiet", remote, &format!("refs/tags/{tag}")])
            .map(drop)
    }

    /// `git reset --soft|--mixed|--hard <target>` on the current branch.
    pub fn reset(&self, target: &str, mode: ResetMode) -> anyhow::Result<()> {
        self.git().run(["reset", mode.flag(), target]).map(drop)
    }

    pub fn checkout_detached(&self, rev: &str) -> anyhow::Result<()> {
        self.git().run(["switch", "--detach", rev]).map(drop)
    }

    /// Merge commits are picked relative to their first parent.
    pub fn cherry_pick(&self, rev: &str, is_merge: bool) -> anyhow::Result<()> {
        let mut args = vec!["cherry-pick"];
        if is_merge {
            args.extend(["-m", "1"]);
        }
        args.push(rev);
        self.git().run(args).map(drop)
    }

    /// Merge commits are reverted relative to their first parent.
    pub fn revert(&self, rev: &str, is_merge: bool) -> anyhow::Result<()> {
        let mut args = vec!["revert", "--no-edit"];
        if is_merge {
            args.extend(["-m", "1"]);
        }
        args.push(rev);
        self.git().run(args).map(drop)
    }

    /// The commit as a mailbox patch (`git format-patch`).
    pub fn format_patch(&self, rev: &str) -> anyhow::Result<String> {
        self.git().run(["format-patch", "-1", "--stdout", rev])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}
