//! Forge integration (GitHub, GitLab, Bitbucket): find the pull request
//! associated with a branch, its CI and review state.

use std::ffi::OsStr;
use std::process::Command;

mod bitbucket;
mod forge;
mod gitlab;

pub use bitbucket::Bitbucket;
pub use forge::{Forge, ForgeKind, ForgeRepo};
pub use gitlab::Gitlab;

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

    pub fn web_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// `…/commit/<sha>`.
    pub fn commit_url(&self, sha: &str) -> String {
        format!("{}/commit/{sha}", self.web_url())
    }

    /// Permalink to a file at a commit, optionally to lines (`#L3`, `#L3-L9`).
    pub fn blob_url(&self, sha: &str, path: &str, lines: Option<(usize, usize)>) -> String {
        let anchor = match lines {
            Some((a, b)) if a == b => format!("#L{a}"),
            Some((a, b)) => format!("#L{}-L{}", a.min(b), a.max(b)),
            None => String::new(),
        };
        format!(
            "{}/blob/{sha}/{}{anchor}",
            self.web_url(),
            encode_path(path)
        )
    }

    /// A file's diff in a commit's page, optionally to lines: `L` for the old
    /// side, `R` for the new one (`#diff-<sha256(path)>L4-R6`).
    pub fn commit_diff_url(
        &self,
        sha: &str,
        path: &str,
        lines: Option<((bool, usize), (bool, usize))>,
    ) -> String {
        use sha2::Digest as _;
        let hash: String = sha2::Sha256::digest(path.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let side = |(new, n): (bool, usize)| format!("{}{n}", if new { 'R' } else { 'L' });
        let lines = match lines {
            Some((a, b)) if a == b => side(a),
            Some((a, b)) => format!("{}-{}", side(a), side(b)),
            None => String::new(),
        };
        format!("{}#diff-{hash}{lines}", self.commit_url(sha))
    }
}

/// Percent-encodes a repository path for a URL, keeping its slashes.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
    /// Head and base branch names.
    pub head: String,
    pub base: String,
    /// Head branch lives in the repository itself (not a fork).
    pub same_repo: bool,
    pub author: String,
    /// Logins whose review is requested and still pending.
    pub requested_reviewers: Vec<String>,
    /// ISO 8601 timestamps.
    pub updated_at: String,
    pub merged_at: Option<String>,
}

/// What the PR list doesn't carry: description, size, labels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrDetails {
    /// Markdown.
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub commits: u64,
    /// Conversation and review comments.
    pub comments: u64,
    pub labels: Vec<String>,
    /// ISO 8601.
    pub created_at: String,
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
struct ApiBase {
    #[serde(rename = "ref")]
    branch: String,
}

#[derive(Deserialize)]
struct ApiPull {
    #[serde(default)]
    head: Option<ApiHead>,
    #[serde(default)]
    base: Option<ApiBase>,
    #[serde(default)]
    user: Option<ApiOwner>,
    #[serde(default)]
    requested_reviewers: Vec<ApiOwner>,
    #[serde(default)]
    updated_at: String,
    number: u64,
    title: String,
    html_url: String,
    state: String,
    #[serde(default)]
    draft: bool,
    merged_at: Option<String>,
}

impl ApiPull {
    fn into_pr(self, owner: &str) -> PullRequest {
        let same_repo = self.head.as_ref().is_some_and(|h| {
            h.repo
                .as_ref()
                .is_some_and(|r| r.owner.login.eq_ignore_ascii_case(owner))
        });
        let mut pr = PullRequest::from(self);
        pr.same_repo = same_repo;
        pr
    }
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
            head: p.head.map(|h| h.branch).unwrap_or_default(),
            base: p.base.map(|b| b.branch).unwrap_or_default(),
            same_repo: false,
            author: p.user.map(|u| u.login).unwrap_or_default(),
            requested_reviewers: p.requested_reviewers.into_iter().map(|u| u.login).collect(),
            updated_at: p.updated_at,
            merged_at: p.merged_at,
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

/// Most relevant PR per head branch, for branches of the repository itself.
pub fn by_branch(prs: &[PullRequest]) -> std::collections::HashMap<String, PullRequest> {
    let mut grouped: std::collections::HashMap<String, Vec<PullRequest>> = Default::default();
    for p in prs.iter().filter(|p| p.same_repo) {
        grouped.entry(p.head.clone()).or_default().push(p.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(branch, prs)| Some((branch, pick(prs)?)))
        .collect()
}

/// Stacked pull requests: open PRs whose base is another open PR's head.
/// Each stack is listed bottom (based on the trunk) to top, depth first;
/// single PRs are not stacks.
pub fn stacks(prs: &[PullRequest]) -> Vec<Vec<PullRequest>> {
    let open: Vec<&PullRequest> = prs
        .iter()
        .filter(|p| p.same_repo && matches!(p.state, PrState::Open | PrState::Draft))
        .collect();
    let heads: std::collections::HashSet<&str> = open.iter().map(|p| p.head.as_str()).collect();
    fn walk<'a>(pr: &'a PullRequest, open: &[&'a PullRequest], out: &mut Vec<PullRequest>) {
        if out.iter().any(|p| p.number == pr.number) {
            return; // cycles
        }
        out.push(pr.clone());
        let mut children: Vec<&&PullRequest> = open.iter().filter(|c| c.base == pr.head).collect();
        children.sort_by_key(|c| c.number);
        for child in children {
            walk(child, open, out);
        }
    }
    let mut roots: Vec<&&PullRequest> = open
        .iter()
        .filter(|p| !heads.contains(p.base.as_str()))
        .collect();
    roots.sort_by_key(|p| std::cmp::Reverse(p.number));
    roots
        .into_iter()
        .map(|root| {
            let mut stack = Vec::new();
            walk(root, &open, &mut stack);
            stack
        })
        .filter(|s| s.len() > 1)
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
    let out = command(gh_binary).args(["auth", "token"]).output().ok()?;
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

    /// The 100 most recently updated PRs of `repo`, newest first.
    pub fn recent_pulls(&self, repo: &GithubRepo) -> anyhow::Result<Vec<PullRequest>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls?state=all&sort=updated&direction=desc&per_page=100",
            repo.owner, repo.name
        );
        let pulls: Vec<ApiPull> = self.get_json(&url)?;
        Ok(pulls.into_iter().map(|p| p.into_pr(&repo.owner)).collect())
    }

    /// Login of the token's owner.
    pub fn viewer_login(&self) -> anyhow::Result<String> {
        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        self.token.as_deref().context("no GitHub token")?;
        Ok(self.get_json::<User>("https://api.github.com/user")?.login)
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

    /// Description and size of PR `number`.
    pub fn pull_details(&self, repo: &GithubRepo, number: u64) -> anyhow::Result<PrDetails> {
        #[derive(Deserialize)]
        struct Label {
            name: String,
        }
        #[derive(Deserialize)]
        struct Api {
            #[serde(default)]
            body: Option<String>,
            #[serde(default)]
            additions: u64,
            #[serde(default)]
            deletions: u64,
            #[serde(default)]
            changed_files: u64,
            #[serde(default)]
            commits: u64,
            #[serde(default)]
            comments: u64,
            #[serde(default)]
            review_comments: u64,
            #[serde(default)]
            labels: Vec<Label>,
            #[serde(default)]
            created_at: String,
        }
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{number}",
            repo.owner, repo.name
        );
        let a: Api = self.get_json(&url)?;
        Ok(PrDetails {
            body: a.body.unwrap_or_default(),
            additions: a.additions,
            deletions: a.deletions,
            changed_files: a.changed_files,
            commits: a.commits,
            comments: a.comments + a.review_comments,
            labels: a.labels.into_iter().map(|l| l.name).collect(),
            created_at: a.created_at,
        })
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
        Ok(pick(
            pulls.into_iter().map(|p| p.into_pr(&repo.owner)).collect(),
        ))
    }
}

impl Forge for Github {
    fn recent_pulls(&self, repo: &ForgeRepo) -> anyhow::Result<Vec<PullRequest>> {
        Github::recent_pulls(self, &github_repo(repo)?)
    }

    fn pull_for_branch(
        &self,
        repo: &ForgeRepo,
        branch: &str,
    ) -> anyhow::Result<Option<PullRequest>> {
        Github::pull_for_branch(self, &github_repo(repo)?, branch)
    }

    fn pr_statuses(
        &self,
        repo: &ForgeRepo,
    ) -> anyhow::Result<std::collections::HashMap<u64, PrStatus>> {
        Github::pr_statuses(self, &github_repo(repo)?)
    }

    fn pull_details(&self, repo: &ForgeRepo, number: u64) -> anyhow::Result<PrDetails> {
        Github::pull_details(self, &github_repo(repo)?, number)
    }

    fn viewer_login(&self) -> anyhow::Result<String> {
        Github::viewer_login(self)
    }
}

fn github_repo(repo: &ForgeRepo) -> anyhow::Result<GithubRepo> {
    repo.github().context("not a GitHub repository")
}

/// First non-empty variable among `vars`.
fn env_token(vars: &[&str]) -> Option<String> {
    vars.iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|t| !t.trim().is_empty())
}

/// GitLab token for `host`: `GITLAB_TOKEN` / `GL_TOKEN`, else the GitLab CLI's login.
pub fn gitlab_token(glab_binary: &str, host: &str) -> Option<String> {
    if let Some(t) = env_token(&["GITLAB_TOKEN", "GL_TOKEN"]) {
        return Some(t);
    }
    let host = host.split(':').next().unwrap_or(host);
    let out = command(glab_binary)
        .args(["config", "get", "token", "--host", host])
        .output()
        .ok()?;
    let t = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !t.is_empty()).then_some(t)
}

/// Bitbucket credentials: `BITBUCKET_TOKEN` (access token), or
/// `BITBUCKET_USERNAME` + `BITBUCKET_APP_PASSWORD` as `user:password`.
pub fn bitbucket_token() -> Option<String> {
    env_token(&["BITBUCKET_TOKEN"]).or_else(|| {
        let user = env_token(&["BITBUCKET_USERNAME"])?;
        let pass = env_token(&["BITBUCKET_APP_PASSWORD", "BITBUCKET_API_TOKEN"])?;
        Some(format!("{user}:{pass}"))
    })
}

/// The client for `repo`'s forge.
pub fn client(repo: &ForgeRepo, token: Option<String>) -> Box<dyn Forge> {
    match repo.kind {
        ForgeKind::Github => Box::new(Github::new(token)),
        ForgeKind::Gitlab => Box::new(Gitlab::new(&repo.host, token)),
        ForgeKind::Bitbucket => Box::new(Bitbucket::new(token)),
    }
}

/// Same as `mugetsu_git::process::command`: no console window on Windows.
#[allow(clippy::disallowed_methods, unused_mut)]
fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_web_links() {
        let gh = GithubRepo {
            owner: "o".into(),
            name: "r".into(),
        };
        assert_eq!(gh.commit_url("abc"), "https://github.com/o/r/commit/abc");
        assert_eq!(
            gh.blob_url("abc", "src/a b#.rs", None),
            "https://github.com/o/r/blob/abc/src/a%20b%23.rs"
        );
        assert_eq!(
            gh.blob_url("abc", "x.rs", Some((9, 3))),
            "https://github.com/o/r/blob/abc/x.rs#L3-L9"
        );
        assert_eq!(
            gh.blob_url("abc", "x.rs", Some((4, 4))),
            "https://github.com/o/r/blob/abc/x.rs#L4"
        );
        // sha256("a.txt")
        assert_eq!(
            gh.commit_diff_url("abc", "a.txt", Some(((false, 4), (true, 6)))),
            "https://github.com/o/r/commit/abc#diff-18b7cb099a9ea3f50ba899b5ba81e0d377a5f3b16f8f6eeb8b3e58cd4692b993L4-R6"
        );
    }

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
            base: None,
            user: None,
            requested_reviewers: Vec::new(),
            updated_at: String::new(),
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
            head: String::new(),
            base: String::new(),
            same_repo: true,
            author: String::new(),
            requested_reviewers: Vec::new(),
            updated_at: String::new(),
            merged_at: None,
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
    fn finds_stacks_from_bases() {
        let pr = |number, head: &str, base: &str, state| PullRequest {
            number,
            title: String::new(),
            url: String::new(),
            state,
            status: None,
            head: head.into(),
            base: base.into(),
            same_repo: true,
            author: String::new(),
            requested_reviewers: Vec::new(),
            updated_at: String::new(),
            merged_at: None,
        };
        let prs = vec![
            pr(1, "a", "main", PrState::Open),
            pr(2, "b", "a", PrState::Draft),
            pr(3, "c", "b", PrState::Open),
            pr(4, "lonely", "main", PrState::Open),
            pr(5, "old", "lonely", PrState::Merged),
        ];
        let stacks = stacks(&prs);
        assert_eq!(stacks.len(), 1);
        let numbers: Vec<u64> = stacks[0].iter().map(|p| p.number).collect();
        assert_eq!(numbers, [1, 2, 3]);
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
        let prs: Vec<PullRequest> = pulls.into_iter().map(|p| p.into_pr("me")).collect();
        assert_eq!(prs[0].head, "feat/x");
        let map = by_branch(&prs);
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
