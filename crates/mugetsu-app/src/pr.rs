//! Pull request lookup for branches (GitHub), cached per repository tab.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mugetsu_forge::{Github, GithubRepo, PullRequest};

const TTL: Duration = Duration::from_secs(120);

fn github() -> &'static Github {
    static GITHUB: OnceLock<Github> = OnceLock::new();
    GITHUB.get_or_init(|| Github::new(mugetsu_forge::token("gh")))
}

/// Blocking: call from a background task.
pub fn lookup(repo: &GithubRepo, branch: &str) -> Result<Option<PullRequest>, String> {
    github()
        .pull_for_branch(repo, branch)
        .map_err(|e| e.to_string())
}

/// Blocking: recent PRs of the repository keyed by head branch.
/// With a token, open PRs also get their CI / review state.
pub fn recent(repo: &GithubRepo) -> Result<HashMap<String, PullRequest>, String> {
    let mut prs = github().recent_pulls(repo).map_err(|e| e.to_string())?;
    if let Ok(statuses) = github().pr_statuses(repo) {
        for pr in prs.values_mut() {
            pr.status = statuses.get(&pr.number).cloned();
        }
    }
    Ok(prs)
}

#[derive(Clone, Debug)]
pub enum PrLookup {
    Loading,
    Found(PullRequest),
    None,
    Failed(String),
}

#[derive(Default)]
pub struct PrCache {
    entries: HashMap<String, (PrLookup, Instant)>,
    bulk_at: Option<Instant>,
}

impl PrCache {
    /// True when the repository-wide PR list should be refreshed.
    pub fn needs_bulk(&self, now: Instant) -> bool {
        self.bulk_at.is_none_or(|at| now.duration_since(at) > TTL)
    }

    pub fn mark_bulk(&mut self, now: Instant) {
        self.bulk_at = Some(now);
    }

    /// Records every PR of a repository-wide listing.
    pub fn set_bulk(&mut self, prs: HashMap<String, PullRequest>, now: Instant) {
        for (branch, pr) in prs {
            self.entries.insert(branch, (PrLookup::Found(pr), now));
        }
    }

    pub fn get(&self, branch: &str) -> Option<&PrLookup> {
        self.entries.get(branch).map(|(l, _)| l)
    }

    /// True when the branch has no fresh answer and no request in flight.
    pub fn needs_lookup(&self, branch: &str, now: Instant) -> bool {
        match self.entries.get(branch) {
            None => true,
            Some((PrLookup::Loading, _)) => false,
            Some((_, at)) => now.duration_since(*at) > TTL,
        }
    }

    pub fn set(&mut self, branch: String, lookup: PrLookup, now: Instant) {
        self.entries.insert(branch, (lookup, now));
    }
}

/// Name of the branch on the forge: `origin/feat/x` → `feat/x`, local branches use
/// their upstream's name when they track one.
pub fn forge_branch(name: &str, upstream: Option<&str>, is_remote: bool) -> String {
    let source = if is_remote { Some(name) } else { upstream };
    match source.and_then(|s| s.split_once('/')) {
        Some((_, branch)) => branch.to_string(),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_branch_names() {
        assert_eq!(forge_branch("feat/x", None, false), "feat/x");
        assert_eq!(
            forge_branch("local", Some("origin/feat/remote"), false),
            "feat/remote"
        );
        assert_eq!(forge_branch("origin/feat/x", None, true), "feat/x");
    }

    #[test]
    fn cache_ttl() {
        let mut c = PrCache::default();
        let t = Instant::now();
        assert!(c.needs_lookup("a", t));
        c.set("a".into(), PrLookup::Loading, t);
        assert!(
            !c.needs_lookup("a", t + TTL * 2),
            "in-flight requests are not repeated"
        );
        c.set("a".into(), PrLookup::None, t);
        assert!(!c.needs_lookup("a", t + Duration::from_secs(1)));
        assert!(c.needs_lookup("a", t + TTL + Duration::from_secs(1)));
    }
}
