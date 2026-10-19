//! Interactive rebase driven by a plan instead of an editor: the todo list is
//! written by Mugetsu and handed to `git rebase -i` through
//! `GIT_SEQUENCE_EDITOR`, new messages are applied with `exec git commit --amend`.

use std::ffi::OsStr;
use std::path::PathBuf;

use anyhow::{Context as _, bail};

use crate::{LocalChanges, Repo};

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

    /// Commits on HEAD that `base` lacks, oldest first: the commits after
    /// `base` when it is an ancestor of HEAD, else the ones since their fork
    /// point, replayed onto `base`. Fails when the range holds merge commits.
    pub fn rebase_plan(&self, base: &str) -> anyhow::Result<Vec<RebaseCommit>> {
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
    /// Uncommitted changes are handled as in [`Repo::rebase`].
    /// `Ok(false)`: stopped on conflicts, finished with
    /// [`Repo::continue_operation`] like any other rebase.
    pub fn rebase_interactive(
        &self,
        base: &str,
        steps: &[RebaseStep],
        changes: LocalChanges,
    ) -> anyhow::Result<bool> {
        let commits = self.rebase_plan(base)?;
        self.discard_for(changes)?;
        let (mut lines, messages) = todo(&commits, steps)?;
        let dir = self.git_dir().map(|d| d.join("mugetsu-rebase"))?;
        // Messages of an earlier run stay until now: its `exec` lines read
        // them when a stopped rebase is continued.
        let _ = std::fs::remove_dir_all(&dir);
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
                if changes == LocalChanges::Stash {
                    "--autostash"
                } else {
                    "--no-autostash"
                },
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
                self.check_autostash(true)
            }
            Err(_) if !self.conflicted_paths()?.is_empty() => Ok(false),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
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
