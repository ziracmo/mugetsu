//! The Settings window (⌘,): sections listed on the left (general,
//! appearance, history, repositories, PRs, notifications, forge accounts,
//! AI), applied live.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Div, Entity, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    WindowBounds, WindowOptions, div, prelude::FluentBuilder as _, px, rgb, size,
};

use crate::i18n::tr;
use crate::settings::{self, ACCENTS, AiTool, CommitSort, Lang, MERGED_DAYS, Notifications};
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
    Notifications,
    Accounts,
    Ai,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::General,
        Section::Appearance,
        Section::History,
        Section::Repositories,
        Section::PullRequests,
        Section::Notifications,
        Section::Accounts,
        Section::Ai,
    ];

    fn key(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::History => "history",
            Section::Repositories => "repositories",
            Section::PullRequests => "prs",
            Section::Notifications => "notifications",
            Section::Accounts => "accounts",
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
            Section::Notifications => "Notifications",
            Section::Accounts => tr("Comptes", "Accounts"),
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
            Section::Notifications => Lucide::Bell,
            Section::Accounts => Lucide::KeyRound,
            Section::Ai => Lucide::Sparkles,
        }
    }
}

pub struct SettingsView {
    section: Section,
    name: Entity<InputState>,
    email: Entity<InputState>,
    ai_command: Entity<InputState>,
    gitlab_token: Entity<InputState>,
    gitlab_hosts: Entity<InputState>,
    bitbucket_token: Entity<InputState>,
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
        let current = settings::get();
        let secret = |value: String,
                      placeholder: &'static str,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .masked(true)
                    .default_value(value)
            })
        };
        let gitlab_token = secret(current.gitlab_token, "glpat-…", window, cx);
        let gitlab_hosts = input(
            current.gitlab_hosts.join(", "),
            "git.example.com",
            window,
            cx,
        );
        let bitbucket_token = secret(
            current.bitbucket_token,
            tr("utilisateur:mot-de-passe-app", "user:app-password"),
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
            cx.subscribe(&gitlab_token, |_, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let token = input.read(cx).value().trim().to_string();
                    settings::remember(|s| s.gitlab_token = token);
                }
            }),
            cx.subscribe(&gitlab_hosts, |_, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let hosts = parse_hosts(&input.read(cx).value());
                    settings::remember(|s| s.gitlab_hosts = hosts);
                }
            }),
            cx.subscribe(&bitbucket_token, |_, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let token = input.read(cx).value().trim().to_string();
                    settings::remember(|s| s.bitbucket_token = token);
                }
            }),
        ];
        Self {
            section: Section::General,
            name,
            email,
            ai_command,
            gitlab_token,
            gitlab_hosts,
            bitbucket_token,
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

/// `"a.io, https://b.io/"` → `["a.io", "b.io"]`.
fn parse_hosts(text: &str) -> Vec<String> {
    text.split([',', ' ', '\n'])
        .map(|h| {
            h.trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/')
                .to_lowercase()
        })
        .filter(|h| !h.is_empty())
        .collect()
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

fn pick_clone_dir(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(tr("Choisir", "Choose").into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = picked.await
            && let Some(dir) = paths.into_iter().next()
        {
            cx.update(|cx| settings::update(cx, |s| s.clone_dir = Some(dir)));
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
                row(
                    tr("Supprimer sans demander", "Delete without asking"),
                    v_flex()
                        .gap_2p5()
                        .child(
                            Switch::new("force-delete-local")
                                .checked(current.force_delete.local)
                                .color(theme::accent())
                                .label(tr("Branches locales", "Local branches"))
                                .on_click(|on: &bool, _, cx| {
                                    let on = *on;
                                    settings::update(cx, |s| s.force_delete.local = on)
                                }),
                        )
                        .child(
                            Switch::new("force-delete-remote")
                                .checked(current.force_delete.remote)
                                .color(theme::accent())
                                .label(tr("Branches distantes", "Remote branches"))
                                .on_click(|on: &bool, _, cx| {
                                    let on = *on;
                                    settings::update(cx, |s| s.force_delete.remote = on)
                                }),
                        ),
                )
                .into_any_element(),
                hint(tr(
                    "Une branche locale est alors forcée (git branch -D), même non mergée.",
                    "A local branch is then forced (git branch -D), even unmerged.",
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
                row(
                    tr("Glisser-déposer", "Drag and drop"),
                    Switch::new("confirm-drag-rebase")
                        .checked(current.confirm_drag_rebase)
                        .color(theme::accent())
                        .label(tr(
                            "Demander avant de rebaser une branche déposée sur une autre",
                            "Ask before rebasing a branch dropped on another",
                        ))
                        .on_click(|on: &bool, _, cx| {
                            let on = *on;
                            settings::update(cx, |s| s.confirm_drag_rebase = on)
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
                row(
                    tr("Dossier workspace", "Workspace folder"),
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .text_sm()
                        .child(
                            Icon::new(Lucide::FolderDown)
                                .xsmall()
                                .text_color(theme::muted()),
                        )
                        .child(match &current.clone_dir {
                            Some(dir) => div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(dir.display().to_string()),
                            None => div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::muted())
                                .child(tr(
                                    "Aucun : demandé à chaque clone",
                                    "None: asked on each clone",
                                )),
                        })
                        .child(
                            Button::new("pick-clone-dir")
                                .outline()
                                .xsmall()
                                .label(tr("Choisir…", "Choose…"))
                                .on_click(|_, _, cx| pick_clone_dir(cx)),
                        )
                        .when(current.clone_dir.is_some(), |s| {
                            s.child(
                                Button::new("clear-clone-dir")
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(Lucide::X))
                                    .on_click(|_, _, cx| {
                                        settings::update(cx, |s| s.clone_dir = None)
                                    }),
                            )
                        }),
                )
                .into_any_element(),
                row(
                    "",
                    Switch::new("clone-always")
                        .checked(current.clone_dir.is_some() && !current.clone_ask)
                        .disabled(current.clone_dir.is_none())
                        .color(theme::accent())
                        .label(tr(
                            "Toujours cloner ici, sans demander",
                            "Always clone here, without asking",
                        ))
                        .on_click(|on: &bool, _, cx| {
                            let ask = !*on;
                            settings::update(cx, |s| s.clone_ask = ask)
                        }),
                )
                .into_any_element(),
                row(
                    tr("Cloner via", "Clone over"),
                    h_flex()
                        .p(px(2.))
                        .gap_0p5()
                        .rounded_lg()
                        .bg(theme::alpha(gpui_kit::white(), 0.05))
                        .border_1()
                        .border_color(theme::hairline())
                        .children(
                            [
                                (None, tr("Auto", "Auto")),
                                (Some(false), "SSH"),
                                (Some(true), "HTTPS"),
                            ]
                            .into_iter()
                            .map(|(value, label)| {
                                let id = format!("clone-proto-{label}").to_lowercase();
                                choice(id, label, current.clone_https == value)
                                    .on_click(move |_, _, cx| {
                                        settings::update(cx, |s| s.clone_https = value)
                                    })
                                    .test_support()
                            }),
                        ),
                )
                .into_any_element(),
                hint(tr(
                    "Les dépôts distants de vos comptes et les URL collées dans ⌘O sont clonés dans le dossier workspace. Auto : SSH si une clé existe dans ~/.ssh, sinon HTTPS.",
                    "Remote repositories from your accounts and URLs pasted in ⌘O are cloned into the workspace folder. Auto: SSH when a key exists in ~/.ssh, else HTTPS.",
                )),
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
            Section::Notifications => {
                let prefs = current.notifications;
                let switch = |id: &'static str,
                              label: &'static str,
                              checked: bool,
                              set: fn(&mut Notifications, bool)| {
                    Switch::new(id)
                        .checked(checked)
                        .color(theme::accent())
                        .label(label)
                        .on_click(move |on: &bool, _, cx| {
                            let on = *on;
                            settings::update(cx, |s| set(&mut s.notifications, on))
                        })
                };
                vec![
                    row(
                        tr("Me prévenir quand", "Notify me when"),
                        v_flex()
                            .gap_2p5()
                            .child(switch(
                                "notify-ci",
                                tr("La CI d'une PR est terminée", "A PR's CI finishes"),
                                prefs.ci,
                                |n, on| n.ci = on,
                            ))
                            .child(switch(
                                "notify-review",
                                tr("Ma review est demandée", "My review is requested"),
                                prefs.review,
                                |n, on| n.review = on,
                            ))
                            .child(switch(
                                "notify-merged",
                                tr("Une PR est mergée", "A PR is merged"),
                                prefs.merged,
                                |n, on| n.merged = on,
                            )),
                    )
                    .into_any_element(),
                    row(
                        tr("Recap hebdo", "Weekly recap"),
                        Switch::new("recap-reminder")
                            .checked(current.recap_reminder)
                            .color(theme::accent())
                            .label(tr(
                                "Le proposer le vendredi à partir de 16 h",
                                "Offer it on Fridays from 4 pm",
                            ))
                            .on_click(|on: &bool, _, cx| {
                                let on = *on;
                                settings::update(cx, |s| s.recap_reminder = on)
                            }),
                    )
                    .into_any_element(),
                    row(
                        tr("Portée", "Scope"),
                        switch(
                            "notify-others",
                            tr(
                                "CI et merges des PRs des autres aussi",
                                "CI and merges of other people's PRs too",
                            ),
                            prefs.others,
                            |n, on| n.others = on,
                        ),
                    )
                    .into_any_element(),
                    row(
                        "",
                        Button::new("notify-test")
                            .outline()
                            .xsmall()
                            .icon(Icon::new(Lucide::BellRing))
                            .label(tr("Envoyer un test", "Send a test"))
                            .on_click(|_, _, _| {
                                crate::notify::show(&crate::notify::Note {
                                    title: "Mugetsu".into(),
                                    body: tr(
                                        "Les notifications fonctionnent.",
                                        "Notifications work.",
                                    )
                                    .into(),
                                    url: String::new(),
                                })
                            }),
                    )
                    .into_any_element(),
                    hint(tr(
                        "Pour les dépôts ouverts en onglet, avec un token (voir Comptes). Les onglets en arrière-plan vérifient leurs PRs toutes les 3 min.",
                        "For repositories open in a tab, with a token (see Accounts). Background tabs check their PRs every 3 min.",
                    )),
                ]
            }
            Section::Accounts => {
                let field = |input: &Entity<InputState>| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(input).small().mask_toggle())
                };
                vec![
                    row(
                        "GitHub",
                        div().text_sm().text_color(theme::muted()).child(tr(
                            "Connexion de la CLI gh, ou GH_TOKEN.",
                            "The gh CLI login, or GH_TOKEN.",
                        )),
                    )
                    .into_any_element(),
                    row(
                        tr("Token GitLab", "GitLab token"),
                        field(&self.gitlab_token),
                    )
                    .into_any_element(),
                    row(
                        tr("Serveurs GitLab", "GitLab servers"),
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.gitlab_hosts).small()),
                    )
                    .into_any_element(),
                    hint(tr(
                        "Token personnel (scope read_api). Vide : GITLAB_TOKEN ou la connexion glab. Serveurs : vos instances GitLab auto-hébergées, séparées par des virgules (gitlab.com et gitlab.* sont reconnus d'office).",
                        "Personal access token (read_api scope). Empty: GITLAB_TOKEN or the glab login. Servers: your self-hosted GitLab instances, comma-separated (gitlab.com and gitlab.* are recognised already).",
                    )),
                    row(
                        tr("Token Bitbucket", "Bitbucket token"),
                        field(&self.bitbucket_token),
                    )
                    .into_any_element(),
                    hint(tr(
                        "« utilisateur:mot-de-passe-app » (ou « e-mail:token API »), ou un access token de dépôt / workspace. Vide : BITBUCKET_TOKEN. Les dépôts publics marchent sans token.",
                        "“user:app-password” (or “email:API token”), or a repository / workspace access token. Empty: BITBUCKET_TOKEN. Public repositories work without one.",
                    )),
                ]
            }
            Section::Ai => {
                let tools =
                    AiTool::ALL.iter().map(|&tool| {
                        let id = format!("ai-tool-{tool:?}").to_lowercase();
                        choice(id, tool.label(), current.ai_tool == tool)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pick_tool(tool, window, cx)
                            }))
                            .test_support()
                    });
                let terminals = crate::ai_review::terminals().iter().map(|&terminal| {
                    let id = format!("ai-terminal-{terminal:?}").to_lowercase();
                    choice(id, terminal.label(), current.ai_terminal == terminal)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            settings::remember(|s| s.ai_terminal = terminal);
                            cx.notify();
                        }))
                        .test_support()
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
                    row(
                        tr("Lancer dans", "Open in"),
                        h_flex()
                            .flex_wrap()
                            .p(px(2.))
                            .gap_0p5()
                            .rounded_lg()
                            .bg(theme::alpha(gpui_kit::white(), 0.05))
                            .border_1()
                            .border_color(theme::hairline())
                            .children(terminals),
                    )
                    .into_any_element(),
                    hint(tr(
                        "Lancée dans le terminal choisi, dans le dossier du dépôt, depuis « Review IA » sur une PR. Variables : {url} {number} {branch} {base} {title}. Claude Desktop (s'il est installé) ouvre une nouvelle session Claude Code, préremplie avec la demande, après confirmation du dossier.",
                        "Run in the chosen terminal, in the repository folder, from a PR's “AI review”. Variables: {url} {number} {branch} {base} {title}. Claude Desktop (when installed) opens a new Claude Code session, prefilled with the request, once you confirm the folder.",
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

/// One option of a segmented picker.
fn choice(id: String, label: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
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

#[cfg(test)]
mod tests {
    #[test]
    fn parses_gitlab_hosts() {
        assert_eq!(
            super::parse_hosts("git.acme.io, https://GitLab.Corp:8443/ \n"),
            ["git.acme.io", "gitlab.corp:8443"]
        );
        assert!(super::parse_hosts(" , ").is_empty());
    }
}
