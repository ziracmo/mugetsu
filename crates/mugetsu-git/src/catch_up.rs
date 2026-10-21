//! Catching branches up with their target (the base of their PR, else the
//! trunk). Each branch is first replayed onto its target in memory (gix tree
//! merges, no checkout, nothing written), so only those that would rebase
//! without a conflict are offered. The rebase itself replays the same way,
//! moves the refs, then force-pushes (with lease) the branches that were
//! pushed: a refused push puts that branch back where it was.
//!
//! A target that is itself a branch of the list (stacked PRs) is caught up
//! first, and the branch follows it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use gix::ObjectId;
use gix::merge::tree::TreatAsUnresolved;

use crate::{Git, Repo};

/// A branch to bring onto `target`: a remote-tracking ref (`origin/main`),
/// or a local branch caught up in the same pass (a stack).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpRequest {
    pub branch: String,
    pub target: String,
}

/// Whether a branch can be caught up, and why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Replays onto its target without a conflict.
    Clean,
    /// Its commit `commit` conflicts with the target on `paths`.
    Conflict {
        commit: String,
        summary: String,
        paths: Vec<String>,
    },
    /// Checked out in `worktree` with uncommitted changes.
    Dirty { worktree: PathBuf },
    /// Its remote branch has commits it lacks: pull first.
    RemoteAhead { upstream: String },
    /// It has merge commits, which are not replayed.
    Merges,
    /// It rests on a branch of the list that cannot be caught up.
    ParentBlocked { parent: String },
}

impl Verdict {
    pub fn is_clean(&self) -> bool {
        matches!(self, Verdict::Clean)
    }
}

/// Where a branch is pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    pub remote: String,
    /// Branch name on the remote.
    pub branch: String,
    /// Its remote-tracking tip, the lease of the force-push.
    pub tip: ObjectId,
}

/// A branch behind its target, and whether it can be caught up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUp {
    pub branch: String,
    pub target: String,
    pub verdict: Verdict,
    /// Commits of the target it does not have (its parent's, in a stack).
    pub behind: usize,
    /// Its own commits, replayed.
    pub commits: usize,
    pub upstream: Option<Upstream>,
    /// Its tip when scanned: the rebase refuses a branch that moved since.
    pub tip: ObjectId,
    /// Worktree it is checked out in, if any.
    pub worktree: Option<PathBuf>,
}

/// A branch the rebase moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub branch: String,
    pub old: ObjectId,
    pub new: ObjectId,
    pub upstream: Option<Upstream>,
    pub worktree: Option<PathBuf>,
}

/// Local branch facts read in one `for-each-ref`.
struct Local {
    tip: ObjectId,
    upstream: Option<Upstream>,
}

impl Repo {
    /// Every requested branch that lags behind its target, stacks parent
    /// first, each with whether it would rebase cleanly. Reads only: the
    /// replayed commits stay in memory.
    pub fn catch_up_scan(&self, requests: &[CatchUpRequest]) -> anyhow::Result<Vec<CatchUp>> {
        let locals = self.locals()?;
        let worktrees = self.checkouts()?;
        let repo = self.gix()?.with_object_memory();
        let mut out: Vec<CatchUp> = Vec::new();
        // Branch → (old tip, new tip) once replayed, for the branches resting on it.
        let mut replayed: HashMap<String, (ObjectId, ObjectId)> = HashMap::new();
        for req in topo_order(requests) {
            let Some(local) = locals.get(&req.branch) else {
                continue;
            };
            let parent_item = out.iter().find(|c| c.branch == req.target);
            let parent = replayed.get(&req.target).copied();
            let (onto, base) = match parent {
                Some((old, new)) => (new, old),
                None => match self.rev(&req.target) {
                    Ok(t) => {
                        let t = ObjectId::from_hex(t.as_bytes())?;
                        (t, t)
                    }
                    Err(_) => continue,
                },
            };
            let worktree = worktrees.get(&req.branch).cloned();
            let mut item = CatchUp {
                branch: req.branch.clone(),
                target: req.target.clone(),
                verdict: Verdict::Clean,
                behind: 0,
                commits: 0,
                upstream: local.upstream.clone(),
                tip: local.tip,
                worktree: worktree.clone(),
            };
            if let Some(p) = parent_item.filter(|p| !p.verdict.is_clean()) {
                item.verdict = Verdict::ParentBlocked {
                    parent: p.branch.clone(),
                };
                item.behind = p.behind;
                out.push(item);
                continue;
            }
            if parent.is_none() && self.is_ancestor(&onto.to_string(), &local.tip.to_string()) {
                continue;
            }
            item.behind = match parent_item {
                Some(p) => p.behind,
                None => self.count(&format!("{}..{onto}", local.tip))?,
            };
            let commits = self.own_commits(base, local.tip)?;
            // Merged already: nothing of its own to replay.
            if commits.is_empty() && parent.is_none() {
                continue;
            }
            item.commits = commits.len();
            item.verdict = if commits.iter().any(|(_, merge)| *merge) {
                Verdict::Merges
            } else if let Some(u) = &local.upstream
                && !self.is_ancestor(&u.tip.to_string(), &local.tip.to_string())
            {
                Verdict::RemoteAhead {
                    upstream: format!("{}/{}", u.remote, u.branch),
                }
            } else if let Some(wt) = worktree.as_deref().filter(|wt| is_dirty(wt)) {
                Verdict::Dirty {
                    worktree: wt.to_path_buf(),
                }
            } else {
                let ids: Vec<ObjectId> = commits.iter().map(|(id, _)| *id).collect();
                match replay(&repo, onto, &ids)? {
                    Ok(new) => {
                        replayed.insert(req.branch.clone(), (local.tip, new));
                        Verdict::Clean
                    }
                    Err(conflict) => conflict,
                }
            };
            out.push(item);
        }
        Ok(out)
    }

    /// Rebases one scanned branch onto its target, or onto its parent moved
    /// earlier in `moved`. `Ok(None)`: nothing left to do. Fails, moving
    /// nothing, on a conflict or when the branch changed since the scan.
    pub fn catch_up_one(&self, item: &CatchUp, moved: &[Moved]) -> anyhow::Result<Option<Moved>> {
        let b = &item.branch;
        let tip = ObjectId::from_hex(self.rev(&format!("refs/heads/{b}"))?.as_bytes())?;
        if tip != item.tip {
            bail!("'{b}' moved since it was checked: fetch and try again");
        }
        let (onto, base) = match moved.iter().find(|m| m.branch == item.target) {
            Some(p) => (p.new, p.old),
            None => {
                let t = ObjectId::from_hex(self.rev(&item.target)?.as_bytes())?;
                if self.is_ancestor(&t.to_string(), &tip.to_string()) {
                    return Ok(None);
                }
                (t, t)
            }
        };
        if let Some(wt) = item.worktree.as_deref().filter(|wt| is_dirty(wt)) {
            bail!(
                "'{b}' has uncommitted changes in {}: commit or stash them first",
                wt.display()
            );
        }
        let commits = self.own_commits(base, tip)?;
        if commits.is_empty() && base == onto {
            return Ok(None);
        }
        if commits.iter().any(|(_, merge)| *merge) {
            bail!("'{b}' has merge commits");
        }
        let ids: Vec<ObjectId> = commits.iter().map(|(id, _)| *id).collect();
        let new = match replay(&self.gix()?, onto, &ids)? {
            Ok(new) => new,
            Err(Verdict::Conflict {
                commit,
                summary,
                paths,
            }) => bail!(
                "'{b}' conflicts with {}: commit {commit} ({summary}) on {}",
                item.target,
                paths.join(", ")
            ),
            Err(other) => bail!("'{b}' cannot be rebased: {other:?}"),
        };
        self.set_branch(b, item.worktree.as_deref(), new, tip, &item.target)?;
        Ok(Some(Moved {
            branch: b.clone(),
            old: tip,
            new,
            upstream: item.upstream.clone(),
            worktree: item.worktree.clone(),
        }))
    }

    /// Force-pushes (with lease) the moved branches that have a remote
    /// branch, one result per such branch. A branch whose push is refused
    /// goes back to its old tip.
    pub fn catch_up_push(&self, moved: &[Moved]) -> Vec<(String, anyhow::Result<()>)> {
        let mut remotes: Vec<&str> = moved
            .iter()
            .filter_map(|m| Some(m.upstream.as_ref()?.remote.as_str()))
            .collect();
        remotes.sort();
        remotes.dedup();
        let mut results = Vec::new();
        for remote in remotes {
            let group: Vec<(&Moved, &Upstream)> = moved
                .iter()
                .filter_map(|m| Some((m, m.upstream.as_ref().filter(|u| u.remote == remote)?)))
                .collect();
            let mut args = vec!["push".to_string(), "--porcelain".into()];
            for (_, u) in &group {
                args.push(format!(
                    "--force-with-lease=refs/heads/{}:{}",
                    u.branch, u.tip
                ));
            }
            args.push(remote.to_string());
            for (m, u) in &group {
                args.push(format!("refs/heads/{}:refs/heads/{}", m.branch, u.branch));
            }
            let (stdout, stderr) = match self.git().run_full(&args) {
                Ok((_, out, err)) => (out, err),
                Err(e) => (String::new(), format!("{e:#}")),
            };
            for (m, u) in group {
                let result = push_result(&stdout, &stderr, &u.branch);
                if result.is_err()
                    && let Err(undo) =
                        self.set_branch(&m.branch, m.worktree.as_deref(), m.old, m.new, "")
                {
                    let e = result
                        .unwrap_err()
                        .context(format!("undo failed: {undo:#}"));
                    results.push((m.branch.clone(), Err(e)));
                    continue;
                }
                results.push((m.branch.clone(), result));
            }
        }
        results
    }

    fn gix(&self) -> anyhow::Result<gix::Repository> {
        let mut repo = gix::open(self.root()).context("opening repository with gix")?;
        repo.object_cache_size_if_unset(32 * 1024 * 1024);
        Ok(repo)
    }

    fn count(&self, range: &str) -> anyhow::Result<usize> {
        Ok(self
            .git()
            .run(["rev-list", "--count", range])?
            .trim()
            .parse()
            .unwrap_or(0))
    }

    /// Commits of `tip` not in `base`, oldest first, each with whether it is a merge.
    fn own_commits(&self, base: ObjectId, tip: ObjectId) -> anyhow::Result<Vec<(ObjectId, bool)>> {
        let out = self.git().run([
            "rev-list",
            "--reverse",
            "--topo-order",
            "--parents",
            &format!("{base}..{tip}"),
        ])?;
        out.lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut ids = l.split(' ');
                let id = ObjectId::from_hex(ids.next().unwrap_or_default().as_bytes())?;
                Ok((id, ids.count() > 1))
            })
            .collect()
    }

    /// Moves `branch` from `old` to `new`: its worktree follows when it is
    /// checked out (`reset --keep`), else only the ref moves.
    fn set_branch(
        &self,
        branch: &str,
        worktree: Option<&Path>,
        new: ObjectId,
        old: ObjectId,
        onto: &str,
    ) -> anyhow::Result<()> {
        let reason = if onto.is_empty() {
            "mugetsu: catch-up undone".to_string()
        } else {
            format!("mugetsu: rebase onto {onto}")
        };
        match worktree {
            Some(wt) => {
                let git = Git::new(wt);
                let head = git.run(["rev-parse", "HEAD"])?;
                if ObjectId::from_hex(head.trim().as_bytes())? != old {
                    bail!("'{branch}' moved in {}", wt.display());
                }
                git.run_with_env(
                    ["reset", "--quiet", "--keep", &new.to_string()],
                    &[("GIT_REFLOG_ACTION", reason.as_ref())],
                )
                .map(drop)
            }
            None => self
                .git()
                .run([
                    "update-ref",
                    "-m",
                    &reason,
                    &format!("refs/heads/{branch}"),
                    &new.to_string(),
                    &old.to_string(),
                ])
                .map(drop),
        }
    }

    fn locals(&self) -> anyhow::Result<HashMap<String, Local>> {
        let out = self.git().run([
            "for-each-ref",
            "--format=%(refname:short)%00%(objectname)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
            "refs/heads",
        ])?;
        let mut map = HashMap::new();
        for line in out.lines() {
            let f: Vec<&str> = line.split('\0').collect();
            let [name, tip, remote, remote_ref, tracking] = f[..] else {
                continue;
            };
            let upstream = (!remote.is_empty() && remote != ".")
                .then(|| {
                    let tip = self.rev(tracking).ok()?;
                    Some(Upstream {
                        remote: remote.to_string(),
                        branch: remote_ref.strip_prefix("refs/heads/")?.to_string(),
                        tip: ObjectId::from_hex(tip.as_bytes()).ok()?,
                    })
                })
                .flatten();
            map.insert(
                name.to_string(),
                Local {
                    tip: ObjectId::from_hex(tip.as_bytes())?,
                    upstream,
                },
            );
        }
        Ok(map)
    }

    /// Branch → worktree it is checked out in.
    fn checkouts(&self) -> anyhow::Result<HashMap<String, PathBuf>> {
        let out = self.git().run(["worktree", "list", "--porcelain"])?;
        let mut map = HashMap::new();
        let mut path = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/")
                && let Some(p) = path.clone()
            {
                map.insert(b.to_string(), p);
            }
        }
        Ok(map)
    }
}

/// Tracked changes in a worktree (untracked files don't block `reset --keep`
/// unless the rebase writes over them, which it then refuses).
fn is_dirty(worktree: &Path) -> bool {
    Git::new(worktree)
        .run(["status", "--porcelain", "--untracked-files=no"])
        .map_or(true, |s| !s.trim().is_empty())
}

/// Requests ordered so a branch comes after the branch it rests on.
fn topo_order(requests: &[CatchUpRequest]) -> Vec<&CatchUpRequest> {
    fn visit<'a>(
        i: usize,
        requests: &'a [CatchUpRequest],
        state: &mut [u8],
        out: &mut Vec<&'a CatchUpRequest>,
    ) {
        if state[i] != 0 {
            return;
        }
        state[i] = 1;
        if let Some(p) = requests.iter().position(|r| r.branch == requests[i].target) {
            visit(p, requests, state, out);
        }
        state[i] = 2;
        out.push(&requests[i]);
    }
    let mut state = vec![0u8; requests.len()];
    let mut out = Vec::with_capacity(requests.len());
    for i in 0..requests.len() {
        visit(i, requests, &mut state, &mut out);
    }
    out
}

/// Replays `commits` (oldest first, no merges) onto `onto` like a rebase:
/// a commit whose change is already there is dropped. Returns the new tip,
/// or the first conflict.
fn replay(
    repo: &gix::Repository,
    onto: ObjectId,
    commits: &[ObjectId],
) -> anyhow::Result<Result<ObjectId, Verdict>> {
    let committer = repo
        .committer()
        .context("set user.name and user.email first")??
        .to_owned()?;
    let options = repo.tree_merge_options()?;
    let mut tip = onto;
    for &id in commits {
        let commit = repo.find_commit(id)?;
        let theirs = commit.tree_id()?.detach();
        let base = match commit.parent_ids().next() {
            Some(p) => repo.find_commit(p)?.tree_id()?.detach(),
            None => ObjectId::empty_tree(repo.object_hash()),
        };
        let ours = repo.find_commit(tip)?.tree_id()?.detach();
        let mut merged =
            repo.merge_trees(base, ours, theirs, Default::default(), options.clone())?;
        if merged.has_unresolved_conflicts(TreatAsUnresolved::git()) {
            let mut paths: Vec<String> = merged
                .conflicts
                .iter()
                .filter(|c| c.is_unresolved(TreatAsUnresolved::git()))
                .map(|c| c.ours.location().to_string())
                .collect();
            paths.dedup();
            let message = commit.message_raw_sloppy().to_string();
            return Ok(Err(Verdict::Conflict {
                commit: id.to_hex_with_len(7).to_string(),
                summary: message.lines().next().unwrap_or_default().to_string(),
                paths,
            }));
        }
        let tree = merged.tree.write()?.detach();
        if tree == ours {
            continue;
        }
        let mut new = commit.decode()?.into_owned()?;
        new.tree = tree;
        new.parents = [tip].into_iter().collect();
        new.committer = committer.clone();
        new.extra_headers
            .retain(|(k, _)| k != "gpgsig" && k != "gpgsig-sha256");
        tip = repo.write_object(&new)?.detach();
    }
    Ok(Ok(tip))
}

/// The outcome of one ref in `git push --porcelain` output.
fn push_result(stdout: &str, stderr: &str, remote_branch: &str) -> anyhow::Result<()> {
    let dst = format!("refs/heads/{remote_branch}");
    let line = stdout.lines().find(|l| {
        l.split('\t')
            .nth(1)
            .and_then(|r| r.split(':').nth(1))
            .is_some_and(|d| d == dst)
    });
    match line {
        Some(l) if !l.starts_with('!') => Ok(()),
        Some(l) => bail!(
            "push of '{remote_branch}' refused: {}",
            l.split('\t').nth(2).unwrap_or("rejected").trim()
        ),
        None if stderr.trim().is_empty() => bail!("push of '{remote_branch}' failed"),
        None => bail!("push of '{remote_branch}' failed: {}", stderr.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(branch: &str, target: &str) -> CatchUpRequest {
        CatchUpRequest {
            branch: branch.into(),
            target: target.into(),
        }
    }

    #[test]
    fn stacks_come_parent_first() {
        let reqs = [
            req("c", "b"),
            req("x", "origin/main"),
            req("b", "a"),
            req("a", "origin/main"),
        ];
        let order: Vec<&str> = topo_order(&reqs)
            .iter()
            .map(|r| r.branch.as_str())
            .collect();
        assert_eq!(order, ["a", "b", "c", "x"]);
    }

    #[test]
    fn a_cycle_does_not_loop() {
        let reqs = [req("a", "b"), req("b", "a")];
        assert_eq!(topo_order(&reqs).len(), 2);
    }

    #[test]
    fn push_results_are_read_per_ref() {
        let out = "To ../remote\n\
                   +\trefs/heads/a:refs/heads/a\tabc...def (forced update)\n\
                   !\trefs/heads/b:refs/heads/b\t[rejected] (stale info)\n\
                   Done\n";
        assert!(push_result(out, "", "a").is_ok());
        let e = push_result(out, "", "b").unwrap_err().to_string();
        assert!(e.contains("stale info"), "{e}");
        let e = push_result("", "fatal: could not read", "c")
            .unwrap_err()
            .to_string();
        assert!(e.contains("could not read"), "{e}");
    }
}
