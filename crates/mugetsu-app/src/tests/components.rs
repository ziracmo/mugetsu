//! Component tests: one view at a time, checked through what it renders.

use super::*;

/// Index of the open context menu's item labelled `label`.
pub(super) fn menu_item(window: &mut gpui_kit::Window, label: &str) -> Option<usize> {
    let menu = window.within("popup-menu");
    (0..64usize).find(|&i| menu.try_find(i).is_some_and(|e| e.label() == Some(label)))
}

#[gpui_kit::test]
fn empty_workspace_shows_the_welcome_screen(cx: &mut TestAppContext) {
    init(cx);
    let session = crate::session::Session::new(
        vec![crate::session::RepoSet {
            name: "Vide".into(),
            repos: vec![],
            active: 0,
        }],
        0,
    );
    let (handle, ws) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(session, false, window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| ws.read(cx).active().is_none()));
    ui_step(cx, handle, &|window, _| {
        for id in ["welcome", "open-first"] {
            assert!(window.try_find(id).is_some(), "{id} is shown");
        }
    });
}

#[gpui_kit::test]
fn clean_worktree_has_no_uncommitted_banner(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("wip").is_none());
    });

    std::fs::write(dir.path().join("a.txt"), "edited\n").unwrap();
    cx.update(|cx| view.update(cx, |v, cx| v.reload(cx)));
    settle(cx, &view);
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("wip").is_some(), "banner once dirty");
    });
}

#[gpui_kit::test]
fn changes_view_splits_staged_and_unstaged(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("a.txt"), "staged\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    std::fs::write(dir.path().join("new.txt"), "untracked\n").unwrap();
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update(|cx| view.update(cx, |v, cx| v.set_mode(Mode::Changes, cx)));
    ui_step(cx, handle, &|window, _| {
        assert!(window.try_find("commit-message").is_some());
        assert!(
            window.try_find("toggle:true:a.txt").is_some(),
            "a.txt is listed as staged"
        );
        assert!(
            window.try_find("toggle:false:new.txt").is_some(),
            "new.txt is listed as unstaged"
        );
        assert!(window.try_find("toggle:false:a.txt").is_none());
        assert!(window.try_find("toggle:true:new.txt").is_none());
    });
}

#[gpui_kit::test]
fn commit_context_menu_offers_the_commit_actions(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (handle, view) = open_repo_view(cx, dir.path());
    let first = git(dir.path(), &["rev-list", "--max-parents=0", "HEAD"]);
    let first = first.trim().to_string();
    ui_step(cx, handle, &|window, cx| {
        window.right_click(gpui_kit::SharedString::from(first.clone()), cx)
    });
    ui_step(cx, handle, &|window, _| {
        for label in [
            "Nouvelle branche…",
            "Nouveau tag…",
            "Checkout du commit",
            "Cherry-pick",
            "Revert",
            "Copier le SHA",
        ] {
            assert!(menu_item(window, label).is_some(), "menu has {label:?}");
        }
    });
    let _ = view;
}

#[gpui_kit::test]
fn shift_click_range_and_select_all_move_several_files(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for name in ["n1.txt", "n2.txt", "n3.txt", "n4.txt"] {
        std::fs::write(dir.path().join(name), "new\n").unwrap();
    }
    let (handle, view) = open_repo_view(cx, dir.path());
    cx.update(|cx| view.update(cx, |v, cx| v.set_mode(Mode::Changes, cx)));
    settle(cx, &view);
    let sel = |path: &str, staged: bool| crate::repo_view::SelectedChange {
        path: path.into(),
        staged,
    };
    let staged = |cx: &mut TestAppContext| -> Vec<String> {
        cx.read(|cx| {
            view.read(cx)
                .data()
                .status
                .staged()
                .map(|f| f.path.clone())
                .collect()
        })
    };

    // n1 is selected first; ⇧ n3 picks n1..n3, which the header button moves.
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.select_change_range(sel("n3.txt", false), false, cx)
        })
    });
    ui_step(cx, handle, &|window, cx| window.click("stage-all", cx));
    settle(cx, &view);
    assert_eq!(staged(cx), ["n1.txt", "n2.txt", "n3.txt"]);

    // ⌘A in the staged list, then unstage: all go back.
    cx.update(|cx| {
        view.update(cx, |v, cx| {
            v.select_change_range(sel("n2.txt", true), false, cx);
            v.select_all_changes(cx);
        })
    });
    ui_step(cx, handle, &|window, cx| window.click("unstage-all", cx));
    settle(cx, &view);
    assert!(staged(cx).is_empty());
}
