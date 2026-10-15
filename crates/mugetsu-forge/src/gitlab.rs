//! GitLab (gitlab.com or self-hosted): merge requests through the REST API,
//! pipelines, approvals and threads through GraphQL.

use std::collections::HashMap;

use anyhow::Context as _;
use serde::Deserialize;

use crate::forge::{Forge, ForgeRepo, agent, encode};
use crate::{CheckState, PrDetails, PrState, PrStatus, PullRequest, ReviewDecision};

pub struct Gitlab {
    agent: ureq::Agent,
    /// Web host the API lives on (`gitlab.com`, `gitlab.acme.io:8443`).
    host: String,
    token: Option<String>,
}

#[derive(Deserialize)]
struct ApiUser {
    username: String,
}

#[derive(Deserialize)]
struct ApiMr {
    iid: u64,
    title: String,
    web_url: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    #[serde(default)]
    source_branch: String,
    #[serde(default)]
    target_branch: String,
    source_project_id: Option<u64>,
    target_project_id: Option<u64>,
    author: Option<ApiUser>,
    #[serde(default)]
    reviewers: Vec<ApiUser>,
    #[serde(default)]
    updated_at: String,
    merged_at: Option<String>,
}

impl From<ApiMr> for PullRequest {
    fn from(m: ApiMr) -> Self {
        let state = match m.state.as_str() {
            "merged" => PrState::Merged,
            "opened" if m.draft || m.work_in_progress => PrState::Draft,
            "opened" => PrState::Open,
            _ => PrState::Closed,
        };
        Self {
            number: m.iid,
            title: m.title,
            url: m.web_url,
            state,
            status: None,
            head: m.source_branch,
            base: m.target_branch,
            same_repo: m.source_project_id.is_some() && m.source_project_id == m.target_project_id,
            author: m.author.map(|a| a.username).unwrap_or_default(),
            requested_reviewers: m.reviewers.into_iter().map(|r| r.username).collect(),
            updated_at: m.updated_at,
            merged_at: m.merged_at,
        }
    }
}

const STATUS_QUERY: &str = r#"query($path: ID!) {
  project(fullPath: $path) {
    mergeRequests(state: opened, first: 50, sort: UPDATED_DESC) {
      nodes {
        iid
        approved
        detailedMergeStatus
        resolvableDiscussionsCount
        resolvedDiscussionsCount
        reviewers { nodes { username mergeRequestInteraction { reviewState } } }
        headPipeline { status jobs(first: 100, retried: false) { nodes { name status allowFailure } } }
      }
    }
  }
}"#;

/// For GitLab versions without review states or detailed merge status.
const STATUS_QUERY_COMPAT: &str = r#"query($path: ID!) {
  project(fullPath: $path) {
    mergeRequests(state: opened, first: 50) {
      nodes {
        iid
        approved
        headPipeline { status jobs(first: 100) { nodes { name status allowFailure } } }
      }
    }
  }
}"#;

fn check_of_pipeline(status: &str) -> Option<CheckState> {
    match status {
        "SUCCESS" | "SKIPPED" => Some(CheckState::Passing),
        "FAILED" | "CANCELED" | "CANCELING" => Some(CheckState::Failing),
        "" => None,
        _ => Some(CheckState::Pending),
    }
}

/// MR iid → status, from the GraphQL answer to [`STATUS_QUERY`].
fn parse_statuses(json: &serde_json::Value) -> HashMap<u64, PrStatus> {
    let nodes = json["data"]["project"]["mergeRequests"]["nodes"].as_array();
    let mut out = HashMap::new();
    for mr in nodes.into_iter().flatten() {
        // `iid` is a string in GraphQL.
        let Some(iid) = mr["iid"].as_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        let changes_requested = mr["detailedMergeStatus"] == "REQUESTED_CHANGES"
            || mr["reviewers"]["nodes"].as_array().is_some_and(|r| {
                r.iter()
                    .any(|r| r["mergeRequestInteraction"]["reviewState"] == "REQUESTED_CHANGES")
            });
        let resolvable = mr["resolvableDiscussionsCount"].as_u64().unwrap_or(0);
        let resolved = mr["resolvedDiscussionsCount"].as_u64().unwrap_or(0);
        let mut status = PrStatus {
            unresolved: resolvable.saturating_sub(resolved) as usize,
            review: if changes_requested {
                Some(ReviewDecision::ChangesRequested)
            } else if mr["approved"] == true && mr["detailedMergeStatus"] != "NOT_APPROVED" {
                Some(ReviewDecision::Approved)
            } else if mr["detailedMergeStatus"] == "NOT_APPROVED" {
                Some(ReviewDecision::ReviewRequired)
            } else {
                None
            },
            ..Default::default()
        };
        let pipeline = &mr["headPipeline"];
        status.checks = check_of_pipeline(pipeline["status"].as_str().unwrap_or(""));
        for job in pipeline["jobs"]["nodes"].as_array().into_iter().flatten() {
            let allowed = job["allowFailure"] == true;
            match (job["status"].as_str().unwrap_or(""), allowed) {
                ("SUCCESS" | "SKIPPED", _) | ("FAILED" | "CANCELED", true) => status.passed += 1,
                ("FAILED" | "CANCELED", false) => status
                    .failing
                    .push(job["name"].as_str().unwrap_or("?").to_string()),
                // Optional manual jobs never run on their own.
                ("MANUAL", true) => {}
                _ => status.pending += 1,
            }
        }
        out.insert(iid, status);
    }
    out
}

impl Gitlab {
    pub fn new(host: &str, token: Option<String>) -> Self {
        Self {
            agent: agent(),
            host: host.to_string(),
            token,
        }
    }

    fn api(&self, path: &str) -> String {
        format!("https://{}/api/v4/{path}", self.host)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        let mut req = self.agent.get(url).header("User-Agent", "mugetsu");
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        Ok(req
            .call()
            .context("GitLab request")?
            .body_mut()
            .read_json()?)
    }

    fn graphql(&self, query: &str, repo: &ForgeRepo) -> anyhow::Result<serde_json::Value> {
        let body = serde_json::json!({"query": query, "variables": {"path": repo.path()}});
        let mut req = self
            .agent
            .post(&format!("https://{}/api/graphql", self.host))
            .header("User-Agent", "mugetsu");
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        let json: serde_json::Value = req
            .send_json(&body)
            .context("GitLab GraphQL request")?
            .body_mut()
            .read_json()?;
        if json["data"]["project"].is_null()
            && let Some(err) = json["errors"][0]["message"].as_str()
        {
            anyhow::bail!("GitLab GraphQL: {err}");
        }
        Ok(json)
    }

    fn project(repo: &ForgeRepo) -> String {
        format!("projects/{}", encode(&repo.path()))
    }
}

impl Forge for Gitlab {
    fn recent_pulls(&self, repo: &ForgeRepo) -> anyhow::Result<Vec<PullRequest>> {
        let url = self.api(&format!(
            "{}/merge_requests?state=all&order_by=updated_at&sort=desc&per_page=100",
            Self::project(repo)
        ));
        let mrs: Vec<ApiMr> = self.get_json(&url)?;
        Ok(mrs.into_iter().map(PullRequest::from).collect())
    }

    fn pull_for_branch(
        &self,
        repo: &ForgeRepo,
        branch: &str,
    ) -> anyhow::Result<Option<PullRequest>> {
        let url = self.api(&format!(
            "{}/merge_requests?state=all&per_page=10&source_branch={}",
            Self::project(repo),
            encode(branch)
        ));
        let mrs: Vec<ApiMr> = self.get_json(&url)?;
        Ok(crate::pick(
            mrs.into_iter()
                .map(PullRequest::from)
                .filter(|p| p.same_repo)
                .collect(),
        ))
    }

    fn pr_statuses(&self, repo: &ForgeRepo) -> anyhow::Result<HashMap<u64, PrStatus>> {
        let json = self
            .graphql(STATUS_QUERY, repo)
            .or_else(|_| self.graphql(STATUS_QUERY_COMPAT, repo))?;
        Ok(parse_statuses(&json))
    }

    fn pull_details(&self, repo: &ForgeRepo, number: u64) -> anyhow::Result<PrDetails> {
        #[derive(Deserialize)]
        struct Api {
            description: Option<String>,
            #[serde(default)]
            user_notes_count: u64,
            #[serde(default)]
            labels: Vec<String>,
            #[serde(default)]
            created_at: String,
            /// A string, `"1000+"` past the limit.
            changes_count: Option<String>,
        }
        let url = self.api(&format!("{}/merge_requests/{number}", Self::project(repo)));
        let a: Api = self.get_json(&url)?;
        Ok(PrDetails {
            body: a.description.unwrap_or_default(),
            changed_files: a
                .changes_count
                .and_then(|c| c.trim_end_matches('+').parse().ok())
                .unwrap_or(0),
            comments: a.user_notes_count,
            labels: a.labels,
            created_at: a.created_at,
            ..Default::default()
        })
    }

    fn viewer_login(&self) -> anyhow::Result<String> {
        self.token.as_deref().context("no GitLab token")?;
        Ok(self.get_json::<ApiUser>(&self.api("user"))?.username)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_merge_requests() {
        let json = r#"[
            {"iid": 7, "title": "t", "web_url": "u", "state": "opened", "draft": true,
             "source_branch": "feat/x", "target_branch": "main", "source_project_id": 1,
             "target_project_id": 1, "author": {"username": "alex"},
             "reviewers": [{"username": "bob"}], "updated_at": "2026-01-01T00:00:00Z", "merged_at": null},
            {"iid": 6, "title": "t", "web_url": "u", "state": "merged",
             "source_branch": "fork", "target_branch": "main", "source_project_id": 2,
             "target_project_id": 1, "merged_at": "2026-01-01T00:00:00Z"},
            {"iid": 5, "title": "t", "web_url": "u", "state": "locked"}
        ]"#;
        let mrs: Vec<ApiMr> = serde_json::from_str(json).unwrap();
        let prs: Vec<PullRequest> = mrs.into_iter().map(PullRequest::from).collect();
        assert_eq!(prs[0].state, PrState::Draft);
        assert!(prs[0].same_repo);
        assert_eq!(
            (prs[0].head.as_str(), prs[0].base.as_str()),
            ("feat/x", "main")
        );
        assert_eq!(prs[0].author, "alex");
        assert_eq!(prs[0].requested_reviewers, ["bob"]);
        assert_eq!(prs[1].state, PrState::Merged);
        assert!(!prs[1].same_repo);
        assert_eq!(prs[2].state, PrState::Closed);
    }

    #[test]
    fn parses_pipelines_and_approvals() {
        let json: serde_json::Value = serde_json::from_str(r#"{"data": {"project": {"mergeRequests": {"nodes": [
            {"iid": "7", "approved": false, "detailedMergeStatus": "NOT_APPROVED",
             "resolvableDiscussionsCount": 3, "resolvedDiscussionsCount": 1,
             "reviewers": {"nodes": [{"username": "a", "mergeRequestInteraction": {"reviewState": "UNREVIEWED"}}]},
             "headPipeline": {"status": "FAILED", "jobs": {"nodes": [
                {"name": "test", "status": "FAILED", "allowFailure": false},
                {"name": "lint", "status": "SUCCESS", "allowFailure": false},
                {"name": "flaky", "status": "FAILED", "allowFailure": true},
                {"name": "deploy", "status": "MANUAL", "allowFailure": true},
                {"name": "e2e", "status": "RUNNING", "allowFailure": false}
             ]}}},
            {"iid": "8", "approved": true, "detailedMergeStatus": "MERGEABLE",
             "reviewers": {"nodes": []}, "headPipeline": null},
            {"iid": "9", "approved": false, "detailedMergeStatus": "REQUESTED_CHANGES",
             "headPipeline": {"status": "RUNNING", "jobs": {"nodes": []}}}
        ]}}}}"#).unwrap();
        let map = parse_statuses(&json);
        let s = &map[&7];
        assert_eq!(s.checks, Some(CheckState::Failing));
        assert_eq!(s.failing, ["test"]);
        assert_eq!((s.passed, s.pending), (2, 1));
        assert_eq!(s.unresolved, 2);
        assert_eq!(s.review, Some(ReviewDecision::ReviewRequired));
        assert_eq!(map[&8].review, Some(ReviewDecision::Approved));
        assert_eq!(map[&8].checks, None);
        assert_eq!(map[&9].review, Some(ReviewDecision::ChangesRequested));
        assert_eq!(map[&9].checks, Some(CheckState::Pending));
    }
}
