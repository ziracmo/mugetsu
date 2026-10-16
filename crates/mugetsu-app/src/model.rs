//! Everything a repository tab displays, loaded off the UI thread in one go.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gix::ObjectId;
use mugetsu_forge::{ForgeRepo, GithubRepo};
use mugetsu_git::{Change, Folded, History, OperationState, RefLabel, RefsSnapshot, Repo, Status};

#[derive(Clone, Default)]
pub struct RepoData {
    pub refs: RefsSnapshot,
    /// Shared so optimistic updates copy only the small parts.
    pub history: Arc<History>,
    /// Rows shown once merged branches are folded, and their graph.
    pub folded: Arc<Folded>,
    pub decorations: HashMap<ObjectId, Vec<RefLabel>>,
    pub status: Status,
    /// The remote's forge (GitHub, GitLab, Bitbucket).
    pub forge: Option<ForgeRepo>,
    /// Set when that forge is GitHub, for GitHub-only features.
    pub github: Option<GithubRepo>,
    /// Merge, rebase, cherry-pick or revert in progress.
    pub operation: Option<OperationState>,
}

impl RepoData {
    /// Loads everything; merges outside `expanded` start folded.
    pub fn load(repo: &Repo, expanded: &HashSet<ObjectId>) -> anyhow::Result<Arc<Self>> {
        let refs = repo.refs()?;
        let history = repo.history(&refs.tips(), crate::settings::get().commit_order.into())?;
        let decorations = refs.decorations();
        let folded = Self::fold(&history, &decorations, refs.head, expanded);
        let status = repo.status()?;
        let operation = repo.operation().unwrap_or_default();
        let gitlab_hosts = crate::settings::get().gitlab_hosts;
        let forge = refs
            .remotes
            .iter()
            .find(|r| r.name == "origin")
            .or(refs.remotes.first())
            .and_then(|r| ForgeRepo::from_remote_url(&r.url, &gitlab_hosts));
        let github = forge.as_ref().and_then(ForgeRepo::github);
        Ok(Arc::new(Self {
            refs,
            history: Arc::new(history),
            folded: Arc::new(folded),
            decorations,
            status,
            forge,
            github,
            operation,
        }))
    }

    /// Folds merged branches; commits carrying a ref or HEAD always show.
    pub fn fold(
        history: &History,
        decorations: &HashMap<ObjectId, Vec<RefLabel>>,
        head: Option<ObjectId>,
        expanded: &HashSet<ObjectId>,
    ) -> Folded {
        let pinned: HashSet<ObjectId> = decorations.keys().copied().chain(head).collect();
        Folded::new(history, &pinned, expanded)
    }

    /// Folds again after a merge was expanded or folded.
    pub fn refold(&self, expanded: &HashSet<ObjectId>) -> Self {
        Self {
            folded: Arc::new(Self::fold(
                &self.history,
                &self.decorations,
                self.refs.head,
                expanded,
            )),
            ..self.clone()
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mugetsu_git::{Branch, FileStatus};

    fn oid(c: char) -> ObjectId {
        ObjectId::from_hex(c.to_string().repeat(40).as_bytes()).unwrap()
    }

    fn branch(name: &str, target: ObjectId, is_head: bool) -> Branch {
        Branch {
            name: name.into(),
            target,
            upstream: None,
            ahead: 0,
            behind: 0,
            is_head,
            upstream_gone: false,
        }
    }

    /// main (checked out) on `a`, topic on `b`, origin/remote-only on `c`.
    fn data() -> RepoData {
        let mut data = RepoData::default();
        data.refs.head = Some(oid('a'));
        data.refs.head_branch = Some("main".into());
        data.refs.branches = vec![
            branch("main", oid('a'), true),
            branch("topic", oid('b'), false),
        ];
        data.refs.remote_branches = vec![branch("origin/remote-only", oid('c'), false)];
        data.decorations.insert(
            oid('a'),
            vec![RefLabel::Branch {
                name: "main".into(),
                head: true,
            }],
        );
        data.decorations.insert(
            oid('b'),
            vec![RefLabel::Branch {
                name: "topic".into(),
                head: false,
            }],
        );
        data
    }

    fn head_labels(data: &RepoData) -> Vec<String> {
        data.decorations
            .values()
            .flatten()
            .filter_map(|l| match l {
                RefLabel::Branch { name, head: true } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn checking_out_an_existing_branch_moves_head() {
        let next = data().with_checkout("topic", None).unwrap();
        assert_eq!(next.refs.head, Some(oid('b')));
        assert_eq!(next.refs.head_branch.as_deref(), Some("topic"));
        let heads: Vec<_> = next.refs.branches.iter().filter(|b| b.is_head).collect();
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].name, "topic");
        assert_eq!(head_labels(&next), ["topic"]);
    }

    #[test]
    fn checking_out_a_remote_branch_adds_a_tracking_branch() {
        let next = data()
            .with_checkout("remote-only", Some("origin/remote-only"))
            .unwrap();
        assert_eq!(next.refs.head, Some(oid('c')));
        let names: Vec<_> = next.refs.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["main", "remote-only", "topic"], "kept sorted");
        let created = next
            .refs
            .branches
            .iter()
            .find(|b| b.name == "remote-only")
            .unwrap();
        assert_eq!(created.upstream.as_deref(), Some("origin/remote-only"));
        assert!(created.is_head);
        assert_eq!(head_labels(&next), ["remote-only"]);
        assert!(matches!(
            next.decorations[&oid('c')][0],
            RefLabel::Branch { head: true, .. }
        ));
    }

    #[test]
    fn unknown_branches_cannot_be_previewed() {
        assert!(data().with_checkout("nope", None).is_none());
        assert!(data().with_checkout("nope", Some("origin/nope")).is_none());
    }

    #[test]
    fn detached_head_label_goes_away() {
        let mut detached = data();
        detached
            .decorations
            .entry(oid('a'))
            .or_default()
            .push(RefLabel::DetachedHead);
        let next = detached.with_checkout("topic", None).unwrap();
        assert!(
            next.decorations
                .values()
                .flatten()
                .all(|l| !matches!(l, RefLabel::DetachedHead))
        );
    }

    #[test]
    fn conflicted_and_uncommitted_counts() {
        let mut data = data();
        let file = |path: &str, index: Change| FileStatus {
            path: path.into(),
            orig_path: None,
            index,
            worktree: Change::Modified,
        };
        data.status.files = vec![
            file("a", Change::Modified),
            file("b", Change::Conflicted),
            file("c", Change::Conflicted),
        ];
        assert_eq!(data.conflicted().collect::<Vec<_>>(), ["b", "c"]);
        assert_eq!(data.uncommitted(), 3);
    }
}
