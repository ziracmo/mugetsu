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
