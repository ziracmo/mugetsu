//! Restacking a stack of branches (stacked pull requests): each branch is
//! rebased onto its parent, bottom first, then all are force-pushed at once.
//!
//! All or nothing: a conflict or a refused push puts every branch and HEAD
//! back where they were.

use anyhow::{Context as _, bail};

use crate::Repo;

/// One branch of the stack and the branch it is based on (`None`: the trunk).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackLink {
    pub branch: String,
    pub parent: Option<String>,
}

/// A restack ready to run, with what is needed to undo it.
#[derive(Clone, Debug)]
pub struct Restack {
    remote: String,
    trunk: String,
    links: Vec<StackLink>,
    /// Local tip of each branch before the restack (after syncing with the remote).
    before: Vec<(String, String)>,
    /// Remote tip of each branch, the lease for the force-push.
    remote_tips: Vec<(String, String)>,
    /// `refs/heads/x` or a detached sha.
    head: String,
}

impl Restack {
    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    pub fn branch(&self, i: usize) -> &str {
        &self.links[i].branch
    }

    fn before(&self, branch: &str) -> &str {
        lookup(&self.before, branch)
    }
}

fn lookup<'a>(list: &'a [(String, String)], key: &str) -> &'a str {
    list.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .unwrap_or_default()
}

impl Repo {
    fn rev(&self, rev: &str) -> anyhow::Result<String> {
        Ok(self
            .git()
            .run([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ])
            .with_context(|| format!("{rev} not found"))?
            .trim()
            .to_string())
    }

    fn is_ancestor(&self, a: &str, b: &str) -> bool {
        self.git()
            .run(["merge-base", "--is-ancestor", a, b])
            .is_ok()
    }

    /// Fetches `remote`, then brings every local branch of the stack level
    /// with its remote counterpart (created when missing, fast-forwarded when
    /// behind). Refuses with uncommitted changes or a diverged branch.
    pub fn restack_prepare(
        &self,
        remote: &str,
        trunk: &str,
        links: Vec<StackLink>,
    ) -> anyhow::Result<Restack> {
        if self.is_dirty()? {
            bail!("commit or stash your changes first");
        }
        self.git().run(["fetch", "--quiet", "--prune", remote])?;
        let head = match self.git().run(["symbolic-ref", "--quiet", "HEAD"]) {
            Ok(r) => r.trim().to_string(),
            Err(_) => self.rev("HEAD")?,
        };
        self.rev(&format!("{remote}/{trunk}"))?;
        let mut before = Vec::new();
        let mut remote_tips = Vec::new();
        for link in &links {
            let b = &link.branch;
            let tracking = format!("{remote}/{b}");
            let theirs = self
                .rev(&tracking)
                .with_context(|| format!("'{b}' is not on '{remote}'"))?;
            if !self.has_local_branch(b) {
                self.git()
                    .run(["branch", "--quiet", "--track", b, &tracking])?;
            } else {
                let ours = self.rev(b)?;
                if ours != theirs && self.is_ancestor(&ours, &theirs) {
                    if head == format!("refs/heads/{b}") {
                        self.git()
                            .run(["merge", "--quiet", "--ff-only", &tracking])?;
                    } else {
                        self.git().run(["branch", "--force", b, &tracking])?;
                    }
                } else if ours != theirs && !self.is_ancestor(&theirs, &ours) {
                    bail!("'{b}' and '{tracking}' have diverged: pull or reset it first");
                }
            }
            before.push((b.clone(), self.rev(b)?));
            remote_tips.push((b.clone(), theirs));
        }
        Ok(Restack {
            remote: remote.into(),
            trunk: trunk.into(),
            links,
            before,
            remote_tips,
            head,
        })
    }

    /// Rebases branch `i` onto its parent (already restacked): only its own
    /// commits move, those it had from the parent's old tip stay behind.
    pub fn restack_step(&self, plan: &Restack, i: usize) -> anyhow::Result<()> {
        let link = &plan.links[i];
        let result = match &link.parent {
            None => {
                let onto = format!("{}/{}", plan.remote, plan.trunk);
                self.git().run(["rebase", "--quiet", &onto, &link.branch])
            }
            Some(parent) => self.git().run([
                "rebase",
                "--quiet",
                "--onto",
                parent,
                plan.before(parent),
                &link.branch,
            ]),
        };
        result.map(drop).map_err(|e| {
            let _ = self.git().run(["rebase", "--abort"]);
            e.context(format!("conflict while restacking '{}'", link.branch))
        })
    }

    /// Force-pushes (with lease) the branches that moved, atomically, and
    /// puts HEAD back. Returns how many branches were pushed.
    pub fn restack_push(&self, plan: &Restack) -> anyhow::Result<usize> {
        let mut args = vec![
            "push".to_string(),
            "--quiet".into(),
            "--atomic".into(),
            plan.remote.clone(),
        ];
        let mut refspecs = Vec::new();
        for (b, theirs) in &plan.remote_tips {
            if self.rev(b)? == *theirs {
                continue;
            }
            args.push(format!("--force-with-lease=refs/heads/{b}:{theirs}"));
            refspecs.push(format!("refs/heads/{b}:refs/heads/{b}"));
        }
        let pushed = refspecs.len();
        if pushed > 0 {
            args.extend(refspecs);
            self.git().run(&args)?;
        }
        self.restore_head(plan)?;
        Ok(pushed)
    }

    /// Undoes a restack that failed: every branch back to its old tip, HEAD
    /// back where it was.
    pub fn restack_undo(&self, plan: &Restack) -> anyhow::Result<()> {
        if self
            .git()
            .run(["rev-parse", "--verify", "--quiet", "REBASE_HEAD"])
            .is_ok()
        {
            let _ = self.git().run(["rebase", "--abort"]);
        }
        self.git().run(["switch", "--quiet", "--detach", "HEAD"])?;
        for (b, sha) in &plan.before {
            self.git().run(["branch", "--force", b, sha])?;
        }
        self.restore_head(plan)
    }

    fn restore_head(&self, plan: &Restack) -> anyhow::Result<()> {
        match plan.head.strip_prefix("refs/heads/") {
            Some(b) => self.git().run(["switch", "--quiet", b]),
            None => self
                .git()
                .run(["switch", "--quiet", "--detach", &plan.head]),
        }
        .map(drop)
    }
}
