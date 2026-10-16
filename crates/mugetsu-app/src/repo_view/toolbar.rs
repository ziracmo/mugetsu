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

pub fn check_look(checks: CheckState) -> (Lucide, gpui_kit::Hsla) {
    match checks {
        CheckState::Passing => (Lucide::CircleCheck, theme::lime()),
        CheckState::Failing => (Lucide::CircleX, theme::red()),
        CheckState::Pending => (Lucide::CircleDashed, theme::amber()),
    }
}

/// Hover card of a PR: state, title, branches, CI, reviews, conversations.
/// Fixed width so long names wrap instead of being cut.
pub fn pr_card(pr: &PullRequest, cx: &mut gpui_kit::App) -> AnyElement {
    let line = |icon: Lucide, color: gpui_kit::Hsla, text: String| {
        h_flex()
            .w_full()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .pt(px(2.))
                    .child(Icon::new(icon).xsmall().text_color(color)),
            )
            .child(div().flex_1().min_w_0().child(text))
    };
    let (state, state_icon) = match pr.state {
        PrState::Open => (tr("Ouverte", "Open"), Lucide::GitPullRequest),
        PrState::Draft => (tr("Brouillon", "Draft"), Lucide::GitPullRequestDraft),
        PrState::Merged => (tr("Mergée", "Merged"), Lucide::GitMerge),
        PrState::Closed => (tr("Fermée", "Closed"), Lucide::GitPullRequestClosed),
    };
    let color = pr_color(pr.state);
    let header = h_flex()
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .gap_1()
                .px_1p5()
                .py(px(1.))
                .rounded_md()
                .bg(theme::alpha(color, 0.18))
                .text_color(color)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(Icon::new(state_icon).xsmall())
                .child(state),
        )
        .child(
            div()
                .text_color(theme::muted())
                .child(format!("#{}", pr.number)),
        )
        .child(div().flex_1())
        .when(!pr.author.is_empty(), |s| {
            s.child(
                h_flex()
                    .gap_1p5()
                    .child(crate::avatars::pr_author(pr, 16., cx))
                    .child(div().text_color(theme::muted()).child(pr.author.clone())),
            )
        });
    let mut card = v_flex()
        .w(px(380.))
        .py_1()
        .gap_2()
        .text_xs()
        .text_color(gpui_kit::white())
        .child(header)
        .child(
            div()
                .w_full()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(pr.title.clone()),
        )
        .when(!pr.head.is_empty(), |s| {
            s.child(line(
                Lucide::GitBranch,
                theme::muted(),
                format!("{} → {}", pr.head, pr.base),
            ))
        });
    let mut details = v_flex()
        .w_full()
        .gap_1p5()
        .pt_2()
        .border_t_1()
        .border_color(theme::hairline());
    if let Some(s) = &pr.status {
        details = match s.checks {
            Some(checks) => {
                let (icon, color) = check_look(checks);
                let summary = match checks {
                    CheckState::Failing => trf!(
                        "CI en échec · {} échec(s), {} ok",
                        "CI failing · {} failed, {} passed",
                        s.failing.len(),
                        s.passed
                    ),
                    CheckState::Pending => trf!(
                        "CI en cours · {} en attente, {} ok",
                        "CI running · {} pending, {} passed",
                        s.pending,
                        s.passed
                    ),
                    CheckState::Passing => {
                        trf!("CI OK · {} checks", "CI passing · {} checks", s.passed)
                    }
                };
                details
                    .child(line(icon, color, summary))
                    .children(s.failing.iter().map(|name| {
                        div()
                            .pl_5()
                            .child(line(Lucide::X, theme::red(), name.clone()))
                    }))
            }
            None => details.child(line(
                Lucide::CircleDashed,
                theme::muted(),
                tr("Pas de CI", "No checks").into(),
            )),
        };
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
                    theme::amber(),
                    tr("Review en attente", "Review required"),
                ),
            };
            details = details.child(line(icon, color, text.into()));
        }
        if !pr.requested_reviewers.is_empty() {
            details = details.child(line(
                Lucide::Eye,
                theme::muted(),
                trf!(
                    "Reviewers attendus : {}",
                    "Awaiting: {}",
                    pr.requested_reviewers.join(", ")
                ),
            ));
        }
        details = details.child(if s.unresolved > 0 {
            line(
                Lucide::MessageSquare,
                theme::amber(),
                trf!(
                    "{} conversation(s) à résoudre",
                    "{} unresolved conversation(s)",
                    s.unresolved
                ),
            )
        } else {
            line(
                Lucide::MessageSquare,
                theme::muted(),
                tr("Aucune conversation ouverte", "No unresolved conversations").into(),
            )
        });
        card = card.child(details);
    } else if matches!(pr.state, PrState::Open | PrState::Draft) {
        card = card.child(
            details.child(line(
                Lucide::CircleDashed,
                theme::muted(),
                tr(
                    "Statut CI indisponible (token GitHub requis)",
                    "CI status unavailable (GitHub token needed)",
                )
                .into(),
            )),
        );
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
                    .tooltip_show_delay(super::pr_tip::DELAY)
                    .tooltip(move |window, cx| super::pr_tip::view(&card, window, cx))
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
    /// Phase of the Fetch / Pull / Push button named `label`.
    fn op_phase(&self, label: &'static str) -> OpPhase {
        let running = match label {
            "Fetch" => self.fetching,
            _ => self.busy == Some(label),
        };
        match self.op_flash {
            _ if running => OpPhase::Running,
            Some((l, ok, seq)) if l == label => OpPhase::Done { ok, seq },
            _ => OpPhase::Idle,
        }
    }

    fn fetch_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let tip = match &self.fetch_error {
            Some(e) => trf!("Dernier fetch en échec : {e}", "Last fetch failed: {e}"),
            None => tr(
                "Fetch, auto toutes les 10 s (⇧⌘F)",
                "Fetch, automatic every 10 s (⇧⌘F)",
            )
            .into(),
        };
        op_button(
            "fetch",
            Lucide::RefreshCw,
            "Fetch",
            Motion::Spin,
            self.op_phase("Fetch"),
            false,
            self.fetch_error.is_some(),
            tip,
            cx.listener(|this, _, _, cx| this.fetch_now(cx)),
        )
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
                .test_support()
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
                    )
                    .child(
                        tab("mode-stats", "Stats".into(), "⌘3", mode == Mode::Stats)
                            .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Stats, cx))),
                    )
                    .when(mode == Mode::Stack, |s| {
                        s.child(tab("mode-stack", "Stack".into(), "", true))
                    })
                    .when_some(
                        self.pr_page.as_ref().filter(|_| mode == Mode::Pr),
                        |s, page| s.child(tab("mode-pr", format!("PR #{}", page.number), "", true)),
                    )
                    .when_some(
                        self.file_page().filter(|_| mode == Mode::File),
                        |s, page| {
                            let name = page.path.rsplit('/').next().unwrap_or(&page.path);
                            s.child(tab("mode-file", name.to_string(), "", true))
                        },
                    ),
            )
            .when(mode == Mode::History, |s| {
                let compact = self.compact;
                s.child(
                    h_flex()
                        .id("toggle-compact")
                        .test_support()
                        .size(px(26.))
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .when(compact, |s| {
                            s.bg(theme::alpha(theme::accent(), 0.18))
                                .text_color(theme::accent())
                        })
                        .when(!compact, |s| s.text_color(theme::muted()))
                        .hover(|s| s.bg(theme::raised()))
                        .child(
                            Icon::new(if compact {
                                Lucide::Rows3
                            } else {
                                Lucide::ListCollapse
                            })
                            .small(),
                        )
                        .tooltip(move |window, cx| {
                            Tooltip::new(if compact {
                                tr("Tous les commits (⇧⌘K)", "All commits (⇧⌘K)")
                            } else {
                                tr(
                                    "Vue compacte : branches (⇧⌘K)",
                                    "Compact view: branches (⇧⌘K)",
                                )
                            })
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_compact(cx))),
                )
                .child({
                    let side = self.details_side;
                    h_flex()
                        .id("toggle-details-side")
                        .test_support()
                        .size(px(26.))
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .text_color(theme::muted())
                        .hover(|s| s.bg(theme::raised()))
                        .child(
                            Icon::new(if side {
                                Lucide::PanelBottom
                            } else {
                                Lucide::PanelRight
                            })
                            .small(),
                        )
                        .tooltip(move |window, cx| {
                            Tooltip::new(if side {
                                tr("Détails du commit en bas", "Commit details below")
                            } else {
                                tr("Détails du commit à droite", "Commit details on the right")
                            })
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_details_side(cx)))
                })
            })
            .child(div().flex_1())
            .child(self.fetch_button(cx))
            .child(op_button(
                "pull",
                Lucide::ArrowDownToLine,
                "Pull",
                Motion::Down,
                self.op_phase("Pull"),
                busy && self.busy != Some("Pull"),
                false,
                tr(
                    "Pull, fast-forward uniquement (⇧⌘L)",
                    "Pull, fast-forward only (⇧⌘L)",
                )
                .into(),
                cx.listener(|this, _, window, cx| this.pull(window, cx)),
            ))
            .child(op_button(
                "push",
                Lucide::ArrowUpFromLine,
                "Push",
                Motion::Up,
                self.op_phase("Push"),
                busy && self.busy != Some("Push"),
                false,
                "Push (⇧⌘P)".into(),
                cx.listener(|this, _, window, cx| this.open_push_dialog(None, window, cx)),
            ))
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpPhase {
    Idle,
    Running,
    /// Just finished; `seq` keys its one-shot animation.
    Done {
        ok: bool,
        seq: usize,
    },
}

/// How the icon moves while its operation runs.
#[derive(Clone, Copy)]
pub enum Motion {
    Spin,
    Down,
    Up,
}

/// Fetch / Pull / Push: a liquid accent wave runs through the button while
/// the operation works, then a lime ring and sparks on success, or a red
/// shake on failure.
#[allow(clippy::too_many_arguments)]
fn op_button(
    id: &'static str,
    icon: Lucide,
    label: &'static str,
    motion: Motion,
    phase: OpPhase,
    disabled: bool,
    warn: bool,
    tip: String,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let accent = theme::accent();
    let glyph = match phase {
        OpPhase::Done { ok: true, .. } => {
            Icon::new(Lucide::Check).small().text_color(theme::lime())
        }
        OpPhase::Done { ok: false, .. } => Icon::new(Lucide::X).small().text_color(theme::red()),
        _ => Icon::new(icon).small(),
    };
    let glyph: AnyElement = match (phase, motion) {
        (OpPhase::Running, Motion::Spin) => glyph
            .with_animation(
                (id, 0usize),
                Animation::new(Duration::from_millis(800))
                    .repeat()
                    .with_easing(gpui_kit::ease_in_out),
                |icon, t| icon.rotate(gpui_kit::radians(t * std::f32::consts::TAU)),
            )
            .into_any_element(),
        (OpPhase::Running, Motion::Down | Motion::Up) => {
            let dir = if matches!(motion, Motion::Down) {
                1.
            } else {
                -1.
            };
            div()
                .relative()
                .child(glyph)
                .with_animation(
                    (id, 1usize),
                    Animation::new(Duration::from_millis(700)).repeat(),
                    move |el, t| {
                        // Drops out one side, comes back in from the other.
                        let y = if t < 0.5 { t * 2. } else { t * 2. - 2. };
                        el.top(px(dir * 9. * y)).opacity(1. - y.abs())
                    },
                )
                .into_any_element()
        }
        (OpPhase::Done { seq, .. }, _) => div()
            .child(glyph)
            .with_animation(
                (id, seq * 4 + 2),
                Animation::new(Duration::from_millis(420)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t),
            )
            .into_any_element(),
        _ => glyph.into_any_element(),
    };
    let mut button = h_flex()
        .id(id)
        .test_support()
        .relative()
        .gap_1p5()
        .h(px(26.))
        .px_2()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .border_1()
        .border_color(theme::alpha(accent, 0.))
        .when(warn, |s| s.text_color(theme::amber()))
        .when(disabled, |s| s.opacity(0.45).cursor_default())
        .when(!disabled, |s| {
            s.hover(|s| s.bg(theme::raised())).on_click(on_click)
        })
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    match phase {
        OpPhase::Running => {
            // The liquid: a soft accent band rolling through, over a tinted base,
            // clipped by its own rounded box so the button keeps its size.
            let wave = div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(gpui_kit::relative(0.7))
                .bg(gpui_kit::linear_gradient(
                    90.,
                    gpui_kit::linear_color_stop(theme::alpha(accent, 0.), 0.),
                    gpui_kit::linear_color_stop(theme::alpha(accent, 0.38), 1.),
                ))
                .with_animation(
                    (id, 3usize),
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(gpui_kit::ease_in_out),
                    |el, t| el.left(gpui_kit::relative(-0.7 + 1.7 * t)),
                );
            button = button
                .text_color(gpui_kit::white())
                .bg(theme::alpha(accent, 0.14))
                .border_color(theme::alpha(accent, 0.5))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded_md()
                        .overflow_hidden()
                        .child(wave),
                );
        }
        OpPhase::Done { ok, seq } => {
            let color = if ok { theme::lime() } else { theme::red() };
            let ring = div()
                .absolute()
                .inset_0()
                .rounded_md()
                .border_1()
                .border_color(color)
                .with_animation(
                    (id, seq * 4),
                    Animation::new(Duration::from_millis(900))
                        .with_easing(gpui_kit::ease_out_quint()),
                    move |el, t| {
                        el.opacity(1. - t).shadow(vec![gpui_kit::BoxShadow {
                            color: theme::alpha(color, 0.55 * (1. - t)),
                            offset: gpui_kit::point(px(0.), px(0.)),
                            blur_radius: px(4. + 14. * t),
                            spread_radius: px(1. + 5. * t),
                            inset: false,
                        }])
                    },
                );
            button = button
                .text_color(color)
                .bg(theme::alpha(color, 0.1))
                .child(ring);
            if ok {
                button = button.child(super::particles::burst(seq));
            }
        }
        OpPhase::Idle => {}
    }
    let content = h_flex().gap_1p5().relative().child(glyph).child(label);
    let content: AnyElement = match phase {
        OpPhase::Done { ok: false, seq } => content
            .with_animation(
                (id, seq * 4 + 1),
                Animation::new(Duration::from_millis(480)),
                |el, t| el.left(px((t * 28.).sin() * 4. * (1. - t))),
            )
            .into_any_element(),
        _ => content.into_any_element(),
    };
    button.child(content).into_any_element()
}
