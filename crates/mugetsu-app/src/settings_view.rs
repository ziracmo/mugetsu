//! The Settings window (⌘,): language, accent colour, commit order and
//! repository folders, applied live.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, WindowBounds, WindowOptions, div, prelude::FluentBuilder as _, px, rgb,
    size,
};

use crate::i18n::tr;
use crate::settings::{self, ACCENTS, CommitSort, Lang, MERGED_DAYS};
use crate::theme;

struct OpenSettingsWindow(AnyWindowHandle);
impl Global for OpenSettingsWindow {}

/// Opens the window, or brings the existing one to the front.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<OpenSettingsWindow>().map(|g| g.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(600.), px(600.)), cx)),
        is_resizable: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Blurred,
        ..TitleBar::window_options()
    };
    if let Ok(handle) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(tr("Réglages", "Settings"));
        cx.new(|_| SettingsView)
    }) {
        cx.set_global(OpenSettingsWindow(handle.0));
    }
}

pub struct SettingsView;

fn add_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: Some(tr("Ajouter", "Add").into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = picked.await {
            cx.update(|cx| {
                settings::update(cx, |s| {
                    for p in paths {
                        if !s.repo_folders.contains(&p) {
                            s.repo_folders.push(p);
                        }
                    }
                })
            });
        }
    })
    .detach();
}

fn row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_4()
        .items_center()
        .child(
            div()
                .w(px(150.))
                .text_sm()
                .text_right()
                .text_color(theme::muted())
                .child(label),
        )
        .child(control)
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = settings::get();
        let lang_button = |id: &'static str, label: &'static str, lang: Lang| {
            let active = current.lang == lang;
            div()
                .id(id)
                .test_support()
                .px_3()
                .h(px(24.))
                .flex()
                .items_center()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .when(active, |s| {
                    s.bg(theme::alpha(gpui_kit::white(), 0.14))
                        .border_1()
                        .border_color(theme::glass_edge())
                        .shadow_sm()
                        .text_color(gpui_kit::white())
                })
                .when(!active, |s| s.text_color(theme::muted()))
                .child(label)
                .on_click(move |_, _, cx| settings::update(cx, |s| s.lang = lang))
        };
        let swatches = ACCENTS.iter().map(|&(color, fr, en)| {
            let active = current.accent == color;
            div()
                .id(SharedString::from(format!("accent-{color:06x}")))
                .test_support()
                .size(px(26.))
                .rounded_full()
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(color))
                .border_2()
                .border_color(if active {
                    gpui_kit::white()
                } else {
                    theme::alpha(gpui_kit::white(), 0.12)
                })
                .shadow_sm()
                .when(active, |s| {
                    s.child(
                        Icon::new(Lucide::Check)
                            .xsmall()
                            .text_color(gpui_kit::white()),
                    )
                })
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tr(fr, en)).build(window, cx)
                })
                .on_click(move |_, _, cx| settings::update(cx, |s| s.accent = color))
        });
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().text_sm().child(tr("Réglages", "Settings"))))
            .child(
                v_flex()
                    .flex_1()
                    .p_6()
                    .gap_5()
                    .child(row(
                        tr("Langue", "Language"),
                        h_flex()
                            .p(px(2.))
                            .gap_0p5()
                            .rounded_lg()
                            .bg(theme::alpha(gpui_kit::white(), 0.05))
                            .border_1()
                            .border_color(theme::hairline())
                            .child(lang_button("lang-fr", "Français", Lang::Fr))
                            .child(lang_button("lang-en", "English", Lang::En)),
                    ))
                    .child(row(
                        tr("Couleur d'accent", "Accent colour"),
                        h_flex().gap_2().children(swatches),
                    ))
                    .child(row(
                        tr("Tri des commits", "Sort commits"),
                        RadioGroup::horizontal("commit-order")
                            .child(
                                Radio::new("order-topo").label(tr("Topologique", "Topologically")),
                            )
                            .child(Radio::new("order-date").label(tr("Par date", "By date")))
                            .selected_index(Some(match current.commit_order {
                                CommitSort::Topological => 0,
                                CommitSort::Date => 1,
                            }))
                            .on_click(|ix: &usize, _, cx| {
                                let order = if *ix == 0 {
                                    CommitSort::Topological
                                } else {
                                    CommitSort::Date
                                };
                                settings::update(cx, |s| s.commit_order = order)
                            }),
                    ))
                    .child(row(
                        tr("PRs mergées récentes", "Recently merged PRs"),
                        MERGED_DAYS
                            .iter()
                            .fold(RadioGroup::horizontal("merged-days"), |g, d| {
                                g.child(
                                    Radio::new(SharedString::from(format!("merged-{d}")))
                                        .label(trf!("{d} jours", "{d} days")),
                                )
                            })
                            .selected_index(
                                MERGED_DAYS.iter().position(|d| *d == current.merged_days),
                            )
                            .on_click(|ix: &usize, _, cx| {
                                let days = MERGED_DAYS[*ix];
                                settings::update(cx, |s| s.merged_days = days)
                            }),
                    ))
                    .child(row(
                        tr("Dossiers de dépôts", "Repository folders"),
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .children(current.repo_folders.iter().enumerate().map(|(i, f)| {
                                let shown = f.display().to_string();
                                h_flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(
                                        Icon::new(Lucide::Folder)
                                            .xsmall()
                                            .text_color(theme::muted()),
                                    )
                                    .child(div().flex_1().min_w_0().truncate().child(shown))
                                    .child(
                                        Button::new(("remove-folder", i))
                                            .ghost()
                                            .xsmall()
                                            .icon(Icon::new(Lucide::X))
                                            .on_click(move |_, _, cx| {
                                                settings::update(cx, |s| {
                                                    if i < s.repo_folders.len() {
                                                        s.repo_folders.remove(i);
                                                    }
                                                })
                                            }),
                                    )
                            }))
                            .child(
                                h_flex().child(
                                    Button::new("add-folder")
                                        .outline()
                                        .xsmall()
                                        .icon(Icon::new(Lucide::Plus))
                                        .label(tr("Ajouter un dossier…", "Add folder…"))
                                        .on_click(|_, _, cx| add_folder(cx)),
                                ),
                            )
                            .child(div().text_xs().text_color(theme::muted()).child(tr(
                                "Les dépôts de ces dossiers sont proposés à l'ouverture (⌘O).",
                                "Repositories in these folders are offered when opening (⌘O).",
                            ))),
                    ))
                    .child(row(
                        tr("Aperçu", "Preview"),
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .px_3()
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .text_sm()
                                    .bg(theme::accent())
                                    .text_color(gpui_kit::white())
                                    .child(tr("Bouton", "Button")),
                            )
                            .child(
                                div()
                                    .px_1p5()
                                    .h(px(18.))
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .text_xs()
                                    .border_1()
                                    .border_color(theme::alpha(theme::accent(), 0.6))
                                    .bg(theme::alpha(theme::accent(), 0.15))
                                    .text_color(theme::accent())
                                    .child("main"),
                            ),
                    ))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(theme::muted()).child(tr(
                        "Les changements s'appliquent immédiatement.",
                        "Changes apply immediately.",
                    ))),
            )
    }
}
