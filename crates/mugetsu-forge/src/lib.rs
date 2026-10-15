//! GitHub integration: find the pull request associated with a branch.

use std::process::Command;

use anyhow::Context;
use serde::Deserialize;

/// `owner/name` of a GitHub repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GithubRepo {
    pub owner: String,
    pub name: String,
}

impl GithubRepo {
    /// Recognises `git@github.com:o/r.git`, `ssh://git@github.com/o/r`, `https://github.com/o/r.git`.
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let rest = url
            .strip_prefix("git@github.com:")
            .or_else(|| url.strip_prefix("ssh://git@github.com/"))
            .or_else(|| url.strip_prefix("https://github.com/"))
            .or_else(|| url.strip_prefix("http://github.com/"))
            .or_else(|| {
                // https://user@github.com/o/r
                let after_scheme = url.strip_prefix("https://")?;
                after_scheme.split_once("@github.com/").map(|(_, r)| r)
            })?;
        let rest = rest.trim_end_matches('/').trim_end_matches(".git");
        let (owner, name) = rest.split_once('/')?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    /// CI and review state (open PRs, when a token allows GraphQL).
    pub status: Option<PrStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Passing,
    Failing,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrStatus {
    /// Overall CI result of the head commit; `None` without checks.
    pub checks: Option<CheckState>,
    pub passed: usize,
    pub pending: usize,
    /// Names of failing checks.
    pub failing: Vec<String>,
    /// Review threads not resolved yet.
    pub unresolved: usize,
    pub review: Option<ReviewDecision>,
}

const STATUS_QUERY: &str = r#"query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    pullRequests(first: 50, states: [OPEN], orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number
        reviewDecision
        reviewThreads(first: 100) { nodes { isResolved } }
        commits(last: 1) { nodes { commit { statusCheckRollup {
          state
          contexts(first: 100) { nodes {
            __typename
            ... on CheckRun { name status conclusion }
            ... on StatusContext { context state }
          } }
        } } } }
      }
    }
  }
}"#;

/// PR number → status, from the GraphQL answer to [`STATUS_QUERY`].
fn parse_statuses(json: &serde_json::Value) -> std::collections::HashMap<u64, PrStatus> {
    let nodes = json["data"]["repository"]["pullRequests"]["nodes"].as_array();
    let mut out = std::collections::HashMap::new();
    for pr in nodes.into_iter().flatten() {
        let Some(number) = pr["number"].as_u64() else {
            continue;
        };
        let mut status = PrStatus {
            unresolved: pr["reviewThreads"]["nodes"]
                .as_array()
                .map_or(0, |t| t.iter().filter(|t| t["isResolved"] == false).count()),
            review: match pr["reviewDecision"].as_str() {
                Some("APPROVED") => Some(ReviewDecision::Approved),
                Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
                Some("REVIEW_REQUIRED") => Some(ReviewDecision::ReviewRequired),
                _ => None,
            },
            ..Default::default()
        };
        let rollup = &pr["commits"]["nodes"][0]["commit"]["statusCheckRollup"];
        status.checks = match rollup["state"].as_str() {
            Some("SUCCESS") => Some(CheckState::Passing),
            Some("FAILURE" | "ERROR") => Some(CheckState::Failing),
            Some("PENDING" | "EXPECTED") => Some(CheckState::Pending),
            _ => None,
        };
        for ctx in rollup["contexts"]["nodes"].as_array().into_iter().flatten() {
            let (name, outcome) = match ctx["__typename"].as_str() {
                Some("CheckRun") => (
                    ctx["name"].as_str(),
                    match (ctx["status"].as_str(), ctx["conclusion"].as_str()) {
                        (Some("COMPLETED"), Some("SUCCESS" | "NEUTRAL" | "SKIPPED")) => {
                            Some(CheckState::Passing)
                        }
                        (Some("COMPLETED"), _) => Some(CheckState::Failing),
                        _ => Some(CheckState::Pending),
                    },
                ),
                Some("StatusContext") => (
                    ctx["context"].as_str(),
                    match ctx["state"].as_str() {
                        Some("SUCCESS") => Some(CheckState::Passing),
                        Some("FAILURE" | "ERROR") => Some(CheckState::Failing),
                        _ => Some(CheckState::Pending),
                    },
                ),
                _ => (None, None),
            };
            match outcome {
                Some(CheckState::Passing) => status.passed += 1,
                Some(CheckState::Pending) => status.pending += 1,
                Some(CheckState::Failing) => status.failing.push(name.unwrap_or("?").to_string()),
                None => {}
            }
        }
        out.insert(number, status);
    }
    out
}

#[derive(Deserialize)]
struct ApiHead {
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<ApiHeadRepo>,
}

#[derive(Deserialize)]
struct ApiHeadRepo {
    owner: ApiOwner,
}

#[derive(Deserialize)]
struct ApiOwner {
    login: String,
}

#[derive(Deserialize)]
struct ApiPull {
    #[serde(default)]
    head: Option<ApiHead>,
    number: u64,
    title: String,
    html_url: String,
    state: String,
    #[serde(default)]
    draft: bool,
    merged_at: Option<String>,
}

impl From<ApiPull> for PullRequest {
    fn from(p: ApiPull) -> Self {
        let state = match (p.state.as_str(), p.draft, p.merged_at.is_some()) {
            (_, _, true) => PrState::Merged,
            ("open", true, _) => PrState::Draft,
            ("open", false, _) => PrState::Open,
            _ => PrState::Closed,
        };
        Self {
            number: p.number,
            title: p.title,
            url: p.html_url,
            state,
            status: None,
        }
    }
}

/// Picks the most relevant PR: open first, then the most recent.
pub fn pick(prs: Vec<PullRequest>) -> Option<PullRequest> {
    let mut prs = prs;
    prs.sort_by_key(|p| {
        (
            !matches!(p.state, PrState::Open | PrState::Draft),
            std::cmp::Reverse(p.number),
        )
    });
    prs.into_iter().next()
}

fn by_branch(pulls: Vec<ApiPull>, owner: &str) -> std::collections::HashMap<String, PullRequest> {
    let mut grouped: std::collections::HashMap<String, Vec<PullRequest>> = Default::default();
    for mut p in pulls {
        let Some(head) = p.head.take() else { continue };
        let same_owner = head
            .repo
            .is_some_and(|r| r.owner.login.eq_ignore_ascii_case(owner));
        if same_owner {
            grouped.entry(head.branch).or_default().push(p.into());
        }
    }
    grouped
        .into_iter()
        .filter_map(|(branch, prs)| Some((branch, pick(prs)?)))
        .collect()
}

/// Token from `GH_TOKEN` / `GITHUB_TOKEN`, else the GitHub CLI's stored login.
pub fn token(gh_binary: &str) -> Option<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var)
            && !t.is_empty()
        {
            return Some(t);
        }
    }
    let out = Command::new(gh_binary)
        .args(["auth", "token"])
        .output()
        .ok()?;
    let t = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !t.is_empty()).then_some(t)
}

pub struct Github {
    agent: ureq::Agent,
    token: Option<String>,
}

impl Github {
    pub fn new(token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(10)))
            .build()
            .into();
        Self { agent, token }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let mut req = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "mugetsu");
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        Ok(req
            .call()
            .context("GitHub request")?
            .body_mut()
            .read_json()?)
    }

    /// The most recently updated PRs of `repo`, keyed by head branch (same-repo
    /// branches only), most relevant PR per branch.
    pub fn recent_pulls(
        &self,
        repo: &GithubRepo,
    ) -> anyhow::Result<std::collections::HashMap<String, PullRequest>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls?state=all&sort=updated&direction=desc&per_page=100",
            repo.owner, repo.name
        );
        let pulls: Vec<ApiPull> = self.get_json(&url)?;
        Ok(by_branch(pulls, &repo.owner))
    }

    /// CI / review state of the open PRs (GraphQL needs a token).
    pub fn pr_statuses(
        &self,
        repo: &GithubRepo,
    ) -> anyhow::Result<std::collections::HashMap<u64, PrStatus>> {
        let token = self.token.as_deref().context("no GitHub token")?;
        let body = serde_json::json!({
            "query": STATUS_QUERY,
            "variables": {"owner": repo.owner, "name": repo.name},
        });
        let json: serde_json::Value = self
            .agent
            .post("https://api.github.com/graphql")
            .header("User-Agent", "mugetsu")
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(&body)
            .context("GitHub GraphQL request")?
            .body_mut()
            .read_json()?;
        Ok(parse_statuses(&json))
    }

    /// PR whose head is `branch` in `repo` (same-repo branches; forks are matched by owner).
    pub fn pull_for_branch(
        &self,
        repo: &GithubRepo,
        branch: &str,
    ) -> anyhow::Result<Option<PullRequest>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls?state=all&per_page=10&head={}:{}",
            repo.owner, repo.name, repo.owner, branch
        );
        let pulls: Vec<ApiPull> = self.get_json(&url)?;
        Ok(pick(pulls.into_iter().map(Into::into).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_remote_urls() {
        let expected = Some(GithubRepo {
            owner: "ziracmo".into(),
            name: "mugetsu".into(),
        });
        for url in [
            "git@github.com:ziracmo/mugetsu.git",
            "ssh://git@github.com/ziracmo/mugetsu",
            "https://github.com/ziracmo/mugetsu.git",
            "https://github.com/ziracmo/mugetsu/",
            "https://token@github.com/ziracmo/mugetsu",
        ] {
            assert_eq!(GithubRepo::from_remote_url(url), expected, "{url}");
        }
        assert_eq!(GithubRepo::from_remote_url("git@gitlab.com:a/b.git"), None);
        assert_eq!(
            GithubRepo::from_remote_url("https://github.com/only-owner"),
            None
        );
    }

    #[test]
    fn maps_api_states() {
        let api = |state: &str, draft, merged: bool| ApiPull {
            head: None,
            number: 1,
            title: String::new(),
            html_url: String::new(),
            state: state.into(),
            draft,
            merged_at: merged.then(|| "x".into()),
        };
        assert_eq!(
            PullRequest::from(api("open", false, false)).state,
            PrState::Open
        );
        assert_eq!(
            PullRequest::from(api("open", true, false)).state,
            PrState::Draft
        );
        assert_eq!(
            PullRequest::from(api("closed", false, true)).state,
            PrState::Merged
        );
        assert_eq!(
            PullRequest::from(api("closed", false, false)).state,
            PrState::Closed
        );
    }

    #[test]
    fn prefers_open_then_newest() {
        let pr = |number, state| PullRequest {
            number,
            title: String::new(),
            url: String::new(),
            state,
            status: None,
        };
        let picked = pick(vec![
            pr(9, PrState::Closed),
            pr(3, PrState::Open),
            pr(5, PrState::Merged),
        ]);
        assert_eq!(picked.unwrap().number, 3);
        let picked = pick(vec![pr(2, PrState::Merged), pr(7, PrState::Closed)]);
        assert_eq!(picked.unwrap().number, 7);
    }

    #[test]
    fn parses_ci_and_review_state() {
        let json: serde_json::Value = serde_json::from_str(r#"{"data": {"repository": {"pullRequests": {"nodes": [
            {"number": 7, "reviewDecision": "CHANGES_REQUESTED",
             "reviewThreads": {"nodes": [{"isResolved": false}, {"isResolved": true}, {"isResolved": false}]},
             "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE", "contexts": {"nodes": [
                {"__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "name": "e2e", "status": "IN_PROGRESS", "conclusion": null},
                {"__typename": "StatusContext", "context": "ci/legacy", "state": "SUCCESS"}
             ]}}}}]}},
            {"number": 8, "reviewDecision": null, "reviewThreads": {"nodes": []},
             "commits": {"nodes": [{"commit": {"statusCheckRollup": null}}]}}
        ]}}}}"#).unwrap();
        let map = parse_statuses(&json);
        let s = &map[&7];
        assert_eq!(s.checks, Some(CheckState::Failing));
        assert_eq!(s.failing, ["test"]);
        assert_eq!((s.passed, s.pending), (2, 1));
        assert_eq!(s.unresolved, 2);
        assert_eq!(s.review, Some(ReviewDecision::ChangesRequested));
        assert_eq!(map[&8], PrStatus::default());
    }

    #[test]
    fn groups_pulls_by_head_branch() {
        let json = r#"[
            {"number": 3, "title": "t3", "html_url": "u3", "state": "open", "draft": false, "merged_at": null,
             "head": {"ref": "feat/x", "repo": {"owner": {"login": "Me"}}}},
            {"number": 2, "title": "t2", "html_url": "u2", "state": "closed", "draft": false, "merged_at": null,
             "head": {"ref": "feat/x", "repo": {"owner": {"login": "me"}}}},
            {"number": 1, "title": "t1", "html_url": "u1", "state": "open", "draft": false, "merged_at": null,
             "head": {"ref": "fork-branch", "repo": {"owner": {"login": "someone"}}}},
            {"number": 4, "title": "t4", "html_url": "u4", "state": "open", "draft": false, "merged_at": null,
             "head": {"ref": "gone", "repo": null}}
        ]"#;
        let pulls: Vec<ApiPull> = serde_json::from_str(json).unwrap();
        let map = by_branch(pulls, "me");
        assert_eq!(map.len(), 1);
        assert_eq!(map["feat/x"].number, 3);
    }
}

/// Where to find an author's picture: GitHub for `…@users.noreply.github.com`
/// addresses, Gravatar (404 when unset) otherwise.
pub fn avatar_url(email: &str, size: u32) -> String {
    let email = email.trim().to_lowercase();
    if let Some(local) = email.strip_suffix("@users.noreply.github.com") {
        // `12345+login` or plain `login`.
        let login = local.split_once('+').map_or(local, |(_, l)| l);
        return format!("https://github.com/{login}.png?size={size}");
    }
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(email.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("https://www.gravatar.com/avatar/{hex}?s={size}&d=404")
}

/// Downloads an avatar; `Ok(None)` when the person has none.
pub fn fetch_avatar(email: &str, size: u32) -> anyhow::Result<Option<Vec<u8>>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(&avatar_url(email, size))
        .header("User-Agent", "mugetsu")
        .call()?;
    if resp.status() != 200 {
        return Ok(None);
    }
    Ok(Some(resp.body_mut().read_to_vec()?))
}

#[cfg(test)]
mod avatar_tests {
    use super::*;

    #[test]
    fn avatar_urls() {
        assert_eq!(
            avatar_url("123+octo@users.noreply.github.com", 64),
            "https://github.com/octo.png?size=64"
        );
        // sha256("test@example.com")
        assert_eq!(
            avatar_url(" Test@Example.com ", 32),
            "https://www.gravatar.com/avatar/973dfe463ec85785f5f95af5ba3906eedb2d931c24e69824a89ea65dba4e813b?s=32&d=404"
        );
    }
}
