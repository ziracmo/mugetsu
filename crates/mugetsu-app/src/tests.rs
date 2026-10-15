//! UI integration tests: real views in headless windows, driven by native
//! clicks and keys, against real repositories built with the git CLI.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};

use crate::repo_view::{Mode, RepoView};
use crate::workspace::Workspace;

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
    String::from_utf8(out.stdout).unwrap()
}

fn commit_file(dir: &Path, file: &str, content: &str, msg: &str) {
    std::fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// main with a merged `feature` branch and an unmerged `topic` branch.
fn fixture(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    commit_file(dir, "a.txt", "one\n", "c1");
    commit_file(dir, "a.txt", "one\ntwo\n", "c2");
    git(dir, &["switch", "-q", "-c", "feature"]);
    commit_file(dir, "b.txt", "b\n", "f1");
    git(dir, &["switch", "-q", "main"]);
    git(
        dir,
        &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
    );
    git(dir, &["switch", "-q", "-c", "topic", "HEAD~1"]);
    commit_file(dir, "t.txt", "t\n", "topic work");
    git(dir, &["switch", "-q", "main"]);
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1400.), px(900.)),
        ))),
        ..Default::default()
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::init(cx);
        crate::workspace::init(cx);
        crate::repo_view::init(cx);
    });
}

fn open_repo_view(cx: &mut TestAppContext, path: &Path) -> (AnyWindowHandle, Entity<RepoView>) {
    let repo = mugetsu_git::Repo::open(path).unwrap();
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| RepoView::new(repo, window, cx))
        })
        .unwrap()
    });
    settle(cx, &view);
    (handle, view)
}

/// Runs background work (git processes) until the view is idle.
fn settle(cx: &mut TestAppContext, view: &Entity<RepoView>) {
    for _ in 0..200 {
        cx.run_until_parked();
        if cx.read(|cx| {
            let v = view.read(cx);
            !v.is_loading() && !v.is_fetching() && !v.is_busy()
        }) {
            cx.run_until_parked();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("view never settled");
}

fn head_summary(cx: &TestAppContext, view: &Entity<RepoView>) -> String {
    cx.read(|cx| {
        let data = view.read(cx).data().clone();
        let head = data.refs.head.unwrap();
        data.history.commits[data.history.row_of(&head).unwrap()]
            .summary
            .clone()
    })
}

// ---- Component tests -------------------------------------------------------

#[gpui_kit::test]
fn loads_history_and_selects_head(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (_, view) = open_repo_view(cx, dir.path());
    cx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.data().history.len(), 5);
        assert_eq!(
            v.selected_commit(),
            v.data().refs.head,
            "HEAD is selected on first load"
        );
        assert!(
            v.data().graph.max_width >= 2,
            "topic branch needs its own lane"
        );
    });
}

#[gpui_kit::test]
fn clicking_a_branch_selects_its_tip(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let topic = cx.read(|cx| {
        view.read(cx)
            .data()
            .refs
            .branches
            .iter()
            .find(|b| b.name == "topic")
            .unwrap()
            .target
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("branch:topic", cx);
    })
    .unwrap();
    settle(cx, &view);
    cx.read(|cx| {
        assert_eq!(view.read(cx).selected_commit(), Some(topic));
        assert_eq!(
            view.read(cx).selected_ref().map(|s| s.as_ref()),
            Some("branch:topic")
        );
    });
}

#[gpui_kit::test]
fn double_clicking_a_branch_checks_it_out(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click("branch:topic", cx);
    })
    .unwrap();
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "topic"
    );
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).data().refs.head_branch.as_deref(),
            Some("topic")
        )
    });
}

#[gpui_kit::test]
fn keyboard_moves_through_history(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let second = cx.read(|cx| view.read(cx).data().history.commits[1].id);
    let first = cx.read(|cx| view.read(cx).data().history.commits[0].id);
    cx.update(|cx| view.update(cx, |v, cx| v.select_commit(first, true, cx)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::SharedString::from(first.to_string()), cx);
        window.press("down", cx);
    })
    .unwrap();
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).selected_commit(), Some(second)));
}

// ---- End-to-end flows ------------------------------------------------------

#[gpui_kit::test]
fn stage_and_commit_from_the_changes_view(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("wip").is_some(),
            "uncommitted banner is shown"
        );
        window.click("wip", cx);
    })
    .unwrap();
    settle(cx, &view);
    assert_eq!(cx.read(|cx| view.read(cx).mode()), Mode::Changes);

    // Stage only a.txt with its + button.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle:false:a.txt", cx);
    })
    .unwrap();
    settle(cx, &view);
    let staged: Vec<String> = cx.read(|cx| {
        view.read(cx)
            .data()
            .status
            .staged()
            .map(|f| f.path.clone())
            .collect()
    });
    assert_eq!(staged, ["a.txt"]);

    // Commit is refused without a message (button stays disabled → nothing happens).
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("commit-message", cx);
        window.input("Add three", cx);
        window.click("commit", cx);
    })
    .unwrap();
    settle(cx, &view);

    assert_eq!(head_summary(cx, &view), "Add three");
    let remaining: Vec<String> = cx.read(|cx| {
        view.read(cx)
            .data()
            .status
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect()
    });
    assert_eq!(
        remaining,
        ["new.txt"],
        "untracked file stays out of the commit"
    );
    assert_eq!(
        git(dir.path(), &["log", "-1", "--format=%s"]).trim(),
        "Add three"
    );
}

#[gpui_kit::test]
fn background_tab_shows_a_dot_when_remote_moves(cx: &mut TestAppContext) {
    init(cx);
    let origin = tempfile::tempdir().unwrap();
    fixture(origin.path());
    let clones = tempfile::tempdir().unwrap();
    git(
        clones.path(),
        &["clone", "-q", origin.path().to_str().unwrap(), "clone"],
    );
    let other = tempfile::tempdir().unwrap();
    fixture(other.path());
    let clone = clones.path().join("clone");

    let paths = vec![clone.clone(), other.path().to_path_buf()];
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(paths, 1, false, window, cx))
        })
        .unwrap()
    });
    let tabs = cx.read(|cx| ws.read(cx).tabs());
    for t in &tabs {
        settle(cx, t);
    }
    assert_eq!(
        cx.read(|cx| ws.read(cx).active_index()),
        1,
        "second repo is focused"
    );

    // Someone pushes to origin; the (background) clone fetches.
    commit_file(origin.path(), "a.txt", "upstream\n", "upstream change");
    cx.update(|cx| tabs[0].update(cx, |v, cx| v.fetch(cx)));
    settle(cx, &tabs[0]);
    assert!(cx.read(|cx| tabs[0].read(cx).has_updates()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("updates-clone").is_some(),
            "pulsing dot on the background tab"
        );
        window.click("tab-clone", cx);
    })
    .unwrap();
    settle(cx, &tabs[0]);

    cx.read(|cx| {
        let v = tabs[0].read(cx);
        assert!(!v.has_updates(), "dot clears once the tab is viewed");
        let main = v
            .data()
            .refs
            .branches
            .iter()
            .find(|b| b.name == "main")
            .unwrap();
        assert_eq!(
            main.behind, 1,
            "the new remote commit is visible after reload"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("updates-clone").is_none());
    })
    .unwrap();
}
