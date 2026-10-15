use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use mugetsu_forge::{CheckState, PrState, PullRequest, ReviewDecision};

use super::{Mode, RepoView};
use crate::i18n::tr;
use crate::pr::PrLookup;
use crate::theme;

pub fn pr_color(state: PrState) -> gpui_kit::Hsla {
    match state {
        PrState::Open => theme::lime(),
        PrState::Draft => theme::muted(),
        PrState::Merged => theme::violet(),
        PrState::Closed => theme::red(),
    }
}

fn check_look(checks: CheckState) -> (Lucide, gpui_kit::Hsla) {
    match checks {
        CheckState::Passing => (Lucide::CircleCheck, theme::lime()),
        CheckState::Failing => (Lucide::CircleX, theme::red()),
        CheckState::Pending => (Lucide::CircleDashed, theme::amber()),
    }
}

/// Hover card of a PR: title, state, CI, reviews, unresolved conversations.
fn pr_card(pr: &PullRequest) -> AnyElement {
    let line = |icon: Lucide, color: gpui_kit::Hsla, text: String| {
        h_flex()
            .gap_1p5()
            .child(Icon::new(icon).xsmall().text_color(color))
            .child(text)
    };
    let state = match pr.state {
        PrState::Open => tr("Ouverte", "Open"),
        PrState::Draft => tr("Brouillon", "Draft"),
        PrState::Merged => tr("Mergée", "Merged"),
        PrState::Closed => tr("Fermée", "Closed"),
    };
    let mut card = v_flex()
        .gap_1()
        .max_w(px(360.))
        .text_xs()
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(format!("#{} {}", pr.number, pr.title)),
        )
        .child(line(
            Lucide::GitPullRequest,
            pr_color(pr.state),
            state.into(),
        ));
    if let Some(s) = &pr.status {
        card = card.child(match s.checks {
            Some(CheckState::Failing) => {
                let (icon, color) = check_look(CheckState::Failing);
                line(
                    icon,
                    color,
                    trf!("CI en échec : {}", "CI failing: {}", s.failing.join(", ")),
                )
            }
            Some(CheckState::Pending) => {
                let (icon, color) = check_look(CheckState::Pending);
                line(
                    icon,
                    color,
                    trf!(
                        "CI en cours ({} en attente, {} ok)",
                        "CI running ({} pending, {} passed)",
                        s.pending,
                        s.passed
                    ),
                )
            }
            Some(CheckState::Passing) => {
                let (icon, color) = check_look(CheckState::Passing);
                line(
                    icon,
                    color,
                    trf!("CI OK ({} checks)", "CI passing ({} checks)", s.passed),
                )
            }
            None => line(
                Lucide::CircleDashed,
                theme::muted(),
                tr("Pas de CI", "No checks").into(),
            ),
        });
        if let Some(review) = s.review {
            let (icon, color, text) = match review {
                ReviewDecision::Approved => (
                    Lucide::UserCheck,
                    theme::lime(),
                    tr("Approuvée", "Approved"),
                ),
                ReviewDecision::ChangesRequested => (
                    Lucide::CircleAlert,
                    theme::red(),
                    tr("Changements demandés", "Changes requested"),
                ),
                ReviewDecision::ReviewRequired => (
                    Lucide::Clock,
                    theme::muted(),
                    tr("Review en attente", "Review required"),
                ),
            };
            card = card.child(line(icon, color, text.into()));
        }
        card = card.child(if s.unresolved > 0 {
            line(
                Lucide::MessageSquare,
                theme::amber(),
                trf!(
                    "{} commentaire(s) à résoudre",
                    "{} unresolved conversation(s)",
                    s.unresolved
                ),
            )
        } else {
            line(
                Lucide::MessageSquare,
                theme::muted(),
                tr(
                    "Aucun commentaire à résoudre",
                    "No unresolved conversations",
                )
                .into(),
            )
        });
    } else if matches!(pr.state, PrState::Open | PrState::Draft) {
        card = card.child(div().text_color(theme::muted()).child(tr(
            "Statut CI indisponible (token GitHub requis)",
            "CI status unavailable (GitHub token needed)",
        )));
    }
    card.into_any_element()
}

/// Small clickable "#123" chip opening the PR in the browser.
pub fn pr_chip(
    id: impl Into<gpui_kit::ElementId>,
    lookup: Option<&PrLookup>,
) -> Option<AnyElement> {
    match lookup? {
        PrLookup::Found(pr) => {
            let url = pr.url.clone();
            let color = pr_color(pr.state);
            let status = pr.status.clone();
            let card = pr.clone();
            Some(
                h_flex()
                    .id(id)
                    .test_support()
                    .flex_none()
                    .gap_1()
                    .px_1p5()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::alpha(color, 0.5))
                    .bg(theme::alpha(color, 0.12))
                    .text_color(color)
                    .text_xs()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(color, 0.25)))
                    .child(Icon::new(Lucide::GitPullRequest).xsmall())
                    .child(format!("#{}", pr.number))
                    .when_some(status.as_ref().and_then(|s| s.checks), |s, checks| {
                        let (icon, color) = check_look(checks);
                        s.child(Icon::new(icon).xsmall().text_color(color))
                    })
                    .when_some(
                        status.as_ref().map(|s| s.unresolved).filter(|n| *n > 0),
                        |s, n| {
                            s.child(
                                h_flex()
                                    .gap_0p5()
                                    .text_color(theme::amber())
                                    .child(Icon::new(Lucide::MessageSquare).xsmall())
                                    .child(n.to_string()),
                            )
                        },
                    )
                    .tooltip(move |window, cx| {
                        let card = card.clone();
                        Tooltip::element(move |_, _| pr_card(&card)).build(window, cx)
                    })
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .into_any_element(),
            )
        }
        PrLookup::Loading => Some(
            div()
                .flex_none()
                .text_xs()
                .text_color(theme::muted())
                .child("PR…")
                .with_animation(
                    "pr-loading",
                    Animation::new(Duration::from_millis(900))
                        .repeat()
                        .with_easing(gpui_kit::pulsating_between(0.3, 1.)),
                    |el, t| el.opacity(t),
                )
                .into_any_element(),
        ),
        PrLookup::Failed(err) => {
            let err = err.clone();
            Some(
                div()
                    .id(id)
                    .flex_none()
                    .text_color(theme::muted())
                    .child(Icon::new(Lucide::GitPullRequest).xsmall())
                    .tooltip(move |window, cx| {
                        Tooltip::new(trf!("PR introuvable : {err}", "PR lookup failed: {err}"))
                            .build(window, cx)
                    })
                    .into_any_element(),
            )
        }
        PrLookup::None => None,
    }
}

impl RepoView {
    fn fetch_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let icon = Icon::new(Lucide::RefreshCw).small();
        let icon = if self.fetching {
            icon.with_animation(
                "fetch-spin",
                Animation::new(Duration::from_millis(900)).repeat(),
                |icon, t| icon.rotate(gpui_kit::radians(t * std::f32::consts::TAU)),
            )
            .into_any_element()
        } else {
            icon.into_any_element()
        };
        let tip = match &self.fetch_error {
            Some(e) => trf!("Dernier fetch en échec : {e}", "Last fetch failed: {e}"),
            None => tr(
                "Fetch, auto toutes les 10 s (⇧⌘F)",
                "Fetch, automatic every 10 s (⇧⌘F)",
            )
            .into(),
        };
        h_flex()
            .id("fetch")
            .gap_1p5()
            .h(px(26.))
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .when(self.fetching, |s| s.text_color(theme::accent()))
            .when(self.fetch_error.is_some(), |s| s.text_color(theme::amber()))
            .hover(|s| s.bg(theme::raised()))
            .child(icon)
            .child("Fetch")
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.fetch(cx)))
            .into_any_element()
    }

    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let data = self.data.clone();
        let head = data.refs.branches.iter().find(|b| b.is_head).cloned();
        let busy = self.busy.is_some();
        let changes = data.uncommitted();
        let mode = self.mode;
        let head_label = data.refs.head_branch.clone().unwrap_or_else(|| {
            data.refs
                .head
                .map(|h| trf!("détaché @ {}", "detached @ {}", h.to_hex_with_len(8)))
                .unwrap_or_default()
        });
        let head_pr = head.as_ref().and_then(|b| {
            pr_chip(
                "head-pr",
                self.pr_for(&b.name, b.upstream.as_deref(), false),
            )
        });

        // macOS-style segmented control.
        let tab = |id: &'static str, label: String, keys: &'static str, active: bool| {
            h_flex()
                .id(id)
                .h(px(24.))
                .px_3()
                .gap_1p5()
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
                .when(!active, |s| {
                    s.text_color(theme::muted())
                        .hover(|s| s.text_color(gpui_kit::white()))
                })
                .child(label)
                .tooltip(move |window, cx| Tooltip::new(keys).build(window, cx))
        };

        h_flex()
            .h(px(44.))
            .flex_none()
            .px_3()
            .gap_2()
            .child(
                Button::new("toggle-sidebar")
                    .ghost()
                    .small()
                    .icon(Icon::new(if self.sidebar_collapsed {
                        Lucide::PanelLeftOpen
                    } else {
                        Lucide::PanelLeftClose
                    }))
                    .tooltip(tr(
                        "Afficher / masquer la barre latérale (⌃⌘S)",
                        "Show / hide sidebar (⌃⌘S)",
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .child(
                        Icon::new(Lucide::GitBranch)
                            .small()
                            .text_color(theme::accent()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .truncate()
                            .child(head_label),
                    )
                    .when_some(head.as_ref().filter(|b| b.ahead > 0), |s, b| {
                        s.child(
                            div()
                                .text_xs()
                                .text_color(theme::lime())
                                .child(format!("↑{}", b.ahead)),
                        )
                    })
                    .when_some(head.as_ref().filter(|b| b.behind > 0), |s, b| {
                        s.child(
                            div()
                                .text_xs()
                                .text_color(theme::amber())
                                .child(format!("↓{}", b.behind)),
                        )
                    })
                    .children(head_pr),
            )
            .child(div().w(px(12.)))
            .child(
                h_flex()
                    .p(px(2.))
                    .gap_0p5()
                    .rounded_lg()
                    .bg(theme::alpha(gpui_kit::white(), 0.05))
                    .border_1()
                    .border_color(theme::hairline())
                    .child(
                        tab(
                            "mode-history",
                            tr("Historique", "History").into(),
                            "⌘2",
                            mode == Mode::History,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::History, cx))),
                    )
                    .child(
                        tab(
                            "mode-changes",
                            if changes > 0 {
                                trf!("Changements · {changes}", "Changes · {changes}")
                            } else {
                                tr("Changements", "Changes").into()
                            },
                            "⌘1",
                            mode == Mode::Changes,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Changes, cx))),
                    ),
            )
            .child(div().flex_1())
            .child(self.fetch_button(cx))
            .child(
                Button::new("pull")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::ArrowDownToLine))
                    .label("Pull")
                    .disabled(busy)
                    .tooltip(tr(
                        "Pull, fast-forward uniquement (⇧⌘L)",
                        "Pull, fast-forward only (⇧⌘L)",
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.pull(window, cx))),
            )
            .child(
                Button::new("push")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::ArrowUpFromLine))
                    .label("Push")
                    .tooltip("Push (⇧⌘P)")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_push_dialog(None, window, cx)),
                    ),
            )
            .child(
                Button::new("stash")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::Archive))
                    .label("Stash")
                    .disabled(busy || changes == 0)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_op("Stash", |r| r.stash_push(None), window, cx)
                    })),
            )
            .child(
                Button::new("pop")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::ArchiveRestore))
                    .label("Pop")
                    .disabled(busy || data.refs.stashes.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_op("Stash pop", |r| r.stash_pop(0), window, cx)
                    })),
            )
    }
}
