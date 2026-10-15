//! Remembers workspaces (named lists of repositories, like Fork) and which
//! one was open between launches.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A named set of repositories opened together as tabs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoSet {
    pub name: String,
    pub repos: Vec<PathBuf>,
    pub active: usize,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    #[serde(default)]
    pub workspaces: Vec<RepoSet>,
    /// Index of the open workspace.
    #[serde(default)]
    pub current: usize,
    /// Format before workspaces existed: read once, then migrated.
    #[serde(default, skip_serializing)]
    repos: Vec<PathBuf>,
    #[serde(default, skip_serializing)]
    active: usize,
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Mugetsu").join("session.json"))
}

pub fn default_name() -> String {
    crate::i18n::tr("Général", "General").to_string()
}

impl Session {
    pub fn new(workspaces: Vec<RepoSet>, current: usize) -> Self {
        Self {
            workspaces,
            current,
            ..Self::default()
        }
        .normalized()
    }

    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<Session>(&b).ok())
            .unwrap_or_default()
            .normalized()
    }

    /// Migrates the old format and guarantees at least one workspace.
    fn normalized(mut self) -> Self {
        if self.workspaces.is_empty() {
            self.workspaces.push(RepoSet {
                name: default_name(),
                repos: std::mem::take(&mut self.repos),
                active: self.active,
            });
        }
        self.current = self.current.min(self.workspaces.len() - 1);
        self
    }

    /// Adds repositories given on the command line to the open workspace
    /// (at the end, the first of them focused).
    pub fn with_args(mut self, args: Vec<PathBuf>) -> Self {
        let canonical = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        let set = &mut self.workspaces[self.current];
        for (i, arg) in args.into_iter().enumerate() {
            let arg = canonical(&arg);
            let index = match set.repos.iter().position(|r| canonical(r) == arg) {
                Some(index) => index,
                None => {
                    set.repos.push(arg);
                    set.repos.len() - 1
                }
            };
            if i == 0 {
                set.active = index;
            }
        }
        self
    }

    pub fn save(&self) {
        let Some(f) = file() else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(f, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_repos_join_the_open_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        for p in [&a, &b, &c] {
            std::fs::create_dir(p).unwrap();
        }
        let session = || {
            Session::new(
                vec![
                    RepoSet {
                        name: "other".into(),
                        repos: vec![],
                        active: 0,
                    },
                    RepoSet {
                        name: "work".into(),
                        repos: vec![a.clone(), b.clone()],
                        active: 1,
                    },
                ],
                1,
            )
        };
        let open = |s: Session| {
            let set = s.workspaces[s.current].clone();
            (set.repos, set.active)
        };
        assert_eq!(
            open(session().with_args(vec![])),
            (vec![a.clone(), b.clone()], 1)
        );
        let (repos, active) = open(session().with_args(vec![c.clone()]));
        assert_eq!(repos.len(), 3);
        assert_eq!(active, 2);
        let (repos, active) = open(session().with_args(vec![a.join("../a")]));
        assert_eq!(repos.len(), 2, "already open repos are not duplicated");
        assert_eq!(active, 0);
    }

    #[test]
    fn migrates_the_single_list_format() {
        let old: Session = serde_json::from_str(r#"{"repos":["/x","/y"],"active":1}"#).unwrap();
        let s = old.normalized();
        assert_eq!(s.workspaces.len(), 1);
        assert_eq!(
            s.workspaces[0].repos,
            [PathBuf::from("/x"), PathBuf::from("/y")]
        );
        assert_eq!(s.workspaces[0].active, 1);
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("\"repos\":[\"/x\",\"/y\"],\"active\":1,\"workspaces\""));
        assert!(json.starts_with("{\"workspaces\""), "{json}");
    }
}
