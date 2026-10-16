//! Git engine for Mugetsu.
//!
//! Reads that dominate on large repositories (history walk) use `gix` in-process.
//! Everything else (status, diff, refs, writes, network) goes through the `git`
//! CLI so that user config, hooks, credential helpers and fsmonitor keep working.

pub mod batch;
pub mod blame;
pub mod cli;
pub mod clone;
pub mod conflict;
pub mod diff;
pub mod fold;
pub mod graph;
pub mod log;
pub mod ops;
pub mod process;
pub mod rebase;
pub mod refs;
pub mod restack;
pub mod stats;
pub mod status;
pub mod worktrees;

use std::path::{Path, PathBuf};

pub use batch::BatchDelete;
pub use blame::{BlameLine, FileCommit};
pub use cli::{Git, Identity, global_identity, set_global_identity};
pub use clone::{ClonePhase, CloneProgress, clone, clone_name, is_clone_url};
pub use conflict::{Chunk, Conflict, ConflictFile, Operation, OperationState, Resolution};
pub use diff::{DiffFile, DiffLine, FileDiff, Hunk, LineKind};
pub use fold::Folded;
pub use graph::{Edge, Graph, GraphRow, Lane, Node, Span, layout, layout_with};
pub use log::{Commit, CommitOrder, History};
pub use ops::{LocalChanges, PushOptions, ResetMode};
pub use rebase::{RebaseAction, RebaseCommit, RebaseStep};
pub use refs::{Branch, RefLabel, RefsSnapshot, Remote, Stash, Tag, Worktree};
pub use restack::{Restack, StackLink};
pub use stats::{FileHeat, Person, RepoStats};
pub use status::{Change, FileStatus, Status};
pub use worktrees::{DeadReason, DeadWorktree};

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
