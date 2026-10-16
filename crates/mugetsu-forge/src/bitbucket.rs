//! Bitbucket Cloud: pull requests, build statuses and reviews (REST API 2.0).

use std::collections::HashMap;

use anyhow::Context as _;
use serde::Deserialize;

use crate::forge::{Forge, ForgeKind, ForgeRepo, REPO_PAGES, RemoteRepo, agent, encode};
use crate::{CheckState, PrDetails, PrState, PrStatus, PullRequest, ReviewDecision};

const API: &str = "https://api.bitbucket.org/2.0";
/// Open PRs whose build statuses are fetched (one request each).
const STATUS_LIMIT: usize = 30;

pub struct Bitbucket {
    agent: ureq::Agent,
    /// `Authorization` header value: `user:app-password` (or `email:api-token`)
    /// goes as Basic, anything else as a Bearer access token.
    auth: Option<String>,
}

#[derive(Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    values: Vec<T>,
    /// URL of the next page, if any.
    next: Option<String>,
}

#[derive(Deserialize)]
struct ApiCloneLink {
    name: String,
    href: String,
}

#[derive(Deserialize, Default)]
struct ApiRepoLinks {
    #[serde(default)]
    clone: Vec<ApiCloneLink>,
}

#[derive(Deserialize)]
struct ApiUserRepo {
    full_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    is_private: bool,
    #[serde(default)]
    links: ApiRepoLinks,
    #[serde(default)]
    updated_on: String,
}

impl From<ApiUserRepo> for RemoteRepo {
    fn from(r: ApiUserRepo) -> Self {
        let link = |name: &str| {
            r.links
                .clone
                .iter()
                .find(|l| l.name == name)
                .map(|l| l.href.clone())
        };
        let https_url =
            link("https").unwrap_or_else(|| format!("https://bitbucket.org/{}.git", r.full_name));
        let ssh_url =
            link("ssh").unwrap_or_else(|| format!("git@bitbucket.org:{}.git", r.full_name));
        RemoteRepo {
            kind: ForgeKind::Bitbucket,
            full_name: r.full_name,
            description: r.description,
            private: r.is_private,
            https_url,
            ssh_url,
            updated_at: r.updated_on,
        }
    }
}

#[derive(Deserialize)]
struct ApiUser {
    nickname: Option<String>,
    #[serde(default)]
    display_name: String,
}

impl ApiUser {
    fn login(self) -> String {
        self.nickname.unwrap_or(self.display_name)
    }
}

#[derive(Deserialize)]
struct ApiBranch {
    name: String,
}

#[derive(Deserialize)]
struct ApiRepo {
    full_name: String,
}

#[derive(Deserialize)]
struct ApiEnd {
    branch: Option<ApiBranch>,
    repository: Option<ApiRepo>,
}

#[derive(Deserialize)]
struct ApiParticipant {
    user: Option<ApiUser>,
    #[serde(default)]
    role: String,
    #[serde(default)]
    approved: bool,
    state: Option<String>,
}

#[derive(Deserialize)]
struct ApiLinks {
    html: ApiHref,
}

#[derive(Deserialize)]
struct ApiHref {
    href: String,
}

#[derive(Deserialize)]
struct ApiPull {
    id: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    links: ApiLinks,
    source: Option<ApiEnd>,
    destination: Option<ApiEnd>,
    author: Option<ApiUser>,
    #[serde(default)]
    participants: Vec<ApiParticipant>,
    #[serde(default)]
    updated_on: String,
}

fn full_name(end: &Option<ApiEnd>) -> Option<&str> {
    Some(end.as_ref()?.repository.as_ref()?.full_name.as_str())
}

impl ApiPull {
    /// Review state from the participants, `None` when nobody reviews.
    fn review(&self) -> Option<ReviewDecision> {
        let reviewers: Vec<&ApiParticipant> = self
            .participants
            .iter()
            .filter(|p| p.role == "REVIEWER" || p.approved || p.state.is_some())
            .collect();
        if reviewers
            .iter()
            .any(|p| p.state.as_deref() == Some("changes_requested"))
        {
            Some(ReviewDecision::ChangesRequested)
        } else if reviewers.iter().any(|p| p.approved) {
            Some(ReviewDecision::Approved)
        } else if !reviewers.is_empty() {
            Some(ReviewDecision::ReviewRequired)
        } else {
            None
        }
    }

    fn into_pr(self) -> PullRequest {
        let state = match (self.state.as_str(), self.draft) {
            ("MERGED", _) => PrState::Merged,
            ("OPEN", true) => PrState::Draft,
            ("OPEN", false) => PrState::Open,
            _ => PrState::Closed,
        };
        let same_repo = full_name(&self.source).is_some_and(|s| {
            full_name(&self.destination).is_some_and(|d| d.eq_ignore_ascii_case(s))
        });
        let review = self.review();
        let requested_reviewers = self
            .participants
            .into_iter()
            .filter(|p| p.role == "REVIEWER" && !p.approved && p.state.is_none())
            .filter_map(|p| Some(p.user?.login()))
            .collect();
        PullRequest {
            number: self.id,
            title: self.title,
            url: self.links.html.href,
            state,
            // Reviews are known from the listing; checks are added by `pr_statuses`.
            status: review.map(|r| PrStatus {
                review: Some(r),
                ..Default::default()
            }),
            head: self
                .source
                .and_then(|s| s.branch)
                .map(|b| b.name)
                .unwrap_or_default(),
            base: self
                .destination
                .and_then(|d| d.branch)
                .map(|b| b.name)
                .unwrap_or_default(),
            same_repo,
            author: self.author.map(ApiUser::login).unwrap_or_default(),
            requested_reviewers,
            // Bitbucket has no merge date: the last update is the merge for merged PRs.
            merged_at: (state == PrState::Merged).then(|| self.updated_on.clone()),
            updated_at: self.updated_on,
        }
    }
}

#[derive(Deserialize)]
struct ApiStatus {
    #[serde(default)]
    state: String,
    name: Option<String>,
    #[serde(default)]
    key: String,
}

/// Rolls a PR's build statuses up into a [`PrStatus`].
fn rollup(statuses: Vec<ApiStatus>, review: Option<ReviewDecision>) -> PrStatus {
    let mut s = PrStatus {
        review,
        ..Default::default()
    };
    for st in statuses {
        match st.state.as_str() {
            "SUCCESSFUL" => s.passed += 1,
            "FAILED" | "STOPPED" => s.failing.push(st.name.unwrap_or(st.key)),
            _ => s.pending += 1,
        }
    }
    s.checks = if !s.failing.is_empty() {
        Some(CheckState::Failing)
    } else if s.pending > 0 {
        Some(CheckState::Pending)
    } else if s.passed > 0 {
        Some(CheckState::Passing)
    } else {
        None
    };
    s
}

/// Base64 for HTTP Basic auth.
fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

const PULL_FIELDS: &str = "%2Bvalues.participants,%2Bvalues.draft";

impl Bitbucket {
    pub fn new(token: Option<String>) -> Self {
        let auth = token.map(|t| match t.split_once(':') {
            Some(_) => format!("Basic {}", base64(t.as_bytes())),
            None => format!("Bearer {t}"),
        });
        Self {
            agent: agent(),
            auth,
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let mut req = self.agent.get(url).header("User-Agent", "mugetsu");
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a);
        }
        Ok(req
            .call()
            .context("Bitbucket request")?
            .body_mut()
            .read_json()?)
    }

    fn pulls(&self, repo: &ForgeRepo, query: &str) -> anyhow::Result<Vec<ApiPull>> {
        let url = format!(
            "{API}/repositories/{}/{}/pullrequests?state=OPEN&state=MERGED&state=DECLINED&state=SUPERSEDED&sort=-updated_on&pagelen=50&fields={PULL_FIELDS}{query}",
            encode(&repo.owner),
            encode(&repo.name)
        );
        Ok(self.get_json::<Page<ApiPull>>(&url)?.values)
    }
}

impl Forge for Bitbucket {
    fn recent_pulls(&self, repo: &ForgeRepo) -> anyhow::Result<Vec<PullRequest>> {
        Ok(self
            .pulls(repo, "")?
            .into_iter()
            .map(ApiPull::into_pr)
            .collect())
    }

    fn pull_for_branch(
        &self,
        repo: &ForgeRepo,
        branch: &str,
    ) -> anyhow::Result<Option<PullRequest>> {
        let q = format!("source.branch.name=\"{}\"", branch.replace('"', "\\\""));
        let prs = self.pulls(repo, &format!("&q={}", encode(&q)))?;
        Ok(crate::pick(
            prs.into_iter()
                .map(ApiPull::into_pr)
                .filter(|p| p.same_repo)
                .collect(),
        ))
    }

    fn pr_statuses(&self, repo: &ForgeRepo) -> anyhow::Result<HashMap<u64, PrStatus>> {
        let open = format!("&q={}", encode("state=\"OPEN\""));
        let mut out = HashMap::new();
        for pr in self.pulls(repo, &open)?.into_iter().take(STATUS_LIMIT) {
            let url = format!(
                "{API}/repositories/{}/{}/pullrequests/{}/statuses?pagelen=100",
                encode(&repo.owner),
                encode(&repo.name),
                pr.id
            );
            let statuses = self
                .get_json::<Page<ApiStatus>>(&url)
                .map(|p| p.values)
                .unwrap_or_default();
            out.insert(pr.id, rollup(statuses, pr.review()));
        }
        Ok(out)
    }

    fn pull_details(&self, repo: &ForgeRepo, number: u64) -> anyhow::Result<PrDetails> {
        #[derive(Deserialize)]
        struct Api {
            #[serde(default)]
            description: String,
            #[serde(default)]
            comment_count: u64,
            #[serde(default)]
            created_on: String,
        }
        #[derive(Deserialize)]
        struct Stat {
            #[serde(default)]
            lines_added: u64,
            #[serde(default)]
            lines_removed: u64,
        }
        let base = format!(
            "{API}/repositories/{}/{}/pullrequests/{number}",
            encode(&repo.owner),
            encode(&repo.name)
        );
        let a: Api = self.get_json(&base)?;
        let stats = self
            .get_json::<Page<Stat>>(&format!("{base}/diffstat?pagelen=500"))
            .map(|p| p.values)
            .unwrap_or_default();
        Ok(PrDetails {
            body: a.description,
            additions: stats.iter().map(|s| s.lines_added).sum(),
            deletions: stats.iter().map(|s| s.lines_removed).sum(),
            changed_files: stats.len() as u64,
            comments: a.comment_count,
            created_at: a.created_on,
            ..Default::default()
        })
    }

    fn viewer_login(&self) -> anyhow::Result<String> {
        self.auth.as_deref().context("no Bitbucket token")?;
        Ok(self.get_json::<ApiUser>(&format!("{API}/user"))?.login())
    }

    fn user_repos(&self) -> anyhow::Result<Vec<RemoteRepo>> {
        self.auth.as_deref().context("no Bitbucket token")?;
        let mut out = Vec::new();
        let mut url = Some(format!(
            "{API}/repositories?role=member&sort=-updated_on&pagelen=100"
        ));
        for _ in 0..REPO_PAGES {
            let Some(u) = url.take() else { break };
            let page: Page<ApiUserRepo> = self.get_json(&u)?;
            out.extend(page.values.into_iter().map(RemoteRepo::from));
            url = page.next;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_pull_requests() {
        let json = r#"{"values": [
            {"id": 4, "title": "t", "state": "OPEN", "draft": false,
             "links": {"html": {"href": "https://bitbucket.org/t/a/pull-requests/4"}},
             "source": {"branch": {"name": "feat/x"}, "repository": {"full_name": "t/a"}},
             "destination": {"branch": {"name": "main"}, "repository": {"full_name": "T/a"}},
             "author": {"nickname": "alex", "display_name": "Alex"},
             "participants": [
                {"user": {"nickname": "bob"}, "role": "REVIEWER", "approved": false, "state": null},
                {"user": {"nickname": "eve"}, "role": "REVIEWER", "approved": true, "state": "approved"}
             ],
             "updated_on": "2026-01-02T00:00:00+00:00"},
            {"id": 3, "title": "t", "state": "MERGED", "links": {"html": {"href": "u"}},
             "source": {"branch": {"name": "fork"}, "repository": {"full_name": "someone/a"}},
             "destination": {"branch": {"name": "main"}, "repository": {"full_name": "t/a"}},
             "author": {"display_name": "Bot"}, "updated_on": "2026-01-01T00:00:00+00:00"},
            {"id": 2, "title": "t", "state": "DECLINED", "links": {"html": {"href": "u"}}}
        ]}"#;
        let page: Page<ApiPull> = serde_json::from_str(json).unwrap();
        let prs: Vec<PullRequest> = page.values.into_iter().map(ApiPull::into_pr).collect();
        assert_eq!(prs[0].state, PrState::Open);
        assert!(prs[0].same_repo);
        assert_eq!(prs[0].head, "feat/x");
        assert_eq!(prs[0].author, "alex");
        assert_eq!(prs[0].requested_reviewers, ["bob"]);
        assert_eq!(
            prs[0].status.as_ref().unwrap().review,
            Some(ReviewDecision::Approved)
        );
        assert_eq!(prs[1].state, PrState::Merged);
        assert_eq!(
            prs[1].merged_at.as_deref(),
            Some("2026-01-01T00:00:00+00:00")
        );
        assert!(!prs[1].same_repo);
        assert_eq!(prs[1].author, "Bot");
        assert_eq!(prs[2].state, PrState::Closed);
    }

    #[test]
    fn rolls_up_build_statuses() {
        let st = |state: &str, name: &str| ApiStatus {
            state: state.into(),
            name: Some(name.into()),
            key: String::new(),
        };
        let s = rollup(
            vec![
                st("SUCCESSFUL", "a"),
                st("FAILED", "b"),
                st("INPROGRESS", "c"),
            ],
            None,
        );
        assert_eq!(s.checks, Some(CheckState::Failing));
        assert_eq!(s.failing, ["b"]);
        assert_eq!((s.passed, s.pending), (1, 1));
        assert_eq!(rollup(vec![], None).checks, None);
        assert_eq!(
            rollup(vec![st("INPROGRESS", "c")], None).checks,
            Some(CheckState::Pending)
        );
    }

    #[test]
    fn maps_repositories() {
        let json = r#"{"values": [
            {"full_name": "team/app", "is_private": true, "updated_on": "2026-01-01",
             "links": {"clone": [
                {"name": "https", "href": "https://alex@bitbucket.org/team/app.git"},
                {"name": "ssh", "href": "git@bitbucket.org:team/app.git"}]}},
            {"full_name": "team/bare"}
        ], "next": "https://api.bitbucket.org/2.0/repositories?page=2"}"#;
        let page: Page<ApiUserRepo> = serde_json::from_str(json).unwrap();
        assert!(page.next.is_some());
        let repos: Vec<RemoteRepo> = page.values.into_iter().map(RemoteRepo::from).collect();
        assert!(repos[0].private);
        assert_eq!(repos[0].ssh_url, "git@bitbucket.org:team/app.git");
        assert_eq!(
            repos[0].https_url,
            "https://alex@bitbucket.org/team/app.git"
        );
        assert_eq!(repos[1].name(), "bare");
        assert_eq!(repos[1].ssh_url, "git@bitbucket.org:team/bare.git");
    }

    #[test]
    fn basic_auth() {
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }
}
