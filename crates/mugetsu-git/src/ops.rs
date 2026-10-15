//! Write and network operations. All go through the git CLI.

use crate::{DiffFile, Repo};

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

    /// Checks out a remote branch as a new local tracking branch.
    pub fn checkout_remote(&self, remote_branch: &str) -> anyhow::Result<()> {
        let local = remote_branch
            .split_once('/')
            .map_or(remote_branch, |(_, b)| b);
        self.git()
            .run(["switch", "--track", "-c", local, remote_branch])
            .map(drop)
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
}
