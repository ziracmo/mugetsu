//! macOS menu bar: Mugetsu (Settings…, Quit), File, Repository, View.

use gpui_kit::{AnyWindowHandle, App, Entity, Global, KeyBinding, Menu, MenuItem};

use crate::i18n::tr;
use crate::repo_view as repo;
use crate::workspace as ws;

gpui_kit::actions!(app, [OpenSettings, Quit]);

/// File > Open Recent > n-th repository of `Settings::recent`.
#[derive(Clone, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct OpenRecent {
    pub index: usize,
}

/// The main window's workspace, for menu actions without a focused view.
pub struct MainWorkspace(pub AnyWindowHandle, pub Entity<crate::workspace::Workspace>);
impl Global for MainWorkspace {}

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

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
    cx.on_action(|_: &OpenSettings, cx| crate::settings_view::open(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(open_recent);
    set(cx);
}

/// (Re)builds the menus, e.g. after a language change.
pub fn set(cx: &mut App) {
    cx.set_menus([
        Menu::new("Mugetsu").items([
            MenuItem::action(tr("Réglages…", "Settings…"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(tr("Quitter Mugetsu", "Quit Mugetsu"), Quit),
        ]),
        Menu::new(tr("Fichier", "File")).items([
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
                        .map(|(index, p)| {
                            let name = p
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| p.display().to_string());
                            MenuItem::action(name, OpenRecent { index })
                        }),
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
            MenuItem::action(tr("Aller à HEAD", "Reveal HEAD"), repo::RevealHead),
            MenuItem::action(
                tr("Rechercher des commits", "Search Commits"),
                repo::SearchCommits,
            ),
            MenuItem::separator(),
            MenuItem::action(tr("Barre latérale", "Toggle Sidebar"), repo::ToggleSidebar),
        ]),
    ]);
}
