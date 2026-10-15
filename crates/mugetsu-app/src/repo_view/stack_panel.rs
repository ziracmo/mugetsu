//! Main view of a PR stack (a click on a stack in the PRs tab): how far it
//! is, which PRs are ready, and the next thing blocking the merge, as an
//! animated feed from the top of the stack down to the trunk.

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Context, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    div, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, relative,
};
use mugetsu_forge::{CheckState, PrState, PullRequest, ReviewDecision};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// Where a PR of the stack stands, from blocked to mergeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Draft,
    Failing,
    ChangesRequested,
    Running,
    Threads,
    Review,
    Ready,
}

pub fn readiness(pr: &PullRequest) -> Readiness {
    if pr.state == PrState::Draft {
        return Readiness::Draft;
    }
    let Some(s) = &pr.status else {
        return Readiness::Review;
    };
    match (s.checks, s.review) {
        (Some(CheckState::Failing), _) => Readiness::Failing,
        (_, Some(ReviewDecision::ChangesRequested)) => Readiness::ChangesRequested,
        (Some(CheckState::Pending), _) => Readiness::Running,
        _ if s.unresolved > 0 => Readiness::Threads,
        (_, Some(ReviewDecision::ReviewRequired)) => Readiness::Review,
        _ => Readiness::Ready,
    }
}

impl Readiness {
    fn look(self) -> (Lucide, Hsla) {
        match self {
            Readiness::Ready => (Lucide::CircleCheck, theme::lime()),
            Readiness::Draft => (Lucide::GitPullRequestDraft, theme::muted()),
            Readiness::Failing => (Lucide::CircleX, theme::red()),
            Readiness::ChangesRequested => (Lucide::MessageSquareWarning, theme::red()),
            Readiness::Running => (Lucide::LoaderCircle, theme::amber()),
            Readiness::Threads => (Lucide::MessagesSquare, theme::amber()),
            Readiness::Review => (Lucide::Eye, theme::violet()),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Readiness::Ready => tr("Prête", "Ready"),
            Readiness::Draft => tr("Brouillon", "Draft"),
            Readiness::Failing => tr("CI en échec", "CI failing"),
            Readiness::ChangesRequested => tr("Changements demandés", "Changes requested"),
            Readiness::Running => tr("CI en cours", "CI running"),
            Readiness::Threads => tr("Conversations ouvertes", "Open conversations"),
            Readiness::Review => tr("En attente de review", "Awaiting review"),
        }
    }
}

/// What is left to do on `pr`, in a sentence.
pub fn todo(pr: &PullRequest) -> String {
    let s = pr.status.clone().unwrap_or_default();
    match readiness(pr) {
        Readiness::Ready => tr("Prête à merger", "Ready to merge").into(),
        Readiness::Draft => tr("Passer en « Ready for review »", "Mark ready for review").into(),
        Readiness::Failing if !s.failing.is_empty() => {
            trf!("Réparer la CI : {}", "Fix CI: {}", s.failing.join(", "))
        }
        Readiness::Failing => tr("Réparer la CI", "Fix CI").into(),
        Readiness::ChangesRequested => tr(
            "Traiter les changements demandés",
            "Address the requested changes",
        )
        .into(),
        Readiness::Running => trf!(
            "Attendre la CI ({} en cours)",
            "Wait for CI ({} running)",
            s.pending
        ),
        Readiness::Threads => trf!(
            "Résoudre {} conversation(s)",
            "Resolve {} conversation(s)",
            s.unresolved
        ),
        Readiness::Review if pr.requested_reviewers.is_empty() => {
            tr("Obtenir une review", "Get a review").into()
        }
        Readiness::Review => trf!(
            "Review attendue de {}",
            "Review pending from {}",
            pr.requested_reviewers.join(", ")
        ),
    }
}

/// The first PR from the bottom that is not ready: it blocks every PR above.
pub fn next_blocker(stack: &[PullRequest]) -> Option<&PullRequest> {
    stack.iter().find(|p| readiness(p) != Readiness::Ready)
}

fn ink() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.93)
}
fn soft() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.45)
}

/// 0..1 of a staggered entrance: item `i` starts `i` steps late.
fn stagger(t: f32, total_ms: f32, i: usize, step_ms: f32, run_ms: f32) -> f32 {
    let local = (t * total_ms - i as f32 * step_ms) / run_ms;
    let x = local.clamp(0., 1.);
    1. - (1. - x).powi(3)
}

impl RepoView {
    /// Shows the stack holding the PR whose head is `head` in the main view.
    pub fn open_stack(&mut self, head: String, cx: &mut Context<Self>) {
        self.stack_focus = Some(head);
        self.celebrate_seq += 1;
        self.stack_seq = self.celebrate_seq;
        self.set_mode(super::Mode::Stack, cx);
    }

    fn focused_stack(&self) -> Option<Vec<PullRequest>> {
        let head = self.stack_focus.as_deref()?;
        mugetsu_forge::stacks(&self.pr_list)
            .into_iter()
            .find(|s| s.iter().any(|p| p.head == head))
    }

    pub(super) fn render_stack(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(stack) = self.focused_stack() else {
            return v_flex()
                .id("stack-view")
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_sm()
                .text_color(soft())
                .child(Icon::new(Lucide::Layers).large().text_color(soft()))
                .child(tr(
                    "Cette stack n'existe plus (PRs mergées ou fermées).",
                    "This stack is gone (PRs merged or closed).",
                ))
                .into_any_element();
        };
        let seq = self.stack_seq;
        let ready = stack
            .iter()
            .filter(|p| readiness(p) == Readiness::Ready)
            .count();
        let blocker = next_blocker(&stack).map(|p| p.number);
        v_flex()
            .id("stack-view")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(860.))
                    .mx_auto()
                    .px_6()
                    .pt_5()
                    .pb_8()
                    .gap_5()
                    .child(self.stack_header(&stack, ready, seq))
                    .child(next_step(&stack, seq))
                    .child(self.stack_feed(&stack, blocker, seq, cx)),
            )
            .into_any_element()
    }

    /// Title, a segmented progress bar (one segment per PR, bottom first)
    /// and how many PRs sit in each state.
    fn stack_header(&self, stack: &[PullRequest], ready: usize, seq: usize) -> AnyElement {
        let total = stack.len();
        let base = stack.first().map(|p| p.base.clone()).unwrap_or_default();
        let top = stack.last().map(|p| p.head.clone()).unwrap_or_default();
        let fraction = ready as f32 / total.max(1) as f32;
        let segments = stack.iter().enumerate().map(|(i, pr)| {
            let (_, color) = readiness(pr).look();
            let done = readiness(pr) == Readiness::Ready;
            div()
                .flex_1()
                .h(px(8.))
                .rounded_full()
                .bg(theme::alpha(gpui_kit::white(), 0.06))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(if done {
                            linear_gradient(
                                90.,
                                linear_color_stop(theme::alpha(color, 0.55), 0.),
                                linear_color_stop(color, 1.),
                            )
                        } else {
                            theme::alpha(color, 0.35).into()
                        })
                        .with_animation(
                            ("stack-seg", seq * 64 + i),
                            Animation::new(Duration::from_millis(500 + 90 * total as u64)),
                            move |el, t| {
                                let k = stagger(t, 500. + 90. * total as f32, i, 90., 500.);
                                el.w(relative(k))
                            },
                        ),
                )
        });
        let mut counts: Vec<(Readiness, usize)> = Vec::new();
        for pr in stack {
            let r = readiness(pr);
            match counts.iter_mut().find(|(k, _)| *k == r) {
                Some((_, n)) => *n += 1,
                None => counts.push((r, 1)),
            }
        }
        counts.sort_by_key(|(r, _)| std::cmp::Reverse(*r as u8));
        let pills = counts.into_iter().map(|(r, n)| {
            let (icon, color) = r.look();
            h_flex()
                .gap_1()
                .px_2()
                .h(px(22.))
                .rounded_full()
                .text_xs()
                .bg(theme::alpha(color, 0.12))
                .border_1()
                .border_color(theme::alpha(color, 0.3))
                .text_color(color)
                .child(Icon::new(icon).xsmall())
                .child(format!("{n} · {}", r.label()))
        });
        let all_ready = ready == total;
        v_flex()
            .id("stack-header")
            .w_full()
            .relative()
            .p_5()
            .gap_4()
            .rounded_xl()
            .border_1()
            .border_color(theme::hairline())
            .bg(linear_gradient(
                135.,
                linear_color_stop(theme::alpha(theme::violet(), 0.16), 0.),
                linear_color_stop(theme::alpha(gpui_kit::white(), 0.02), 1.),
            ))
            .overflow_hidden()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .size(px(40.))
                            .flex_none()
                            .rounded_lg()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme::alpha(theme::violet(), 0.2))
                            .child(Icon::new(Lucide::Layers).text_color(theme::violet())),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(ink())
                                    .truncate()
                                    .child(trf!("Stack de {total} PRs", "Stack of {total} PRs")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(soft())
                                    .truncate()
                                    .child(format!("{base} ← … ← {top}")),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_end()
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_2xl()
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .text_color(if all_ready {
                                                theme::lime()
                                            } else {
                                                ink()
                                            })
                                            .child(ready.to_string())
                                            .with_animation(
                                                ("stack-count", seq),
                                                Animation::new(Duration::from_millis(700))
                                                    .with_easing(gpui_kit::ease_out_quint()),
                                                |el, t| el.opacity(t),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(soft())
                                            .child(format!("/ {total}")),
                                    ),
                            )
                            .child(div().text_xs().text_color(soft()).child(trf!(
                                "{}% prêtes",
                                "{}% ready",
                                (fraction * 100.).round() as u32
                            ))),
                    ),
            )
            .child(h_flex().w_full().gap_1().children(segments))
            .child(h_flex().flex_wrap().gap_1p5().children(pills))
            .into_any_element()
    }

    /// The PRs, top of the stack first, threaded on a rail down to the trunk.
    fn stack_feed(
        &self,
        stack: &[PullRequest],
        blocker: Option<u64>,
        seq: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total = stack.len();
        let run = 420. + 80. * total as f32;
        let items = stack
            .iter()
            .enumerate()
            .rev()
            .enumerate()
            .map(|(row, (pos, pr))| {
                let r = readiness(pr);
                let (icon, color) = r.look();
                let is_blocker = blocker == Some(pr.number);
                let last = pos == 0;
                let url = pr.url.clone();
                let head = pr.head.clone();
                let view = cx.entity().downgrade();
                let author = pr.author.clone();
                let avatar = crate::avatars::github(&author, 18., cx);
                let node = div()
                    .size(px(28.))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(color, 0.16))
                    .border_1()
                    .border_color(theme::alpha(color, 0.55))
                    .child(Icon::new(icon).small().text_color(color));
                let node = if is_blocker {
                    node.with_animation(
                        ("stack-pulse", seq * 64 + pos),
                        Animation::new(Duration::from_millis(1600))
                            .repeat()
                            .with_easing(gpui_kit::bounce(gpui_kit::ease_in_out)),
                        move |el, t| {
                            el.shadow(vec![BoxShadow {
                                color: theme::alpha(color, 0.45 * (1. - t * 0.6)),
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(4. + 10. * t),
                                spread_radius: px(1. + 4. * t),
                                inset: false,
                            }])
                        },
                    )
                    .into_any_element()
                } else {
                    node.into_any_element()
                };
                let rail = v_flex()
                    .w(px(28.))
                    .flex_none()
                    .items_center()
                    .child(node)
                    .child(div().flex_1().w(px(2.)).my_1().rounded_full().bg(if last {
                        theme::alpha(gpui_kit::white(), 0.)
                    } else {
                        theme::alpha(gpui_kit::white(), 0.08)
                    }));
                let chip = |icon: Lucide, color: Hsla, text: String| {
                    h_flex()
                        .gap_1()
                        .px_1p5()
                        .h(px(20.))
                        .rounded_md()
                        .text_xs()
                        .bg(theme::alpha(color, 0.1))
                        .text_color(color)
                        .child(Icon::new(icon).xsmall())
                        .child(text)
                };
                let s = pr.status.clone().unwrap_or_default();
                let mut chips: Vec<AnyElement> =
                    vec![chip(icon, color, r.label().into()).into_any_element()];
                if s.checks.is_some() {
                    chips.push(
                        chip(
                            Lucide::CircleCheck,
                            theme::muted(),
                            trf!("{} ok", "{} passed", s.passed),
                        )
                        .into_any_element(),
                    );
                }
                if s.unresolved > 0 && r != Readiness::Threads {
                    chips.push(
                        chip(
                            Lucide::MessagesSquare,
                            theme::amber(),
                            s.unresolved.to_string(),
                        )
                        .into_any_element(),
                    );
                }
                let card = v_flex()
                    .id(SharedString::from(format!("stack-pr-{}", pr.number)))
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .mb_3()
                    .p_3()
                    .gap_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(if is_blocker {
                        theme::alpha(color, 0.5)
                    } else {
                        theme::hairline()
                    })
                    .bg(if r == Readiness::Ready {
                        theme::alpha(theme::lime(), 0.05)
                    } else {
                        theme::alpha(gpui_kit::white(), 0.028)
                    })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.05)))
                    .on_click(move |ev, _, cx| {
                        if ev.modifiers().platform || ev.modifiers().control {
                            cx.open_url(&url);
                        } else {
                            let _ = view.update(cx, |this, cx| this.show_pr_branch(&head, cx));
                        }
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .px_1p5()
                                    .rounded_md()
                                    .bg(theme::alpha(theme::violet(), 0.14))
                                    .text_color(theme::violet())
                                    .child(format!("{}/{total}", pos + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(ink())
                                    .child(pr.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(soft())
                                    .child(format!("#{}", pr.number)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(soft())
                            .child(avatar)
                            .child(author)
                            .child(Icon::new(Lucide::GitBranch).xsmall())
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(format!("{} → {}", pr.head, pr.base)),
                            ),
                    )
                    .child(h_flex().flex_wrap().gap_1().children(chips))
                    .when(r != Readiness::Ready, |c| {
                        c.child(
                            h_flex()
                                .gap_1p5()
                                .text_xs()
                                .text_color(if is_blocker { ink() } else { soft() })
                                .child(Icon::new(Lucide::ArrowRight).xsmall().text_color(color))
                                .child(todo(pr)),
                        )
                    });
                h_flex()
                    .w_full()
                    .items_stretch()
                    .gap_3()
                    .relative()
                    .child(rail)
                    .child(card)
                    .with_animation(
                        ("stack-row", seq * 64 + row),
                        Animation::new(Duration::from_millis(run as u64)),
                        move |el, t| {
                            let k = stagger(t, run, row, 80., 420.);
                            el.opacity(k).left(px(18. * (1. - k)))
                        },
                    )
            });
        let base = stack.first().map(|p| p.base.clone()).unwrap_or_default();
        v_flex()
            .w_full()
            .children(items)
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div().w(px(28.)).flex().justify_center().child(
                            Icon::new(Lucide::GitCommitHorizontal)
                                .small()
                                .text_color(soft()),
                        ),
                    )
                    .child(div().text_xs().text_color(soft()).child(base)),
            )
            .into_any_element()
    }
}

/// Callout naming the PR that blocks the stack and what it needs, or a
/// celebration when every PR is ready.
fn next_step(stack: &[PullRequest], seq: usize) -> AnyElement {
    let (icon, color, title, body) = match next_blocker(stack) {
        Some(pr) => (
            Lucide::Target,
            theme::accent(),
            trf!(
                "Prochaine étape · #{} {}",
                "Next step · #{} {}",
                pr.number,
                pr.title
            ),
            todo(pr),
        ),
        None => (
            Lucide::PartyPopper,
            theme::lime(),
            tr("Tout est prêt", "Everything is ready").to_string(),
            tr(
                "Mergez de bas en haut, en commençant par la PR basée sur le tronc.",
                "Merge from the bottom up, starting with the PR based on the trunk.",
            )
            .to_string(),
        ),
    };
    h_flex()
        .id("stack-next")
        .test_support()
        .w_full()
        .relative()
        .overflow_hidden()
        .gap_3()
        .p_3()
        .rounded_xl()
        .border_1()
        .border_color(theme::alpha(color, 0.45))
        .bg(theme::alpha(color, 0.08))
        .child(
            div()
                .size(px(32.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::alpha(color, 0.2))
                .child(Icon::new(icon).text_color(color)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(ink())
                        .truncate()
                        .child(title),
                )
                .child(div().text_xs().text_color(soft()).child(body)),
        )
        .child(super::shimmer::sweep(seq))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mugetsu_forge::PrStatus;

    fn pr(n: u64, state: PrState, status: Option<PrStatus>) -> PullRequest {
        PullRequest {
            number: n,
            title: format!("PR {n}"),
            url: String::new(),
            state,
            status,
            head: format!("h{n}"),
            base: "main".into(),
            same_repo: true,
            author: "me".into(),
            requested_reviewers: vec![],
            updated_at: String::new(),
            merged_at: None,
        }
    }

    fn status(checks: CheckState, review: Option<ReviewDecision>, unresolved: usize) -> PrStatus {
        PrStatus {
            checks: Some(checks),
            review,
            unresolved,
            ..Default::default()
        }
    }

    #[test]
    fn classifies_what_blocks_a_pr() {
        let ready = pr(
            1,
            PrState::Open,
            Some(status(
                CheckState::Passing,
                Some(ReviewDecision::Approved),
                0,
            )),
        );
        assert_eq!(readiness(&ready), Readiness::Ready);
        assert_eq!(readiness(&pr(2, PrState::Draft, None)), Readiness::Draft);
        let failing = pr(
            3,
            PrState::Open,
            Some(status(
                CheckState::Failing,
                Some(ReviewDecision::Approved),
                0,
            )),
        );
        assert_eq!(readiness(&failing), Readiness::Failing);
        let threads = pr(
            4,
            PrState::Open,
            Some(status(
                CheckState::Passing,
                Some(ReviewDecision::Approved),
                2,
            )),
        );
        assert_eq!(readiness(&threads), Readiness::Threads);
        assert_eq!(todo(&threads), "Résoudre 2 conversation(s)");
        let review = pr(
            5,
            PrState::Open,
            Some(status(
                CheckState::Passing,
                Some(ReviewDecision::ReviewRequired),
                0,
            )),
        );
        assert_eq!(readiness(&review), Readiness::Review);
    }

    #[test]
    fn the_lowest_unready_pr_blocks_the_stack() {
        let ok = || {
            Some(status(
                CheckState::Passing,
                Some(ReviewDecision::Approved),
                0,
            ))
        };
        let stack = vec![
            pr(1, PrState::Open, ok()),
            pr(2, PrState::Draft, None),
            pr(3, PrState::Open, None),
        ];
        assert_eq!(next_blocker(&stack).map(|p| p.number), Some(2));
        assert!(next_blocker(&stack[..1]).is_none());
    }
}
