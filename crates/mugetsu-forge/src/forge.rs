//! Forge-neutral view of a remote: which service hosts it (GitHub, GitLab,
//! Bitbucket), its web links, and the [`Forge`] trait each service implements.

use std::collections::HashMap;

use crate::{GithubRepo, PrDetails, PrStatus, PullRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForgeKind {
    Github,
    Gitlab,
    Bitbucket,
}

impl ForgeKind {
    pub fn label(self) -> &'static str {
        match self {
            ForgeKind::Github => "GitHub",
            ForgeKind::Gitlab => "GitLab",
            ForgeKind::Bitbucket => "Bitbucket",
        }
    }

    /// What the service calls a pull request.
    pub fn pr_noun(self) -> &'static str {
        match self {
            ForgeKind::Gitlab => "Merge Request",
            _ => "Pull Request",
        }
    }

    /// `#12` on GitHub and Bitbucket, `!12` on GitLab.
    pub fn pr_ref(self, number: u64) -> String {
        match self {
            ForgeKind::Gitlab => format!("!{number}"),
            _ => format!("#{number}"),
        }
    }
}

/// A repository on a forge. GitLab namespaces can be nested (`group/sub`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ForgeRepo {
    pub kind: ForgeKind,
    /// Web host, with its port when the remote is HTTP(S) on a custom one.
    pub host: String,
    /// Owner, workspace or (GitLab) namespace path.
    pub owner: String,
    pub name: String,
}

impl ForgeRepo {
    /// Recognises GitHub (github.com), Bitbucket (bitbucket.org) and GitLab
    /// (gitlab.com, any `gitlab.*` host, or one of `gitlab_hosts`) remotes, over
    /// SSH (`git@host:path`, `ssh://…`) or HTTP(S).
    pub fn from_remote_url(url: &str, gitlab_hosts: &[String]) -> Option<Self> {
        let (host, path) = split_remote(url.trim())?;
        let path = path.trim_matches('/').trim_end_matches(".git");
        let bare = host.split(':').next().unwrap_or(&host).to_lowercase();
        let kind = if bare == "github.com" {
            ForgeKind::Github
        } else if bare == "bitbucket.org" {
            ForgeKind::Bitbucket
        } else if bare.starts_with("gitlab.") || listed(&host, gitlab_hosts) {
            ForgeKind::Gitlab
        } else {
            return None;
        };
        let (owner, name) = path.rsplit_once('/')?;
        let nested = owner.contains('/');
        if owner.is_empty() || name.is_empty() || (nested && kind != ForgeKind::Gitlab) {
            return None;
        }
        let host = if kind == ForgeKind::Gitlab {
            host
        } else {
            bare
        };
        Some(Self {
            kind,
            host,
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    /// The same repository for GitHub-only code.
    pub fn github(&self) -> Option<GithubRepo> {
        (self.kind == ForgeKind::Github).then(|| GithubRepo {
            owner: self.owner.clone(),
            name: self.name.clone(),
        })
    }

    /// `owner/name` (GitLab: full project path).
    pub fn path(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn web_url(&self) -> String {
        format!("https://{}/{}", self.host, self.path())
    }

    /// Page that opens a new pull / merge request from `branch`.
    pub fn new_pr_url(&self, branch: &str) -> String {
        let base = self.web_url();
        match self.kind {
            ForgeKind::Github => format!("{base}/compare/{branch}?expand=1"),
            ForgeKind::Gitlab => format!(
                "{base}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}",
                encode(branch)
            ),
            ForgeKind::Bitbucket => format!("{base}/pull-requests/new?source={}", encode(branch)),
        }
    }

    pub fn commit_url(&self, sha: &str) -> String {
        let base = self.web_url();
        match self.kind {
            ForgeKind::Github => format!("{base}/commit/{sha}"),
            ForgeKind::Gitlab => format!("{base}/-/commit/{sha}"),
            ForgeKind::Bitbucket => format!("{base}/commits/{sha}"),
        }
    }

    /// A file at `rev`, optionally pointing at a line.
    pub fn file_url(&self, rev: &str, path: &str, line: Option<u32>) -> String {
        let base = self.web_url();
        let path: Vec<String> = path.split('/').map(encode).collect();
        let path = path.join("/");
        let (url, anchor) = match self.kind {
            ForgeKind::Github => (format!("{base}/blob/{rev}/{path}"), "#L"),
            ForgeKind::Gitlab => (format!("{base}/-/blob/{rev}/{path}"), "#L"),
            ForgeKind::Bitbucket => (format!("{base}/src/{rev}/{path}"), "#lines-"),
        };
        match line {
            Some(l) => format!("{url}{anchor}{l}"),
            None => url,
        }
    }
}

/// Whether `host` is gitlab.com or one of the servers in `gitlab_hosts`.
fn listed(host: &str, gitlab_hosts: &[String]) -> bool {
    let bare = host.split(':').next().unwrap_or(host);
    bare.eq_ignore_ascii_case("gitlab.com")
        || gitlab_hosts.iter().any(|h| {
            let h = h
                .trim()
                .trim_start_matches("https://")
                .trim_end_matches('/');
            !h.is_empty() && (h.eq_ignore_ascii_case(host) || h.eq_ignore_ascii_case(bare))
        })
}

/// Whether a GitLab token that isn't tied to a host (Settings, `GITLAB_TOKEN`)
/// may be sent to `host`: gitlab.com and the servers the user listed, not any
/// `gitlab.*` remote, which anyone can name a server.
pub fn gitlab_token_allowed(host: &str, gitlab_hosts: &[String]) -> bool {
    listed(host, gitlab_hosts)
}

/// `(host, path)` of a remote URL; the host keeps an HTTP(S) port.
fn split_remote(url: &str) -> Option<(String, String)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = match scheme {
            "http" | "https" => host,
            // SSH ports are not web ports.
            _ => host.split(':').next()?,
        };
        return (!host.is_empty()).then(|| (host.to_lowercase(), path.to_string()));
    }
    // scp-like: [user@]host:path
    let (authority, path) = url.split_once(':')?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.is_empty() || host.contains('/') || path.starts_with("//") {
        return None;
    }
    Some((host.to_lowercase(), path.to_string()))
}

/// Percent-encodes everything but unreserved characters.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A repository the token's owner can reach, offered for cloning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRepo {
    pub kind: ForgeKind,
    /// `owner/name` (GitLab: full project path).
    pub full_name: String,
    pub description: String,
    pub private: bool,
    pub https_url: String,
    pub ssh_url: String,
    /// ISO 8601, last push or activity.
    pub updated_at: String,
}

impl RemoteRepo {
    /// Last segment of the path: the folder a clone goes into.
    pub fn name(&self) -> &str {
        self.full_name.rsplit('/').next().unwrap_or(&self.full_name)
    }
}

/// Pull / merge requests of one forge, the same shape whatever the service.
pub trait Forge: Send + Sync {
    /// The most recently updated PRs, newest first.
    fn recent_pulls(&self, repo: &ForgeRepo) -> anyhow::Result<Vec<PullRequest>>;
    /// Most relevant PR whose head is `branch` (a branch of the repository itself).
    fn pull_for_branch(
        &self,
        repo: &ForgeRepo,
        branch: &str,
    ) -> anyhow::Result<Option<PullRequest>>;
    /// CI / review state of the open PRs, by number.
    fn pr_statuses(&self, repo: &ForgeRepo) -> anyhow::Result<HashMap<u64, PrStatus>>;
    /// Description, labels and size of PR `number`.
    fn pull_details(&self, repo: &ForgeRepo, number: u64) -> anyhow::Result<PrDetails>;
    /// Login of the token's owner.
    fn viewer_login(&self) -> anyhow::Result<String>;
    /// Repositories the token's owner owns or is a member of, most recently
    /// active first.
    fn user_repos(&self) -> anyhow::Result<Vec<RemoteRepo>>;
}

/// Pages fetched at most when listing repositories (100 per page).
pub(crate) const REPO_PAGES: usize = 3;

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(kind: ForgeKind, host: &str, owner: &str, name: &str) -> Option<ForgeRepo> {
        Some(ForgeRepo {
            kind,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        })
    }

    #[test]
    fn detects_the_forge_from_the_remote() {
        let none: &[String] = &[];
        let gh = repo(ForgeKind::Github, "github.com", "ziracmo", "mugetsu");
        for url in [
            "git@github.com:ziracmo/mugetsu.git",
            "ssh://git@github.com/ziracmo/mugetsu",
            "https://github.com/ziracmo/mugetsu.git",
            "https://token@github.com/ziracmo/mugetsu/",
        ] {
            assert_eq!(ForgeRepo::from_remote_url(url, none), gh, "{url}");
        }
        let gl = repo(ForgeKind::Gitlab, "gitlab.com", "group/sub", "proj");
        for url in [
            "git@gitlab.com:group/sub/proj.git",
            "https://gitlab.com/group/sub/proj",
            "ssh://git@gitlab.com:22/group/sub/proj.git",
        ] {
            assert_eq!(ForgeRepo::from_remote_url(url, none), gl, "{url}");
        }
        let bb = repo(ForgeKind::Bitbucket, "bitbucket.org", "team", "app");
        for url in [
            "git@bitbucket.org:team/app.git",
            "https://alex@bitbucket.org/team/app.git",
        ] {
            assert_eq!(ForgeRepo::from_remote_url(url, none), bb, "{url}");
        }
        assert_eq!(
            ForgeRepo::from_remote_url("https://gitlab.acme.io:8443/a/b.git", none),
            repo(ForgeKind::Gitlab, "gitlab.acme.io:8443", "a", "b")
        );
        let hosts = vec!["https://git.acme.io/".to_string()];
        assert_eq!(
            ForgeRepo::from_remote_url("git@git.acme.io:team/b.git", &hosts),
            repo(ForgeKind::Gitlab, "git.acme.io", "team", "b")
        );
        assert_eq!(
            ForgeRepo::from_remote_url("git@git.acme.io:team/b.git", none),
            None
        );
        assert_eq!(
            ForgeRepo::from_remote_url("https://github.com/a/b/c", none),
            None
        );
        assert_eq!(
            ForgeRepo::from_remote_url("https://github.com/only", none),
            None
        );
        assert_eq!(ForgeRepo::from_remote_url("/local/path/repo", none), None);
    }

    #[test]
    fn web_links() {
        let gl = repo(ForgeKind::Gitlab, "gitlab.com", "g/s", "p").unwrap();
        assert_eq!(
            gl.new_pr_url("feat/x"),
            "https://gitlab.com/g/s/p/-/merge_requests/new?merge_request%5Bsource_branch%5D=feat%2Fx"
        );
        assert_eq!(
            gl.file_url("abc", "src/a b.rs", Some(3)),
            "https://gitlab.com/g/s/p/-/blob/abc/src/a%20b.rs#L3"
        );
        let bb = repo(ForgeKind::Bitbucket, "bitbucket.org", "t", "a").unwrap();
        assert_eq!(
            bb.new_pr_url("x"),
            "https://bitbucket.org/t/a/pull-requests/new?source=x"
        );
        assert_eq!(
            bb.file_url("abc", "f.rs", Some(9)),
            "https://bitbucket.org/t/a/src/abc/f.rs#lines-9"
        );
        assert_eq!(
            bb.commit_url("abc"),
            "https://bitbucket.org/t/a/commits/abc"
        );
        let gh = repo(ForgeKind::Github, "github.com", "o", "r").unwrap();
        assert_eq!(
            gh.new_pr_url("feat/x"),
            "https://github.com/o/r/compare/feat/x?expand=1"
        );
        assert_eq!(gh.github().unwrap().owner, "o");
        assert_eq!(ForgeKind::Gitlab.pr_ref(4), "!4");
    }
}
