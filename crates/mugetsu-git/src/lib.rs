//! Git engine for Mugetsu.
//!
//! Reads that dominate on large repositories (history walk) use `gix` in-process.
//! Everything else (status, diff, refs, writes, network) goes through the `git`
//! CLI so that user config, hooks, credential helpers and fsmonitor keep working.

pub mod cli;
pub mod diff;
pub mod graph;
pub mod log;
pub mod ops;
pub mod refs;
pub mod status;

use std::path::{Path, PathBuf};

pub use cli::Git;
pub use diff::{DiffFile, DiffLine, FileDiff, Hunk, LineKind};
pub use graph::{Edge, Graph, GraphRow, Lane, Node, Span, layout};
pub use log::{Commit, History};
pub use refs::{Branch, RefLabel, RefsSnapshot, Remote, Stash, Tag, Worktree};
pub use status::{Change, FileStatus, Status};

/// Handle on one repository. Cheap to clone, safe to send to background threads.
#[derive(Clone, Debug)]
pub struct Repo {
    root: PathBuf,
    git: Git,
}

impl Repo {
    /// Opens the repository containing `path` (walks up to the worktree root).
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let out = Git::new(path).run(["rev-parse", "--show-toplevel"])?;
        let root = PathBuf::from(out.trim());
        Ok(Self {
            git: Git::new(&root),
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn git(&self) -> &Git {
        &self.git
    }
}
