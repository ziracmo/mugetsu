//! Write operations (stage, commit, branches, tags, stashes, worktrees, remotes)
//! against real repositories built with the git CLI.
#![allow(clippy::disallowed_methods)] // test fixtures

use std::path::Path;
use std::process::Command;

use mugetsu_git::{Change, PushOptions, Repo, ResetMode};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
}

/// main: c1 - c2
fn fixture() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    commit_file(dir.path(), "a.txt", "one\n", "c1");
    commit_file(dir.path(), "a.txt", "one\ntwo\n", "c2");
    let repo = Repo::open(dir.path()).unwrap();
    (dir, repo)
}

fn change_of(repo: &Repo, path: &str) -> Option<(Change, Change)> {
    repo.status()
        .unwrap()
        .files
        .into_iter()
        .find(|f| f.path == path)
        .map(|f| (f.index, f.worktree))
}

#[test]
fn stage_unstage_and_discard() {
    let (dir, repo) = fixture();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), "changed\n").unwrap();
    std::fs::write(p.join("new.txt"), "new\n").unwrap();
    assert!(repo.is_dirty().unwrap());
    assert_eq!(
        change_of(&repo, "a.txt"),
        Some((Change::Unmodified, Change::Modified))
    );
    assert_eq!(
        change_of(&repo, "new.txt").map(|c| c.1),
        Some(Change::Untracked)
    );

    repo.stage(&["a.txt", "new.txt"]).unwrap();
    assert_eq!(
        change_of(&repo, "a.txt").map(|c| c.0),
        Some(Change::Modified)
    );
    assert_eq!(
        change_of(&repo, "new.txt").map(|c| c.0),
        Some(Change::Added)
    );

    repo.unstage(&["a.txt"]).unwrap();
    assert_eq!(
        change_of(&repo, "a.txt"),
        Some((Change::Unmodified, Change::Modified))
    );

    repo.discard(&["a.txt"]).unwrap();
    assert_eq!(change_of(&repo, "a.txt"), None);
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    repo.stage_all().unwrap();
    repo.unstage(&["new.txt"]).unwrap();
    assert_eq!(
        change_of(&repo, "new.txt").map(|c| c.1),
        Some(Change::Untracked)
    );
}

#[test]
fn unstage_works_on_an_unborn_branch() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    std::fs::write(dir.path().join("first.txt"), "x\n").unwrap();
    let repo = Repo::open(dir.path()).unwrap();
    repo.stage(&["first.txt"]).unwrap();
    assert_eq!(
        change_of(&repo, "first.txt").map(|c| c.0),
        Some(Change::Added)
    );
    repo.unstage(&["first.txt"]).unwrap();
    assert_eq!(
        change_of(&repo, "first.txt").map(|c| c.1),
        Some(Change::Untracked)
    );
}

#[test]
fn commit_strips_comments_and_amends() {
    let (dir, repo) = fixture();
    let p = dir.path();
    std::fs::write(p.join("b.txt"), "b\n").unwrap();
    repo.stage_all().unwrap();
    repo.commit("Add b\n\n# a comment git strips\nBody line\n", false)
        .unwrap();
    assert_eq!(git(p, &["log", "-1", "--format=%B"]), "Add b\n\nBody line");
    let count = git(p, &["rev-list", "--count", "HEAD"]);

    repo.commit("Add b, reworded", true).unwrap();
    assert_eq!(git(p, &["log", "-1", "--format=%s"]), "Add b, reworded");
    assert_eq!(git(p, &["rev-list", "--count", "HEAD"]), count, "amend");

    assert!(repo.commit("nothing staged", false).is_err());
}

#[test]
fn branches_are_created_switched_and_deleted() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let head = || repo.refs().unwrap().head_branch.unwrap();

    repo.create_branch("from-c1", Some("HEAD~1"), false)
        .unwrap();
    assert!(repo.has_local_branch("from-c1"));
    assert_eq!(head(), "main", "created without checkout");
    assert_eq!(
        git(p, &["rev-parse", "from-c1"]),
        git(p, &["rev-parse", "HEAD~1"])
    );

    repo.create_branch("work", None, true).unwrap();
    assert_eq!(head(), "work");
    commit_file(p, "w.txt", "w\n", "work");
    repo.checkout("main").unwrap();

    assert!(
        repo.delete_branch("work", false).is_err(),
        "unmerged branch needs force"
    );
    repo.delete_branch("work", true).unwrap();
    repo.delete_branch("from-c1", false).unwrap();
    assert!(!repo.has_local_branch("work"));
    assert!(!repo.has_local_branch("from-c1"));
}

#[test]
fn tags_lightweight_and_annotated() {
    let (dir, repo) = fixture();
    let p = dir.path();
    repo.create_tag("light", "HEAD~1", None).unwrap();
    repo.create_tag("v1", "HEAD", Some("first release"))
        .unwrap();
    let tags: Vec<_> = repo
        .refs()
        .unwrap()
        .tags
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(tags.contains(&"light".to_string()), "{tags:?}");
    assert!(tags.contains(&"v1".to_string()), "{tags:?}");
    assert_eq!(git(p, &["cat-file", "-t", "v1"]), "tag");
    assert_eq!(git(p, &["cat-file", "-t", "light"]), "commit");

    repo.delete_tag("light").unwrap();
    assert!(repo.refs().unwrap().tags.iter().all(|t| t.name != "light"));
}

#[test]
fn stashes_push_pop_drop_and_clear() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let stashes = || repo.refs().unwrap().stashes;

    std::fs::write(p.join("a.txt"), "first\n").unwrap();
    repo.stash_push(Some("first")).unwrap();
    std::fs::write(p.join("untracked.txt"), "u\n").unwrap();
    repo.stash_push(None).unwrap();
    assert!(!repo.is_dirty().unwrap(), "untracked files are stashed too");
    assert_eq!(stashes().len(), 2);
    assert!(stashes()[1].message.contains("first"), "{:?}", stashes()[1]);

    repo.stash_pop(1).unwrap();
    assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "first\n");
    assert_eq!(stashes().len(), 1);
    repo.discard(&["a.txt"]).unwrap();

    repo.stash_drop(0).unwrap();
    assert!(stashes().is_empty());
    assert!(!p.join("untracked.txt").exists());

    std::fs::write(p.join("a.txt"), "again\n").unwrap();
    repo.stash_push(None).unwrap();
    std::fs::write(p.join("a.txt"), "and again\n").unwrap();
    repo.stash_push(None).unwrap();
    repo.stash_clear().unwrap();
    assert!(stashes().is_empty());
}

#[test]
fn worktrees_and_remotes() {
    let (dir, repo) = fixture();
    let wt_dir = tempfile::tempdir().unwrap();
    let wt = wt_dir.path().join("side");
    let wt = wt.to_str().unwrap();

    repo.worktree_add(wt, "side", true).unwrap();
    let worktrees = repo.refs().unwrap().worktrees;
    assert_eq!(worktrees.len(), 2, "{worktrees:?}");
    assert!(
        worktrees
            .iter()
            .any(|w| !w.is_main && w.branch.as_deref() == Some("side"))
    );
    assert!(
        repo.checkout("side").is_err(),
        "branch is checked out elsewhere"
    );
    repo.worktree_remove(wt).unwrap();
    assert_eq!(repo.refs().unwrap().worktrees.len(), 1);

    repo.worktree_add(wt, "side", false).unwrap();
    assert_eq!(repo.refs().unwrap().worktrees.len(), 2);
    repo.worktree_remove(wt).unwrap();

    let url = dir.path().to_str().unwrap();
    repo.remote_add("mirror", url).unwrap();
    let remotes = repo.refs().unwrap().remotes;
    assert!(remotes.iter().any(|r| r.name == "mirror"), "{remotes:?}");
    repo.remote_remove("mirror").unwrap();
    assert!(repo.refs().unwrap().remotes.is_empty());
}

#[test]
fn cherry_pick_and_resets() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["switch", "-q", "-c", "side", "HEAD~1"]);
    commit_file(p, "s.txt", "s\n", "side work");
    let side = git(p, &["rev-parse", "HEAD"]);
    repo.checkout("main").unwrap();

    repo.cherry_pick(&side, false).unwrap();
    assert_eq!(git(p, &["log", "-1", "--format=%s"]), "side work");
    assert!(p.join("s.txt").exists());

    // Soft keeps the change staged, mixed unstages it, hard drops it.
    repo.reset("HEAD~1", ResetMode::Soft).unwrap();
    assert_eq!(change_of(&repo, "s.txt").map(|c| c.0), Some(Change::Added));
    repo.reset("HEAD", ResetMode::Mixed).unwrap();
    assert_eq!(
        change_of(&repo, "s.txt").map(|c| c.1),
        Some(Change::Untracked)
    );
    repo.stage_all().unwrap();
    repo.reset("HEAD", ResetMode::Hard).unwrap();
    assert!(!repo.is_dirty().unwrap());
}

#[test]
fn push_branches_and_tags_then_delete_them_on_the_remote() {
    let remote_dir = tempfile::tempdir().unwrap();
    git(remote_dir.path(), &["init", "-q", "--bare", "-b", "main"]);
    let (dir, repo) = fixture();
    let p = dir.path();
    let remote = remote_dir.path();
    repo.remote_add("origin", remote.to_str().unwrap()).unwrap();

    repo.push(Some(("origin", "main"))).unwrap();
    assert_eq!(
        git(p, &["rev-parse", "--abbrev-ref", "main@{upstream}"]),
        "origin/main"
    );
    commit_file(p, "c3.txt", "3\n", "c3");
    repo.push(None).unwrap();
    assert_eq!(
        git(remote, &["rev-parse", "main"]),
        git(p, &["rev-parse", "HEAD"])
    );

    repo.create_branch("feature", None, false).unwrap();
    repo.push_branch("origin", "feature", true).unwrap();
    assert_eq!(
        git(p, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]),
        "origin/feature"
    );
    repo.push_to(
        "origin",
        "feature",
        "renamed-on-remote",
        PushOptions::default(),
    )
    .unwrap();
    assert!(!git(remote, &["branch", "--list", "renamed-on-remote"]).is_empty());

    repo.create_tag("v1", "HEAD", None).unwrap();
    repo.push_tag("origin", "v1").unwrap();
    assert_eq!(git(remote, &["tag", "--list"]), "v1");

    repo.delete_remote_branch("origin", "feature").unwrap();
    assert!(git(remote, &["branch", "--list", "feature"]).is_empty());
    repo.fetch_all().unwrap();
    assert!(
        repo.refs()
            .unwrap()
            .remote_branches
            .iter()
            .all(|b| b.name != "origin/feature"),
        "pruned after fetch"
    );
}

#[test]
fn pull_and_fast_forward_refuse_diverged_history() {
    let (dir, _repo) = fixture();
    let clone_dir = tempfile::tempdir().unwrap();
    git(
        clone_dir.path(),
        &["clone", "-q", dir.path().to_str().unwrap(), "c"],
    );
    let c = clone_dir.path().join("c");
    git(&c, &["config", "user.name", "Test"]);
    git(&c, &["config", "user.email", "test@example.com"]);
    let clone = Repo::open(&c).unwrap();

    commit_file(dir.path(), "up.txt", "up\n", "upstream");
    clone.pull().unwrap();
    assert_eq!(git(&c, &["log", "-1", "--format=%s"]), "upstream");

    commit_file(dir.path(), "up2.txt", "up\n", "upstream 2");
    commit_file(&c, "local.txt", "l\n", "local");
    clone.fetch_all().unwrap();
    assert!(clone.pull().is_err(), "diverged: not a fast-forward");
    assert!(clone.fast_forward("main", "origin/main", true).is_err());

    clone
        .create_branch("behind", Some("HEAD~1"), false)
        .unwrap();
    clone.fast_forward("behind", "origin/main", false).unwrap();
    assert_eq!(
        git(&c, &["rev-parse", "behind"]),
        git(&c, &["rev-parse", "origin/main"])
    );
}

#[test]
fn dead_worktrees_and_purge() {
    let (dir, repo) = fixture();
    let p = dir.path();
    let wt_dir = tempfile::tempdir().unwrap();
    let wt = |name: &str| wt_dir.path().join(name);
    for name in ["fresh", "done", "gone", "wip"] {
        repo.worktree_add(wt(name).to_str().unwrap(), name, true)
            .unwrap();
    }
    // done: one commit, fast-forwarded into main.
    commit_file(&wt("done"), "d.txt", "d\n", "done work");
    git(p, &["merge", "-q", "--ff-only", "done"]);
    std::fs::write(wt("done").join("scratch.txt"), "left over\n").unwrap();
    // wip: work not merged anywhere.
    commit_file(&wt("wip"), "w.txt", "w\n", "wip work");
    // gone: folder deleted by hand.
    std::fs::remove_dir_all(wt("gone")).unwrap();

    let refs = repo.refs().unwrap();
    let mut dead = repo.dead_worktrees(&refs);
    dead.sort_by(|a, b| a.branch.cmp(&b.branch));
    let found: Vec<_> = dead
        .iter()
        .map(|d| (d.branch.as_deref().unwrap_or_default(), d.reason))
        .collect();
    assert_eq!(
        found,
        [
            ("done", mugetsu_git::DeadReason::Merged),
            ("gone", mugetsu_git::DeadReason::Missing)
        ]
    );
    assert!(dead[0].bytes > 0 && dead[0].dirty, "{:?}", dead[0]);
    assert_eq!((dead[1].bytes, dead[1].dirty), (0, false));

    for d in &dead {
        repo.worktree_purge(&d.path).unwrap();
    }
    let left: Vec<_> = repo
        .refs()
        .unwrap()
        .worktrees
        .into_iter()
        .filter_map(|w| w.branch)
        .collect();
    assert_eq!(left, ["main", "fresh", "wip"]);
    assert!(!wt("done").exists());
}

#[test]
fn batch_delete_reports_unmerged_branches() {
    let (dir, repo) = fixture();
    let p = dir.path();
    git(p, &["branch", "merged"]);
    git(p, &["switch", "-q", "-c", "unmerged"]);
    commit_file(p, "u.txt", "u\n", "u");
    git(p, &["switch", "-q", "main"]);

    let names = vec!["merged".to_string(), "unmerged".to_string()];
    let out = repo.delete_branches(&names, false);
    assert_eq!(out.deleted, vec!["merged".to_string()]);
    assert_eq!(out.unmerged, vec!["unmerged".to_string()]);
    assert_eq!(out.failed.len(), 1);
    assert!(repo.has_local_branch("unmerged"));

    let out = repo.delete_branches(&out.unmerged, true);
    assert_eq!(out.deleted, vec!["unmerged".to_string()]);
    assert!(!repo.has_local_branch("unmerged"));
}

#[test]
fn batch_rebase_moves_every_branch_or_none() {
    let (dir, repo) = fixture();
    let p = dir.path();
    for b in ["one", "two"] {
        git(p, &["switch", "-q", "-c", b, "main~1"]);
        commit_file(p, &format!("{b}.txt"), "x\n", b);
    }
    git(p, &["switch", "-q", "-c", "clash", "main~1"]);
    commit_file(p, "a.txt", "one\nclash\n", "clash");
    git(p, &["switch", "-q", "main"]);
    let main = git(p, &["rev-parse", "main"]);
    let parent = |b: &str| git(p, &["rev-parse", &format!("{b}~1")]);

    // A conflict on the last branch: nothing moves, HEAD stays on main.
    let tips: Vec<String> = ["one", "two"]
        .iter()
        .map(|b| git(p, &["rev-parse", b]))
        .collect();
    let all = ["one", "two", "clash"].map(String::from).to_vec();
    assert!(repo.rebase_branches("main", &all).is_err());
    assert_eq!(git(p, &["rev-parse", "one"]), tips[0]);
    assert_eq!(git(p, &["rev-parse", "two"]), tips[1]);
    assert_eq!(repo.refs().unwrap().head_branch.as_deref(), Some("main"));
    assert!(repo.conflicted_paths().unwrap().is_empty());

    let names = ["one", "two"].map(String::from).to_vec();
    repo.rebase_branches("main", &names).unwrap();
    assert_eq!(parent("one"), main);
    assert_eq!(parent("two"), main);
    assert_eq!(repo.refs().unwrap().head_branch.as_deref(), Some("main"));
}
