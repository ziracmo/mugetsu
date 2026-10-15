//! Integration tests against real repositories built with the git CLI.

use std::path::Path;
use std::process::Command;

use mugetsu_git::{Change, LineKind, Repo};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// main: c1 - c2 ------- merge
///              \       /
/// feature:      f1 - f2
fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "a.txt", "one\ntwo\nthree\n", "c1");
    commit_file(p, "a.txt", "one\ntwo\nthree\nfour\n", "c2");
    git(p, &["switch", "-q", "-c", "feature"]);
    commit_file(p, "b.txt", "b\n", "f1");
    commit_file(p, "b.txt", "b\nb2\n", "f2");
    git(p, &["switch", "-q", "main"]);
    git(
        p,
        &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
    );
    git(p, &["tag", "-a", "v1.0", "-m", "release"]);
    let repo = Repo::open(p).unwrap();
    (dir, repo)
}

#[test]
fn history_and_graph_follow_the_dag() {
    let (_dir, repo) = fixture();
    let refs = repo.refs().unwrap();
    let history = repo.history(&refs.tips()).unwrap();
    let summaries: Vec<_> = history.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries.len(), 5);
    assert_eq!(summaries[0], "merge feature");
    assert_eq!(*summaries.last().unwrap(), "c1");

    let graph = mugetsu_git::layout(&history.commits);
    assert!(graph.row(0).is_merge());
    assert_eq!(graph.max_width, 2);
}

#[test]
fn refs_snapshot_lists_everything() {
    let (dir, repo) = fixture();
    let wt = dir
        .path()
        .join("../")
        .join(format!("wt-{}", std::process::id()));
    repo.worktree_add(wt.to_str().unwrap(), "wt-branch", true)
        .unwrap();
    std::fs::write(dir.path().join("a.txt"), "dirty\n").unwrap();
    repo.stash_push(Some("wip")).unwrap();

    let refs = repo.refs().unwrap();
    assert_eq!(refs.head_branch.as_deref(), Some("main"));
    let names: Vec<_> = refs.branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["feature", "main", "wt-branch"]);
    assert_eq!(refs.tags.len(), 1);
    assert_eq!(
        refs.tags[0].target,
        refs.head.unwrap(),
        "annotated tag peeled to commit"
    );
    assert_eq!(refs.worktrees.len(), 2);
    assert_eq!(refs.stashes.len(), 1);
    assert!(refs.stashes[0].message.contains("wip"));

    let history = repo.history(&refs.tips()).unwrap();
    assert!(
        history.row_of(&refs.stashes[0].id).is_some(),
        "stash commits are part of the walk"
    );
    repo.worktree_remove(wt.to_str().unwrap()).unwrap();
}

#[test]
fn stage_hunk_commit_cycle() {
    let (dir, repo) = fixture();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), "ONE\ntwo\nthree\nfour\n").unwrap();
    std::fs::write(p.join("new.txt"), "fresh\n").unwrap();

    let status = repo.status().unwrap();
    assert_eq!(status.unstaged().count(), 2);
    let untracked = status.files.iter().find(|f| f.path == "new.txt").unwrap();
    assert_eq!(untracked.worktree, Change::Untracked);
    assert_eq!(
        repo.diff_untracked("new.txt").unwrap().files[0].hunks[0].lines[0].kind,
        LineKind::Added
    );

    let diff = repo.diff_worktree(false, Some("a.txt")).unwrap();
    repo.apply_hunk(&diff.files[0], 0, false).unwrap();
    let staged = repo.diff_worktree(true, None).unwrap();
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].stats(), (1, 1));

    repo.apply_hunk(&staged.files[0], 0, true).unwrap();
    assert_eq!(repo.status().unwrap().staged().count(), 0);

    repo.stage(&["a.txt", "new.txt"]).unwrap();
    repo.unstage(&["new.txt"]).unwrap();
    assert_eq!(repo.status().unwrap().staged().count(), 1);
    repo.commit("edit a\n\nbody", false).unwrap();

    let refs = repo.refs().unwrap();
    let history = repo.history(&refs.tips()).unwrap();
    assert_eq!(history.commits[0].summary, "edit a");
    let diff = repo.diff_commit("HEAD").unwrap();
    assert_eq!(diff.files[0].path(), "a.txt");
}

#[test]
fn fetch_moves_fingerprint() {
    let (dir, repo) = fixture();
    let clone_dir = tempfile::tempdir().unwrap();
    git(
        clone_dir.path(),
        &["clone", "-q", dir.path().to_str().unwrap(), "c"],
    );
    let clone = Repo::open(clone_dir.path().join("c")).unwrap();
    let before = clone.refs_fingerprint().unwrap();

    commit_file(dir.path(), "a.txt", "upstream\n", "upstream change");
    assert_eq!(clone.refs_fingerprint().unwrap(), before);
    clone.fetch_all().unwrap();
    assert_ne!(clone.refs_fingerprint().unwrap(), before);
    let main = clone
        .refs()
        .unwrap()
        .branches
        .into_iter()
        .find(|b| b.name == "main")
        .unwrap();
    assert_eq!(main.behind, 1);
    let _ = repo;
}
