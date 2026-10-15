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
        let rename = window.within("popup-menu").find(8usize);
        assert_eq!(rename.label(), Some("Renommer…"));
        window.within("popup-menu").click(8usize, cx);
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
