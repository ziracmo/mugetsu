//! Interactive rebase driven by a plan instead of an editor: the todo list is
//! written by Mugetsu and handed to `git rebase -i` through
//! `GIT_SEQUENCE_EDITOR`, new messages are applied with `exec git commit --amend`.

use std::ffi::OsStr;
use std::path::PathBuf;

use anyhow::{Context as _, bail};

use crate::Repo;

/// What happens to one commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RebaseAction {
    Pick,
    /// Keep the changes, change the message.
    Reword,
    /// Meld into the previous commit, messages joined.
    Squash,
    /// Meld into the previous commit, its message kept.
    Fixup,
    Drop,
}

impl RebaseAction {
    pub const ALL: [RebaseAction; 5] = [
        RebaseAction::Pick,
        RebaseAction::Reword,
        RebaseAction::Squash,
        RebaseAction::Fixup,
        RebaseAction::Drop,
    ];

    /// Melds into the previous commit.
    pub fn melds(self) -> bool {
        matches!(self, RebaseAction::Squash | RebaseAction::Fixup)
    }
}

/// A commit of the range being rebased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseCommit {
    pub id: String,
    pub author: String,
    /// Full message (subject and body).
    pub message: String,
}

impl RebaseCommit {
    pub fn short_id(&self) -> &str {
        &self.id[..self.id.len().min(7)]
    }

    pub fn summary(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

/// One line of the plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseStep {
    pub id: String,
    pub action: RebaseAction,
    /// New message for a reword.
    pub message: Option<String>,
}

/// How a rebase run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    Done,
    /// Stopped on a conflict: see [`Repo::rebase_progress`].
    Stopped,
}

/// A rebase in progress (`.git/rebase-merge` or `.git/rebase-apply`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RebaseProgress {
    /// Branch being rebased (without `refs/heads/`), `None` when detached.
    pub branch: Option<String>,
    /// Steps done so far, the current one included.
    pub step: usize,
    pub total: usize,
    /// Commit git stopped on, when it did.
    pub stopped_at: Option<String>,
}

/// A planned commit group: a kept commit and what melds into it.
struct Group<'a> {
    lines: Vec<String>,
    leader: &'a RebaseCommit,
    message: String,
}

/// Checks a plan (oldest first) and turns it into a todo list, with the
/// messages to write for the `exec` lines.
fn todo(
    commits: &[RebaseCommit],
    steps: &[RebaseStep],
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    if steps.len() != commits.len() {
        bail!("the plan doesn't cover the commits being rebased");
    }
    let mut groups: Vec<Group> = Vec::new();
    // Listed rather than left out, for `rebase.missingCommitsCheck`.
    let mut drops = Vec::new();
    for step in steps {
        let commit = commits
            .iter()
            .find(|c| c.id == step.id)
            .with_context(|| format!("unknown commit {}", step.id))?;
        match step.action {
            RebaseAction::Drop => drops.push(format!("drop {}", commit.id)),
            RebaseAction::Pick | RebaseAction::Reword => {
                let message = match (step.action, &step.message) {
                    (RebaseAction::Reword, Some(m)) if !m.trim().is_empty() => m.trim().to_string(),
                    _ => commit.message.trim().to_string(),
                };
                groups.push(Group {
                    lines: vec![format!("pick {}", commit.id)],
                    leader: commit,
                    message,
                });
            }
            RebaseAction::Squash | RebaseAction::Fixup => {
                let Some(group) = groups.last_mut() else {
                    bail!(
                        "{} can't be melded: there is no earlier commit to meld into",
                        commit.short_id()
                    );
                };
                group.lines.push(format!("fixup {}", commit.id));
                if step.action == RebaseAction::Squash {
                    group.message = format!("{}\n\n{}", group.message, commit.message.trim());
                }
            }
        }
    }
    let mut lines = Vec::new();
    let mut messages = Vec::new();
    for g in groups {
        lines.extend(g.lines);
        if g.message != g.leader.message.trim() {
            lines.push(format!("exec {{msg{}}}", messages.len()));
            messages.push(g.message);
        }
    }
    lines.extend(drops);
    Ok((lines, messages))
}

/// `'path'` for `sh`, with forward slashes (Git for Windows runs `sh` too).
fn sh_quote(path: &std::path::Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    format!("'{}'", p.replace('\'', r"'\''"))
}

impl Repo {
    /// `.git` of this worktree, where a rebase keeps its state.
    fn git_dir(&self) -> anyhow::Result<PathBuf> {
        let out = self.git().run(["rev-parse", "--absolute-git-dir"])?;
        Ok(PathBuf::from(out.trim()))
    }

    /// Commits after `base` on HEAD, oldest first. Fails when `base` is not
    /// an ancestor of HEAD or the range holds merge commits.
    pub fn rebase_plan(&self, base: &str) -> anyhow::Result<Vec<RebaseCommit>> {
        if self
            .git()
            .run(["merge-base", "--is-ancestor", base, "HEAD"])
            .is_err()
        {
            bail!("this commit is not an ancestor of HEAD");
        }
        let range = format!("{base}..HEAD");
        if !self
            .git()
            .run(["rev-list", "--merges", &range])?
            .trim()
            .is_empty()
        {
            bail!("the range holds merge commits, which an interactive rebase can't keep");
        }
        let out = self
            .git()
            .run(["log", "--reverse", "--format=%H%x1f%an%x1f%B%x1e", &range])?;
        Ok(out
            .split('\x1e')
            .filter_map(|rec| {
                let mut f = rec.trim_start_matches('\n').splitn(3, '\x1f');
                let id = f.next()?.trim();
                if id.is_empty() {
                    return None;
                }
                Some(RebaseCommit {
                    id: id.to_string(),
                    author: f.next()?.to_string(),
                    message: f.next()?.trim_end().to_string(),
                })
            })
            .collect())
    }

    /// Rebases the commits after `base` following `steps` (oldest first).
    /// Uncommitted changes are stashed and reapplied (`--autostash`).
    pub fn rebase_interactive(
        &self,
        base: &str,
        steps: &[RebaseStep],
    ) -> anyhow::Result<RebaseOutcome> {
        let commits = self.rebase_plan(base)?;
        let (mut lines, messages) = todo(&commits, steps)?;
        let dir = self.git_dir().map(|d| d.join("mugetsu-rebase"))?;
        std::fs::create_dir_all(&dir)?;
        for (i, message) in messages.iter().enumerate() {
            let file = dir.join(format!("msg{i}"));
            std::fs::write(&file, format!("{message}\n"))?;
            let exec = format!(
                "exec git commit --amend --allow-empty --no-verify --cleanup=strip -q -F {}",
                sh_quote(&file)
            );
            let placeholder = format!("exec {{msg{i}}}");
            for line in lines.iter_mut().filter(|l| **l == placeholder) {
                *line = exec.clone();
            }
        }
        let todo_file = dir.join("todo");
        std::fs::write(&todo_file, lines.join("\n") + "\n")?;
        let sequence_editor = format!("cp {}", sh_quote(&todo_file));
        let result = self.git().run_with_env(
            [
                "rebase",
                "-i",
                "--autostash",
                "--empty=drop",
                "--no-autosquash",
                base,
            ],
            &[
                ("GIT_SEQUENCE_EDITOR", OsStr::new(&sequence_editor)),
                ("GIT_EDITOR", OsStr::new("true")),
            ],
        );
        match result {
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                Ok(RebaseOutcome::Done)
            }
            Err(_) if self.rebase_progress().is_some() => Ok(RebaseOutcome::Stopped),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    /// The rebase in progress, if any.
    pub fn rebase_progress(&self) -> Option<RebaseProgress> {
        let read = |dir: &PathBuf, name: &str| {
            std::fs::read_to_string(dir.join(name))
                .ok()
                .map(|s| s.trim().to_string())
        };
        let num = |s: Option<String>| s.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
        let git_dir = self.git_dir().ok()?;
        for (name, step, total) in [
            ("rebase-merge", "msgnum", "end"),
            ("rebase-apply", "next", "last"),
        ] {
            let dir = git_dir.join(name);
            if !dir.is_dir() {
                continue;
            }
            let branch = read(&dir, "head-name")
                .filter(|h| h != "detached HEAD")
                .map(|h| h.trim_start_matches("refs/heads/").to_string());
            return Some(RebaseProgress {
                branch,
                step: num(read(&dir, step)),
                total: num(read(&dir, total)),
                stopped_at: read(&dir, "stopped-sha").filter(|s| !s.is_empty()),
            });
        }
        None
    }

    fn rebase_step(&self, flag: &str) -> anyhow::Result<RebaseOutcome> {
        let result = self
            .git()
            .run_with_env(["rebase", flag], &[("GIT_EDITOR", OsStr::new("true"))]);
        let finished = self.rebase_progress().is_none();
        if finished && let Ok(dir) = self.git_dir().map(|d| d.join("mugetsu-rebase")) {
            let _ = std::fs::remove_dir_all(dir);
        }
        match result {
            Ok(_) if finished => Ok(RebaseOutcome::Done),
            Ok(_) => Ok(RebaseOutcome::Stopped),
            // Stopped on the next conflict: that's progress, not a failure.
            Err(_) if !finished && flag != "--abort" && self.unmerged()? => {
                Ok(RebaseOutcome::Stopped)
            }
            Err(e) => Err(e),
        }
    }

    fn unmerged(&self) -> anyhow::Result<bool> {
        Ok(!self
            .git()
            .run(["diff", "--name-only", "--diff-filter=U"])?
            .trim()
            .is_empty())
    }

    /// `git rebase --continue` (messages are kept as they are).
    pub fn rebase_continue(&self) -> anyhow::Result<RebaseOutcome> {
        if self.unmerged()? {
            bail!("resolve and stage the conflicted files first");
        }
        self.rebase_step("--continue")
    }

    /// Skips the commit git stopped on.
    pub fn rebase_skip(&self) -> anyhow::Result<RebaseOutcome> {
        self.rebase_step("--skip")
    }

    /// Puts the branch back where it was before the rebase.
    pub fn rebase_abort(&self) -> anyhow::Result<()> {
        self.rebase_step("--abort").map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, message: &str) -> RebaseCommit {
        RebaseCommit {
            id: id.into(),
            author: "A".into(),
            message: message.into(),
        }
    }

    fn step(id: &str, action: RebaseAction) -> RebaseStep {
        RebaseStep {
            id: id.into(),
            action,
            message: None,
        }
    }

    #[test]
    fn todo_groups_melded_commits_and_rewrites_messages() {
        let commits = [commit("a", "A"), commit("b", "B"), commit("c", "C")];
        let (lines, messages) = todo(
            &commits,
            &[
                step("c", RebaseAction::Pick),
                step("a", RebaseAction::Pick),
                step("b", RebaseAction::Squash),
            ],
        )
        .unwrap();
        assert_eq!(lines, ["pick c", "pick a", "fixup b", "exec {msg0}"]);
        assert_eq!(messages, ["A\n\nB"]);

        let (lines, messages) = todo(
            &commits,
            &[
                RebaseStep {
                    message: Some("A2".into()),
                    ..step("a", RebaseAction::Reword)
                },
                step("b", RebaseAction::Fixup),
                step("c", RebaseAction::Drop),
            ],
        )
        .unwrap();
        assert_eq!(lines, ["pick a", "fixup b", "exec {msg0}", "drop c"]);
        assert_eq!(messages, ["A2"]);
    }

    #[test]
    fn todo_refuses_melding_into_nothing() {
        let commits = [commit("a", "A"), commit("b", "B")];
        assert!(
            todo(
                &commits,
                &[
                    step("a", RebaseAction::Drop),
                    step("b", RebaseAction::Squash)
                ]
            )
            .is_err()
        );
        assert!(todo(&commits, &[step("a", RebaseAction::Pick)]).is_err());
    }
}
