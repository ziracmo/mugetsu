//! macOS menu bar: Mugetsu (About, Settings…, Quit), File, Repository, View.

use gpui_kit::{AnyWindowHandle, App, Entity, Global, KeyBinding, Menu, MenuItem};

use crate::i18n::tr;
use crate::repo_view as repo;
use crate::workspace as ws;

gpui_kit::actions!(app, [About, OpenSettings, Quit, ClearRecent, NewWindow]);

/// File > Open Recent > n-th repository of `Settings::recent`.
#[derive(Clone, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct OpenRecent {
    pub index: usize,
}

/// The main window's workspace, for menu actions without a focused view.
pub struct MainWorkspace(pub AnyWindowHandle, pub Entity<crate::workspace::Workspace>);
impl Global for MainWorkspace {}

/// `~/Workspace/x` for a path in the home folder, like VS Code's Open Recent.
pub fn display_path(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|h| path.strip_prefix(h).ok().map(|p| p.to_path_buf())) {
        Some(rel) => format!("~/{}", rel.display()),
        None => path.display().to_string(),
    }
}

fn open_recent(action: &OpenRecent, cx: &mut App) {
    let Some(path) = crate::settings::get().recent.get(action.index).cloned() else {
        return;
    };
    let Some(MainWorkspace(handle, ws)) = cx.try_global::<MainWorkspace>() else {
        return;
    };
    let ws = ws.clone();
    let _ = handle.update(cx, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_or_report(path, window, cx))
    });
}

/// File > New Window: a second repository window starting empty; its tabs
/// are not saved (the first window keeps the session).
pub(crate) fn new_window(cx: &mut App) {
    new_window_with(Vec::new(), None, cx);
}

/// A second repository window on `repos` (a tab dragged out of a window),
/// at `bounds` or centred.
pub(crate) fn new_window_with(
    repos: Vec<std::path::PathBuf>,
    bounds: Option<gpui_kit::Bounds<gpui_kit::Pixels>>,
    cx: &mut App,
) {
    let session = crate::session::Session::new(
        vec![crate::session::RepoSet {
            name: tr("Fenêtre", "Window").into(),
            repos,
            active: 0,
        }],
        0,
    );
    let _ = crate::open_workspace_window(session, false, bounds, cx);
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-shift-n", NewWindow, None),
    ]);
    cx.on_action(|_: &About, cx| crate::about::open(cx));
    cx.on_action(|_: &NewWindow, cx| new_window(cx));
    cx.on_action(|_: &OpenSettings, cx| crate::settings_view::open(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(open_recent);
    cx.on_action(|_: &ClearRecent, cx| crate::settings::update(cx, |s| s.recent.clear()));
    set(cx);
}

/// (Re)builds the menus, e.g. after a language change.
pub fn set(cx: &mut App) {
    cx.set_menus([
        Menu::new("Mugetsu").items([
            MenuItem::action(tr("À propos de Mugetsu", "About Mugetsu"), About),
            MenuItem::separator(),
            MenuItem::action(tr("Réglages…", "Settings…"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(tr("Quitter Mugetsu", "Quit Mugetsu"), Quit),
        ]),
        Menu::new(tr("Fichier", "File")).items([
            MenuItem::action(tr("Nouvelle fenêtre", "New Window"), NewWindow),
            MenuItem::separator(),
            MenuItem::action(
                tr("Ouvrir un dépôt…", "Open Repository…"),
                ws::OpenRepository,
            ),
            MenuItem::submenu(
                Menu::new(tr("Ouvrir un dépôt récent", "Open Recent")).items(
                    crate::settings::get()
                        .recent
                        .iter()
                        .enumerate()
                        .map(|(index, p)| MenuItem::action(display_path(p), OpenRecent { index }))
                        .chain([
                            MenuItem::separator(),
                            MenuItem::action(
                                tr("Effacer la liste", "Clear Recently Opened"),
                                ClearRecent,
                            ),
                        ]),
                ),
            ),
            MenuItem::separator(),
            MenuItem::action(tr("Fermer l'onglet", "Close Tab"), ws::CloseTab),
        ]),
        Menu::new(tr("Dépôt", "Repository")).items([
            MenuItem::action("Fetch", repo::Fetch),
            MenuItem::action("Pull", repo::Pull),
            MenuItem::action("Push", repo::Push),
            MenuItem::separator(),
            MenuItem::action(tr("Nouvelle branche…", "New Branch…"), repo::NewBranch),
            MenuItem::action(tr("Nouveau tag…", "New Tag…"), repo::NewTag),
            MenuItem::action(
                tr("Checkout d'une branche…", "Checkout Branch…"),
                repo::CheckoutBranch,
            ),
            MenuItem::action("Stash", repo::Stash),
            MenuItem::separator(),
            MenuItem::action(tr("Rafraîchir", "Refresh"), repo::Refresh),
        ]),
        Menu::new(tr("Présentation", "View")).items([
            MenuItem::action(tr("Changements", "Changes"), repo::ShowChanges),
            MenuItem::action(tr("Historique", "History"), repo::ShowHistory),
            MenuItem::action("Stats", repo::ShowStats),
            MenuItem::action(tr("Aller à HEAD", "Reveal HEAD"), repo::RevealHead),
            MenuItem::separator(),
            MenuItem::action(tr("Précédent", "Back"), repo::GoBack),
            MenuItem::action(tr("Suivant", "Forward"), repo::GoForward),
            MenuItem::separator(),
            MenuItem::action(
                tr("Rechercher des commits", "Search Commits"),
                repo::SearchCommits,
            ),
            MenuItem::separator(),
            MenuItem::action(tr("Barre latérale", "Toggle Sidebar"), repo::ToggleSidebar),
        ]),
    ]);
}

#[cfg(test)]
mod tests {
    #[test]
    fn home_paths_are_shortened() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            super::display_path(&home.join("Workspace/mugetsu")),
            "~/Workspace/mugetsu"
        );
        assert_eq!(
            super::display_path(std::path::Path::new("/opt/x")),
            "/opt/x"
        );
    }
}
