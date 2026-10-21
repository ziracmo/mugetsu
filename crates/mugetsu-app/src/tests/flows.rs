//! Application tests: whole user flows, from a click in the history to the
//! repository on disk.

use super::components::menu_item;
use super::*;

/// Right-clicks the commit `rev` in the history and picks `label`.
fn commit_action(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    dir: &Path,
    rev: &str,
    label: &str,
) {
    let sha = git(dir, &["rev-parse", rev]).trim().to_string();
    ui_step(cx, handle, &|window, cx| {
        window.right_click(gpui_kit::SharedString::from(sha.clone()), cx)
    });
    ui_step(cx, handle, &|window, cx| {
        let i = menu_item(window, label).unwrap_or_else(|| panic!("no {label:?} item"));
        window.within("popup-menu").click(i, cx);
    });
}

/// Types `text` in the open prompt and confirms it.
fn answer_prompt(cx: &mut TestAppContext, handle: AnyWindowHandle, text: &str) {
    ui_step(cx, handle, &|window, cx| window.input(text, cx));
    ui_step(cx, handle, &|window, cx| window.press("enter", cx));
}

fn branches(cx: &TestAppContext, view: &Entity<RepoView>) -> Vec<String> {
    cx.read(|cx| {
        view.read(cx)
            .data()
            .refs
            .branches
            .iter()
            .map(|b| b.name.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn new_branch_from_a_commit_is_created_and_checked_out(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "main~1", "Nouvelle branche…");
    answer_prompt(cx, handle, "hotfix");
    settle(cx, &view);

    assert!(branches(cx, &view).contains(&"hotfix".to_string()));
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "hotfix"
    );
    assert_eq!(
        git(dir.path(), &["rev-parse", "HEAD"]),
        git(dir.path(), &["rev-parse", "main~1"])
    );
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).data().refs.head_branch.as_deref(),
            Some("hotfix"),
            "the view followed the checkout"
        )
    });
}

#[gpui_kit::test]
fn new_tag_on_a_commit_shows_up_in_the_refs(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "topic", "Nouveau tag…");
    answer_prompt(cx, handle, "v0.1");
    settle(cx, &view);

    assert_eq!(
        git(dir.path(), &["rev-parse", "v0.1^{commit}"]),
        git(dir.path(), &["rev-parse", "topic"])
    );
    cx.read(|cx| {
        assert!(
            view.read(cx)
                .data()
                .refs
                .tags
                .iter()
                .any(|t| t.name == "v0.1")
        )
    });
}

#[gpui_kit::test]
fn cherry_pick_brings_a_commit_onto_head(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "topic", "Cherry-pick");
    settle(cx, &view);

    assert_eq!(head_summary(cx, &view), "topic work");
    assert_eq!(
        git(dir.path(), &["branch", "--show-current"]).trim(),
        "main"
    );
    assert!(dir.path().join("t.txt").exists());
}

#[gpui_kit::test]
fn revert_undoes_a_commit_with_a_new_one(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    commit_file(dir.path(), "c.txt", "c\n", "add c");
    let (handle, view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "HEAD", "Revert");
    settle(cx, &view);

    assert!(head_summary(cx, &view).starts_with("Revert \"add c\""));
    assert!(!dir.path().join("c.txt").exists());
}

#[gpui_kit::test]
fn detached_checkout_of_a_commit(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "main~1", "Checkout du commit");
    settle(cx, &view);

    assert!(
        git(dir.path(), &["branch", "--show-current"])
            .trim()
            .is_empty()
    );
    cx.read(|cx| {
        let data = view.read(cx).data();
        assert!(data.refs.head_branch.is_none());
        assert_eq!(
            data.refs.head.map(|h| h.to_string()),
            Some(git(dir.path(), &["rev-parse", "HEAD"]).trim().to_string())
        );
    });
}

#[gpui_kit::test]
fn copy_sha_from_the_menu(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, _view) = open_repo_view(cx, dir.path());
    commit_action(cx, handle, dir.path(), "topic", "Copier le SHA");
    let topic = git(dir.path(), &["rev-parse", "topic"]).trim().to_string();
    cx.read(|cx| assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(topic)));
}

#[gpui_kit::test]
fn a_pasted_url_is_cloned_into_the_workspace_folder_and_opened(cx: &mut TestAppContext) {
    init(cx);
    let origin = tempfile::tempdir().unwrap();
    fixture(origin.path());
    let open = tempfile::tempdir().unwrap();
    fixture(open.path());
    let workspace = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        crate::settings::update(cx, |s| {
            s.clone_dir = Some(workspace.path().to_path_buf());
            s.clone_ask = false;
        })
    });
    let session = session_of(vec![open.path().to_path_buf()], 0);
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    ui_step(cx, handle, &|window, cx| {
        window.dispatch_action(Box::new(crate::workspace::OpenRepository), cx)
    });
    let url = format!("file://{}/", origin.path().display());
    ui_step(cx, handle, &|window, cx| window.input(&url, cx));
    ui_step(cx, handle, &|window, cx| {
        assert!(
            window.try_find("open-url").is_some(),
            "a clone row for the URL"
        );
        window.press("enter", cx)
    });
    let name = origin.path().file_name().unwrap().to_owned();
    let dest = workspace.path().join(&name);
    for _ in 0..300 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if cx.read(|cx| ws.read(cx).tab_roots(cx).len()) == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        dest.join("a.txt").exists(),
        "cloned into the workspace folder"
    );
    let roots = cx.read(|cx| ws.read(cx).tab_roots(cx));
    assert_eq!(roots.len(), 2, "the clone opens in a new tab");
    assert_eq!(
        roots[1].canonicalize().unwrap(),
        dest.canonicalize().unwrap()
    );
    cx.update(|cx| crate::settings::update(cx, |s| s.clone_dir = None));
}

#[gpui_kit::test]
fn catch_up_rebases_the_picked_branches_from_the_keyboard(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fixture(p);
    // A second branch lagging behind main, like topic.
    git(p, &["branch", "side", "topic~1"]);
    git(p, &["switch", "-q", "side"]);
    commit_file(p, "s.txt", "s\n", "side work");
    git(p, &["switch", "-q", "main"]);
    let side_before = git(p, &["rev-parse", "side"]);
    let (handle, view) = open_repo_view(cx, p);
    for _ in 0..200 {
        cx.run_until_parked();
        if cx.read(|cx| view.read(cx).catch_up_ready()) == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(cx.read(|cx| view.read(cx).catch_up_ready()), 2);
    ui_step(cx, handle, &|window, cx| window.click("catch-up-badge", cx));
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("catch-up:side").is_some());
        assert!(window.try_find("catch-up:topic").is_some());
    });
    // Rows are alphabetical: S skips side and moves to topic, ⌘↵ rebases.
    ui_step(cx, handle, &|window, cx| window.press("s", cx));
    ui_step(cx, handle, &|window, cx| {
        window.press("secondary-enter", cx)
    });
    settle(cx, &view);
    assert_eq!(
        git(p, &["rev-parse", "topic~1"]),
        git(p, &["rev-parse", "main"])
    );
    assert_eq!(git(p, &["rev-parse", "side"]), side_before, "skipped");
    assert_eq!(git(p, &["branch", "--show-current"]).trim(), "main");
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("catch-up-close").is_some(), "outcome shown")
    });
}
