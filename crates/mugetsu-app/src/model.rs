//! Everything a repository tab displays, loaded off the UI thread in one go.

use std::collections::HashMap;
use std::sync::Arc;

use gix::ObjectId;
use mugetsu_forge::GithubRepo;
use mugetsu_git::{Graph, History, RefLabel, RefsSnapshot, Repo, Status};

#[derive(Default)]
pub struct RepoData {
    pub refs: RefsSnapshot,
    pub history: History,
    pub graph: Graph,
    pub decorations: HashMap<ObjectId, Vec<RefLabel>>,
    pub status: Status,
    pub github: Option<GithubRepo>,
}

impl RepoData {
    pub fn load(repo: &Repo) -> anyhow::Result<Arc<Self>> {
        let refs = repo.refs()?;
        let history = repo.history(&refs.tips(), crate::settings::get().commit_order.into())?;
        let graph = mugetsu_git::layout(&history.commits);
        let decorations = refs.decorations();
        let status = repo.status()?;
        let github = refs
            .remotes
            .iter()
            .find(|r| r.name == "origin")
            .or(refs.remotes.first())
            .and_then(|r| GithubRepo::from_remote_url(&r.url));
        Ok(Arc::new(Self {
            refs,
            history,
            graph,
            decorations,
            status,
            github,
        }))
    }

    pub fn uncommitted(&self) -> usize {
        self.status.files.len()
    }
}
