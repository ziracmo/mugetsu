//! Pull request lookup for branches (GitHub, GitLab, Bitbucket), cached per
//! repository tab.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use mugetsu_forge::{Forge, ForgeKind, ForgeRepo, PullRequest};

const TTL: Duration = Duration::from_secs(120);
/// Background tabs list their PRs only for notifications, less often (rate limits).
const BACKGROUND_TTL: Duration = Duration::from_secs(180);

/// One forge's API client and who "me" is there.
struct Client {
    api: Box<dyn Forge>,
    me: OnceLock<Option<String>>,
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Client for `repo`'s forge. A token from Settings wins over the environment
/// and CLI logins; changing it there makes a new client.
fn client(repo: &ForgeRepo) -> Arc<Client> {
    /// Forge, host and the token set in Settings.
    type Key = (ForgeKind, String, Option<String>);
    static CLIENTS: OnceLock<Mutex<HashMap<Key, Arc<Client>>>> = OnceLock::new();
    let settings = crate::settings::get();
    let configured = match repo.kind {
        ForgeKind::Github => None,
        ForgeKind::Gitlab => non_empty(settings.gitlab_token),
        ForgeKind::Bitbucket => non_empty(settings.bitbucket_token),
    };
    let key = (repo.kind, repo.host.clone(), configured.clone());
    let mut clients = CLIENTS.get_or_init(Default::default).lock().unwrap();
    clients
        .entry(key)
        .or_insert_with(|| {
            let token = configured.or_else(|| match repo.kind {
                ForgeKind::Github => mugetsu_forge::token("gh"),
                ForgeKind::Gitlab => mugetsu_forge::gitlab_token("glab", &repo.host),
                ForgeKind::Bitbucket => mugetsu_forge::bitbucket_token(),
            });
            Arc::new(Client {
                api: mugetsu_forge::client(repo, token),
                me: OnceLock::new(),
            })
        })
        .clone()
}

/// Blocking: call from a background task.
pub fn lookup(repo: &ForgeRepo, branch: &str) -> Result<Option<PullRequest>, String> {
    client(repo)
        .api
        .pull_for_branch(repo, branch)
        .map_err(|e| e.to_string())
}

/// Blocking: description and size of one PR.
pub fn details(repo: &ForgeRepo, number: u64) -> Result<mugetsu_forge::PrDetails, String> {
    client(repo)
        .api
        .pull_details(repo, number)
        .map_err(|e| e.to_string())
}

/// Recent PRs of a repository and who "me" is on its forge.
pub struct RecentPrs {
    pub list: Vec<PullRequest>,
    pub me: Option<String>,
}

/// Blocking. With a token, open PRs also get their CI / review state.
pub fn recent(repo: &ForgeRepo) -> Result<RecentPrs, String> {
    let client = client(repo);
    let mut list = client.api.recent_pulls(repo).map_err(|e| e.to_string())?;
    if let Ok(statuses) = client.api.pr_statuses(repo) {
        for pr in &mut list {
            if let Some(status) = statuses.get(&pr.number) {
                pr.status = Some(status.clone());
            }
        }
    }
    let me = client
        .me
        .get_or_init(|| client.api.viewer_login().ok())
        .clone();
    Ok(RecentPrs { list, me })
}

#[derive(Clone, Debug)]
pub enum PrLookup {
    Loading,
    Found(Box<PullRequest>),
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
    pub fn needs_bulk(&self, now: Instant, focused: bool) -> bool {
        let ttl = if focused { TTL } else { BACKGROUND_TTL };
        self.bulk_at.is_none_or(|at| now.duration_since(at) > ttl)
    }

    pub fn mark_bulk(&mut self, now: Instant) {
        self.bulk_at = Some(now);
    }

    /// Records every PR of a repository-wide listing.
    pub fn set_bulk(&mut self, prs: HashMap<String, PullRequest>, now: Instant) {
        for (branch, pr) in prs {
            self.entries
                .insert(branch, (PrLookup::Found(Box::new(pr)), now));
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
