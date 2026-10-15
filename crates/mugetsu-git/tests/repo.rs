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

/// main: base - one - two - three (each on its own file).
fn linear() -> (tempfile::TempDir, Repo, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "base.txt", "base\n", "base");
    let base = rev(p, "HEAD");
    commit_file(p, "one.txt", "1\n", "one");
    commit_file(p, "two.txt", "2\n", "two");
    commit_file(p, "three.txt", "3\n", "three");
    let repo = Repo::open(p).unwrap();
    (dir, repo, base)
}

fn rev(dir: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", rev])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn subjects(dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["log", "--format=%B%x1e", "--reverse"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .split('\x1e')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn interactive_rebase_reorders_squashes_rewords_and_drops() {
    use mugetsu_git::{RebaseAction as A, RebaseOutcome, RebaseStep};
    let (dir, repo, base) = linear();
    let plan = repo.rebase_plan(&base).unwrap();
    let names: Vec<_> = plan.iter().map(|c| c.summary()).collect();
    assert_eq!(names, ["one", "two", "three"]);
    let step = |i: usize, action, message: Option<&str>| RebaseStep {
        id: plan[i].id.clone(),
        action,
        message: message.map(str::to_string),
    };
    let outcome = repo
        .rebase_interactive(
            &base,
            &[
                step(2, A::Reword, Some("three, reworded\n\nwith a body")),
                step(0, A::Pick, None),
                step(1, A::Squash, None),
            ],
        )
        .unwrap();
    assert_eq!(outcome, RebaseOutcome::Done);
    assert_eq!(
        subjects(dir.path()),
        ["base", "three, reworded\n\nwith a body", "one\n\ntwo"]
    );
    assert!(dir.path().join("two.txt").exists());
    assert!(repo.rebase_progress().is_none());

    // Drop and fixup.
    let plan = repo.rebase_plan(&base).unwrap();
    let outcome = repo
        .rebase_interactive(
            &base,
            &[
                RebaseStep {
                    id: plan[0].id.clone(),
                    action: A::Drop,
                    message: None,
                },
                RebaseStep {
                    id: plan[1].id.clone(),
                    action: A::Pick,
                    message: None,
                },
            ],
        )
        .unwrap();
    assert_eq!(outcome, RebaseOutcome::Done);
    assert_eq!(subjects(dir.path()), ["base", "one\n\ntwo"]);
    assert!(!dir.path().join("three.txt").exists());
}

#[test]
fn interactive_rebase_stops_on_conflicts_then_aborts_or_continues() {
    use mugetsu_git::{RebaseAction as A, RebaseOutcome, RebaseStep};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "a.txt", "base\n", "base");
    let base = rev(p, "HEAD");
    commit_file(p, "a.txt", "one\n", "one");
    commit_file(p, "a.txt", "two\n", "two");
    let head = rev(p, "HEAD");
    let repo = Repo::open(p).unwrap();
    let plan = repo.rebase_plan(&base).unwrap();
    // Swapping two edits of the same line conflicts.
    let swapped = [
        RebaseStep {
            id: plan[1].id.clone(),
            action: A::Pick,
            message: None,
        },
        RebaseStep {
            id: plan[0].id.clone(),
            action: A::Pick,
            message: None,
        },
    ];
    assert_eq!(
        repo.rebase_interactive(&base, &swapped).unwrap(),
        RebaseOutcome::Stopped
    );
    let progress = repo.rebase_progress().unwrap();
    assert_eq!(progress.branch.as_deref(), Some("main"));
    assert_eq!((progress.step, progress.total), (1, 2));
    assert!(repo.rebase_continue().is_err(), "conflicts are unresolved");
    repo.rebase_abort().unwrap();
    assert!(repo.rebase_progress().is_none());
    assert_eq!(rev(p, "HEAD"), head);

    // Resolve each stop by taking the commit's side, then continue.
    assert_eq!(
        repo.rebase_interactive(&base, &swapped).unwrap(),
        RebaseOutcome::Stopped
    );
    std::fs::write(p.join("a.txt"), "two\n").unwrap();
    git(p, &["add", "a.txt"]);
    assert_eq!(repo.rebase_continue().unwrap(), RebaseOutcome::Stopped);
    std::fs::write(p.join("a.txt"), "one\n").unwrap();
    git(p, &["add", "a.txt"]);
    assert_eq!(repo.rebase_continue().unwrap(), RebaseOutcome::Done);
    assert_eq!(subjects(p), ["base", "two", "one"]);
}

#[test]
fn rebase_plan_refuses_merges_and_non_ancestors() {
    let (_dir, repo) = fixture();
    let c1 = repo
        .git()
        .run(["rev-list", "--max-parents=0", "HEAD"])
        .unwrap();
    assert!(repo.rebase_plan(c1.trim()).is_err(), "range holds a merge");
    repo.git().run(["switch", "-q", "feature"]).unwrap();
    assert_eq!(repo.rebase_plan(c1.trim()).unwrap().len(), 3);
    assert!(repo.rebase_plan("main").is_err(), "main is not an ancestor");
}
