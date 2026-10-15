//! Everything a repository tab displays, loaded off the UI thread in one go.

use std::collections::HashMap;
use std::sync::Arc;

use gix::ObjectId;
use mugetsu_forge::GithubRepo;
use mugetsu_git::{Change, Graph, History, OperationState, RefLabel, RefsSnapshot, Repo, Status};

#[derive(Clone, Default)]
pub struct RepoData {
    pub refs: RefsSnapshot,
    /// Shared so optimistic updates copy only the small parts.
    pub history: Arc<History>,
    pub graph: Arc<Graph>,
    pub decorations: HashMap<ObjectId, Vec<RefLabel>>,
    pub status: Status,
    pub github: Option<GithubRepo>,
    /// Merge, rebase, cherry-pick or revert in progress.
    pub operation: Option<OperationState>,
}

impl RepoData {
    pub fn load(repo: &Repo) -> anyhow::Result<Arc<Self>> {
        let refs = repo.refs()?;
        let history = repo.history(&refs.tips(), crate::settings::get().commit_order.into())?;
        let graph = mugetsu_git::layout(&history.commits);
        let decorations = refs.decorations();
        let status = repo.status()?;
        let operation = repo.operation().unwrap_or_default();
        let github = refs
            .remotes
            .iter()
            .find(|r| r.name == "origin")
            .or(refs.remotes.first())
            .and_then(|r| GithubRepo::from_remote_url(&r.url));
        Ok(Arc::new(Self {
            refs,
            history: Arc::new(history),
            graph: Arc::new(graph),
            decorations,
            status,
            github,
            operation,
        }))
    }

    /// What the data will look like once `local` is checked out (created from
    /// `track` when it doesn't exist yet), shown before git is done.
    pub fn with_checkout(&self, local: &str, track: Option<&str>) -> Option<Self> {
        let refs = &self.refs;
        let target = refs
            .branches
            .iter()
            .find(|b| b.name == local)
            .map(|b| b.target)
            .or_else(|| {
                let track = track?;
                refs.remote_branches
                    .iter()
                    .find(|b| b.name == track)
                    .map(|b| b.target)
            })?;
        let mut next = self.clone();
        next.refs.head = Some(target);
        next.refs.head_branch = Some(local.to_string());
        let mut found = false;
        for b in &mut next.refs.branches {
            b.is_head = b.name == local;
            found |= b.is_head;
        }
        if !found {
            next.refs.branches.push(mugetsu_git::Branch {
                name: local.to_string(),
                target,
                upstream: track.map(str::to_string),
                ahead: 0,
                behind: 0,
                is_head: true,
                upstream_gone: false,
            });
            next.refs.branches.sort_by(|a, b| a.name.cmp(&b.name));
        }
        for labels in next.decorations.values_mut() {
            labels.retain(|l| !matches!(l, RefLabel::DetachedHead));
            for l in labels.iter_mut() {
                if let RefLabel::Branch { head, .. } = l {
                    *head = false;
                }
            }
        }
        let labels = next.decorations.entry(target).or_default();
        match labels
            .iter_mut()
            .find(|l| matches!(l, RefLabel::Branch { name, .. } if name == local))
        {
            Some(RefLabel::Branch { head, .. }) => *head = true,
            _ => labels.insert(
                0,
                RefLabel::Branch {
                    name: local.to_string(),
                    head: true,
                },
            ),
        }
        Some(next)
    }

    /// Paths with unresolved conflicts, in status order.
    pub fn conflicted(&self) -> impl Iterator<Item = &str> {
        self.status
            .files
            .iter()
            .filter(|f| f.index == Change::Conflicted)
            .map(|f| f.path.as_str())
    }

    pub fn uncommitted(&self) -> usize {
        self.status.files.len()
    }
}
