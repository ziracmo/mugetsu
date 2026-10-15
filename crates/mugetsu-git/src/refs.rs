use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gix::ObjectId;

use crate::Repo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// Short name (`main`, or `origin/main` for remote branches).
    pub name: String,
    pub target: ObjectId,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub is_head: bool,
    pub upstream_gone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// Commit the tag points to (peeled for annotated tags).
    pub target: ObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub head: Option<ObjectId>,
    pub branch: Option<String>,
    pub is_main: bool,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub index: usize,
    pub id: ObjectId,
    pub message: String,
}

/// Every ref-like thing shown in the sidebar, loaded in a handful of git calls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefsSnapshot {
    pub head: Option<ObjectId>,
    pub head_branch: Option<String>,
    pub branches: Vec<Branch>,
    pub remote_branches: Vec<Branch>,
    pub tags: Vec<Tag>,
    pub remotes: Vec<Remote>,
    pub worktrees: Vec<Worktree>,
    pub stashes: Vec<Stash>,
    /// Hash of all ref targets; changes iff something moved (used after background fetch).
    pub fingerprint: u64,
}

impl RefsSnapshot {
    /// Commit ids to start the history walk from.
    pub fn tips(&self) -> Vec<ObjectId> {
        let mut tips: Vec<ObjectId> = self
            .branches
            .iter()
            .chain(&self.remote_branches)
            .map(|b| b.target)
            .chain(self.tags.iter().map(|t| t.target))
            .chain(self.stashes.iter().map(|s| s.id))
            .chain(self.head)
            .collect();
        tips.sort();
        tips.dedup();
        tips
    }

    /// Labels per commit for decorating history rows.
    pub fn decorations(&self) -> HashMap<ObjectId, Vec<RefLabel>> {
        let mut map: HashMap<ObjectId, Vec<RefLabel>> = HashMap::new();
        for b in &self.branches {
            map.entry(b.target).or_default().push(RefLabel::Branch {
                name: b.name.clone(),
                head: b.is_head,
            });
        }
        for b in &self.remote_branches {
            map.entry(b.target)
                .or_default()
                .push(RefLabel::Remote(b.name.clone()));
        }
        for t in &self.tags {
            map.entry(t.target)
                .or_default()
                .push(RefLabel::Tag(t.name.clone()));
        }
        if self.head_branch.is_none()
            && let Some(h) = self.head
        {
            map.entry(h).or_default().insert(0, RefLabel::DetachedHead);
        }
        map
    }

    /// Parses `for-each-ref` output produced with [`FOR_EACH_REF_FORMAT`].
    pub fn parse_refs(&mut self, raw: &str) {
        let mut hasher = DefaultHasher::new();
        for line in raw.lines() {
            line.hash(&mut hasher);
            let f: Vec<&str> = line.split('\0').collect();
            if f.len() < 6 {
                continue;
            }
            let (refname, oid, peeled, upstream, track, head) =
                (f[0], f[1], f[2], f[3], f[4], f[5]);
            let Ok(oid) = ObjectId::from_hex(oid.as_bytes()) else {
                continue;
            };
            if let Some(name) = refname.strip_prefix("refs/heads/") {
                let (ahead, behind, gone) = parse_track(track);
                let is_head = head == "*";
                if is_head {
                    self.head_branch = Some(name.to_string());
                }
                self.branches.push(Branch {
                    name: name.to_string(),
                    target: oid,
                    upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                    ahead,
                    behind,
                    is_head,
                    upstream_gone: gone,
                });
            } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
                if name.ends_with("/HEAD") {
                    continue;
                }
                self.remote_branches.push(Branch {
                    name: name.to_string(),
                    target: oid,
                    upstream: None,
                    ahead: 0,
                    behind: 0,
                    is_head: false,
                    upstream_gone: false,
                });
            } else if let Some(name) = refname.strip_prefix("refs/tags/") {
                let target = ObjectId::from_hex(peeled.as_bytes()).unwrap_or(oid);
                self.tags.push(Tag {
                    name: name.to_string(),
                    target,
                });
            }
        }
        self.fingerprint = hasher.finish();
    }
}

/// Decoration shown next to a commit summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefLabel {
    Branch { name: String, head: bool },
    Remote(String),
    Tag(String),
    DetachedHead,
}

pub const FOR_EACH_REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(*objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(HEAD)";

fn parse_track(track: &str) -> (u32, u32, bool) {
    if track == "gone" {
        return (0, 0, true);
    }
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

pub fn parse_remotes(raw: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(url)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !remotes.iter().any(|r| r.name == name) {
            remotes.push(Remote {
                name: name.to_string(),
                url: url.to_string(),
            });
        }
    }
    remotes
}

/// Parses `git worktree list --porcelain -z`.
pub fn parse_worktrees(raw: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in raw.split('\0') {
        if field.is_empty() {
            if let Some(wt) = current.take() {
                out.push(wt);
            }
            continue;
        }
        if let Some(path) = field.strip_prefix("worktree ") {
            if let Some(wt) = current.take() {
                out.push(wt);
            }
            current = Some(Worktree {
                path: path.to_string(),
                head: None,
                branch: None,
                is_main: out.is_empty(),
                locked: false,
                prunable: false,
            });
        } else if let Some(wt) = current.as_mut() {
            if let Some(h) = field.strip_prefix("HEAD ") {
                wt.head = ObjectId::from_hex(h.as_bytes()).ok();
            } else if let Some(b) = field.strip_prefix("branch ") {
                wt.branch = Some(b.trim_start_matches("refs/heads/").to_string());
            } else if field.starts_with("locked") {
                wt.locked = true;
            } else if field.starts_with("prunable") {
                wt.prunable = true;
            }
        }
    }
    if let Some(wt) = current {
        out.push(wt);
    }
    out
}

pub fn parse_stashes(raw: &str) -> Vec<Stash> {
    raw.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let (id, message) = line.split_once('\0')?;
            Some(Stash {
                index,
                id: ObjectId::from_hex(id.as_bytes()).ok()?,
                message: message.to_string(),
            })
        })
        .collect()
}

impl Repo {
    pub fn refs(&self) -> anyhow::Result<RefsSnapshot> {
        let git = self.git();
        let mut snap = RefsSnapshot::default();
        snap.parse_refs(&git.run([
            "for-each-ref",
            FOR_EACH_REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?);
        snap.head = git
            .run(["rev-parse", "--verify", "-q", "HEAD"])
            .ok()
            .and_then(|s| ObjectId::from_hex(s.trim().as_bytes()).ok());
        snap.remotes = parse_remotes(&git.run(["remote", "-v"])?);
        snap.worktrees = parse_worktrees(&git.run(["worktree", "list", "--porcelain", "-z"])?);
        snap.stashes = parse_stashes(
            &git.run(["stash", "list", "--format=%H%x00%gs"])
                .unwrap_or_default(),
        );
        let mut h = DefaultHasher::new();
        snap.fingerprint.hash(&mut h);
        snap.head.hash(&mut h);
        snap.fingerprint = h.finish();
        Ok(snap)
    }

    /// Cheap check used by background polling: hash of all ref targets.
    pub fn refs_fingerprint(&self) -> anyhow::Result<u64> {
        let mut snap = RefsSnapshot::default();
        snap.parse_refs(&self.git().run([
            "for-each-ref",
            FOR_EACH_REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])?);
        Ok(snap.fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn parses_branches_tags_and_tracking() {
        let raw = format!(
            "refs/heads/main\0{A}\0\0origin/main\0ahead 1, behind 2\0*\n\
             refs/heads/old\0{A}\0\0origin/old\0gone\0 \n\
             refs/remotes/origin/HEAD\0{A}\0\0\0\0 \n\
             refs/remotes/origin/main\0{B}\0\0\0\0 \n\
             refs/tags/v1\0{A}\0{B}\0\0\0 \n"
        );
        let mut s = RefsSnapshot::default();
        s.parse_refs(&raw);
        assert_eq!(s.head_branch.as_deref(), Some("main"));
        assert_eq!(s.branches.len(), 2);
        assert_eq!((s.branches[0].ahead, s.branches[0].behind), (1, 2));
        assert!(s.branches[1].upstream_gone);
        assert_eq!(s.remote_branches.len(), 1, "origin/HEAD is skipped");
        assert_eq!(s.tags[0].target.to_string(), B, "annotated tag is peeled");
        assert_eq!(s.tips().len(), 2);
    }

    #[test]
    fn fingerprint_changes_when_a_ref_moves() {
        let mut a = RefsSnapshot::default();
        a.parse_refs(&format!("refs/heads/main\0{A}\0\0\0\0*\n"));
        let mut b = RefsSnapshot::default();
        b.parse_refs(&format!("refs/heads/main\0{B}\0\0\0\0*\n"));
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn parses_worktrees() {
        let raw = format!(
            "worktree /repo\0HEAD {A}\0branch refs/heads/main\0\0worktree /repo-wt\0HEAD {B}\0detached\0locked\0\0"
        );
        let w = parse_worktrees(&raw);
        assert_eq!(w.len(), 2);
        assert!(w[0].is_main && !w[1].is_main);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert!(w[1].branch.is_none() && w[1].locked);
    }

    #[test]
    fn parses_remotes_once_per_name() {
        let r = parse_remotes(
            "origin\tgit@github.com:a/b.git (fetch)\norigin\tgit@github.com:a/b.git (push)\n",
        );
        assert_eq!(
            r,
            vec![Remote {
                name: "origin".into(),
                url: "git@github.com:a/b.git".into()
            }]
        );
    }
}
