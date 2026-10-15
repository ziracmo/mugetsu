//! Remembers open repositories between launches.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub repos: Vec<PathBuf>,
    pub active: usize,
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Mugetsu").join("session.json"))
}

impl Session {
    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Repositories to open at launch: the previous session, plus any given on
    /// the command line (added at the end, the first of them focused).
    pub fn with_args(self, args: Vec<PathBuf>) -> (Vec<PathBuf>, usize) {
        let canonical = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        let mut repos = self.repos;
        let mut active = self.active;
        for (i, arg) in args.into_iter().enumerate() {
            let arg = canonical(&arg);
            let index = match repos.iter().position(|r| canonical(r) == arg) {
                Some(index) => index,
                None => {
                    repos.push(arg);
                    repos.len() - 1
                }
            };
            if i == 0 {
                active = index;
            }
        }
        (repos, active)
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
    fn command_line_repos_join_the_previous_session() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        for p in [&a, &b, &c] {
            std::fs::create_dir(p).unwrap();
        }
        let session = || Session {
            repos: vec![a.clone(), b.clone()],
            active: 1,
        };
        assert_eq!(session().with_args(vec![]), (vec![a.clone(), b.clone()], 1));
        let (repos, active) = session().with_args(vec![c.clone()]);
        assert_eq!(repos.len(), 3);
        assert_eq!(active, 2);
        let (repos, active) = session().with_args(vec![a.join("../a")]);
        assert_eq!(repos.len(), 2, "already open repos are not duplicated");
        assert_eq!(active, 0);
    }
}
