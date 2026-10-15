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
    let history = repo
        .history(&refs.tips(), mugetsu_git::CommitOrder::Date)
        .unwrap();
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

    let history = repo
        .history(&refs.tips(), mugetsu_git::CommitOrder::Date)
        .unwrap();
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
    let history = repo
        .history(&refs.tips(), mugetsu_git::CommitOrder::Date)
        .unwrap();
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

#[test]
fn branch_menu_operations() {
    let (dir, repo) = fixture();
    let clone_dir = tempfile::tempdir().unwrap();
    git(
        clone_dir.path(),
        &["clone", "-q", dir.path().to_str().unwrap(), "c"],
    );
    let clone = Repo::open(clone_dir.path().join("c")).unwrap();
    let branch = |name: &str| {
        clone
            .refs()
            .unwrap()
            .branches
            .into_iter()
            .find(|b| b.name == name)
    };

    // Fast-forward a branch that is not checked out.
    clone.checkout_remote("origin/feature").unwrap();
    clone.checkout("main").unwrap();
    git(dir.path(), &["switch", "-q", "feature"]);
    commit_file(dir.path(), "b.txt", "b\nb2\nb3\n", "f3");
    clone.fetch_all().unwrap();
    assert_eq!(branch("feature").unwrap().behind, 1);
    clone
        .fast_forward("feature", "origin/feature", false)
        .unwrap();
    assert_eq!(branch("feature").unwrap().behind, 0);

    // Rename, then detached checkout, revert and reset.
    clone.rename_branch("feature", "feat/renamed").unwrap();
    assert!(branch("feature").is_none());
    assert!(branch("feat/renamed").is_some());

    let head = clone.refs().unwrap().head.unwrap().to_string();
    let patch = clone.format_patch(&head).unwrap();
    assert!(patch.starts_with("From "), "{patch}");

    clone.revert(&head, true).unwrap();
    assert_ne!(clone.refs().unwrap().head.unwrap().to_string(), head);
    clone.reset(&head, mugetsu_git::ResetMode::Hard).unwrap();
    assert_eq!(clone.refs().unwrap().head.unwrap().to_string(), head);

    clone.checkout_detached(&head).unwrap();
    assert!(clone.refs().unwrap().head_branch.is_none());
    let _ = repo;
}

#[test]
fn checkout_with_local_changes_reset_to_remote_and_push() {
    use mugetsu_git::{LocalChanges, PushOptions};
    let (dir, _repo) = fixture();
    let clone_dir = tempfile::tempdir().unwrap();
    git(
        clone_dir.path(),
        &["clone", "-q", dir.path().to_str().unwrap(), "c"],
    );
    let path = clone_dir.path().join("c");
    git(&path, &["config", "user.name", "Test"]);
    git(&path, &["config", "user.email", "test@example.com"]);
    let clone = Repo::open(&path).unwrap();
    let head = |r: &Repo| r.refs().unwrap().head_branch.unwrap();

    // Remote checkout of a branch that already exists locally just switches.
    clone.checkout_remote("origin/main").unwrap();
    assert_eq!(head(&clone), "main");

    // Stash and reapply across a checkout.
    std::fs::write(path.join("wip.txt"), "wip\n").unwrap();
    clone
        .switch_to("feature", Some("origin/feature"), LocalChanges::Stash)
        .unwrap();
    assert_eq!(head(&clone), "feature");
    assert_eq!(
        std::fs::read_to_string(path.join("wip.txt")).unwrap(),
        "wip\n"
    );
    assert!(clone.refs().unwrap().stashes.is_empty(), "stash was popped");

    // Discard drops tracked changes.
    git(&path, &["add", "wip.txt"]);
    clone
        .switch_to("main", None, LocalChanges::Discard)
        .unwrap();
    assert_eq!(head(&clone), "main");
    assert!(!path.join("wip.txt").exists());
    assert!(!clone.is_dirty().unwrap());

    // Local commit diverging from origin, then reset back onto it.
    commit_file(&path, "local.txt", "x\n", "local only");
    let remote_tip = out(&path, &["rev-parse", "origin/main"]);
    clone.reset_branch("main", "origin/main", true).unwrap();
    assert_eq!(out(&path, &["rev-parse", "HEAD"]), remote_tip);
    commit_file(&path, "other.txt", "y\n", "on feature");
    clone
        .switch_to("feature", None, LocalChanges::Keep)
        .unwrap();
    clone.reset_branch("main", "origin/feature", false).unwrap();
    assert_eq!(
        out(&path, &["rev-parse", "main"]),
        out(&path, &["rev-parse", "origin/feature"])
    );

    // Rewritten history needs --force-with-lease.
    commit_file(&path, "f.txt", "1\n", "f1");
    clone
        .push_to("origin", "feature", "feature", PushOptions::default())
        .unwrap();
    git(&path, &["commit", "-q", "--amend", "-m", "f1 amended"]);
    assert!(
        clone
            .push_to("origin", "feature", "feature", PushOptions::default())
            .is_err()
    );
    clone
        .push_to(
            "origin",
            "feature",
            "feature",
            PushOptions {
                force_with_lease: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        out(dir.path(), &["log", "-1", "--format=%s", "feature"]),
        "f1 amended"
    );
}

fn out(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

#[test]
fn stats_merge_identities_and_count_lines() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    commit_file(p, "a.txt", "1\n2\n3\n", "first");
    std::fs::write(p.join("a.txt"), "1\n2\n").unwrap();
    git(p, &["add", "a.txt"]);
    git(
        p,
        &[
            "-c",
            "user.name=Jane Doe",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--author=Jane Doe <test@example.com>",
            "-m",
            "second\n\nReviewed-by: Bob <bob@x.org>\nCo-authored-by: Al <al@x.org>",
        ],
    );
    let repo = Repo::open(p).unwrap();
    assert_eq!(repo.default_branch(), "main");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 60;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut partial = 0;
    let stats = repo
        .stats("main", Some(now - 30 * 86_400), now, &cancel, |_| {
            partial += 1
        })
        .unwrap();
    assert!(partial >= 1, "metadata reported before line counts");
    assert!(stats.lines_done);
    assert_eq!(stats.commits, 2);
    // "Test" and "Jane Doe" share an email: one person.
    assert_eq!(stats.authors.len(), 1);
    assert_eq!(stats.authors[0].name, "Jane Doe");
    assert_eq!((stats.additions, stats.deletions), (3, 1));
    assert_eq!(stats.reviewers[0].name, "Bob");
    assert_eq!(stats.coauthors[0].name, "Al");
    assert_eq!(stats.files[0].path, "a.txt");
    assert_eq!(stats.files[0].commits, 2);
    assert_eq!(stats.activity.iter().sum::<u32>(), 2);
}

/// origin: main ← a ← b (a stack of two PRs); main then moves on.
fn stack_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin");
    std::fs::create_dir(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    git(&origin, &["config", "user.name", "Test"]);
    git(&origin, &["config", "user.email", "test@example.com"]);
    commit_file(&origin, "base.txt", "base\n", "base");
    git(&origin, &["switch", "-q", "-c", "a"]);
    commit_file(&origin, "a.txt", "a\n", "a1");
    git(&origin, &["switch", "-q", "-c", "b"]);
    commit_file(&origin, "b.txt", "b\n", "b1");
    git(&origin, &["switch", "-q", "main"]);
    commit_file(&origin, "main.txt", "main\n", "main moved");
    git(&origin, &["config", "receive.denyCurrentBranch", "ignore"]);
    git(dir.path(), &["clone", "-q", origin.to_str().unwrap(), "c"]);
    let clone = dir.path().join("c");
    git(&clone, &["config", "user.name", "Test"]);
    git(&clone, &["config", "user.email", "test@example.com"]);
    (dir, origin, clone)
}

fn links() -> Vec<mugetsu_git::StackLink> {
    vec![
        mugetsu_git::StackLink {
            branch: "a".into(),
            parent: None,
        },
        mugetsu_git::StackLink {
            branch: "b".into(),
            parent: Some("a".into()),
        },
    ]
}

#[test]
fn restack_rebases_each_branch_on_its_parent_and_pushes() {
    let (_dir, origin, clone_path) = stack_fixture();
    let clone = Repo::open(&clone_path).unwrap();
    let plan = clone.restack_prepare("origin", "main", links()).unwrap();
    for i in 0..plan.len() {
        clone.restack_step(&plan, i).unwrap();
    }
    assert_eq!(clone.restack_push(&plan).unwrap(), 2);

    let o = Repo::open(&origin).unwrap();
    let is_ancestor = |a: &str, b: &str| o.git().run(["merge-base", "--is-ancestor", a, b]).is_ok();
    assert!(is_ancestor("main", "a"));
    assert!(is_ancestor("a", "b"));
    // b carries only its own commit on top of a.
    let count = o.git().run(["rev-list", "--count", "a..b"]).unwrap();
    assert_eq!(count.trim(), "1");
    // HEAD is back on main.
    assert_eq!(clone.refs().unwrap().head_branch.as_deref(), Some("main"));

    // A second run has nothing to push.
    let plan = clone.restack_prepare("origin", "main", links()).unwrap();
    for i in 0..plan.len() {
        clone.restack_step(&plan, i).unwrap();
    }
    assert_eq!(clone.restack_push(&plan).unwrap(), 0);
}

#[test]
fn restack_conflict_puts_everything_back() {
    let (_dir, origin, clone_path) = stack_fixture();
    // main now conflicts with a.
    commit_file(&origin, "a.txt", "conflict\n", "main touches a.txt");
    let clone = Repo::open(&clone_path).unwrap();
    let plan = clone.restack_prepare("origin", "main", links()).unwrap();
    let before = clone.git().run(["rev-parse", "a", "b"]).unwrap();
    let err = clone.restack_step(&plan, 0).unwrap_err();
    assert!(format!("{err:#}").contains("'a'"), "{err:#}");
    clone.restack_undo(&plan).unwrap();
    assert_eq!(clone.git().run(["rev-parse", "a", "b"]).unwrap(), before);
    assert_eq!(clone.refs().unwrap().head_branch.as_deref(), Some("main"));
    assert!(!clone.is_dirty().unwrap());
}

#[test]
fn restack_refuses_uncommitted_changes() {
    let (_dir, _origin, clone_path) = stack_fixture();
    std::fs::write(clone_path.join("base.txt"), "dirty\n").unwrap();
    let clone = Repo::open(&clone_path).unwrap();
    assert!(clone.restack_prepare("origin", "main", links()).is_err());
}

/// main and `topic` both change line 2 of a.txt; `topic` also deletes gone.txt
/// that main edits.
fn conflicting() -> (tempfile::TempDir, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "a.txt", "one\ntwo\nthree\nfour\nfive\n", "base");
    commit_file(p, "gone.txt", "x\n", "gone");
    git(p, &["switch", "-q", "-c", "topic"]);
    commit_file(
        p,
        "a.txt",
        "one\nTWO-topic\nthree\nfour\nfive!\n",
        "topic a",
    );
    git(p, &["rm", "-q", "gone.txt"]);
    git(p, &["commit", "-q", "-m", "topic rm"]);
    git(p, &["switch", "-q", "main"]);
    commit_file(
        p,
        "a.txt",
        "zero\none\nTWO-main\nthree\nfour\nfive\n",
        "main a",
    );
    commit_file(p, "gone.txt", "y\n", "main gone");
    let repo = Repo::open(p).unwrap();
    (dir, repo)
}

#[test]
fn merge_conflict_resolves_and_continues() {
    use mugetsu_git::{Chunk, Operation, Resolution};
    let (dir, repo) = conflicting();
    assert_eq!(repo.operation().unwrap(), None);
    assert!(!repo.merge("topic").unwrap(), "stops on conflicts");

    let state = repo.operation().unwrap().unwrap();
    assert_eq!(state.op, Operation::Merge);
    assert_eq!(state.ours, "main");
    assert_eq!(state.theirs, "topic");
    assert_eq!(repo.conflicted_paths().unwrap(), ["a.txt", "gone.txt"]);
    let status = repo.status().unwrap();
    assert!(status.files.iter().all(|f| f.index == Change::Conflicted));

    let a = repo.conflict_file("a.txt").unwrap();
    assert!(!a.whole_file_only());
    assert_eq!(a.conflict_count(), 1);
    // Non-conflicting changes from both sides are already merged.
    assert!(matches!(&a.chunks[0], Chunk::Clean(l) if l[0] == "zero\n"));
    let c = a.conflicts().next().unwrap();
    assert_eq!(c.ours, ["TWO-main\n"]);
    assert_eq!(c.base, ["two\n"]);
    assert_eq!(c.theirs, ["TWO-topic\n"]);
    let merged = a.render(&[Some(Resolution::Theirs)]);
    assert_eq!(merged, "zero\none\nTWO-topic\nthree\nfour\nfive!\n");

    let gone = repo.conflict_file("gone.txt").unwrap();
    assert!(gone.whole_file_only());
    assert!(gone.theirs.is_none());

    repo.resolve_with("a.txt", &merged).unwrap();
    repo.resolve_side("gone.txt", false).unwrap();
    assert!(repo.conflicted_paths().unwrap().is_empty());
    assert!(!dir.path().join("gone.txt").exists());
    assert!(repo.continue_operation(Operation::Merge).unwrap());
    assert_eq!(repo.operation().unwrap(), None);
    let parents = repo.git().run(["log", "-1", "--format=%p"]).unwrap();
    assert_eq!(parents.split_whitespace().count(), 2);
}

#[test]
fn rebase_conflict_reports_progress_and_aborts() {
    use mugetsu_git::Operation;
    let (_dir, repo) = conflicting();
    let before = repo.git().run(["rev-parse", "HEAD"]).unwrap();
    assert!(!repo.rebase("topic").unwrap());
    let state = repo.operation().unwrap().unwrap();
    assert_eq!(state.op, Operation::Rebase);
    assert!(state.theirs.ends_with("main a"), "{state:?}");
    assert_eq!(state.step.map(|(_, total)| total), Some(2));
    repo.abort_operation(Some(state.op)).unwrap();
    assert_eq!(repo.operation().unwrap(), None);
    assert_eq!(repo.git().run(["rev-parse", "HEAD"]).unwrap(), before);
}

#[test]
fn cherry_pick_conflict_is_detected() {
    use mugetsu_git::Operation;
    let (_dir, repo) = conflicting();
    assert!(repo.cherry_pick("topic~1", false).is_err());
    let state = repo.operation().unwrap().unwrap();
    assert_eq!(state.op, Operation::CherryPick);
    assert!(state.theirs.ends_with("topic a"));
    repo.resolve_side("a.txt", true).unwrap();
    // Keeping ours leaves nothing to commit: skip it.
    assert!(repo.skip_operation(Operation::CherryPick).unwrap());
    assert_eq!(repo.operation().unwrap(), None);
}
