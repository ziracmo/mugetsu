//! UI integration tests: real views in headless windows, driven by native
//! clicks and keys, against real repositories built with the git CLI.
#![allow(clippy::disallowed_methods)] // test fixtures

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
use mugetsu_git::RefLabel;

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
        crate::palette::init(cx);
        crate::recap::init(cx);
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
            v.data().folded.graph.max_width >= 2,
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
        );
        assert_eq!(
            view.read(cx).celebrating(),
            view.read(cx).data().refs.head,
            "the new HEAD row celebrates"
        );
    });
}

#[gpui_kit::test]
fn keyboard_moves_through_history(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    // Shown rows: the merged `feature` commit may sit folded between them.
    let shown = |cx: &TestAppContext, ix: usize| {
        cx.read(|cx| {
            let data = view.read(cx).data();
            data.history.commits[data.folded.history_row(ix)].id
        })
    };
    let first = shown(cx, 0);
    let second = shown(cx, 1);
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
            cx.new(|cx| Workspace::new(session_of(paths, 1), false, window, cx))
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

#[gpui_kit::test]
fn branches_are_foldered_and_searchable(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(dir.path(), &["branch", "feat/alpha"]);
    git(dir.path(), &["branch", "feat/beta"]);
    let (handle, view) = open_repo_view(cx, dir.path());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("folder:local:feat").is_some());
        assert!(
            window.try_find("branch:feat/alpha").is_none(),
            "folders start closed unless they hold HEAD"
        );
        window.click("folder:local:feat", cx);
        window.render_frame(cx);
        assert!(window.try_find("branch:feat/alpha").is_some());

        window.click("sidebar-search", cx);
        window.input("beta", cx);
        window.render_frame(cx);
        assert!(window.try_find("branch:feat/beta").is_some());
        assert!(window.try_find("branch:feat/alpha").is_none());
        assert!(window.try_find("branch:main").is_none());
    })
    .unwrap();
    let _ = view;
}

#[gpui_kit::test]
fn commit_descriptions_start_hidden(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let body: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.path().join("a.txt"), "long\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(
        dir.path(),
        &["commit", "-q", "-m", "subject", "-m", body.as_str()],
    );
    let (handle, view) = open_repo_view(cx, dir.path());
    assert!(!cx.read(|cx| view.read(cx).body_expanded()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("commit-body").is_none(),
            "hidden by default"
        );
        window.click("commit-body-toggle", cx);
        window.render_frame(cx);
        assert!(window.try_find("commit-body").is_some());
    })
    .unwrap();
    assert!(cx.read(|cx| view.read(cx).body_expanded()));

    // No description, nothing to show.
    let first = cx.read(|cx| {
        let d = view.read(cx).data().clone();
        d.history
            .commits
            .iter()
            .find(|c| c.summary == "c1")
            .unwrap()
            .id
    });
    cx.update(|cx| view.update(cx, |v, cx| v.select_commit(first, true, cx)));
    settle(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("commit-body-toggle").is_none());
    })
    .unwrap();
    let _ = view;
}

#[gpui_kit::test]
fn branch_context_menu_renames(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let step = |cx: &mut TestAppContext, f: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    step(cx, &|window, cx| window.right_click("branch:topic", cx));
    step(cx, &|window, cx| {
        let rename = window.within("popup-menu").find(11usize);
        assert_eq!(rename.label(), Some("Renommer…"));
        window.within("popup-menu").click(11usize, cx);
    });
    step(cx, &|window, cx| {
        window.press("secondary-a", cx);
        window.input("feat/renamed", cx);
    });
    step(cx, &|window, cx| window.press("enter", cx));
    settle(cx, &view);
    let names: Vec<String> = cx.read(|cx| {
        view.read(cx)
            .data()
            .refs
            .branches
            .iter()
            .map(|b| b.name.clone())
            .collect()
    });
    assert!(names.contains(&"feat/renamed".to_string()), "{names:?}");
    assert!(!names.contains(&"topic".to_string()));
}

#[gpui_kit::test]
fn command_f_finds_commits_by_message_and_sha(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let (topic, f1) = cx.read(|cx| {
        let d = view.read(cx).data().clone();
        let find = |s: &str| {
            d.history
                .commits
                .iter()
                .find(|c| c.summary == s)
                .unwrap()
                .id
        };
        (find("topic work"), find("f1"))
    });
    let head = cx.read(|cx| view.read(cx).data().refs.head.unwrap());

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::SharedString::from(head.to_string()), cx);
        window.press("secondary-f", cx);
        window.render_frame(cx);
        assert!(window.try_find("commit-search").is_some());
        window.input("topic", cx);
    })
    .unwrap();
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).selected_commit(), Some(topic)));

    // A SHA prefix jumps too.
    let prefix = f1.to_hex_with_len(7).to_string();
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-a", cx);
        window.input(&prefix, cx);
    })
    .unwrap();
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).selected_commit(), Some(f1)));

    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("commit-search").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_can_be_collapsed(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sidebar-search").is_some());
        window.click("toggle-sidebar", cx);
        window.render_frame(cx);
        assert!(window.try_find("sidebar-search").is_none());
    })
    .unwrap();
    assert!(cx.read(|cx| view.read(cx).sidebar_collapsed()));
}

#[gpui_kit::test]
fn sidebar_sections_fold_and_unfold(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, _view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let shown = |window: &mut gpui_kit::Window| window.try_find("branch:main").is_some();
        assert!(shown(window));
        window.click("section-branches", cx);
        window.render_frame(cx);
        assert!(!shown(window));
        window.click("section-branches", cx);
        window.render_frame(cx);
        assert!(shown(window));
        // A trackpad click often moves the pointer a few pixels, which starts
        // the reorder drag: it still folds and unfolds.
        let at = window.find("section-branches").bounds().center();
        let nudge = gpui_kit::point(px(4.), px(1.));
        window.drag(at, at + nudge, cx);
        window.render_frame(cx);
        assert!(!shown(window));
        window.drag(at, at + nudge, cx);
        window.render_frame(cx);
        assert!(shown(window));
    })
    .unwrap();
}

fn session_of(repos: Vec<std::path::PathBuf>, active: usize) -> crate::session::Session {
    crate::session::Session::new(
        vec![crate::session::RepoSet {
            name: "Général".into(),
            repos,
            active,
        }],
        0,
    )
}

#[gpui_kit::test]
fn workspaces_switch_their_tabs(cx: &mut TestAppContext) {
    init(cx);
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fixture(a.path());
    fixture(b.path());
    let session = crate::session::Session::new(
        vec![
            crate::session::RepoSet {
                name: "Perso".into(),
                repos: vec![a.path().to_path_buf()],
                active: 0,
            },
            crate::session::RepoSet {
                name: "Boulot".into(),
                repos: vec![b.path().to_path_buf()],
                active: 0,
            },
        ],
        0,
    );
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    let root = |cx: &TestAppContext| {
        cx.read(|cx| {
            ws.read(cx)
                .active()
                .map(|v| v.read(cx).repo().root().to_path_buf())
        })
    };
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
    assert_eq!(root(cx).map(|p| canon(&p)), Some(canon(a.path())));

    handle
        .update(cx, |_, window, cx| {
            ws.update(cx, |w, cx| w.switch_workspace(1, window, cx))
        })
        .unwrap();
    assert_eq!(root(cx).map(|p| canon(&p)), Some(canon(b.path())));

    handle
        .update(cx, |_, window, cx| {
            ws.update(cx, |w, cx| w.new_workspace("Vide".into(), window, cx))
        })
        .unwrap();
    assert!(root(cx).is_none(), "a new workspace starts empty");
    let (names, current) = cx.read(|cx| ws.read(cx).workspace_names());
    assert_eq!(names, ["Perso", "Boulot", "Vide"]);
    assert_eq!(current, 2);

    handle
        .update(cx, |_, window, cx| {
            ws.update(cx, |w, cx| w.delete_workspace(window, cx))
        })
        .unwrap();
    let (names, current) = cx.read(|cx| ws.read(cx).workspace_names());
    assert_eq!(names, ["Perso", "Boulot"]);
    assert_eq!(current, 0);
    assert_eq!(
        root(cx).map(|p| canon(&p)),
        Some(canon(a.path())),
        "tabs come back"
    );
}

fn ui_step(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn double_clicking_a_graph_chip_checks_it_out(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        window.double_click("chip-topic", cx)
    });
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "topic"
    );
}

#[gpui_kit::test]
fn dropping_a_branch_chip_on_another_rebases_it_after_asking(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let drop_topic_on_main = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        let from = window.find("chip-topic").bounds().center();
        let to = window.find("chip-main").bounds().center();
        window.drag(from, to, cx);
    };
    // Cancelled: nothing moves.
    ui_step(cx, handle, &drop_topic_on_main);
    ui_step(cx, handle, &|window, cx| {
        window.click("drag-rebase-cancel", cx)
    });
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("drag-rebase").is_none())
    });
    let topic_before = git(dir.path(), &["rev-parse", "topic"]);
    // Confirmed: topic is replayed on main, main stays checked out.
    ui_step(cx, handle, &drop_topic_on_main);
    assert_eq!(git(dir.path(), &["rev-parse", "topic"]), topic_before);
    ui_step(cx, handle, &|window, cx| window.click("drag-rebase-ok", cx));
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["rev-parse", "topic~1"]),
        git(dir.path(), &["rev-parse", "main"])
    );
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
}

#[gpui_kit::test]
fn rebasing_the_checkout_over_changes_asks_to_stash_or_discard_them(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(dir.path(), &["switch", "-q", "topic"]);
    let t = dir.path().join("t.txt");
    std::fs::write(&t, "mine\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());
    let drop_topic_on_main = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
        let from = window.find("chip-topic").bounds().center();
        let to = window.find("chip-main").bounds().center();
        window.drag(from, to, cx);
    };
    // Stashed, rebased, reapplied.
    ui_step(cx, handle, &drop_topic_on_main);
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("drag-rebase").is_none())
    });
    ui_step(cx, handle, &|window, cx| {
        window.click("rebase-changes-stash", cx)
    });
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["rev-parse", "topic~1"]),
        git(dir.path(), &["rev-parse", "main"])
    );
    assert_eq!(std::fs::read_to_string(&t).unwrap(), "mine\n");
    assert!(git(dir.path(), &["stash", "list"]).is_empty());

    // Discarded.
    git(dir.path(), &["reset", "-q", "--hard", "main~1"]);
    commit_file(dir.path(), "t.txt", "t\n", "topic work");
    std::fs::write(&t, "mine\n").unwrap();
    view.update(cx, |v, cx| v.reload(cx));
    settle(cx, &view);
    ui_step(cx, handle, &drop_topic_on_main);
    ui_step(cx, handle, &|window, cx| {
        window.click("rebase-changes-discard", cx)
    });
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["rev-parse", "topic~1"]),
        git(dir.path(), &["rev-parse", "main"])
    );
    assert_eq!(std::fs::read_to_string(&t).unwrap(), "t\n");
}

#[gpui_kit::test]
fn dropping_a_branch_chip_on_a_row_rebases_it_without_touching_the_checkout(
    cx: &mut TestAppContext,
) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.txt"), "dirty\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());
    let main = git(dir.path(), &["rev-parse", "main"]).trim().to_string();
    ui_step(cx, handle, &|window, cx| {
        let from = window.find("chip-topic").bounds().center();
        // Far right of main's row, away from its chips.
        let row = window
            .find(gpui_kit::SharedString::from(main.clone()))
            .bounds();
        let to = gpui_kit::point(row.right() - px(40.), row.center().y);
        window.drag(from, to, cx);
    });
    ui_step(cx, handle, &|window, cx| window.click("drag-rebase-ok", cx));
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["rev-parse", "topic~1"]),
        git(dir.path(), &["rev-parse", "main"])
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
}

#[gpui_kit::test]
fn merged_branch_is_folded_until_its_arrow_is_clicked(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    // The merged branch's ref still exists: it folds all the same.
    let (handle, view) = open_repo_view(cx, dir.path());
    let shown = |cx: &TestAppContext| {
        cx.read(|cx| {
            let data = view.read(cx).data().clone();
            data.folded
                .rows
                .iter()
                .map(|&r| data.history.commits[r as usize].summary.clone())
                .collect::<Vec<_>>()
        })
    };
    assert!(!shown(cx).contains(&"f1".to_string()), "{:?}", shown(cx));
    let merge = cx.read(|cx| view.read(cx).data().refs.head.unwrap());
    ui_step(cx, handle, &|window, cx| {
        window.click(gpui_kit::SharedString::from(format!("fold-{merge}")), cx)
    });
    assert!(shown(cx).contains(&"f1".to_string()), "{:?}", shown(cx));
    ui_step(cx, handle, &|window, cx| {
        window.click(gpui_kit::SharedString::from(format!("fold-{merge}")), cx)
    });
    assert!(!shown(cx).contains(&"f1".to_string()));
}

#[gpui_kit::test]
fn checkout_with_local_changes_offers_to_stash(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nwip\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        window.double_click("branch:topic", cx)
    });
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("dialog").is_some(), "asks first");
    });
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
    ui_step(cx, handle, &|window, cx| window.click("checkout-ok", cx));
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "topic"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\nwip\n",
        "changes were stashed and reapplied"
    );
    assert!(git(dir.path(), &["stash", "list"]).trim().is_empty());
}

#[gpui_kit::test]
fn double_clicking_a_remote_branch_offers_to_reset_its_local_copy(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(remote.path(), &["init", "-q", "--bare"]);
    git(
        dir.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "topic"]);
    // The local copy falls behind its remote.
    git(dir.path(), &["branch", "-q", "-f", "topic", "topic~1"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        view.update(cx, |v, cx| {
            v.request_checkout("origin/topic".into(), true, window, cx)
        })
    });
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("dialog").is_some(), "asks first");
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    ui_step(cx, handle, &|window, cx| window.click("reset-ok", cx));
    // The sheet plays its send-off before resetting.
    cx.executor().advance_clock(Duration::from_millis(500));
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "topic"
    );
    assert_eq!(
        git(dir.path(), &["rev-parse", "topic"]),
        git(dir.path(), &["rev-parse", "origin/topic"]),
        "reset on the remote branch"
    );
}

#[gpui_kit::test]
fn push_dialog_can_force_with_lease(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(remote.path(), &["init", "-q", "--bare"]);
    git(
        dir.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "main"]);
    git(dir.path(), &["commit", "-q", "--amend", "-m", "rewritten"]);
    let (handle, view) = open_repo_view(cx, dir.path());

    let push = |cx: &mut TestAppContext, force: bool| {
        ui_step(cx, handle, &|window, cx| {
            view.update(cx, |v, cx| v.open_push_dialog(None, window, cx))
        });
        if force {
            // Let the sheet finish sliding in before aiming at a small target.
            std::thread::sleep(Duration::from_millis(500));
            ui_step(cx, handle, &|window, cx| window.click("push-force", cx));
        }
        ui_step(cx, handle, &|window, cx| window.click("push-ok", cx));
        settle(cx, &view);
    };
    let remote_head = || git(remote.path(), &["log", "-1", "--format=%s", "main"]);
    push(cx, true);
    assert_eq!(remote_head().trim(), "rewritten");
    cx.read(|cx| assert_eq!(view.read(cx).pushed_branch().as_deref(), Some("main")));
}

#[gpui_kit::test]
fn changes_shortcuts_stage_all_and_stash(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.path().join("new.txt"), "new\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_focused(true, cx);
            v.set_mode(crate::repo_view::Mode::Changes, cx);
        })
    });
    ui_step(cx, handle, &|_, _| {});
    ui_step(cx, handle, &|window, cx| window.press("secondary-a", cx));
    settle(cx, &view);
    let staged = git(dir.path(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "a.txt\nnew.txt");
    ui_step(cx, handle, &|window, cx| window.press("secondary-s", cx));
    settle(cx, &view);
    assert!(
        git(dir.path(), &["status", "--porcelain"])
            .trim()
            .is_empty()
    );
    assert_eq!(git(dir.path(), &["stash", "list"]).lines().count(), 1);
}

#[gpui_kit::test]
fn commit_details_list_files_and_confirm_copies(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/new.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["rm", "-q", "b.txt"]);
    git(dir.path(), &["add", "src/new.rs"]);
    git(dir.path(), &["commit", "-q", "-m", "files"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    settle(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("commit-file-src/new.rs").is_some());
        assert!(window.try_find("commit-file-b.txt").is_some());
        window.click("commit-file-src/new.rs", cx);
        window.click("copy-sha", cx);
    })
    .unwrap();
    let head = cx.read(|cx| view.read(cx).data().refs.head.unwrap());
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()),
            Some(head.to_string())
        );
        assert!(view.read(cx).copied_feedback());
    });
}

#[gpui_kit::test]
fn commit_diff_scrolls_through_every_file_and_the_tree_follows(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let body: String = (0..60).map(|i| format!("line {i}\n")).collect();
    for i in 0..12 {
        std::fs::write(dir.path().join(format!("f{i:02}.txt")), &body).unwrap();
    }
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "many files"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    settle(cx, &view);
    let active = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.read(|cx| view.read(cx).commit_active_file(cx))
            .map(|p| p.to_string())
    };
    cx.update_window(handle, |_, window, cx| {
        window.click("commit-file-f05.txt", cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(active(cx).as_deref(), Some("f05.txt"));
    // The wheel goes on past the clicked file, through the next ones.
    let mut seen = vec![];
    for _ in 0..40 {
        cx.update_window(handle, |_, window, cx| {
            window.scroll(
                "diff-body",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-400.))),
                cx,
            );
        })
        .unwrap();
        let now = active(cx).unwrap();
        if seen.last() != Some(&now) {
            seen.push(now);
        }
    }
    assert_eq!(seen[..2], ["f05.txt", "f06.txt"], "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("f11.txt"), "{seen:?}");
    // And back up, before the clicked file.
    for _ in 0..80 {
        cx.update_window(handle, |_, window, cx| {
            window.scroll(
                "diff-body",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(400.))),
                cx,
            );
        })
        .unwrap();
    }
    assert_eq!(active(cx).as_deref(), Some("f00.txt"));
    // A file near the end can't reach the top: the clicked one stays lit.
    cx.update_window(handle, |_, window, cx| {
        window.click("commit-file-f11.txt", cx);
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    assert_eq!(active(cx).as_deref(), Some("f11.txt"));
}

#[gpui_kit::test]
fn command_f_finds_branch_names_and_sidebar_has_a_prs_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(dir.path(), &["branch", "zebra", "HEAD~2"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    let target = cx.read(|cx| {
        let d = view.read(cx).data().clone();
        d.refs
            .branches
            .iter()
            .find(|b| b.name == "zebra")
            .unwrap()
            .target
    });
    let head = cx.read(|cx| view.read(cx).data().refs.head.unwrap());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(gpui_kit::SharedString::from(head.to_string()), cx);
        window.press("secondary-f", cx);
        window.input("zebra", cx);
    })
    .unwrap();
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).selected_commit(), Some(target)));

    cx.update_window(handle, |_, window, cx| {
        window.press("escape", cx);
        window.render_frame(cx);
        window.click("sidebar-tab-prs", cx);
        window.render_frame(cx);
        assert!(
            window.try_find("branch:main").is_none(),
            "repo sections hidden"
        );
    })
    .unwrap();
    cx.read(|cx| assert!(view.read(cx).prs_tab_open()));
    cx.update_window(handle, |_, window, cx| {
        window.click("sidebar-tab-repo", cx);
        window.render_frame(cx);
        assert!(window.try_find("branch:main").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn stats_view_ranks_authors_of_the_default_branch(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("c.txt"), "c\n").unwrap();
    git(dir.path(), &["add", "c.txt"]);
    git(
        dir.path(),
        &[
            "commit",
            "-q",
            "--author=Alice <alice@x.org>",
            "-m",
            "by alice",
        ],
    );
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("mode-stats", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let wait_stats = |cx: &mut TestAppContext| {
        for _ in 0..500 {
            // The tab polls the background work on a timer.
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            if let Some((stats, false)) = cx.read(|cx| view.read(cx).stats_snapshot()) {
                return stats;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("stats never finished");
    };
    let stats = wait_stats(cx);
    assert_eq!(stats.branch, "main");
    // c1, c2, f1, the merge and Alice's commit; `topic` is not on main.
    assert_eq!(stats.commits, 5);
    assert_eq!(stats.merges, 1);
    let authors: Vec<_> = stats.authors.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(authors, ["Test", "Alice"]);
    assert!(stats.lines_done);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("stats-authors-0").is_some());
        window.click("stats-period-all", cx);
    })
    .unwrap();
    let all = wait_stats(cx);
    assert_eq!(all.commits, 5);
    cx.read(|cx| assert_eq!(view.read(cx).mode(), Mode::Stats));
}

#[gpui_kit::test]
fn control_tab_shows_a_switcher_until_released(cx: &mut TestAppContext) {
    init(cx);
    let (a, b, c) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    for d in [&a, &b, &c] {
        fixture(d.path());
    }
    let session = session_of(
        vec![
            a.path().to_path_buf(),
            b.path().to_path_buf(),
            c.path().to_path_buf(),
        ],
        0,
    );
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    let active = |cx: &TestAppContext| cx.read(|cx| ws.read(cx).active_index());
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.run_until_parked();
    let ctrl = gpui_kit::Modifiers {
        control: true,
        ..Default::default()
    };
    vcx.simulate_modifiers_change(ctrl);
    vcx.simulate_keystrokes("ctrl-tab ctrl-tab");
    vcx.run_until_parked();
    assert_eq!(active(cx), 0, "nothing switches while ⌃ is held");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tab-switcher").is_some());
    })
    .unwrap();
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_modifiers_change(gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(active(cx), 2, "release picks the highlighted tab");

    // Keys keep working right after a switch (focus follows the tab).
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_modifiers_change(ctrl);
    vcx.simulate_keystrokes("ctrl-tab");
    vcx.simulate_modifiers_change(gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(active(cx), 0);
}

#[gpui_kit::test]
fn tabs_reorder_by_drag_and_detach_out_of_the_window(cx: &mut TestAppContext) {
    init(cx);
    let (a, b, c) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    for d in [&a, &b, &c] {
        fixture(d.path());
    }
    let session = session_of(
        vec![
            a.path().to_path_buf(),
            b.path().to_path_buf(),
            c.path().to_path_buf(),
        ],
        1,
    );
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let name = |p: &Path| format!("tab-{}", p.file_name().unwrap().to_string_lossy());
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
    let order = |cx: &TestAppContext| {
        cx.read(|cx| {
            let ws = ws.read(cx);
            let roots: Vec<_> = ws.tab_roots(cx).iter().map(|p| canon(p)).collect();
            (roots, ws.active_index())
        })
    };

    // C dropped on A takes its place; B stays the active tab.
    let (from, to) = (name(c.path()), name(a.path()));
    cx.update_window(handle, |_, window, cx| {
        let from = window.find(from.clone()).bounds().center();
        let to = window.find(to.clone()).bounds().center();
        window.drag(from, to, cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        order(cx),
        (vec![canon(c.path()), canon(a.path()), canon(b.path())], 2)
    );

    // A released above the window opens in a new one.
    let windows = cx.update(|cx| cx.windows().len());
    let from = name(a.path());
    cx.update_window(handle, |_, window, cx| {
        let from = window.find(from.clone()).bounds().center();
        window.drag(from, gpui_kit::point(from.x, px(-120.)), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(order(cx).0, vec![canon(c.path()), canon(b.path())]);
    assert_eq!(cx.update(|cx| cx.windows().len()), windows + 1);
}

#[gpui_kit::test]
fn new_commits_shimmer_and_worktree_branches_open_their_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (_handle, view) = open_repo_view(cx, dir.path());
    commit_file(dir.path(), "n.txt", "n\n", "brand new");
    cx.update(|cx| view.update(cx, |v, cx| v.reload(cx)));
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).shimmering(), 1));

    let wt = dir.path().with_extension("wt");
    git(
        dir.path(),
        &["worktree", "add", "-q", wt.to_str().unwrap(), "topic"],
    );
    cx.update(|cx| view.update(cx, |v, cx| v.reload(cx)));
    settle(cx, &view);
    let others = cx.read(|cx| view.read(cx).other_worktrees());
    assert_eq!(
        others
            .get("topic")
            .map(|p| std::fs::canonicalize(p).unwrap()),
        Some(std::fs::canonicalize(&wt).unwrap())
    );
    let _ = std::fs::remove_dir_all(&wt);
}

#[gpui_kit::test]
fn sidebar_pulse_counts_todays_commits_across_branches(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    settle(cx, &view);
    let mut days = cx.read(|cx| view.read(cx).pulse_days());
    for _ in 0..200 {
        if !days.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
        days = cx.read(|cx| view.read(cx).pulse_days());
    }
    assert_eq!(days.len(), 30);
    // c1, c2, f1, the merge and topic's commit, all made just now.
    assert_eq!(days.iter().sum::<u32>(), 5, "{days:?}");
    assert_eq!(*days.last().unwrap(), 5);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn finished_worktrees_are_listed_with_their_size(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let wt = dir.path().with_extension("merged-wt");
    let fresh = dir.path().with_extension("fresh-wt");
    git(
        dir.path(),
        &["worktree", "add", "-q", wt.to_str().unwrap(), "feature"],
    );
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "fresh",
            fresh.to_str().unwrap(),
        ],
    );
    let (handle, view) = open_repo_view(cx, dir.path());
    let dead: Vec<_> = cx.read(|cx| view.read(cx).dead_worktrees().to_vec());
    assert_eq!(dead.len(), 1, "{dead:?}");
    assert_eq!(dead[0].branch.as_deref(), Some("feature"));
    assert_eq!(dead[0].reason, mugetsu_git::DeadReason::Merged);
    assert!(dead[0].bytes > 0);
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();

    std::fs::remove_dir_all(&fresh).unwrap();
    cx.update(|cx| view.update(cx, |v, cx| v.reload(cx)));
    settle(cx, &view);
    let reasons: Vec<_> = cx.read(|cx| {
        view.read(cx)
            .dead_worktrees()
            .iter()
            .map(|d| d.reason)
            .collect()
    });
    assert!(
        reasons.contains(&mugetsu_git::DeadReason::Missing),
        "{reasons:?}"
    );
    let _ = std::fs::remove_dir_all(&wt);
}

#[gpui_kit::test]
fn deleting_a_branch_slashes_its_chip(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.delete_branch("feature".into(), false, window, cx)
        });
    })
    .unwrap();
    settle(cx, &view);
    assert!(!git(dir.path(), &["branch", "--list", "feature"]).contains("feature"));
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).slashed_refs(),
            [RefLabel::Branch {
                name: "feature".into(),
                head: false
            }]
        )
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn an_unmerged_branch_is_only_deleted_when_forced(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let delete = |force, cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, cx| {
                v.delete_branch("topic".into(), force, window, cx)
            });
        })
        .unwrap();
        settle(cx, &view);
    };
    delete(false, cx);
    assert!(git(dir.path(), &["branch", "--list", "topic"]).contains("topic"));
    delete(true, cx);
    assert!(!git(dir.path(), &["branch", "--list", "topic"]).contains("topic"));
}

#[gpui_kit::test]
fn deleting_a_remote_branch_can_delete_its_local_copy_too(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(remote.path(), &["init", "-q", "--bare"]);
    git(
        dir.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(dir.path(), &["push", "-q", "-u", "origin", "feature"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(
            v.local_twin("origin", "feature").as_deref(),
            Some("feature")
        );
        assert_eq!(v.local_twin("origin", "main"), None, "checked out");
    });
    ui_step(cx, handle, &|window, cx| {
        view.update(cx, |v, cx| {
            v.request_delete_remote_branch("origin".into(), "feature".into(), window, cx)
        })
    });
    // The dialog slides in first, as in the reset dialog test.
    std::thread::sleep(Duration::from_millis(500));
    ui_step(cx, handle, &|window, cx| window.click("confirm-option", cx));
    ui_step(cx, handle, &|window, cx| window.click("confirm-ok", cx));
    settle(cx, &view);
    assert!(git(remote.path(), &["branch", "--list", "feature"]).is_empty());
    assert!(
        !git(dir.path(), &["branch", "--list", "feature"]).contains("feature"),
        "the local copy went too"
    );
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).slashed_refs(),
            [
                RefLabel::Remote("origin/feature".into()),
                RefLabel::Branch {
                    name: "feature".into(),
                    head: false
                }
            ]
        )
    });
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn dropping_a_stash_slashes_its_chip(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("stashed.txt"), "wip").unwrap();
    git(dir.path(), &["stash", "push", "-q", "-u"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        view.update(cx, |v, cx| v.drop_stash(0, window, cx))
    });
    settle(cx, &view);
    assert!(git(dir.path(), &["stash", "list"]).trim().is_empty());
    cx.read(|cx| assert_eq!(view.read(cx).slashed_refs(), [RefLabel::Stash(0)]));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

#[gpui_kit::test]
fn f_key_jumps_to_head_and_shimmers_it(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let (head, other) = cx.read(|cx| {
        let v = view.read(cx);
        let head = v.data().refs.head.unwrap();
        let other = v
            .data()
            .history
            .commits
            .iter()
            .map(|c| c.id)
            .find(|id| *id != head)
            .unwrap();
        (head, other)
    });
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            v.select_commit(other, true, cx);
            window.focus(&v.history_focus_handle(), cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("f");
    vcx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(view.read(cx).selected_commit(), Some(head));
        assert_eq!(view.read(cx).shimmering(), 1);
    });
}

#[gpui_kit::test]
fn settings_sections_and_ai_tool_presets(cx: &mut TestAppContext) {
    use crate::settings::AiTool;
    use crate::settings_view::{Section, SettingsView};
    init(cx);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("section-ai", cx);
        window.render_frame(cx);
        window.click("ai-tool-codex", cx);
        window.render_frame(cx);
        window.click("ai-terminal-system", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).section(), Section::Ai));
    let s = crate::settings::get();
    assert_eq!(s.ai_tool, AiTool::Codex);
    assert_eq!(Some(s.ai_command.as_str()), AiTool::Codex.preset());
    assert_eq!(s.ai_terminal, crate::settings::AiTerminal::System);
}

#[gpui_kit::test]
fn notifications_settings_start_switched_off(cx: &mut TestAppContext) {
    use crate::settings_view::{Section, SettingsView};
    init(cx);
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        })
        .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("section-notifications", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).section(), Section::Notifications));
    assert!(!crate::settings::Settings::default().notifications.any());
}

#[gpui_kit::test]
fn a_stack_behind_its_remote_trunk_offers_a_restack(cx: &mut TestAppContext) {
    use mugetsu_forge::{PrState, PullRequest};
    init(cx);
    let origin = tempfile::tempdir().unwrap();
    fixture(origin.path());
    let clones = tempfile::tempdir().unwrap();
    git(
        clones.path(),
        &["clone", "-q", origin.path().to_str().unwrap(), "clone"],
    );
    let (handle, view) = open_repo_view(cx, &clones.path().join("clone"));
    let pr = |n: u64, head: &str, base: &str| PullRequest {
        number: n,
        title: format!("PR {n}"),
        url: String::new(),
        state: PrState::Open,
        status: None,
        head: head.into(),
        base: base.into(),
        same_repo: true,
        author: "me".into(),
        requested_reviewers: vec![],
        updated_at: String::new(),
        merged_at: None,
    };
    // topic forked from main before the merge: main has moved on.
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_pr_list(vec![pr(1, "topic", "main"), pr(2, "feature", "topic")], cx);
            v.open_stack("topic".into(), cx);
        })
    });
    let mut shown = false;
    for _ in 0..200 {
        cx.run_until_parked();
        shown = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("stack-drift").is_some()
            })
            .unwrap();
        if shown {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(shown, "the drift banner never showed");
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("stack-drift-restack").is_some());
        window.click("stack-drift-restack", cx);
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicking_a_stack_shows_its_progress(cx: &mut TestAppContext) {
    use mugetsu_forge::{PrState, PullRequest};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let pr = |n: u64, head: &str, base: &str| PullRequest {
        number: n,
        title: format!("PR {n}"),
        url: String::new(),
        state: PrState::Open,
        status: None,
        head: head.into(),
        base: base.into(),
        same_repo: true,
        author: "me".into(),
        requested_reviewers: vec![],
        updated_at: String::new(),
        merged_at: None,
    };
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_pr_list(vec![pr(1, "a", "master"), pr(2, "b", "a")], cx);
            v.open_stack("b".into(), cx);
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("stack-view").is_some());
        assert!(window.try_find("stack-pr-1").is_some());
        assert!(window.try_find("stack-pr-2").is_some());
        assert!(window.try_find("stack-next").is_some());
        // Clicking a card again and again: a hint about ⌃-click.
        for _ in 0..3 {
            window.click("stack-pr-1", cx);
        }
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(view.read(cx).mode(), crate::repo_view::Mode::Stack);
        assert_eq!(view.read(cx).rage_hints(), 1);
    });

    // A PR from the PRs tab opens in the main view.
    cx.update(|cx| view.update(cx, |v, cx| v.open_pr(2, cx)));
    settle(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("pr-view").is_some());
        assert!(window.try_find("pr-ci").is_some());
    })
    .unwrap();
    cx.read(|cx| assert_eq!(view.read(cx).mode(), crate::repo_view::Mode::Pr));
}

#[gpui_kit::test]
fn a_manual_fetch_plays_its_outcome_on_the_button(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update(|cx| view.update(cx, |v, cx| v.fetch_now(cx)));
    settle(cx, &view);
    cx.read(|cx| assert_eq!(view.read(cx).op_flash().map(|f| f.0), Some("Fetch")));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("fetch").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn compact_view_lists_branches_and_goes_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle-compact", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-history").is_some());
        assert!(window.try_find("compact-topic").is_some());
        window.click("toggle-compact", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-history").is_none());
    })
    .unwrap();
    settle(cx, &view);
}

#[gpui_kit::test]
fn commit_details_move_beside_the_history_and_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    settle(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle-details-side", cx);
        window.render_frame(cx);
        assert!(window.try_find("copy-sha").is_some());
    })
    .unwrap();
    cx.read(|cx| assert!(view.read(cx).details_side()));
    cx.update_window(handle, |_, window, cx| {
        window.click("toggle-details-side", cx);
        window.render_frame(cx);
        assert!(window.try_find("copy-sha").is_some());
    })
    .unwrap();
    cx.read(|cx| assert!(!view.read(cx).details_side()));
}

#[gpui_kit::test]
fn compact_view_filters_by_author_and_sorts(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fixture(p);
    git(p, &["switch", "-q", "-c", "ann-work", "HEAD~1"]);
    for msg in ["ann 1", "ann 2"] {
        git(
            p,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "--author=Ann <ann@example.com>",
                "-m",
                msg,
            ],
        );
    }
    git(p, &["switch", "-q", "main"]);
    let (handle, view) = open_repo_view(cx, p);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle-compact", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-ann-work").is_some());
        // The repository's own identity comes first, then the others.
        window.click("compact-author-test@example.com", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-topic").is_some());
        assert!(window.try_find("compact-ann-work").is_none());
        assert!(window.try_find("compact-main").is_some());
        window.click("compact-author-ann@example.com", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-ann-work").is_some());
        assert!(window.try_find("compact-topic").is_none());
        // Clicking the active author again shows everyone.
        window.click("compact-author-ann@example.com", cx);
        window.click("compact-sort-size", cx);
        window.render_frame(cx);
        assert!(window.try_find("compact-topic").is_some());
        assert!(window.try_find("compact-ann-work").is_some());
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).compact_sort,
            crate::settings::CompactSort::Size
        )
    });
    settle(cx, &view);
}

#[gpui_kit::test]
fn merge_conflicts_resolve_in_three_panes(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "a.txt", "one\ntwo\nthree\n", "base");
    commit_file(p, "gone.txt", "x\n", "gone");
    git(p, &["switch", "-q", "-c", "topic"]);
    commit_file(p, "a.txt", "one\nTWO-topic\nthree\n", "topic a");
    git(p, &["rm", "-q", "gone.txt"]);
    git(p, &["commit", "-q", "-m", "topic rm"]);
    git(p, &["switch", "-q", "main"]);
    commit_file(p, "a.txt", "one\nTWO-main\nthree\n", "main a");
    commit_file(p, "gone.txt", "y\n", "main gone");
    let repo = mugetsu_git::Repo::open(p).unwrap();
    assert!(!repo.merge("topic").unwrap());

    // Conflicts open the resolver on the first file.
    let (handle, view) = open_repo_view(cx, p);
    settle(cx, &view);
    cx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.mode(), Mode::Changes);
        assert_eq!(v.conflict_loaded(), Some("a.txt"));
        assert_eq!(
            v.data().operation.as_ref().map(|o| o.op),
            Some(mugetsu_git::Operation::Merge)
        );
    });

    // Take theirs, mark resolved: the next conflicted file opens.
    ui_step(cx, handle, &|window, cx| window.click("theirs:0", cx));
    ui_step(cx, handle, &|window, cx| {
        window.click("conflict-resolve", cx)
    });
    settle(cx, &view);
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "one\nTWO-topic\nthree\n"
    );
    assert_eq!(
        cx.read(|cx| view.read(cx).conflict_loaded().map(str::to_string)),
        Some("gone.txt".into())
    );

    // Deleted on their side: keep ours, then continue the merge.
    ui_step(cx, handle, &|window, cx| window.click("keep-ours", cx));
    settle(cx, &view);
    ui_step(cx, handle, &|window, cx| window.click("op-continue", cx));
    settle(cx, &view);
    cx.read(|cx| {
        let v = view.read(cx);
        assert!(v.data().operation.is_none());
        assert_eq!(v.data().conflicted().count(), 0);
    });
    assert_eq!(
        git(p, &["log", "-1", "--format=%p"])
            .split_whitespace()
            .count(),
        2,
        "merge commit"
    );
    assert_eq!(std::fs::read_to_string(p.join("gone.txt")).unwrap(), "y\n");
}

#[gpui_kit::test]
fn new_window_opens_an_empty_repository_window(cx: &mut TestAppContext) {
    init(cx);
    let before = cx.update(|cx| cx.windows().len());
    cx.update(crate::app_menu::new_window);
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), before + 1);
}

#[gpui_kit::test]
fn github_links_of_a_commit_its_files_and_lines(cx: &mut TestAppContext) {
    use crate::repo_view::links::LinkSpot;
    use gpui_kit::component::diff::{DiffLineRange, DiffSide};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let gh = mugetsu_forge::GithubRepo {
        owner: "o".into(),
        name: "r".into(),
    };
    cx.update(|cx| view.update(cx, |v, cx| v.set_github(gh, cx)));
    // HEAD is the merge; pick c2, which changes a.txt.
    let (c2, c1) = cx.read(|cx| {
        let data = view.read(cx).data().clone();
        let c2 = data
            .history
            .commits
            .iter()
            .find(|c| c.summary == "c2")
            .unwrap()
            .clone();
        (c2.id.to_string(), c2.parents[0].to_string())
    });
    let id = gix::ObjectId::from_hex(c2.as_bytes()).unwrap();
    cx.update(|cx| view.update(cx, |v, cx| v.select_commit(id, true, cx)));
    settle(cx, &view);
    let clipboard =
        |cx: &TestAppContext| cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let feedback = |cx: &TestAppContext| cx.read(|cx| view.read(cx).link_feedback());

    ui_step(cx, handle, &|window, cx| {
        window.click("copy-commit-link", cx)
    });
    assert_eq!(
        clipboard(cx),
        Some(format!("https://github.com/o/r/commit/{c2}"))
    );
    assert_eq!(feedback(cx), Some(LinkSpot::Commit));

    ui_step(cx, handle, &|window, cx| {
        window.click("copy-file-link-a.txt", cx)
    });
    assert_eq!(
        clipboard(cx),
        Some(format!("https://github.com/o/r/blob/{c2}/a.txt"))
    );
    assert_eq!(feedback(cx), Some(LinkSpot::File("a.txt".into())));

    // Removed lines link to the parent's file.
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_line_selection(DiffLineRange::new("a.txt", DiffSide::Original, 1, 1), cx)
        })
    });
    ui_step(cx, handle, &|window, cx| {
        window.click("copy-lines-link", cx)
    });
    assert_eq!(
        clipboard(cx),
        Some(format!("https://github.com/o/r/blob/{c1}/a.txt#L1"))
    );
    assert_eq!(feedback(cx), Some(LinkSpot::Lines));

    // ⌥⌘C copies the selected lines too.
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_line_selection(DiffLineRange::new("a.txt", DiffSide::Modified, 2, 1), cx)
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    ui_step(&mut vcx, handle, &|window, cx| {
        view.read(cx).history_focus_handle().focus(window, cx)
    });
    vcx.simulate_keystrokes("secondary-alt-c");
    vcx.run_until_parked();
    assert_eq!(
        clipboard(cx),
        Some(format!("https://github.com/o/r/blob/{c2}/a.txt#L1-L2"))
    );
}

#[gpui_kit::test]
fn command_k_palette_runs_actions_and_switches_repositories(cx: &mut TestAppContext) {
    init(cx);
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fixture(a.path());
    fixture(b.path());
    let session = session_of(vec![a.path().to_path_buf(), b.path().to_path_buf()], 0);
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    let palette = |cx: &TestAppContext| cx.read(|cx| ws.read(cx).palette());
    let view_a = cx.read(|cx| ws.read(cx).tabs()[0].clone());
    settle(cx, &view_a);
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.run_until_parked();

    // ⌘K opens it over the tab, grouped, with the repositories and branches.
    vcx.simulate_keystrokes("secondary-k");
    vcx.run_until_parked();
    let p = palette(cx).expect("⌘K opens the palette");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("command-palette").is_some());
        assert!(window.try_find("palette-fetch").is_some());
    })
    .unwrap();
    let ids = cx.read(|cx| p.read(cx).result_ids());
    let b_name = b.path().file_name().unwrap().to_string_lossy().to_string();
    assert!(
        ids.iter()
            .any(|id| id.starts_with("repo:") && id.ends_with(&b_name))
    );
    assert!(ids.iter().any(|id| *id == "branch:topic"));

    // Typing ranks the matches; arrows move, enter runs: Stats on the tab.
    handle
        .update(cx, |_, window, cx| {
            p.update(cx, |p, cx| p.set_query("stats", window, cx))
        })
        .unwrap();
    assert_eq!(
        cx.read(|cx| p.read(cx).selected_id()),
        Some("show-stats".into())
    );
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("down up enter");
    vcx.run_until_parked();
    assert!(cx.read(|cx| p.read(cx).is_closing()));
    assert_eq!(cx.read(|cx| view_a.read(cx).mode()), Mode::Stats);
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(palette(cx).is_none(), "dropped after its exit");

    // The other repository, by name.
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("secondary-k");
    vcx.run_until_parked();
    let p = palette(cx).unwrap();
    handle
        .update(cx, |_, window, cx| {
            p.update(cx, |p, cx| p.set_query(&b_name, window, cx))
        })
        .unwrap();
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| ws.read(cx).active_index()), 1);

    // Escape closes without running anything.
    cx.executor().advance_clock(Duration::from_millis(400));
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("secondary-k");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(palette(cx).is_none());
    assert_eq!(cx.read(|cx| ws.read(cx).active_index()), 1);

    // ⌘P: the same palette, branches only.
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("secondary-p");
    vcx.run_until_parked();
    let p = palette(cx).expect("⌘P opens the palette");
    let ids = cx.read(|cx| p.read(cx).result_ids());
    assert!(!ids.is_empty());
    assert!(ids.iter().all(|id| id.starts_with("branch:")), "{ids:?}");
}

/// main: base - one - two - three, each on its own file.
fn linear_fixture(dir: &Path) -> String {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    commit_file(dir, "base.txt", "base\n", "base");
    let base = git(dir, &["rev-parse", "HEAD"]).trim().to_string();
    commit_file(dir, "one.txt", "1\n", "one");
    commit_file(dir, "two.txt", "2\n", "two");
    commit_file(dir, "three.txt", "3\n", "three");
    base
}

fn messages(dir: &Path) -> Vec<String> {
    git(dir, &["log", "--reverse", "--format=%B%x1e"])
        .split('\x1e')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[gpui_kit::test]
fn interactive_rebase_by_drag_and_drop(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let base = linear_fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let base_id = gix::ObjectId::from_hex(base.as_bytes()).unwrap();
    ui_step(cx, handle, &|window, cx| {
        view.update(cx, |v, cx| v.open_rebase(base_id, window, cx))
    });
    settle(cx, &view);
    // Let the sheet and its rows finish easing in.
    std::thread::sleep(Duration::from_millis(700));
    // Newest first: three, two, one. Move "one" to the top…
    ui_step(cx, handle, &|window, cx| {
        window.drag_to("rebase-row-2", "rebase-row-0", cx)
    });
    // …then squash "three" (now second) into "two" below it.
    ui_step(cx, handle, &|window, cx| {
        window.click("rebase-squash-1", cx)
    });
    ui_step(cx, handle, &|window, cx| window.click("rebase-ok", cx));
    settle(cx, &view);
    assert_eq!(messages(dir.path()), ["base", "two\n\nthree", "one"]);
    for file in ["one.txt", "two.txt", "three.txt"] {
        assert!(dir.path().join(file).exists(), "{file} kept");
    }
}

#[gpui_kit::test]
fn stopped_interactive_rebase_opens_the_resolver(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.name", "Test"]);
    git(p, &["config", "user.email", "test@example.com"]);
    commit_file(p, "a.txt", "base\n", "base");
    commit_file(p, "a.txt", "one\n", "one");
    commit_file(p, "a.txt", "two\n", "two");
    let head = git(p, &["rev-parse", "HEAD"]);
    let base = git(p, &["rev-parse", "HEAD~2"]).trim().to_string();
    let (handle, view) = open_repo_view(cx, p);
    let plan = view.read_with(cx, |v, _| v.repo().rebase_plan(&base).unwrap());
    // Swapping two edits of the same line conflicts.
    let swapped: Vec<_> = plan
        .iter()
        .rev()
        .map(|c| mugetsu_git::RebaseStep {
            id: c.id.clone(),
            action: mugetsu_git::RebaseAction::Pick,
            message: None,
        })
        .collect();
    ui_step(cx, handle, &|window, cx| {
        let (base, steps) = (base.clone(), swapped.clone());
        view.update(cx, |v, cx| {
            v.run_rebase(base, steps, mugetsu_git::LocalChanges::Stash, window, cx)
        })
    });
    settle(cx, &view);
    cx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(
            v.data().operation.as_ref().map(|o| o.op),
            Some(mugetsu_git::Operation::Rebase),
            "stopped on the conflict"
        );
        assert_eq!(v.mode(), Mode::Changes, "the resolver takes over");
    });
    ui_step(cx, handle, &|window, _| {
        assert!(
            window.try_find("op-abort").is_some(),
            "Continue / Skip / Abort"
        );
    });
    git(p, &["rebase", "--abort"]);
    view.update(cx, |v, cx| v.reload(cx));
    settle(cx, &view);
    assert_eq!(git(p, &["rev-parse", "HEAD"]), head);
    cx.read(|cx| assert!(view.read(cx).data().operation.is_none()));
}

#[gpui_kit::test]
fn control_r_fetches_every_tab_and_control_shift_r_replays_the_opening(cx: &mut TestAppContext) {
    init(cx);
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fixture(a.path());
    fixture(b.path());
    let session = session_of(vec![a.path().to_path_buf(), b.path().to_path_buf()], 0);
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    let views = cx.read(|cx| ws.read(cx).tabs());
    for v in &views {
        settle(cx, v);
    }
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-r");
    for v in &views {
        settle(cx, v);
        cx.read(|cx| assert_eq!(v.read(cx).op_flash().map(|f| f.0), Some("Fetch")));
    }

    assert_eq!(cx.read(|cx| ws.read(cx).intro()), None);
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("ctrl-shift-r");
    assert_eq!(cx.read(|cx| ws.read(cx).intro()), Some(1));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("eclipse").is_some());
    })
    .unwrap();
    // A second reload restarts it rather than stacking.
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("ctrl-shift-r");
    assert_eq!(cx.read(|cx| ws.read(cx).intro()), Some(2));
    for v in &views {
        settle(cx, v);
    }
    cx.executor().advance_clock(crate::welcome::ECLIPSE);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| ws.read(cx).intro()), None);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("eclipse").is_none());
    })
    .unwrap();
}

mod components;
mod flows;

/// A click with modifiers held (the test helpers click without any).
fn click_with(
    window: &mut gpui_kit::Window,
    id: &str,
    modifiers: gpui_kit::Modifiers,
    cx: &mut gpui_kit::App,
) {
    use gpui_kit::test::TestWindowExt as _;
    window.render_frame(cx);
    let position = window
        .find(gpui_kit::ElementId::Name(id.to_string().into()))
        .bounds()
        .center();
    let button = gpui_kit::MouseButton::Left;
    window.dispatch_event(
        gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }),
        cx,
    );
    window.dispatch_event(
        gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        gpui_kit::PlatformInput::MouseUp(gpui_kit::MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
    window.render_frame(cx);
}

#[gpui_kit::test]
fn command_click_picks_branches_then_deletes_them_together(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(dir.path(), &["branch", "spare", "HEAD~1"]);
    let (handle, view) = open_repo_view(cx, dir.path());
    let cmd = gpui_kit::Modifiers {
        platform: true,
        ..Default::default()
    };
    cx.update_window(handle, |_, window, cx| {
        window.click("branch:feature", cx);
        click_with(window, "branch:spare", cmd, cx);
        click_with(window, "branch:topic", cmd, cx);
        click_with(window, "branch:topic", cmd, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(view.read(cx).picked_branches(), vec!["feature", "spare"]);
    });

    // A plain click starts over.
    cx.update_window(handle, |_, window, cx| window.click("branch:topic", cx))
        .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(view.read(cx).picked_branches().is_empty()));

    // ⌘-click on a history row picks the branches on that commit.
    let topic = cx.read(|cx| {
        let v = view.read(cx);
        v.data()
            .refs
            .branches
            .iter()
            .find(|b| b.name == "spare")
            .unwrap()
            .target
    });
    cx.update_window(handle, |_, window, cx| {
        click_with(window, &topic.to_string(), cmd, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(view.read(cx).picked_branches(), vec!["spare", "topic"]);
    });

    // topic is unmerged: deleted only once forced.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| {
            let names = v.picked_branches();
            v.delete_branches(names, false, window, cx)
        });
    })
    .unwrap();
    settle(cx, &view);
    let left = git(dir.path(), &["branch", "--format=%(refname:short)"]);
    assert!(!left.contains("spare"), "{left}");
    assert!(left.contains("topic"), "{left}");
    cx.read(|cx| assert_eq!(view.read(cx).picked_branches(), vec!["topic"]));
}

#[gpui_kit::test]
fn file_context_menu_opens_history_and_blame(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    git(dir.path(), &["mv", "a.txt", "moved.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "move"]);
    commit_file(dir.path(), "moved.txt", "one\ntwo\nthree\n", "three");
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        window.right_click("commit-file-moved.txt", cx)
    });
    ui_step(cx, handle, &|window, cx| {
        let history = window.within("popup-menu").find(1usize);
        assert_eq!(history.label(), Some("Historique du fichier"));
        window.within("popup-menu").click(1usize, cx);
    });
    settle(cx, &view);
    cx.run_until_parked();
    let head = cx.read(|cx| view.read(cx).data().refs.head.unwrap());
    cx.read(|cx| {
        let v = view.read(cx);
        assert_eq!(v.mode(), Mode::File);
        let page = v.file_page().unwrap();
        assert_eq!(page.path, "moved.txt");
        assert_eq!(
            page.selected,
            Some(head),
            "opens on the commit it came from"
        );
        let log = match &page.log {
            Some(Ok(log)) => log.clone(),
            other => panic!("{:?}", other.as_ref().map(|r| r.as_ref().map(|l| l.len()))),
        };
        let summaries: Vec<&str> = log.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(
            summaries,
            ["three", "move", "c2", "c1"],
            "follows the rename"
        );
        let blame = match &page.blame {
            Some(Ok(lines)) => lines.clone(),
            _ => panic!("no blame"),
        };
        assert_eq!(blame.len(), 3);
        assert_eq!(blame[0].summary, "c1");
        assert_eq!(blame[2].summary, "three");
    });
    ui_step(cx, handle, &|window, cx| window.click("file-tab-blame", cx));
    ui_step(cx, handle, &|window, cx| {
        assert!(window.try_find("file-commits").is_none());
        window.click(("blame-line", 1usize), cx);
    });
    cx.read(|cx| {
        let page = view.read(cx).file_page().unwrap();
        assert_eq!(
            page.tab,
            crate::repo_view::file_view::FileTab::History,
            "a blame line opens its commit"
        );
        let c2 = page.log.as_ref().unwrap().as_ref().unwrap()[2].id;
        assert_eq!(page.selected, Some(c2));
    });
    ui_step(cx, handle, &|window, cx| window.click("file-close", cx));
    cx.read(|cx| assert_eq!(view.read(cx).mode(), Mode::History));
}

// ---- Overview and weekly recap ---------------------------------------------

fn open_workspace(
    cx: &mut TestAppContext,
    repos: Vec<std::path::PathBuf>,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let session = session_of(repos, 0);
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    for tab in cx.read(|cx| ws.read(cx).tabs()) {
        settle(cx, &tab);
    }
    (handle, ws)
}

#[gpui_kit::test]
fn overview_cards_lead_to_what_needs_doing(cx: &mut TestAppContext) {
    init(cx);
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    fixture(a.path());
    fixture(b.path());
    std::fs::write(b.path().join("draft.txt"), "wip\n").unwrap();
    let (handle, ws) = open_workspace(cx, vec![a.path().to_path_buf(), b.path().to_path_buf()]);
    let tabs = cx.read(|cx| ws.read(cx).tabs());
    let card = cx.read(|cx| tabs[1].read(cx).card());
    assert_eq!(card.signals(), [crate::overview::Signal::Dirty(1)]);
    assert!(cx.read(|cx| tabs[0].read(cx).card().signals().is_empty()));

    ui_step(cx, handle, &|window, cx| {
        window.click("overview-toggle", cx)
    });
    assert!(cx.read(|cx| ws.read(cx).overview_shown()));
    let name = cx.read(|cx| tabs[1].read(cx).name());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("overview").is_some());
        assert!(
            window
                .try_find(gpui_kit::SharedString::from(format!(
                    "overview-card-{name}"
                )))
                .is_some()
        );
    })
    .unwrap();

    // The second card's first signal: its tab, on the Changes view.
    ui_step(cx, handle, &|window, cx| {
        window.click("overview-signal-1-0", cx)
    });
    assert!(!cx.read(|cx| ws.read(cx).overview_shown()));
    assert_eq!(cx.read(|cx| ws.read(cx).active_index()), 1);
    assert_eq!(cx.read(|cx| tabs[1].read(cx).mode()), Mode::Changes);

    // ⇧⌘O comes back to it, a tab click leaves it.
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-o"
    } else {
        "ctrl-shift-o"
    });
    vcx.run_until_parked();
    assert!(cx.read(|cx| ws.read(cx).overview_shown()));
    let first = cx.read(|cx| tabs[0].read(cx).name());
    ui_step(cx, handle, &|window, cx| {
        window.click(gpui_kit::SharedString::from(format!("tab-{first}")), cx)
    });
    assert!(!cx.read(|cx| ws.read(cx).overview_shown()));
    assert_eq!(cx.read(|cx| ws.read(cx).active_index()), 0);
}

#[gpui_kit::test]
fn weekly_recap_counts_my_commits_and_steps_through_slides(cx: &mut TestAppContext) {
    init(cx);
    let a = tempfile::tempdir().unwrap();
    fixture(a.path());
    let (handle, ws) = open_workspace(cx, vec![a.path().to_path_buf()]);
    ui_step(cx, handle, &|window, cx| {
        ws.update(cx, |ws, cx| ws.open_recap(window, cx))
    });
    let recap = cx.read(|cx| ws.read(cx).recap()).expect("recap shown");
    let mut computed = None;
    for _ in 0..200 {
        cx.run_until_parked();
        computed = cx.read(|cx| recap.read(cx).computed());
        if computed.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let computed = computed.expect("recap loaded");
    // c1, c2, f1 and "topic work" (merges left out), all by the repo's user.
    assert_eq!(computed.mine.week.commits, 4);
    assert_eq!(computed.team.week.commits, 4);
    assert_eq!(computed.team.week.people.len(), 1);

    let (slide, count, team) = cx.read(|cx| recap.read(cx).position());
    assert_eq!((slide, team), (0, false));
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("right right");
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| recap.read(cx).position().0), 2);
    vcx.simulate_keystrokes("tab");
    vcx.run_until_parked();
    assert!(
        cx.read(|cx| recap.read(cx).position().2),
        "tab switches to the team"
    );
    for _ in 0..count {
        vcx.simulate_keystrokes("right");
    }
    vcx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find("recap-export").is_some(),
            "last slide exports"
        );
    })
    .unwrap();
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(cx.read(|cx| ws.read(cx).recap()).is_none());
}

#[gpui_kit::test]
fn side_buttons_and_shortcuts_go_back_and_forward(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let (head, other) = cx.read(|cx| {
        let v = view.read(cx);
        let head = v.data().refs.head.unwrap();
        let other = v
            .data()
            .history
            .commits
            .iter()
            .map(|c| c.id)
            .find(|id| *id != head);
        (head, other.unwrap())
    });
    let step = |cx: &mut TestAppContext,
                f: &dyn Fn(&mut RepoView, &mut gpui_kit::Context<RepoView>)| {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |v, cx| f(v, cx));
            window.render_frame(cx);
        })
        .unwrap();
    };
    step(cx, &|_, _| {});
    step(cx, &|v, cx| v.select_commit(other, true, cx));
    step(cx, &|v, cx| v.set_mode(Mode::Stats, cx));
    assert!(cx.read(|cx| !view.read(cx).can_navigate(false)));

    // The mouse's back button, anywhere over the tab (here a stats button).
    cx.update_window(handle, |_, window, cx| {
        let position = window
            .find(gpui_kit::ElementId::Name("stats-period-30".into()))
            .bounds()
            .center();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Navigate(gpui_kit::NavigationDirection::Back),
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(view.read(cx).mode(), Mode::History);
        assert_eq!(view.read(cx).selected_commit(), Some(other));
    });

    // Then the keyboard: back to HEAD, forward twice to the stats.
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |v, cx| window.focus(&v.history_focus_handle(), cx));
        window.render_frame(cx);
    })
    .unwrap();
    let (back, forward) = if cfg!(target_os = "macos") {
        ("cmd-[", "cmd-]")
    } else {
        ("ctrl-alt-left", "ctrl-alt-right")
    };
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes(back);
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| view.read(cx).selected_commit()), Some(head));
    assert!(cx.read(|cx| !view.read(cx).can_navigate(true)));
    let mut vcx = gpui_kit::VisualTestContext::from_window(handle, cx);
    vcx.simulate_keystrokes(forward);
    vcx.simulate_keystrokes(forward);
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| view.read(cx).mode()), Mode::Stats);
}

/// Opens the Changes view of a fixture with `a.txt` modified and `new.txt` untracked.
fn changes_view(cx: &mut TestAppContext, dir: &Path) -> (AnyWindowHandle, Entity<RepoView>) {
    fixture(dir);
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir);
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.set_focused(true, cx);
            v.set_mode(Mode::Changes, cx);
        })
    });
    settle(cx, &view);
    ui_step(cx, handle, &|_, _| {});
    (handle, view)
}

fn shown_staged(cx: &TestAppContext, view: &Entity<RepoView>) -> Vec<String> {
    cx.read(|cx| {
        view.read(cx)
            .data()
            .status
            .staged()
            .map(|f| f.path.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn staging_moves_the_file_before_git_runs(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (handle, view) = changes_view(cx, dir.path());
    // Moved on the click itself, not after git and a reload.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle:false:a.txt", cx);
    })
    .unwrap();
    assert_eq!(shown_staged(cx, &view), ["a.txt"]);
    settle(cx, &view);
    assert_eq!(
        git(dir.path(), &["diff", "--cached", "--name-only"]).trim(),
        "a.txt"
    );
    // Back with a double click on the staged row.
    ui_step(cx, handle, &|window, cx| {
        window.double_click("staged:a.txt", cx)
    });
    assert!(shown_staged(cx, &view).is_empty());
    settle(cx, &view);
    assert!(
        git(dir.path(), &["diff", "--cached", "--name-only"])
            .trim()
            .is_empty()
    );
}

#[gpui_kit::test]
fn a_failed_stage_moves_the_file_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (handle, view) = changes_view(cx, dir.path());
    // Another git holds the index: `git add` fails.
    let lock = dir.path().join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle:false:a.txt", cx);
    })
    .unwrap();
    assert_eq!(shown_staged(cx, &view), ["a.txt"]);
    settle(cx, &view);
    assert!(shown_staged(cx, &view).is_empty(), "rolled back");
    std::fs::remove_file(lock).unwrap();
}

#[gpui_kit::test]
fn keys_stage_and_discard_the_selected_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (handle, view) = changes_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        window.click("unstaged:new.txt", cx)
    });
    ui_step(cx, handle, &|window, cx| window.press("space", cx));
    assert_eq!(shown_staged(cx, &view), ["new.txt"]);
    settle(cx, &view);
    // The selection followed the file; ↑ goes back to a.txt, then ⌘⌫ asks first.
    ui_step(cx, handle, &|window, cx| window.press("up", cx));
    ui_step(cx, handle, &|window, cx| {
        window.press("secondary-backspace", cx)
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed\n",
        "nothing discarded before the confirmation"
    );
    std::thread::sleep(Duration::from_millis(500));
    ui_step(cx, handle, &|window, cx| window.click("ok", cx));
    settle(cx, &view);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(
        git(dir.path(), &["status", "--porcelain"]).trim(),
        "A  new.txt"
    );
}

#[gpui_kit::test]
fn change_menu_shows_shortcuts_and_ignores(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (handle, view) = changes_view(cx, dir.path());
    ui_step(cx, handle, &|window, cx| {
        window.click("unstaged:new.txt", cx)
    });
    ui_step(cx, handle, &|window, cx| {
        window.right_click("unstaged:new.txt", cx)
    });
    ui_step(cx, handle, &|window, _| {
        let menu = window.within("popup-menu");
        assert_eq!(menu.find(0usize).label(), Some("Ajouter à l'index"));
        assert_eq!(
            menu.find(1usize).label(),
            Some("Annuler les modifications…")
        );
    });
    ui_step(cx, handle, &|window, cx| window.press("escape", cx));
    // Ignore › the file itself (the submenu's first entry).
    ui_step(cx, handle, &|window, cx| {
        view.update(cx, |v, cx| {
            v.ignore_change("new.txt".into(), "/new.txt".into(), window, cx)
        })
    });
    assert!(
        cx.read(|cx| view
            .read(cx)
            .data()
            .status
            .files
            .iter()
            .all(|f| f.path != "new.txt")),
        "gone from the list right away"
    );
    settle(cx, &view);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        "/new.txt\n"
    );
}

#[gpui_kit::test]
fn filter_narrows_both_lists(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let (handle, view) = changes_view(cx, dir.path());
    git(dir.path(), &["add", "new.txt"]);
    cx.update(|cx| view.update(cx, |v, cx| v.reload(cx)));
    settle(cx, &view);
    ui_step(cx, handle, &|window, cx| window.press("secondary-f", cx));
    ui_step(cx, handle, &|window, cx| window.input("NEW", cx));
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("staged:new.txt").is_some());
        assert!(window.try_find("unstaged:a.txt").is_none());
    });
    // "Stage shown" leaves what the filter hides.
    ui_step(cx, handle, &|window, cx| window.click("change-filter", cx));
    ui_step(cx, handle, &|window, cx| {
        window.press("secondary-a", cx);
        window.input("a.t", cx);
    });
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("unstaged:a.txt").is_some());
        assert!(window.try_find("staged:new.txt").is_none());
    });
}
