//! Catching branches up with their target, against real repositories.
#![allow(clippy::disallowed_methods)] // test fixtures

use std::path::Path;
use std::process::Command;

use mugetsu_git::{CatchUpRequest, Repo, Verdict};

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

fn req(branch: &str, target: &str) -> CatchUpRequest {
    CatchUpRequest {
        branch: branch.into(),
        target: target.into(),
    }
}

/// A clone of a bare remote whose main moved on (touching `shared.txt`)
/// after these branches were cut from it:
/// - `clean` (pushed), `conflict` (edits shared.txt), `work` (checked out; not `head`, which clashes with HEAD on macOS);
/// - stack `a` ← `b` ← `c`.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Repo) {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let p = tmp.path().join("work");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&p).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&p, &["init", "-q", "-b", "main"]);
    git(&p, &["config", "user.name", "Test"]);
    git(&p, &["config", "user.email", "test@example.com"]);
    git(&p, &["remote", "add", "origin", remote.to_str().unwrap()]);
    commit_file(&p, "shared.txt", "base\n", "base");
    git(&p, &["push", "-q", "-u", "origin", "main"]);
    for b in ["clean", "conflict", "work", "a"] {
        git(&p, &["branch", b]);
    }
    git(&p, &["switch", "-q", "clean"]);
    commit_file(&p, "clean.txt", "1\n", "clean work");
    git(&p, &["push", "-q", "-u", "origin", "clean"]);
    git(&p, &["switch", "-q", "conflict"]);
    commit_file(&p, "shared.txt", "mine\n", "conflicting work");
    git(&p, &["switch", "-q", "a"]);
    commit_file(&p, "a.txt", "a\n", "a work");
    git(&p, &["switch", "-q", "-c", "b"]);
    commit_file(&p, "b.txt", "b\n", "b work");
    git(&p, &["switch", "-q", "-c", "c"]);
    commit_file(&p, "c.txt", "c\n", "c work");
    git(&p, &["switch", "-q", "work"]);
    commit_file(&p, "work.txt", "w\n", "checked-out work");
    // main moves on, upstream.
    git(&p, &["switch", "-q", "main"]);
    commit_file(&p, "shared.txt", "theirs\n", "upstream change");
    git(&p, &["push", "-q", "origin", "main"]);
    git(&p, &["switch", "-q", "work"]);
    let repo = Repo::open(&p).unwrap();
    (tmp, remote, repo)
}

#[test]
fn scan_keeps_what_lags_and_says_what_would_conflict() {
    let (_tmp, _remote, repo) = fixture();
    let reqs = [
        req("c", "b"),
        req("b", "a"),
        req("a", "origin/main"),
        req("clean", "origin/main"),
        req("conflict", "origin/main"),
        req("work", "origin/main"),
        req("main", "origin/main"),
    ];
    let scan = repo.catch_up_scan(&reqs).unwrap();
    let names: Vec<&str> = scan.iter().map(|c| c.branch.as_str()).collect();
    // Up to date `main` is left out; the stack comes parent first.
    assert_eq!(names, ["a", "b", "c", "clean", "conflict", "work"]);
    let verdict = |b: &str| scan.iter().find(|c| c.branch == b).unwrap();
    assert_eq!(verdict("clean").verdict, Verdict::Clean);
    assert_eq!(verdict("clean").behind, 1);
    assert_eq!(verdict("clean").commits, 1);
    assert!(verdict("clean").upstream.is_some());
    assert_eq!(verdict("work").verdict, Verdict::Clean);
    assert!(verdict("work").worktree.is_some());
    assert_eq!(verdict("b").verdict, Verdict::Clean, "follows a");
    match &verdict("conflict").verdict {
        Verdict::Conflict { paths, summary, .. } => {
            assert_eq!(paths, &["shared.txt"]);
            assert_eq!(summary, "conflicting work");
        }
        v => panic!("{v:?}"),
    }
    // Nothing was written: the scan replays in memory.
    assert_eq!(git(repo.root(), &["status", "--porcelain"]), "");
}

#[test]
fn rebase_moves_refs_without_checkout_and_force_pushes() {
    let (_tmp, remote, repo) = fixture();
    let p = repo.root().to_path_buf();
    let reqs = [
        req("a", "origin/main"),
        req("b", "a"),
        req("clean", "origin/main"),
        req("work", "origin/main"),
    ];
    let scan = repo.catch_up_scan(&reqs).unwrap();
    let mut moved = Vec::new();
    for item in &scan {
        if let Some(m) = repo.catch_up_one(item, &moved).unwrap() {
            moved.push(m);
        }
    }
    assert_eq!(moved.len(), 4);
    let main = git(&p, &["rev-parse", "origin/main"]);
    for b in ["a", "clean", "work"] {
        assert_eq!(git(&p, &["merge-base", b, "origin/main"]), main, "{b}");
    }
    assert_eq!(git(&p, &["rev-parse", "b~1"]), git(&p, &["rev-parse", "a"]));
    assert_eq!(git(&p, &["log", "-1", "--format=%s", "b"]), "b work");
    // HEAD's worktree followed its branch.
    assert_eq!(
        std::fs::read_to_string(p.join("shared.txt")).unwrap(),
        "theirs\n"
    );
    assert_eq!(git(&p, &["status", "--porcelain"]), "");

    let pushed = repo.catch_up_push(&moved);
    assert_eq!(pushed.len(), 1, "only `clean` was pushed");
    assert!(pushed[0].1.is_ok(), "{:?}", pushed[0].1);
    assert_eq!(
        git(&remote, &["rev-parse", "clean"]),
        git(&p, &["rev-parse", "clean"])
    );
}

#[test]
fn a_refused_push_puts_the_branch_back() {
    let (tmp, remote, repo) = fixture();
    let p = repo.root().to_path_buf();
    let scan = repo.catch_up_scan(&[req("clean", "origin/main")]).unwrap();
    // Someone pushes to `clean` after our fetch.
    let other = tmp.path().join("other");
    git(
        tmp.path(),
        &["clone", "-q", remote.to_str().unwrap(), "other"],
    );
    git(&other, &["switch", "-q", "clean"]);
    commit_file(&other, "theirs.txt", "x\n", "someone else");
    git(&other, &["push", "-q", "origin", "clean"]);

    let before = git(&p, &["rev-parse", "clean"]);
    let m = repo.catch_up_one(&scan[0], &[]).unwrap().unwrap();
    let pushed = repo.catch_up_push(&[m]);
    assert!(pushed[0].1.is_err());
    assert_eq!(git(&p, &["rev-parse", "clean"]), before);
}

#[test]
fn blocked_branches_are_reported() {
    let (tmp, _remote, repo) = fixture();
    let p = repo.root().to_path_buf();
    // `a` checked out elsewhere with changes; its stack child is blocked too.
    let wt = tmp.path().join("wt");
    git(&p, &["worktree", "add", "-q", wt.to_str().unwrap(), "a"]);
    std::fs::write(wt.join("a.txt"), "dirty\n").unwrap();
    // `clean` diverged from its remote branch.
    git(&p, &["reset", "-q", "--hard", "HEAD"]);
    git(&p, &["branch", "-f", "clean", "clean~1"]);
    git(&p, &["switch", "-q", "clean"]);
    commit_file(&p, "other.txt", "o\n", "diverged work");
    git(&p, &["switch", "-q", "work"]);
    let scan = repo
        .catch_up_scan(&[
            req("a", "origin/main"),
            req("b", "a"),
            req("clean", "origin/main"),
        ])
        .unwrap();
    let verdict = |b: &str| &scan.iter().find(|c| c.branch == b).unwrap().verdict;
    assert!(matches!(verdict("a"), Verdict::Dirty { .. }));
    assert_eq!(verdict("b"), &Verdict::ParentBlocked { parent: "a".into() });
    assert_eq!(
        verdict("clean"),
        &Verdict::RemoteAhead {
            upstream: "origin/clean".into()
        }
    );
}
