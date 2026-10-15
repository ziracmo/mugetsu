//! Pull request lookup for branches (GitHub), cached per repository tab.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mugetsu_forge::{Github, GithubRepo, PrState, PullRequest};

const TTL: Duration = Duration::from_secs(120);

fn github() -> &'static Github {
    static GITHUB: OnceLock<Github> = OnceLock::new();
    GITHUB.get_or_init(|| Github::new(mugetsu_forge::token("gh")))
}

/// Blocking: call from a background task.
pub fn lookup(repo: &GithubRepo, branch: &str) -> Result<Option<PullRequest>, String> {
    github().pull_for_branch(repo, branch).map_err(|e| e.to_string())
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
}

impl PrCache {
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

pub fn state_label(state: PrState) -> &'static str {
    match state {
        PrState::Open => "open",
        PrState::Draft => "draft",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_branch_names() {
        assert_eq!(forge_branch("feat/x", None, false), "feat/x");
        assert_eq!(forge_branch("local", Some("origin/feat/remote"), false), "feat/remote");
        assert_eq!(forge_branch("origin/feat/x", None, true), "feat/x");
    }

    #[test]
    fn cache_ttl() {
        let mut c = PrCache::default();
        let t = Instant::now();
        assert!(c.needs_lookup("a", t));
        c.set("a".into(), PrLookup::Loading, t);
        assert!(!c.needs_lookup("a", t + TTL * 2), "in-flight requests are not repeated");
        c.set("a".into(), PrLookup::None, t);
        assert!(!c.needs_lookup("a", t + Duration::from_secs(1)));
        assert!(c.needs_lookup("a", t + TTL + Duration::from_secs(1)));
    }
}
