//! Repository icons: an emoji or a Lucide icon from the bundled set (saved
//! as `lucide:<name>`), and the searchable picker opened from a tab.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::i18n::tr;
use crate::theme;

const PREFIX: &str = "lucide:";

/// Emoji with search words (French and English).
const EMOJI: &[(&str, &str)] = &[
    ("🔴", "rouge red circle"),
    ("🟠", "orange circle"),
    ("🟡", "jaune yellow circle"),
    ("🟢", "vert green circle"),
    ("🔵", "bleu blue circle"),
    ("🟣", "violet purple circle"),
    ("⚫", "noir black circle"),
    ("⚪", "blanc white circle"),
    ("🚀", "fusee rocket launch"),
    ("🔥", "feu fire hot"),
    ("⭐", "etoile star"),
    ("🌙", "lune moon night"),
    ("⚡", "eclair lightning fast"),
    ("🧪", "test labo lab experiment"),
    ("🛠️", "outils tools build"),
    ("📦", "paquet package box"),
    ("🎨", "design art palette"),
    ("🧠", "cerveau brain ai"),
    ("🛡️", "bouclier shield security"),
    ("🌐", "web globe internet"),
    ("📱", "mobile phone app"),
    ("🐳", "docker baleine whale"),
    ("🦀", "rust crabe crab"),
    ("🐍", "python serpent snake"),
    ("☕", "java cafe coffee"),
    ("🐹", "go gopher hamster"),
    ("💎", "ruby gem diamant"),
    ("🐘", "php elephant postgres"),
    ("⚛️", "react atome atom"),
    ("🟦", "typescript carre bleu blue square"),
    ("🟨", "javascript carre jaune yellow square"),
    ("📚", "docs livres books documentation"),
    ("📝", "notes memo write"),
    ("🧩", "plugin puzzle extension"),
    ("⚙️", "config settings engrenage gear"),
    ("🗄️", "base de donnees database storage"),
    ("☁️", "cloud nuage infra"),
    ("🤖", "robot bot ai"),
    ("🎮", "jeu game"),
    ("🏠", "maison home perso personal"),
    ("💼", "travail work job"),
    ("🔒", "cadenas lock security private"),
    ("🔑", "cle key auth"),
    ("📊", "graphique chart stats analytics"),
    ("🧭", "boussole compass"),
    ("🌱", "pousse plant new"),
    ("🍀", "trefle clover luck"),
    ("🐛", "bug insecte"),
    ("💡", "idee idea light"),
    ("🎯", "cible target goal"),
    ("📡", "antenne satellite network"),
    ("🧱", "brique brick infra"),
    ("🗺️", "carte map"),
    ("🏁", "drapeau flag finish f1"),
    ("🎵", "musique music"),
    ("📷", "photo camera"),
    ("💬", "chat message"),
    ("🔔", "cloche bell notification"),
];

/// Lucide icons shown before any search: handy for code projects.
const FEATURED: &[&str] = &[
    "code",
    "terminal",
    "git-branch",
    "github",
    "gitlab",
    "database",
    "server",
    "cloud",
    "globe",
    "shield",
    "lock",
    "key",
    "rocket",
    "flame",
    "zap",
    "star",
    "heart",
    "bug",
    "flask-conical",
    "package",
    "box",
    "boxes",
    "layers",
    "layout-dashboard",
    "monitor",
    "smartphone",
    "cpu",
    "brain",
    "bot",
    "book-open",
    "file-text",
    "notebook-pen",
    "palette",
    "brush",
    "camera",
    "music",
    "gamepad-2",
    "map",
    "compass",
    "flag",
    "house",
    "briefcase",
    "building-2",
    "users",
    "user",
    "mail",
    "bell",
    "message-square",
    "chart-bar",
    "chart-line",
    "activity",
    "wrench",
    "hammer",
    "settings",
    "puzzle",
    "plug",
    "wifi",
    "radio",
    "satellite",
    "leaf",
    "sun",
    "moon",
];

/// Lucide icon name (file stem, e.g. `git-branch`) → icon.
fn lucide_names() -> &'static HashMap<String, Lucide> {
    static NAMES: OnceLock<HashMap<String, Lucide>> = OnceLock::new();
    NAMES.get_or_init(|| {
        Lucide::ALL
            .iter()
            .filter_map(|icon| {
                let path = icon.path();
                let stem = path.strip_prefix("icons/")?.strip_suffix(".svg")?;
                Some((stem.to_string(), *icon))
            })
            .collect()
    })
}

fn lucide(value: &str) -> Option<Lucide> {
    lucide_names().get(value.strip_prefix(PREFIX)?).copied()
}

/// A repository's icon, `size` pixels tall.
pub fn view(value: &str, size: f32) -> AnyElement {
    match lucide(value) {
        Some(icon) => Icon::new(icon)
            .size(px(size))
            .text_color(theme::accent())
            .into_any_element(),
        None => div()
            .flex_none()
            .text_size(px(size * 0.9))
            .line_height(px(size))
            .child(value.to_string())
            .into_any_element(),
    }
}

/// The icon set for `path` in the settings, if any.
pub fn of(path: &Path) -> Option<String> {
    crate::settings::get().repo_icons.get(path).cloned()
}

fn words_match(hay: &str, query: &str) -> bool {
    let hay = hay.to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// Lucide icon values matching `query` (featured ones when empty).
pub fn search_icons(query: &str, limit: usize) -> Vec<String> {
    if query.trim().is_empty() {
        return FEATURED
            .iter()
            .filter(|n| lucide_names().contains_key(**n))
            .map(|n| format!("{PREFIX}{n}"))
            .collect();
    }
    let mut names: Vec<&String> = lucide_names()
        .keys()
        .filter(|n| words_match(n, query))
        .collect();
    // Exact and shorter names first.
    names.sort_by_key(|n| (n.as_str() != query.trim(), n.len(), n.as_str()));
    names
        .into_iter()
        .take(limit)
        .map(|n| format!("{PREFIX}{n}"))
        .collect()
}

pub fn search_emoji(query: &str) -> Vec<&'static str> {
    EMOJI
        .iter()
        .filter(|(_, words)| query.trim().is_empty() || words_match(words, query))
        .map(|(e, _)| *e)
        .collect()
}

fn set(root: &Path, icon: Option<String>, cx: &mut App) {
    let root = root.to_path_buf();
    crate::settings::update(cx, move |s| match icon {
        Some(i) => {
            s.repo_icons.insert(root, i);
        }
        None => {
            s.repo_icons.remove(&root);
        }
    });
}

pub struct IconPicker {
    root: PathBuf,
    input: Entity<InputState>,
    _input: Subscription,
}

impl IconPicker {
    fn cell(&self, value: String, id: SharedString, selected: bool) -> impl IntoElement {
        let root = self.root.clone();
        let picked = value.clone();
        div()
            .id(id)
            .test_support()
            .size(px(38.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .cursor_pointer()
            .hover(|s| s.bg(theme::raised()))
            .when(selected, |s| {
                s.bg(theme::alpha(theme::accent(), 0.3))
                    .border_1()
                    .border_color(theme::accent())
            })
            .child(view(&value, 20.))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(
                    value.strip_prefix(PREFIX).unwrap_or(&value).to_string(),
                )
                .build(window, cx)
            })
            .on_click(move |_, window, cx| {
                set(&root, Some(picked.clone()), cx);
                window.close_dialog(cx);
            })
    }
}

impl Render for IconPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.input.read(cx).value().to_string();
        let current = of(&self.root);
        let title = |text: &'static str| {
            div()
                .pt_2()
                .pb_1()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child(text)
        };
        let icons: Vec<_> = search_icons(&query, 96)
            .into_iter()
            .map(|v| {
                let id = SharedString::from(format!("icon-{v}"));
                let selected = current.as_deref() == Some(v.as_str());
                self.cell(v, id, selected).into_any_element()
            })
            .collect();
        let emoji: Vec<_> = search_emoji(&query)
            .into_iter()
            .map(|e| {
                let id = SharedString::from(format!("emoji-{e}"));
                let selected = current.as_deref() == Some(e);
                self.cell(e.to_string(), id, selected).into_any_element()
            })
            .collect();
        let empty = icons.is_empty() && emoji.is_empty();
        v_flex()
            .gap_1()
            .child(
                Input::new(&self.input).id("icon-search").prefix(
                    Icon::new(Lucide::Search)
                        .xsmall()
                        .text_color(theme::muted()),
                ),
            )
            .child(
                v_flex()
                    .id("icon-grid")
                    .h(px(340.))
                    .overflow_y_scroll()
                    .when(!icons.is_empty(), |s| {
                        s.child(title(tr("Icônes", "Icons")))
                            .child(h_flex().flex_wrap().gap_1().children(icons))
                    })
                    .when(!emoji.is_empty(), |s| {
                        s.child(title("Emoji"))
                            .child(h_flex().flex_wrap().gap_1().children(emoji))
                    })
                    .when(empty, |s| {
                        s.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(theme::muted())
                                .child(tr("Aucune icône trouvée", "No icon found")),
                        )
                    }),
            )
    }
}

/// Dialog to pick a repository's icon, searchable by name.
pub fn open_picker(root: PathBuf, window: &mut Window, cx: &mut App) {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr(
            "Rechercher (rocket, base, rust…)",
            "Search (rocket, database, rust…)",
        ))
    });
    let picker = cx.new(|cx: &mut Context<IconPicker>| {
        let sub = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        IconPicker {
            root: root.clone(),
            input: input.clone(),
            _input: sub,
        }
    });
    let focus_once = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus_once.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let none_root = root.clone();
        dialog
            .bg(theme::sheet())
            .title(trf!("Icône de « {name} »", "Icon for “{name}”"))
            .w(px(480.))
            .child(picker.clone())
            .footer(
                h_flex()
                    .w_full()
                    .child(
                        Button::new("emoji-none")
                            .ghost()
                            .small()
                            .label(tr("Aucune icône", "No icon"))
                            .on_click(move |_, window, cx| {
                                set(&none_root, None, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        gpui_kit::component::dialog::DialogClose::new()
                            .trigger(|b| b.ghost().small().label(tr("Fermer", "Close"))),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_icons_and_emoji_by_name() {
        let found = search_icons("git-branch", 10);
        assert_eq!(found.first().map(String::as_str), Some("lucide:git-branch"));
        assert!(lucide("lucide:git-branch").is_some());
        assert!(lucide("🦀").is_none());
        assert_eq!(search_emoji("rust"), ["🦀"]);
        assert_eq!(
            search_icons("", 10).len(),
            FEATURED.len().min(search_icons("", 10).len())
        );
        assert!(!search_icons("", 10).is_empty());
    }
}
