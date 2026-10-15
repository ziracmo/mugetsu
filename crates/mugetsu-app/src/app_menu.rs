//! macOS menu bar: Mugetsu (Settings…, Quit), File, Repository, View.

use gpui_kit::{App, KeyBinding, Menu, MenuItem};

use crate::i18n::tr;
use crate::repo_view as repo;
use crate::workspace as ws;

gpui_kit::actions!(app, [OpenSettings, Quit]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
    cx.on_action(|_: &OpenSettings, cx| crate::settings_view::open(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
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
