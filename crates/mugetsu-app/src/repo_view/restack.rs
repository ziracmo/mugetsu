//! Restack of a PR stack from the app: each branch rebased onto its parent,
//! bottom first, then all force-pushed at once (see `mugetsu_git::restack`).
//! Progress shows as a card in the stack view.

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div,
    linear_color_stop, linear_gradient, px, relative,
};
use mugetsu_forge::PullRequest;
use mugetsu_git::StackLink;

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Fetch,
    /// Rebasing branch `i` (bottom first).
    Rebase(usize),
    Push,
    /// Number of branches pushed.
    Done(usize),
    Failed(String),
}

/// A restack running, or just finished, on one stack.
#[derive(Clone, Debug)]
pub struct RestackRun {
    /// Head branches, bottom first.
    pub branches: Vec<String>,
    pub phase: Phase,
    pub seq: usize,
}

impl RestackRun {
    pub fn is_running(&self) -> bool {
        !matches!(self.phase, Phase::Done(_) | Phase::Failed(_))
    }

    /// Branches already rebased.
    fn rebased(&self) -> usize {
        match self.phase {
            Phase::Fetch => 0,
            Phase::Rebase(i) => i,
            Phase::Push | Phase::Done(_) => self.branches.len(),
            Phase::Failed(_) => 0,
        }
    }
}

/// The trunk and, bottom first, each branch with its parent in the stack.
pub fn links(stack: &[PullRequest]) -> (String, Vec<StackLink>) {
    let trunk = stack.first().map(|p| p.base.clone()).unwrap_or_default();
    let links = stack
        .iter()
        .map(|pr| StackLink {
            branch: pr.head.clone(),
            parent: stack
                .iter()
                .any(|p| p.head == pr.base)
                .then(|| pr.base.clone()),
        })
        .collect();
    (trunk, links)
}

impl RepoView {
    fn stack_of(&self, head: &str) -> Option<Vec<PullRequest>> {
        mugetsu_forge::stacks(&self.pr_list)
            .into_iter()
            .find(|s| s.iter().any(|p| p.head == head))
    }

    /// The restack shown for the stack holding `head`, if any.
    pub(super) fn restack_of(&self, stack: &[PullRequest]) -> Option<&RestackRun> {
        self.restack
            .as_ref()
            .filter(|r| stack.iter().any(|p| r.branches.contains(&p.head)))
    }

    /// Asks, then restacks the stack holding `head` (its force-push rewrites
    /// the PRs on GitHub).
    pub fn confirm_restack(&mut self, head: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stack) = self.stack_of(&head) else {
            return;
        };
        let (trunk, _) = links(&stack);
        let n = stack.len();
        self.confirm(
            trf!(
                "Restacker les {n} branches sur '{trunk}' ?",
                "Restack the {n} branches onto '{trunk}'?"
            ),
            tr(
                "Chaque branche est rebasée sur sa parente, de bas en haut, puis toutes sont \
                 force-pushées (avec lease). Au moindre conflit, rien ne bouge.",
                "Each branch is rebased onto its parent, bottom up, then all are force-pushed \
                 (with lease). On any conflict, nothing changes.",
            )
            .into(),
            tr("Restacker", "Restack"),
            move |this, window, cx| this.restack(head.clone(), window, cx),
            window,
            cx,
        );
    }

    /// Restacks the stack holding `head` and shows its progress in the stack view.
    pub fn restack(&mut self, head: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(stack) = self.stack_of(&head) else {
            return;
        };
        let Some(remote) = super::sidebar::default_remote(&self.data.refs) else {
            return;
        };
        let (trunk, links) = links(&stack);
        let branches: Vec<String> = links.iter().map(|l| l.branch.clone()).collect();
        self.open_stack(head, cx);
        self.busy = Some("Restack");
        self.celebrate_seq += 1;
        self.restack = Some(RestackRun {
            branches,
            phase: Phase::Fetch,
            seq: self.celebrate_seq,
        });
        let repo = self.repo.clone();
        let set = |this: &mut Self, phase: Phase, cx: &mut Context<Self>| {
            if let Some(run) = this.restack.as_mut() {
                run.phase = phase;
            }
            cx.notify();
        };
        self._op = cx.spawn_in(window, async move |this, cx| {
            let r = repo.clone();
            let prepared = cx
                .background_spawn(async move { r.restack_prepare(&remote, &trunk, links) })
                .await;
            let result = match prepared {
                Err(e) => Err(e),
                Ok(plan) => {
                    let mut failed = None;
                    for i in 0..plan.len() {
                        let _ = this.update(cx, |this, cx| set(this, Phase::Rebase(i), cx));
                        let (r, p) = (repo.clone(), plan.clone());
                        if let Err(e) = cx
                            .background_spawn(async move { r.restack_step(&p, i) })
                            .await
                        {
                            failed = Some(e);
                            break;
                        }
                    }
                    if failed.is_none() {
                        let _ = this.update(cx, |this, cx| set(this, Phase::Push, cx));
                    }
                    let r = repo.clone();
                    cx.background_spawn(async move {
                        let result = match failed {
                            Some(e) => Err(e),
                            None => r.restack_push(&plan),
                        };
                        if result.is_err()
                            && let Err(undo) = r.restack_undo(&plan)
                        {
                            return result.map_err(|e| e.context(format!("undo failed: {undo:#}")));
                        }
                        result
                    })
                    .await
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                let note = match &result {
                    Ok(0) => Notification::info(tr(
                        "Stack déjà à jour, rien à pousser.",
                        "Stack already up to date, nothing to push.",
                    )),
                    Ok(n) => Notification::success(trf!(
                        "Stack restackée : {n} branche(s) poussée(s).",
                        "Stack restacked: {n} branch(es) pushed."
                    )),
                    Err(e) => Notification::error(trf!("Restack : {e:#}", "Restack: {e:#}")),
                };
                window.push_notification(note, cx);
                let phase = match result {
                    Ok(n) => Phase::Done(n),
                    Err(e) => Phase::Failed(format!("{e:#}")),
                };
                set(this, phase, cx);
                this.reload(cx);
                // PR heads moved: refresh their state.
                this.prs = Default::default();
                this.refresh_prs(true, cx);
            });
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |this, cx| {
                if this.restack.as_ref().is_some_and(|r| !r.is_running()) {
                    this.restack = None;
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    /// "Restack" button of the stack header, or nothing while a restack runs.
    pub(super) fn restack_button(
        &self,
        stack: &[PullRequest],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::Disableable as _;
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        let head = stack.first().map(|p| p.head.clone()).unwrap_or_default();
        Button::new("stack-restack")
            .small()
            .ghost()
            .icon(Icon::new(Lucide::RefreshCw))
            .label(tr("Restacker", "Restack"))
            .disabled(self.busy.is_some() || self.data.refs.remotes.is_empty())
            .tooltip(tr(
                "Rebaser chaque branche sur sa parente et force-pusher la stack",
                "Rebase each branch onto its parent and force-push the stack",
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_restack(head.clone(), window, cx)
            }))
            .into_any_element()
    }

    /// Progress card: a bar, then one chip per branch, bottom first.
    pub(super) fn restack_card(&self, run: &RestackRun) -> AnyElement {
        let total = run.branches.len();
        let done = run.rebased();
        let (icon, color, title): (Lucide, Hsla, String) = match &run.phase {
            Phase::Fetch => (
                Lucide::RefreshCw,
                theme::accent(),
                tr(
                    "Restack · fetch du remote…",
                    "Restack · fetching the remote…",
                )
                .into(),
            ),
            Phase::Rebase(i) => (
                Lucide::RefreshCw,
                theme::accent(),
                trf!(
                    "Restack · rebase {}/{} · {}",
                    "Restack · rebasing {}/{} · {}",
                    i + 1,
                    total,
                    run.branches[*i]
                ),
            ),
            Phase::Push => (
                Lucide::ArrowUpFromLine,
                theme::violet(),
                tr(
                    "Restack · push (force-with-lease)…",
                    "Restack · pushing (force-with-lease)…",
                )
                .into(),
            ),
            Phase::Done(0) => (
                Lucide::CircleCheck,
                theme::lime(),
                tr("Stack déjà à jour", "Stack already up to date").into(),
            ),
            Phase::Done(n) => (
                Lucide::CircleCheck,
                theme::lime(),
                trf!(
                    "Stack restackée · {n} branche(s) poussée(s)",
                    "Stack restacked · {n} branch(es) pushed"
                ),
            ),
            Phase::Failed(_) => (
                Lucide::CircleX,
                theme::red(),
                tr(
                    "Restack annulé, rien n'a bougé",
                    "Restack undone, nothing changed",
                )
                .into(),
            ),
        };
        let running = run.is_running();
        let seq = run.seq;
        let glyph = Icon::new(icon).small().text_color(color);
        let glyph: AnyElement = if running {
            glyph
                .with_animation(
                    ("restack-spin", seq),
                    Animation::new(Duration::from_millis(900))
                        .repeat()
                        .with_easing(gpui_kit::ease_in_out),
                    |icon, t| icon.rotate(gpui_kit::radians(t * std::f32::consts::TAU)),
                )
                .into_any_element()
        } else {
            div()
                .child(glyph)
                .with_animation(
                    ("restack-done", seq),
                    Animation::new(Duration::from_millis(500))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, t| el.opacity(t).top(px(-4. * (1. - t))),
                )
                .into_any_element()
        };
        let fraction = match run.phase {
            Phase::Push => 0.92,
            Phase::Done(_) => 1.,
            Phase::Failed(_) => 1.,
            _ => (done as f32 + 0.5) / (total.max(1) as f32 + 1.),
        };
        let bar = div()
            .w_full()
            .h(px(4.))
            .rounded_full()
            .bg(theme::alpha(gpui_kit::white(), 0.06))
            .overflow_hidden()
            .relative()
            .child(
                div()
                    .h_full()
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(theme::alpha(color, 0.45), 0.),
                        linear_color_stop(color, 1.),
                    )),
            )
            .when_running(running, seq);
        let chips = run.branches.iter().enumerate().map(|(i, b)| {
            let (glyph, c) = match &run.phase {
                Phase::Failed(_) => (Lucide::Undo2, theme::muted()),
                Phase::Rebase(j) if *j == i => (Lucide::LoaderCircle, theme::accent()),
                Phase::Fetch => (Lucide::Circle, theme::muted()),
                _ if i < done => (Lucide::Check, theme::lime()),
                _ => (Lucide::Circle, theme::muted()),
            };
            let chip = h_flex()
                .gap_1()
                .px_1p5()
                .h(px(20.))
                .rounded_md()
                .text_xs()
                .bg(theme::alpha(c, 0.1))
                .border_1()
                .border_color(theme::alpha(c, 0.3))
                .text_color(if c == theme::muted() {
                    theme::muted()
                } else {
                    theme::alpha(gpui_kit::white(), 0.9)
                })
                .child(Icon::new(glyph).xsmall().text_color(c))
                .child(b.clone());
            let chip: AnyElement = if i < done {
                // Lands with a small pop when its rebase completes.
                chip.with_animation(
                    ("restack-chip", seq * 64 + i * 8 + done),
                    Animation::new(Duration::from_millis(380))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, t| el.opacity(0.4 + 0.6 * t).top(px(3. * (1. - t))),
                )
                .into_any_element()
            } else {
                chip.into_any_element()
            };
            h_flex()
                .gap_1()
                .when_arrow(i > 0)
                .child(chip)
                .into_any_element()
        });
        let detail = match &run.phase {
            Phase::Failed(e) => Some(e.clone()),
            _ => None,
        };
        v_flex()
            .id("stack-restack")
            .test_support()
            .w_full()
            .relative()
            .overflow_hidden()
            .gap_2()
            .p_3()
            .rounded_xl()
            .border_1()
            .border_color(theme::alpha(color, 0.45))
            .bg(theme::alpha(color, 0.08))
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme::alpha(gpui_kit::white(), 0.93))
                    .child(glyph)
                    .child(div().flex_1().min_w_0().truncate().child(title)),
            )
            .child(bar)
            .child(h_flex().flex_wrap().gap_1().children(chips))
            .children(detail.map(|e| {
                div()
                    .text_xs()
                    .text_color(theme::alpha(gpui_kit::white(), 0.55))
                    .child(e)
            }))
            .with_animation(
                ("restack-in", seq),
                Animation::new(Duration::from_millis(320)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t).top(px(-6. * (1. - t))),
            )
            .into_any_element()
    }
}

/// Small helpers keeping the card's builder chain readable.
trait CardExt: gpui_kit::ParentElement + Sized {
    /// A light sweep over the bar while the restack runs.
    fn when_running(self, running: bool, seq: usize) -> Self {
        if running {
            self.child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .w(relative(0.25))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(theme::alpha(gpui_kit::white(), 0.), 0.),
                        linear_color_stop(theme::alpha(gpui_kit::white(), 0.35), 1.),
                    ))
                    .with_animation(
                        ("restack-sweep", seq),
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(gpui_kit::ease_in_out),
                        |el, t| el.left(relative(-0.25 + 1.25 * t)),
                    ),
            )
        } else {
            self
        }
    }

    /// A chevron before every chip but the first: the stack reads bottom → top.
    fn when_arrow(self, arrow: bool) -> Self {
        if arrow {
            self.child(
                Icon::new(Lucide::ChevronRight)
                    .xsmall()
                    .text_color(theme::muted()),
            )
        } else {
            self
        }
    }
}

impl<T: gpui_kit::ParentElement + Sized> CardExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use mugetsu_forge::PrState;

    fn pr(n: u64, head: &str, base: &str) -> PullRequest {
        PullRequest {
            number: n,
            title: String::new(),
            url: String::new(),
            state: PrState::Open,
            status: None,
            head: head.into(),
            base: base.into(),
            same_repo: true,
            author: String::new(),
            requested_reviewers: vec![],
            updated_at: String::new(),
            merged_at: None,
        }
    }

    #[test]
    fn links_follow_the_pr_bases() {
        let stack = vec![pr(1, "a", "main"), pr(2, "b", "a"), pr(3, "c", "b")];
        let (trunk, links) = links(&stack);
        assert_eq!(trunk, "main");
        let parents: Vec<Option<&str>> = links.iter().map(|l| l.parent.as_deref()).collect();
        assert_eq!(parents, [None, Some("a"), Some("b")]);
    }
}
