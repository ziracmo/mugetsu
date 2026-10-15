//! The Settings window (⌘,): sections listed on the left (general,
//! appearance, history, repositories, PRs, AI), applied live.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::FluentBuilder as _, px, rgb, size,
};

use crate::i18n::tr;
use crate::settings::{self, ACCENTS, AiTool, CommitSort, Lang, MERGED_DAYS};
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
        window_bounds: Some(WindowBounds::centered(size(px(780.), px(560.)), cx)),
        is_resizable: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Blurred,
        ..TitleBar::window_options()
    };
    if let Ok(handle) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(tr("Réglages", "Settings"));
        cx.new(|cx| SettingsView::new(window, cx))
    }) {
        cx.set_global(OpenSettingsWindow(handle.0));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    History,
    Repositories,
    PullRequests,
    Ai,
}

impl Section {
    const ALL: [Section; 6] = [
        Section::General,
        Section::Appearance,
        Section::History,
        Section::Repositories,
        Section::PullRequests,
        Section::Ai,
    ];

    fn key(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::History => "history",
            Section::Repositories => "repositories",
            Section::PullRequests => "prs",
            Section::Ai => "ai",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Section::General => tr("Général", "General"),
            Section::Appearance => tr("Apparence", "Appearance"),
            Section::History => tr("Historique", "History"),
            Section::Repositories => tr("Dépôts", "Repositories"),
            Section::PullRequests => "Pull requests",
            Section::Ai => tr("IA", "AI"),
        }
    }

    fn icon(self) -> Lucide {
        match self {
            Section::General => Lucide::Settings,
            Section::Appearance => Lucide::Palette,
            Section::History => Lucide::GitCommitVertical,
            Section::Repositories => Lucide::FolderGit2,
            Section::PullRequests => Lucide::GitPullRequest,
            Section::Ai => Lucide::Sparkles,
        }
    }
}

pub struct SettingsView {
    section: Section,
    name: Entity<InputState>,
    email: Entity<InputState>,
    ai_command: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = mugetsu_git::global_identity();
        let input = |value: String,
                     placeholder: &'static str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let name = input(id.name, tr("Prénom Nom", "Full name"), window, cx);
        let email = input(id.email, "you@example.com", window, cx);
        let ai_command = input(
            settings::get().ai_command,
            "claude \"/code-review {url}\"",
            window,
            cx,
        );
        let save_identity =
            |this: &mut Self, _: Entity<InputState>, e: &InputEvent, cx: &mut Context<Self>| {
                if matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.save_identity(cx);
                }
            };
        let _subscriptions = vec![
            cx.subscribe(&name, save_identity),
            cx.subscribe(&email, save_identity),
            cx.subscribe(&ai_command, |_, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let command = input.read(cx).value().to_string();
                    settings::remember(|s| {
                        s.ai_command = command;
                        // A hand-edited preset becomes a custom command.
                        if s.ai_tool.preset() != Some(s.ai_command.as_str()) {
                            s.ai_tool = AiTool::Custom;
                        }
                    });
                    cx.notify();
                }
            }),
        ];
        Self {
            section: Section::General,
            name,
            email,
            ai_command,
            _subscriptions,
        }
    }

    /// Writes the commit identity to `~/.gitconfig` when it changed.
    fn save_identity(&mut self, cx: &mut Context<Self>) {
        let id = mugetsu_git::Identity {
            name: self.name.read(cx).value().trim().to_string(),
            email: self.email.read(cx).value().trim().to_string(),
        };
        if id != mugetsu_git::global_identity() {
            let _ = mugetsu_git::set_global_identity(&id);
        }
    }

    fn pick_tool(&mut self, tool: AiTool, window: &mut Window, cx: &mut Context<Self>) {
        let preset = tool.preset();
        settings::remember(|s| {
            s.ai_tool = tool;
            if let Some(p) = preset {
                s.ai_command = p.to_string();
            }
        });
        if let Some(p) = preset {
            self.ai_command
                .update(cx, |i, cx| i.set_value(p.to_string(), window, cx));
            // set_value emits Change: the tool stays the one picked.
            settings::remember(|s| s.ai_tool = tool);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn section(&self) -> Section {
        self.section
    }
}

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
        .min_w_0()
        .gap_4()
        .items_center()
        .child(
            div()
                .w(px(140.))
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
        let rows: Vec<AnyElement> = match self.section {
            Section::General => vec![
                row(
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
                )
                .into_any_element(),
                row(
                    tr("Nom de commiteur", "Committer name"),
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.name).small()),
                )
                .into_any_element(),
                row(
                    tr("E-mail", "Email"),
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.email).small()),
                )
                .into_any_element(),
                hint(tr(
                    "Enregistrés dans ~/.gitconfig (user.name, user.email), pour tous les dépôts.",
                    "Saved to ~/.gitconfig (user.name, user.email), for every repository.",
                )),
            ],
            Section::Appearance => vec![
                row(
                    tr("Couleur d'accent", "Accent colour"),
                    h_flex().gap_2().children(swatches),
                )
                .into_any_element(),
                row(
                    tr("Disposition", "Layout"),
                    RadioGroup::horizontal("shared-layout")
                        .child(Radio::new("layout-shared").label(tr(
                            "Même sidebar pour tous les dépôts",
                            "Same sidebar for every repository",
                        )))
                        .child(Radio::new("layout-own").label(tr("Par dépôt", "Per repository")))
                        .selected_index(Some(if current.shared_layout { 0 } else { 1 }))
                        .on_click(|ix: &usize, _, cx| {
                            let shared = *ix == 0;
                            settings::update(cx, |s| s.shared_layout = shared)
                        }),
                )
                .into_any_element(),
                row(
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
                )
                .into_any_element(),
            ],
            Section::History => vec![
                row(
                    tr("Tri des commits", "Sort commits"),
                    RadioGroup::horizontal("commit-order")
                        .child(Radio::new("order-topo").label(tr("Topologique", "Topologically")))
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
                )
                .into_any_element(),
                hint(tr(
                    "F dans l'historique ramène sur la branche courante.",
                    "F in the history jumps back to the current branch.",
                )),
            ],
            Section::Repositories => vec![
                row(
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
                )
                .into_any_element(),
            ],
            Section::PullRequests => vec![
                row(
                    tr("PRs mergées récentes", "Recently merged PRs"),
                    MERGED_DAYS
                        .iter()
                        .fold(RadioGroup::horizontal("merged-days"), |g, d| {
                            g.child(
                                Radio::new(SharedString::from(format!("merged-{d}")))
                                    .label(trf!("{d} jours", "{d} days")),
                            )
                        })
                        .selected_index(MERGED_DAYS.iter().position(|d| *d == current.merged_days))
                        .on_click(|ix: &usize, _, cx| {
                            let days = MERGED_DAYS[*ix];
                            settings::update(cx, |s| s.merged_days = days)
                        }),
                )
                .into_any_element(),
            ],
            Section::Ai => {
                let tools =
                    AiTool::ALL.iter().map(|&tool| {
                        let active = current.ai_tool == tool;
                        div()
                            .id(SharedString::from(
                                format!("ai-tool-{tool:?}").to_lowercase(),
                            ))
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
                            .child(tool.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick_tool(tool, window, cx)
                            }))
                    });
                vec![
                    row(
                        tr("Outil", "Tool"),
                        h_flex()
                            .p(px(2.))
                            .gap_0p5()
                            .rounded_lg()
                            .bg(theme::alpha(gpui_kit::white(), 0.05))
                            .border_1()
                            .border_color(theme::hairline())
                            .children(tools),
                    )
                    .into_any_element(),
                    row(
                        tr("Commande", "Command"),
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.ai_command).small()),
                    )
                    .into_any_element(),
                    hint(tr(
                        "Lancée dans un Terminal, dans le dossier du dépôt, depuis « Review IA » sur une PR. Variables : {url} {number} {branch} {base} {title}.",
                        "Run in a Terminal, in the repository folder, from a PR's “AI review”. Variables: {url} {number} {branch} {base} {title}.",
                    )),
                ]
            }
        };
        let nav = Section::ALL.iter().map(|&section| {
            let active = self.section == section;
            h_flex()
                .id(SharedString::from(format!("section-{}", section.key())))
                .test_support()
                .h(px(30.))
                .px_2p5()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .when(active, |s| {
                    s.bg(theme::alpha(theme::accent(), 0.85))
                        .text_color(gpui_kit::white())
                        .shadow_sm()
                })
                .when(!active, |s| {
                    s.text_color(theme::muted())
                        .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.06)))
                })
                .child(Icon::new(section.icon()).small())
                .child(section.title())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = section;
                    cx.notify();
                }))
        });
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().text_sm().child(tr("Réglages", "Settings"))))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .w(px(190.))
                            .h_full()
                            .flex_none()
                            .p_3()
                            .gap_0p5()
                            .border_r_1()
                            .border_color(theme::hairline())
                            .bg(theme::alpha(gpui_kit::black(), 0.12))
                            .children(nav),
                    )
                    .child(
                        v_flex()
                            .id("settings-content")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .p_6()
                            .gap_5()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(self.section.title()),
                            )
                            .children(rows)
                            .child(div().flex_1())
                            .child(hint(tr(
                                "Les changements s'appliquent immédiatement.",
                                "Changes apply immediately.",
                            ))),
                    ),
            )
    }
}

fn hint(text: &'static str) -> AnyElement {
    div()
        .w_full()
        .whitespace_normal()
        .text_xs()
        .text_color(theme::muted())
        .child(text)
        .into_any_element()
}
