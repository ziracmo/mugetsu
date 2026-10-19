//! Operations on several branches at once (multi-selection in the app).

use anyhow::bail;

use crate::{LocalChanges, Repo};

/// What a batch delete did: the branches deleted, and those git refused with
/// its reason (`unmerged` set when only `-D` would delete it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchDelete {
    pub deleted: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub unmerged: Vec<String>,
}

impl Repo {
    /// Deletes every branch, carrying on past refusals.
    pub fn delete_branches(&self, names: &[String], force: bool) -> BatchDelete {
        let mut out = BatchDelete::default();
        for name in names {
            match self.delete_branch(name, force) {
                Ok(()) => out.deleted.push(name.clone()),
                Err(e) => {
                    let msg = format!("{e:#}");
                    if !force && msg.contains("not fully merged") {
                        out.unmerged.push(name.clone());
                    }
                    out.failed.push((name.clone(), msg));
                }
            }
        }
        out
    }

    /// Rebases each branch onto `onto`, then puts HEAD back. All or nothing:
    /// on a conflict every branch goes back to where it was. Uncommitted
    /// changes are refused with [`LocalChanges::Keep`]; stashed ones are
    /// reapplied at the end ([`StashConflict`] if that conflicts).
    ///
    /// [`StashConflict`]: crate::StashConflict
    pub fn rebase_branches(
        &self,
        onto: &str,
        names: &[String],
        changes: LocalChanges,
    ) -> anyhow::Result<()> {
        let dirty = self.has_tracked_changes()?;
        if dirty && changes == LocalChanges::Keep {
            bail!("commit or stash your changes first");
        }
        self.discard_for(changes)?;
        let stashed = dirty && changes == LocalChanges::Stash;
        if stashed {
            self.stash_push(Some(&format!("Mugetsu: rebase onto {onto}")))?;
        }
        let result = self.rebase_each(onto, names);
        if stashed {
            let popped = self.git().run(["stash", "pop"]);
            if popped.is_err() && !self.conflicted_paths()?.is_empty() {
                return result.and(Err(crate::StashConflict.into()));
            }
            popped?;
        }
        result
    }

    fn rebase_each(&self, onto: &str, names: &[String]) -> anyhow::Result<()> {
        let onto_sha = self.rev(onto)?;
        let head = match self.git().run(["symbolic-ref", "--quiet", "HEAD"]) {
            Ok(r) => r.trim().to_string(),
            Err(_) => self.rev("HEAD")?,
        };
        let mut before = Vec::new();
        for name in names {
            before.push((name.clone(), self.rev(name)?));
        }
        let restore_head = || {
            match head.strip_prefix("refs/heads/") {
                Some(b) => self.git().run(["switch", "--quiet", b]),
                None => self.git().run(["switch", "--quiet", "--detach", &head]),
            }
            .map(drop)
        };
        for name in names {
            if let Err(e) = self.git().run(["rebase", "--quiet", &onto_sha, name]) {
                let _ = self.git().run(["rebase", "--abort"]);
                let _ = self.git().run(["switch", "--quiet", "--detach", "HEAD"]);
                for (b, sha) in &before {
                    let _ = self.git().run(["branch", "--force", b, sha]);
                }
                let _ = restore_head();
                return Err(e.context(format!(
                    "rebasing '{name}' failed, every branch is back where it was"
                )));
            }
        }
        restore_head()
    }
}
